//! The setup's CONVERT DISC IMAGE on desktop and Android: the in-app converter (`ok_br2_converter`) writing into the
//! install's `.part` folder (engine addition, feature `converter`).
//!
//! `is_disc` tells a disc image from a folder; `inspect` opens it (`.cue` with its `.bin` beside it, a raw `.bin`,
//! `.chd` or `.iso` by contents; on Android a picked document's file descriptor) and reads its volume, so a wrong
//! disc or an unusable image is said before anything is written; an ISO gets the warning that it has no CD-XA audio.
//! `convert_into` runs the converter on the job's thread into `game_data.part/` (its steps on a few threads),
//! the cancel flag checked between steps; then the full check (every file the converter's summary lists read back
//! and its CRC-32 compared), the quick check and the swap, as a copy does (`job::copy_into`).
//!
//! The imported game's disc (`inspect_br1`, `convert_br1_into`): a `.cue` with its CD audio track's `.bin` beside it
//! (`open_cue_all`; without it no CD audio), a `.chd` (its audio track too) or `.iso`, converted by
//! `imports::br1` (`ok_br1_converter`) on the job's thread (one step at a time) into the imported game's folder's
//! `.part` (`Dirs::sub(BR1_DIR)`), then checked and swapped in the same way.

use super::imports::br1;
use super::job::{Failure, Hooks};
use super::plan::{Kind, br1_disc_error, br1_disc_found, disc_error, disc_found};
use super::screen::{Found, Phase, Progress};
use crate::assets::manifest;
use crate::base::hash::Crc32;
use crate::platform::game_data::Dirs;
use ok_br2_converter::{Converter, Disc, Options, ReadAt, SectorReader, Sink};
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};

/// The largest `.cue` sheet read.
const MAX_CUE: u64 = 64 << 10;

/// A disc image rather than a folder: `.cue`, `.bin`, `.chd`, `.iso`; on Android a picked document (not a tree).
pub fn is_disc(p: &Path) -> bool {
    let s = p.to_string_lossy();
    if s.starts_with("content://") {
        return !s.contains("/tree/");
    }
    let ext = p.extension().and_then(|e| e.to_str()).map(str::to_ascii_lowercase).unwrap_or_default();
    ["cue", "bin", "chd", "iso"].contains(&ext.as_str())
}

/// A picked `.cue` on Android: the system lets the game open only the chosen document, not the `.bin` files the sheet
/// names, so it is refused with what to choose instead.
#[cfg_attr(not(target_os = "android"), allow(dead_code))]
fn cue_on_android(uri: &str) -> Result<(), String> {
    if uri.to_ascii_lowercase().ends_with(".cue") {
        return Err(
            "Android lets the game open only the file you choose, not the .bin files a .cue names: choose the (Track 1) .bin, or a .chd or .iso image.".into(),
        );
    }
    Ok(())
}

/// A file (or an Android file descriptor) as the converter's random-access input.
pub(super) struct FileAt {
    f: std::fs::File,
    len: u64,
}

impl ReadAt for FileAt {
    fn read_at(&mut self, offset: u64, buf: &mut [u8]) -> std::io::Result<()> {
        self.f.seek(SeekFrom::Start(offset))?;
        self.f.read_exact(buf)
    }
    fn len(&self) -> u64 {
        self.len
    }
}

impl FileAt {
    pub(super) fn new(mut f: std::fs::File) -> std::io::Result<FileAt> {
        // a pipe or a stream from a provider cannot be read at random places (seeking fails)
        let len = f.seek(SeekFrom::End(0))?;
        f.seek(SeekFrom::Start(0))?;
        Ok(FileAt { f, len })
    }
}

/// The disc in the image `p` (module header).
fn open_disc(p: &Path) -> Result<Disc, String> {
    let s = p.to_string_lossy();
    let src = if s.starts_with("content://") {
        #[cfg(target_os = "android")]
        {
            cue_on_android(&s)?;
            let f = crate::platform::android::open_uri(&s)?;
            let at = FileAt::new(f).map_err(|_| {
                "This file cannot be read at random places (a cloud or streaming provider?): copy it to the device first, then choose that copy.".to_string()
            })?;
            ok_br2_converter::open_read_at(Box::new(at)).map_err(disc_error)?
        }
        #[cfg(not(target_os = "android"))]
        return Err(format!("{s}: not a file"));
    } else if p.extension().is_some_and(|e| e.eq_ignore_ascii_case("cue")) {
        let mut text = String::new();
        std::fs::File::open(p).and_then(|f| f.take(MAX_CUE).read_to_string(&mut text)).map_err(|e| format!("{}: {e}", p.display()))?;
        let dir = p.parent().map(Path::to_path_buf).unwrap_or_default();
        let mut open = |name: &str| -> std::io::Result<Box<dyn ReadAt>> { Ok(Box::new(FileAt::new(std::fs::File::open(dir.join(name))?)?)) };
        ok_br2_converter::open_cue(&text, &mut open).map_err(disc_error)?
    } else {
        let f = std::fs::File::open(p).map_err(|e| format!("{}: {e}", p.display()))?;
        ok_br2_converter::open_image(f).map_err(disc_error)?
    };
    Disc::open(src).map_err(disc_error)
}

/// The imported game's disc in the image `p` (module header).
fn open_br1(p: &Path) -> Result<br1::Disc, String> {
    let s = p.to_string_lossy();
    let src: Box<dyn SectorReader> = if s.starts_with("content://") {
        #[cfg(target_os = "android")]
        {
            cue_on_android(&s)?;
            let f = crate::platform::android::open_uri(&s)?;
            let at = FileAt::new(f).map_err(|_| {
                "This file cannot be read at random places (a cloud or streaming provider?): copy it to the device first, then choose that copy.".to_string()
            })?;
            ok_br2_converter::open_read_at(Box::new(at)).map_err(disc_error)?
        }
        #[cfg(not(target_os = "android"))]
        return Err(format!("{s}: not a file"));
    } else if p.extension().is_some_and(|e| e.eq_ignore_ascii_case("cue")) {
        let mut text = String::new();
        std::fs::File::open(p).and_then(|f| f.take(MAX_CUE).read_to_string(&mut text)).map_err(|e| format!("{}: {e}", p.display()))?;
        let dir = p.parent().map(Path::to_path_buf).unwrap_or_default();
        let mut open = |name: &str| -> std::io::Result<Box<dyn ReadAt>> { Ok(Box::new(FileAt::new(std::fs::File::open(dir.join(name))?)?)) };
        let img = ok_br2_converter::open_cue_all(&text, &mut open).map_err(disc_error)?;
        for m in &img.missing {
            log::warn!("setup: {m} (named by the .cue) cannot be opened: no CD audio from it");
        }
        Box::new(img)
    } else {
        let f = std::fs::File::open(p).map_err(|e| format!("{}: {e}", p.display()))?;
        ok_br2_converter::open_image(f).map_err(disc_error)?
    };
    br1::open(src).map_err(br1_disc_error)
}

/// Checks an imported disc image (module header).
pub fn inspect_br1(p: &Path) -> Result<Found, String> {
    let disc = open_br1(p)?;
    let (what, warn, bytes) = br1_disc_found(disc.source(), !disc.audio_tracks().is_empty());
    #[cfg(target_os = "android")]
    if let Some(free) = crate::platform::android::free_space()
        && bytes + (64 << 20) > free
    {
        return Err(format!("Not enough free space: the Bloody Roar 1 data needs about {} MB, {} MB are free.", (bytes >> 20) + 64, free >> 20));
    }
    Ok(Found { path: p.to_path_buf(), what: what.into(), bytes, warn })
}

/// Converts the imported image `src` into `dirs.part` (`Dirs::sub(BR1_DIR)`), checks it and installs it (module header).
pub fn convert_br1_into(src: &Path, dirs: &Dirs, progress: &Mutex<Progress>, cancel: &AtomicBool, hooks: &Hooks) -> Result<(), Failure> {
    let set = |phase: Phase, done: u64, total: u64, file: &str| {
        if let Ok(mut p) = progress.lock() {
            *p = Progress { phase, done, total, file: file.to_string() };
        }
    };
    set(Phase::Reading, 0, 0, &src.display().to_string());
    let disc = open_br1(src).map_err(Failure::Io)?;
    let _held = match dirs.lock().map_err(|e| Failure::Io(format!("game_data.lock: {e}")))? {
        Some(f) => f,
        None => return Err(Failure::Io("another copy of the game is installing its data: try again when it is done".into())),
    };
    dirs.fresh_part().map_err(|e| Failure::Io(format!("br1.part: {e}")))?;
    let r = (|| {
        let t = std::time::Instant::now();
        let mut sink = PartSink { root: dirs.part.clone(), full: false };
        let run = br1::Converter::new(disc).run(&mut sink, &mut |p| {
            set(Phase::Converting, p.done as u64, p.total as u64, &p.step);
            if hooks.slow_ms > 0 {
                std::thread::sleep(std::time::Duration::from_millis(hooks.slow_ms));
            }
            !cancel.load(Ordering::Relaxed)
        });
        let summary = match run {
            Ok(s) => s,
            Err(br1::Error::Cancelled) => return Err(Failure::Cancelled),
            Err(br1::Error::Sink(e)) if sink.full => return Err(Failure::NoSpace(e)),
            Err(e) => return Err(Failure::Io(e.to_string())),
        };
        log::info!(
            "setup: the imported game converted ({}, xa {}, cdda {}) in {:.1} s: {} files",
            summary.source,
            summary.xa,
            summary.cdda,
            t.elapsed().as_secs_f64(),
            summary.files.len()
        );
        if let Some(f) = &hooks.before_verify {
            f(&dirs.part);
        }
        verify(dirs, &summary.files, progress, cancel, hooks)?;
        super::job::check_part(dirs, Kind::Br1)?;
        let total: u64 = summary.files.values().map(|f| f.0).sum();
        set(Phase::Installing, total, total, "");
        dirs.install().map_err(|e| Failure::Io(format!("br1: {e}")))
    })();
    if r.is_err() {
        dirs.discard_part();
    }
    r
}

/// The full check of a conversion's `.part`: every file of `files` (path → size, CRC-32) read back and compared.
pub(super) fn verify(
    dirs: &Dirs,
    files: &std::collections::BTreeMap<String, (u64, u32)>,
    progress: &Mutex<Progress>,
    cancel: &AtomicBool,
    hooks: &Hooks,
) -> Result<(), Failure> {
    let total: u64 = files.values().map(|f| f.0).sum();
    let (mut back, mut buf) = (0u64, vec![0u8; 1 << 20]);
    for (path, &(size, crc)) in files {
        if cancel.load(Ordering::Relaxed) {
            return Err(Failure::Cancelled);
        }
        if let Ok(mut p) = progress.lock() {
            *p = Progress { phase: Phase::Verifying, done: back, total, file: path.clone() };
        }
        if hooks.slow_ms > 0 {
            std::thread::sleep(std::time::Duration::from_millis(hooks.slow_ms));
        }
        let mut f = std::fs::File::open(dirs.part.join(path)).map_err(|e| Failure::Io(format!("{path}: {e}")))?;
        let (mut c, mut n) = (Crc32::default(), 0u64);
        loop {
            let k = f.read(&mut buf).map_err(|e| Failure::Io(format!("{path}: {e}")))?;
            if k == 0 {
                break;
            }
            c.update(&buf[..k]);
            n += k as u64;
        }
        if n != size || c.value() != crc {
            return Err(Failure::Verify(path.clone()));
        }
        back += n;
    }
    Ok(())
}

/// Checks a disc image (module header): what the confirmation shows, or why it cannot be converted.
pub fn inspect(p: &Path) -> Result<Found, String> {
    let (what, warn, bytes) = disc_found(open_disc(p)?.source());
    #[cfg(target_os = "android")]
    if let Some(free) = crate::platform::android::free_space()
        && bytes + (64 << 20) > free
    {
        return Err(format!("Not enough free space: the game data needs about {} MB, {} MB are free.", (bytes >> 20) + 64, free >> 20));
    }
    Ok(Found { path: p.to_path_buf(), what: what.into(), bytes, warn })
}

/// The converter's files written into the `.part` folder.
pub(super) struct PartSink {
    pub(super) root: PathBuf,
    /// the disk filled up (the write's error)
    pub(super) full: bool,
}

impl Sink for PartSink {
    fn put(&mut self, path: &str, data: &[u8]) -> Result<(), String> {
        if !manifest::safe_path(path) {
            return Err(format!("{path}: not a path inside the data folder"));
        }
        let to = self.root.join(path);
        let r = to.parent().map_or(Ok(()), std::fs::create_dir_all).and_then(|_| std::fs::write(&to, data));
        r.map_err(|e| {
            self.full |= e.kind() == std::io::ErrorKind::StorageFull;
            format!("{path}: {e}")
        })
    }
}

impl br1::Sink for PartSink {
    fn put(&mut self, path: &str, data: &[u8]) -> Result<(), String> {
        Sink::put(self, path, data)
    }
}

impl super::imports::q3::Sink for PartSink {
    fn put(&mut self, path: &str, data: &[u8]) -> Result<(), String> {
        Sink::put(self, path, data)
    }
}

/// The steps run on this many threads (the conversion's memory grows with them).
fn threads() -> usize {
    std::thread::available_parallelism().map_or(1, |n| n.get()).clamp(1, 4)
}

/// Converts the image `src` into `dirs.part`, checks it and installs it (module header); `hooks` as for a copy (a
/// pause per step and per checked file, the `.part` folder handed over before the full check).
pub fn convert_into(src: &Path, dirs: &Dirs, progress: &Mutex<Progress>, cancel: &AtomicBool, hooks: &Hooks) -> Result<(), Failure> {
    let set = |phase: Phase, done: u64, total: u64, file: &str| {
        if let Ok(mut p) = progress.lock() {
            *p = Progress { phase, done, total, file: file.to_string() };
        }
    };
    let slow = || {
        if hooks.slow_ms > 0 {
            std::thread::sleep(std::time::Duration::from_millis(hooks.slow_ms));
        }
    };
    set(Phase::Reading, 0, 0, &src.display().to_string());
    let disc = open_disc(src).map_err(Failure::Io)?;
    let _held = match dirs.lock().map_err(|e| Failure::Io(format!("game_data.lock: {e}")))? {
        Some(f) => f,
        None => return Err(Failure::Io("another copy of the game is installing its data: try again when it is done".into())),
    };
    dirs.fresh_part().map_err(|e| Failure::Io(format!("game_data.part: {e}")))?;
    let r = (|| {
        let t = std::time::Instant::now();
        let mut sink = PartSink { root: dirs.part.clone(), full: false };
        let run = Converter::new(disc, Options { viewer: false, threads: threads() }).run(&mut sink, &mut |p| {
            set(Phase::Converting, p.done as u64, p.total as u64, &p.step);
            slow();
            !cancel.load(Ordering::Relaxed)
        });
        let summary = match run {
            Ok(s) => s,
            Err(ok_br2_converter::Error::Cancelled) => return Err(Failure::Cancelled),
            Err(ok_br2_converter::Error::Sink(e)) if sink.full => return Err(Failure::NoSpace(e)),
            Err(e) => return Err(Failure::Io(e.to_string())),
        };
        log::info!("setup: converted ({}, xa {}) in {:.1} s: {} files", summary.source, summary.xa, t.elapsed().as_secs_f64(), summary.files.len());
        if let Some(f) = &hooks.before_verify {
            f(&dirs.part);
        }
        // the full check: every file read back
        let total: u64 = summary.files.values().map(|f| f.0).sum();
        verify(dirs, &summary.files, progress, cancel, hooks)?;
        let v = manifest::check_dir(&dirs.part);
        if !v.usable() {
            return Err(Failure::Io(v.reason().unwrap_or_default()));
        }
        set(Phase::Installing, total, total, "");
        dirs.install().map_err(|e| Failure::Io(format!("game_data: {e}")))
    })();
    if r.is_err() {
        dirs.discard_part();
    }
    r
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The project's disc image beside the repo (skipped without it): converted into a temp data folder, installed,
    /// the quick check passes; a cancelled conversion, and one whose output is damaged before the full check, leave
    /// neither a `.part` nor a data folder.
    #[test]
    #[ignore = "converts the whole disc twice (about a minute): cargo test --release -- --ignored convert_the_disc"]
    fn convert_the_disc() {
        let bin = crate::base::env::var("DISC").unwrap_or_else(|_| concat!(env!("CARGO_MANIFEST_DIR"), "/../Bloody Roar II (USA).cue").into());
        let bin = PathBuf::from(bin);
        if !bin.exists() {
            eprintln!("skipped: no disc image");
            return;
        }
        let base = std::env::temp_dir().join(format!("okb_convert_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let dirs = Dirs::at(&base);
        let f = inspect(&bin).unwrap();
        assert!(f.warn.is_none(), "{f:?}");
        let (p, c) = (Mutex::new(Progress::default()), AtomicBool::new(true));
        assert_eq!(convert_into(&bin, &dirs, &p, &c, &Hooks::default()), Err(Failure::Cancelled));
        assert!(!dirs.part.exists() && !dirs.root.exists());
        c.store(false, Ordering::Relaxed);
        let hooks = Hooks {
            slow_ms: 0,
            before_verify: Some(Box::new(|part: &Path| {
                let f = part.join("engine/ALI.ACT");
                let mut b = std::fs::read(&f).unwrap();
                b[0] ^= 1;
                std::fs::write(&f, b).unwrap();
            })),
        };
        assert_eq!(convert_into(&bin, &dirs, &p, &c, &hooks), Err(Failure::Verify("engine/ALI.ACT".into())));
        assert!(!dirs.part.exists() && !dirs.root.exists());
        // over an older install that holds the program file: the new data has none (a fresh folder replaces the old one)
        std::fs::create_dir_all(dirs.root.join("engine")).unwrap();
        for f in ["engine/exe.bin", "engine/tables.bin"] {
            std::fs::write(dirs.root.join(f), f).unwrap();
        }
        convert_into(&bin, &dirs, &p, &c, &Hooks::default()).unwrap();
        assert_eq!(manifest::check_dir(&dirs.root), manifest::Verdict::Ok);
        assert!(!dirs.root.join("engine/exe.bin").exists() && !dirs.root.join("engine/tables.bin").exists());
        std::fs::remove_dir_all(&base).unwrap();
    }

    /// The imported game's disc (BR1_DISC=the .cue, default the second disc's `.cue` under `~/Downloads/Games/PS1/`; skipped
    /// without it) converted into a temp data folder's `br1/`: complete, with its music and CD audio.
    #[test]
    #[ignore = "converts the Bloody Roar 1 disc (about 10 s): cargo test --release -- --ignored convert_the_br1_disc"]
    fn convert_the_br1_disc() {
        let home = std::env::var("HOME").unwrap_or_default();
        let cue = std::env::var("BR1_DISC").unwrap_or_else(|_| format!("{home}/Downloads/Games/PS1/Bloody Roar (USA)/Bloody Roar (USA).cue"));
        let cue = PathBuf::from(cue);
        if !cue.exists() {
            eprintln!("skipped: no Bloody Roar 1 disc image");
            return;
        }
        let base = std::env::temp_dir().join(format!("okb_convert_br1_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let dirs = Dirs::at(&base).sub(crate::platform::game_data::BR1_DIR);
        std::fs::create_dir_all(dirs.root.parent().unwrap()).unwrap();
        let f = inspect_br1(&cue).unwrap();
        assert!(f.warn.is_none() && f.what.contains("BIN/CUE"), "{f:?}");
        // The main game's disc is refused here, and this one there
        let br2 = PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../Bloody Roar II (USA).cue"));
        if br2.exists() {
            assert!(inspect_br1(&br2).unwrap_err().contains("Bloody Roar II disc"));
        }
        assert!(inspect(&cue).unwrap_err().contains("Bloody Roar 1 disc"));
        let (p, c) = (Mutex::new(Progress::default()), AtomicBool::new(false));
        // over an older install that holds the program file: the new one has none
        std::fs::create_dir_all(dirs.root.join("engine")).unwrap();
        std::fs::write(dirs.root.join("engine/SCUS_941.99"), "exe").unwrap();
        convert_br1_into(&cue, &dirs, &p, &c, &Hooks::default()).unwrap();
        assert_eq!(manifest::check_dir_br1(&dirs.root), manifest::Br1Verdict::Ok);
        assert!(!dirs.root.join("engine/SCUS_941.99").exists());
        assert!(dirs.root.join("audio/cdda/track02.pcm").exists() && !dirs.part.exists());
        let pcm = std::fs::read(dirs.root.join("audio/cdda/track02.pcm")).unwrap();
        let m = crate::formats::pcm::decode(&pcm).unwrap();
        assert_eq!((m.rate, m.pcm.len()), (44100, (pcm.len() - crate::formats::pcm::HEADER) / 2));
        std::fs::remove_dir_all(&base).unwrap();
    }

    #[test]
    fn a_cue_document_on_android_is_refused_by_name() {
        assert!(cue_on_android("content://x/document/primary%3ADownload%2FBloody%20Roar%20(USA).cue").unwrap_err().contains("Track 1"));
        assert!(cue_on_android("content://x/document/primary%3ADownload%2Fa.BIN").is_ok());
    }

    #[test]
    fn images_refused_with_a_reason() {
        let base = std::env::temp_dir().join(format!("okb_convert_bad_{}", std::process::id()));
        std::fs::create_dir_all(&base).unwrap();
        std::fs::write(base.join("x.iso"), vec![0u8; 40 << 10]).unwrap();
        assert!(inspect(&base.join("x.iso")).unwrap_err().contains("Not a usable disc image"));
        std::fs::write(base.join("x.cue"), "FILE \"x.bin\" BINARY\n  TRACK 01 MODE1/2048\n    INDEX 01 00:00:00\n").unwrap();
        std::fs::write(base.join("x.bin"), vec![0u8; 4096]).unwrap();
        assert!(inspect(&base.join("x.cue")).is_err());
        assert!(is_disc(Path::new("/a/B.CHD")) && !is_disc(Path::new("/a/br2_export")));
        // another PlayStation game's ISO: the wrong disc, named
        let mut iso = vec![0u8; 20 * 2048];
        let pvd = &mut iso[16 * 2048..17 * 2048];
        pvd[0] = 1;
        pvd[1..6].copy_from_slice(b"CD001");
        pvd[8..40].copy_from_slice(&[b' '; 32]);
        pvd[8..19].copy_from_slice(b"PLAYSTATION");
        pvd[40..72].copy_from_slice(&[b' '; 32]);
        pvd[40..50].copy_from_slice(b"SLUS_00001");
        std::fs::write(base.join("other.iso"), iso).unwrap();
        let e = inspect(&base.join("other.iso")).unwrap_err();
        assert!(e.contains("not Bloody Roar II") && e.contains("SLUS_00001"), "{e}");
        std::fs::remove_dir_all(&base).unwrap();
    }
}
