//! The setup's jobs: checking a source, and copying converted game data into the data folder (engine addition).
//!
//! `inspect` checks what was picked (a folder of converted data, or a disc image the build cannot convert yet) and
//! sizes it. `Job::copy` runs `copy_into` on a thread: the files the engine reads (`plan`: the converter's manifest's
//! `files`) into `game_data.part/`, each one's CRC-32 taken on the way (and compared with the manifest's); then the
//! full check (every file read back from the `.part` and its CRC-32 compared), the quick check, and the swap
//! (`platform::game_data::Dirs::install`). A cancel or a failure leaves the old data as it was and no `.part`. The imported game's disc's data (`Kind::Br1`) goes the same way into its own folder (`Dirs::sub(BR1_DIR)`): a
//! folder with its manifest.json (or a `br1` folder holding one) copied by its manifest's list, checked with
//! `quick_check_br1`; so do Quake III Arena's maps (`Kind::Q3`, engine addition) into `Dirs::sub(Q3_DIR)`: Quake III
//! Arena's `.pk3` files are converted (`q3.rs`, when `q3::paks_in` finds them), a folder this game converted before
//! (its manifest.json of Quake III Arena, or a `q3` folder holding one) copied, checked with `extras::check_dir_q3`.

use super::plan::{self, Kind};
pub use super::plan::{Failure, Hooks, Item};
use super::screen::{Found, Phase, Progress};

/// Disc images (converted in-app with the `converter` feature, stage 3).
const DISC_EXT: [&str; 4] = ["cue", "bin", "chd", "iso"];
use crate::assets::extras::{self, Q3Verdict};
use crate::assets::manifest::{self, Br1Verdict, FileEntry, Record, Verdict};
use crate::base::hash::Crc32;
use crate::platform::game_data::Dirs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

/// Where the files come from: a folder on disk, or (Android) a document tree the player picked through the system's
/// folder picker, listed once (`platform::android::list_tree`), each file opened as a file descriptor.
pub enum Source {
    Dir(PathBuf),
    #[cfg(target_os = "android")]
    Tree {
        uri: String,
        files: std::collections::BTreeMap<String, (String, u64)>,
    },
}

impl Source {
    /// The source a pick names: a folder (a `manifest.json` stands for its folder); on Android a picked tree's
    /// `content://` URI, its top or an `okb_export` (or older `br2_export`) folder in it. The imported game's data (`kind`): the folder, or a `br1`
    /// folder in it, whose manifest.json is of the second disc; Quake III Arena's maps: the folder, or a `q3` folder
    /// in it, whose manifest.json is of Quake III Arena.
    pub fn of(p: &Path, kind: Kind) -> Result<Source, String> {
        // the manifest of the optional install `kind` (the main game's: any)
        let ours = |m: &[u8]| match kind {
            Kind::Br2 => true,
            Kind::Br1 => manifest::is_br1_manifest(m),
            Kind::Q3 => extras::is_q3_manifest(m),
        };
        #[cfg(target_os = "android")]
        if let Some(uri) = p.to_str().filter(|s| s.starts_with("content://")) {
            let all = crate::platform::android::list_tree(uri)?;
            let prefixes: &[&str] = match kind {
                Kind::Br1 => &["br1/", ""],
                Kind::Q3 => &["q3/", ""],
                Kind::Br2 => &["", "okb_export/", "br2_export/"],
            };
            for prefix in prefixes {
                // Quake III Arena's maps have no `engine/`
                let engine = kind == Kind::Q3 || all.iter().any(|(r, ..)| r.starts_with(&format!("{prefix}engine/")));
                if all.iter().any(|(r, ..)| *r == format!("{prefix}manifest.json")) && engine {
                    let files = all.iter().filter_map(|(r, doc, n)| Some((r.strip_prefix(prefix)?.to_string(), (doc.clone(), *n)))).collect();
                    let src = Source::Tree { uri: uri.to_string(), files };
                    if !src.read("manifest.json").is_some_and(|m| ours(&m)) {
                        continue;
                    }
                    return Ok(src);
                }
            }
            return Err(match kind {
                Kind::Br1 => "No converted Bloody Roar 1 data in that folder (it needs its manifest.json and engine/, or a br1 folder with them).".into(),
                Kind::Q3 => "No .pk3 files and no converted Quake III Arena maps in that folder.".into(),
                Kind::Br2 => "No converted game data in that folder (it needs manifest.json and engine/, or an okb_export folder with them).".into(),
            });
        }
        let p = if p.file_name().is_some_and(|n| n == "manifest.json") { p.parent().unwrap_or(p) } else { p };
        if let Some(dir) = kind.dir() {
            let sub = p.join(dir);
            if manifest::read_json(&sub.join("manifest.json")).ok().flatten().is_some_and(|m| ours(&m)) {
                return Ok(Source::Dir(sub));
            }
        }
        Ok(Source::Dir(p.to_path_buf()))
    }

    fn open(&self, rel: &str) -> std::io::Result<Box<dyn Read + Send>> {
        match self {
            Source::Dir(d) => Ok(Box::new(std::fs::File::open(d.join(rel))?)),
            #[cfg(target_os = "android")]
            Source::Tree { uri, files } => {
                let (doc, _) = files.get(rel).ok_or(std::io::ErrorKind::NotFound)?;
                Ok(Box::new(crate::platform::android::open_document(uri, doc).map_err(std::io::Error::other)?))
            }
        }
    }

    fn size(&self, rel: &str) -> Option<u64> {
        match self {
            Source::Dir(d) => std::fs::metadata(d.join(rel)).ok().filter(|m| m.is_file()).map(|m| m.len()),
            #[cfg(target_os = "android")]
            Source::Tree { files, .. } => files.get(rel).map(|f| f.1),
        }
    }

    /// A small file's bytes (the manifest); None when missing or too large.
    fn read(&self, rel: &str) -> Option<Vec<u8>> {
        let mut b = Vec::new();
        self.open(rel).ok()?.take(manifest::MAX_JSON as u64 + 1).read_to_end(&mut b).ok()?;
        (b.len() <= manifest::MAX_JSON).then_some(b)
    }

    /// The quick check of the source (`assets::manifest::quick_check`).
    fn check(&self) -> Verdict {
        manifest::quick_check(self.read("manifest.json").as_deref(), &|p| self.size(p))
    }

    /// The imported quick check of the source (a manifest of other data: none there).
    fn check_br1(&self) -> Br1Verdict {
        let m = self.read("manifest.json").filter(|m| manifest::is_br1_manifest(m));
        manifest::quick_check_br1(m.as_deref(), &|p| self.size(p))
    }

    /// The Quake III quick check of the source (a manifest of other data: none there) and its manifest's bytes.
    fn check_q3(&self) -> (Q3Verdict, Option<Vec<u8>>) {
        let m = self.read("manifest.json");
        let v = extras::quick_check_q3(m.as_deref().filter(|m| extras::is_q3_manifest(m)), &|p| self.size(p));
        (v, m)
    }

    /// The source's top on disk (None: a document tree).
    fn dir(&self) -> Option<&Path> {
        match self {
            Source::Dir(d) => Some(d),
            #[cfg(target_os = "android")]
            Source::Tree { .. } => None,
        }
    }
}

/// What a copy takes from `src`: the files (manifest.json last) and the main game's record (None: the optional
/// installs, whose manifest is their record).
pub fn plan(src: &Source, kind: Kind) -> Result<(Vec<Item>, Option<Record>), String> {
    let text = src.read("manifest.json").ok_or("manifest.json cannot be read")?;
    match kind {
        Kind::Br2 => plan::items(&text).map(|(items, rec)| (items, Some(rec))),
        Kind::Br1 => Ok((plan::items_extra(&text, manifest::required_br1())?, None)),
        Kind::Q3 => Ok((plan::items_q3(&text)?, None)),
    }
}

/// Checks what was picked: a usable folder of converted game data (`Found`), or why not. `data` is the data folder
/// itself, which cannot be its own source. `kind`: the main game's data or an optional install's.
pub fn inspect(p: &Path, data: &Path, kind: Kind) -> Result<Found, String> {
    // Quake III Arena's paks: the converter's check
    #[cfg(feature = "converter")]
    if kind == Kind::Q3
        && let Ok(paks) = super::q3::paks_in(p)
    {
        return super::q3::inspect_q3(p, paks);
    }
    // a disc image: the converter's check
    #[cfg(feature = "converter")]
    if kind != Kind::Q3 && super::convert::is_disc(p) && !p.is_dir() {
        return match kind {
            Kind::Br1 => super::convert::inspect_br1(p),
            _ => super::convert::inspect(p),
        };
    }
    let src = Source::of(p, kind)?;
    if let Some(d) = src.dir() {
        let meta = std::fs::metadata(d).map_err(|e| format!("{}: {e}", d.display()))?;
        if meta.is_file() {
            let ext = d.extension().and_then(|e| e.to_str()).map(str::to_ascii_lowercase).unwrap_or_default();
            return Err(if DISC_EXT.contains(&ext.as_str()) {
                "This build has no converter (feature `converter`): choose a folder of converted game data.".into()
            } else {
                format!("Not a folder of converted game data: {}", d.display())
            });
        }
        if let (Ok(a), Ok(b)) = (d.canonicalize(), data.canonicalize())
            && a.starts_with(&b)
        {
            return Err("That is the game data folder itself: choose the folder to copy from.".into());
        }
    }
    let name = p.display().to_string();
    let (items, what, warn) = if kind == Kind::Br1 {
        let v = src.check_br1();
        let rec = src.read("manifest.json").and_then(|m| Record::parse(&m)).and_then(Result::ok);
        let (what, warn) = plan::describe_br1(&v, rec.as_ref(), &name)?;
        (plan(&src, kind)?.0, what, warn)
    } else if kind == Kind::Q3 {
        let (v, m) = src.check_q3();
        let (what, warn) = plan::describe_q3(&v, m.as_deref(), &name)?;
        (plan(&src, kind)?.0, what, warn)
    } else {
        let v = src.check();
        if !v.usable() {
            return Err(plan::describe(&v, None, &name).err().unwrap_or_default());
        }
        let (items, rec) = plan(&src, kind)?;
        let (what, warn) = plan::describe(&v, rec.as_ref(), &name)?;
        (items, what, warn)
    };
    let bytes: u64 = items.iter().map(|i| i.size).sum();
    // Android: the free space of the app's storage (desktop: a full disk fails the copy as it happens)
    #[cfg(target_os = "android")]
    if let Some(free) = crate::platform::android::free_space()
        && bytes + (64 << 20) > free
    {
        return Err(format!("Not enough free space: the game data needs {} MB, {} MB are free.", (bytes >> 20) + 64, free >> 20));
    }
    Ok(Found { path: p.to_path_buf(), what, bytes, warn })
}

fn io(path: &str, e: std::io::Error) -> Failure {
    if e.kind() == std::io::ErrorKind::StorageFull { Failure::NoSpace(path.to_string()) } else { Failure::Io(format!("{path}: {e}")) }
}

const CHUNK: usize = 1 << 20;

/// Copies `src` into `dirs.part`, checks it and installs it (module header).
pub fn copy_into(src: &Source, dirs: &Dirs, progress: &Mutex<Progress>, cancel: &AtomicBool, hooks: &Hooks, kind: Kind) -> Result<(), Failure> {
    let set = |phase: Phase, done: u64, total: u64, file: &str| {
        if let Ok(mut p) = progress.lock() {
            *p = Progress { phase, done, total, file: file.to_string() };
        }
    };
    set(Phase::Reading, 0, 0, "manifest.json");
    let (items, _) = plan(src, kind).map_err(Failure::Io)?;
    let total: u64 = items.iter().map(|i| i.size).sum();
    // the install lock, held to the end (another instance's start leaves this `.part` alone)
    let _held = match dirs.lock().map_err(|e| io("game_data.lock", e))? {
        Some(f) => f,
        None => return Err(Failure::Io("another copy of the game is installing its data: try again when it is done".into())),
    };
    dirs.fresh_part().map_err(|e| io("game_data.part", e))?;
    let r = (|| {
        let mut done = 0u64;
        let mut entries = Vec::new();
        let mut buf = vec![0u8; CHUNK];
        for it in &items {
            set(Phase::Copying, done, total, &it.path);
            let dst = dirs.part.join(&it.path);
            if let Some(d) = dst.parent() {
                std::fs::create_dir_all(d).map_err(|e| io(&it.path, e))?;
            }
            let mut from = src.open(&it.path).map_err(|e| io(&it.path, e))?;
            let mut to = std::fs::File::create(&dst).map_err(|e| io(&it.path, e))?;
            let (mut crc, mut n) = (Crc32::default(), 0u64);
            loop {
                if cancel.load(Ordering::Relaxed) {
                    return Err(Failure::Cancelled);
                }
                let k = from.read(&mut buf).map_err(|e| io(&it.path, e))?;
                if k == 0 {
                    break;
                }
                n += k as u64;
                // more than the manifest says: stopped here, not at the end of what the source streams
                if n > it.size {
                    return Err(Failure::Io(format!("{}: more than the {} bytes the manifest says", it.path, it.size)));
                }
                crc.update(&buf[..k]);
                to.write_all(&buf[..k]).map_err(|e| io(&it.path, e))?;
                set(Phase::Copying, done + n, total, &it.path);
            }
            to.sync_all().map_err(|e| io(&it.path, e))?;
            if n != it.size {
                return Err(Failure::Io(format!("{}: {n} bytes, the manifest says {}", it.path, it.size)));
            }
            let crc = crc.value();
            if it.crc32.is_some_and(|c| c != crc) {
                return Err(Failure::Io(format!("{} in the source folder is damaged (its checksum differs from the manifest's)", it.path)));
            }
            done += n;
            entries.push(FileEntry { path: it.path.clone(), size: n, crc32: crc });
            if hooks.slow_ms > 0 {
                std::thread::sleep(std::time::Duration::from_millis(hooks.slow_ms));
            }
        }
        if let Some(f) = &hooks.before_verify {
            f(&dirs.part);
        }
        // the full check: every file read back
        let mut back = 0u64;
        for e in &entries {
            set(Phase::Verifying, back, total, &e.path);
            if cancel.load(Ordering::Relaxed) {
                return Err(Failure::Cancelled);
            }
            let mut f = std::fs::File::open(dirs.part.join(&e.path)).map_err(|x| io(&e.path, x))?;
            let (mut crc, mut n) = (Crc32::default(), 0u64);
            loop {
                let k = f.read(&mut buf).map_err(|x| io(&e.path, x))?;
                if k == 0 {
                    break;
                }
                crc.update(&buf[..k]);
                n += k as u64;
            }
            if n != e.size || crc.value() != e.crc32 {
                return Err(Failure::Verify(e.path.clone()));
            }
            back += n;
        }
        check_part(dirs, kind)?;
        set(Phase::Installing, total, total, "");
        dirs.install().map_err(|e| io("game_data", e))
    })();
    if r.is_err() {
        dirs.discard_part();
    }
    r
}

/// The quick check of a written `.part` before the swap (the main game's or an optional install's).
pub fn check_part(dirs: &Dirs, kind: Kind) -> Result<(), Failure> {
    let why = match kind {
        Kind::Br2 => manifest::check_dir(&dirs.part).reason(),
        Kind::Br1 => match manifest::check_dir_br1(&dirs.part) {
            v if v.usable() => None,
            v => Some(v.note()),
        },
        Kind::Q3 => match extras::check_dir_q3(&dirs.part).0 {
            v if v.usable() => None,
            v => Some(v.note()),
        },
    };
    why.map_or(Ok(()), |e| Err(Failure::Io(e)))
}

/// A copy on its own thread: its progress, its cancel flag, its end.
pub struct Job {
    pub progress: Arc<Mutex<Progress>>,
    cancel: Arc<AtomicBool>,
    handle: Option<std::thread::JoinHandle<Result<(), Failure>>>,
    /// the thread could not be started (reported once by `poll`)
    failed: Option<Failure>,
}

impl Job {
    /// A copy of the folder `src`, or (with the converter) the conversion of the disc image (Quake III Arena's paks)
    /// `src`, into `dirs` (`kind`: the main game's data or an optional install's).
    pub fn copy(src: PathBuf, dirs: Dirs, hooks: Hooks, kind: Kind) -> Job {
        let progress = Arc::new(Mutex::new(Progress::default()));
        let cancel = Arc::new(AtomicBool::new(false));
        let (p, c) = (progress.clone(), cancel.clone());
        let run = move || {
            #[cfg(feature = "converter")]
            if kind == Kind::Q3 && super::q3::paks_in(&src).is_ok() {
                return super::q3::convert_q3_into(&src, &dirs, &p, &c, &hooks);
            }
            #[cfg(feature = "converter")]
            if kind != Kind::Q3 && super::convert::is_disc(&src) && !src.is_dir() {
                return match kind {
                    Kind::Br1 => super::convert::convert_br1_into(&src, &dirs, &p, &c, &hooks),
                    _ => super::convert::convert_into(&src, &dirs, &p, &c, &hooks),
                };
            }
            copy_into(&Source::of(&src, kind).map_err(Failure::Io)?, &dirs, &p, &c, &hooks, kind)
        };
        let (handle, failed) = match std::thread::Builder::new().name("okb setup copy".into()).spawn(run) {
            Ok(h) => (Some(h), None),
            Err(e) => (None, Some(Failure::Io(format!("the copy could not start: {e}")))),
        };
        Job { progress, cancel, handle, failed }
    }

    pub fn cancel(&self) {
        self.cancel.store(true, Ordering::Relaxed);
    }

    /// Cancelled and waited for (the app quits: the thread removes its `.part` first).
    pub fn stop(mut self) {
        self.cancel();
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
    }

    /// The progress now.
    pub fn now(&self) -> Progress {
        self.progress.lock().map(|p| p.clone()).unwrap_or_default()
    }

    /// The job's end once it has ended (taken once).
    pub fn poll(&mut self) -> Option<Result<(), Failure>> {
        if let Some(f) = self.failed.take() {
            return Some(Err(f));
        }
        if !self.handle.as_ref()?.is_finished() {
            return None;
        }
        let h = self.handle.take()?;
        Some(h.join().unwrap_or_else(|_| Err(Failure::Io("the copy stopped unexpectedly".into()))))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("okb_setup_job_{name}_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    /// A tiny old export: the required set (a few bytes each), a music file and files a copy leaves out.
    fn legacy_export(root: &Path) {
        for p in manifest::required() {
            let f = root.join(&p);
            std::fs::create_dir_all(f.parent().unwrap()).unwrap();
            std::fs::write(&f, if p == "manifest.json" { b"{\"game\":\"t\"}".to_vec() } else { p.as_bytes().to_vec() }).unwrap();
        }
        for (p, b) in [("audio/music/BR2_MC01_ch0.mp3", "mp3"), ("images/x.png", "png"), ("video/BR.mp4", "mp4")] {
            std::fs::create_dir_all(root.join(p).parent().unwrap()).unwrap();
            std::fs::write(root.join(p), b).unwrap();
        }
    }

    /// The converter's record of `legacy_export`'s required files (sizes, CRCs).
    fn converted_record(root: &Path) -> Record {
        let mut files = Vec::new();
        for p in manifest::required().into_iter().filter(|p| p != "manifest.json") {
            let b = std::fs::read(root.join(&p)).unwrap();
            files.push(FileEntry { path: p, size: b.len() as u64, crc32: crate::base::hash::crc32(&b) });
        }
        Record { format: Some(manifest::FORMAT_MAX), source: "iso".into(), xa: false, content_id: Some(1), files }
    }

    #[test]
    fn a_legacy_export_is_refused_and_a_corrupt_copy_too() {
        let base = temp("copy");
        let src = base.join("export");
        legacy_export(&src);
        let dirs = Dirs::at(&base.join("cfg"));
        let (p, c) = (Mutex::new(Progress::default()), AtomicBool::new(false));
        // an old export: refused at the inspection, and by a copy before it writes anything
        assert!(inspect(&src, &dirs.root, Kind::Br2).unwrap_err().contains("made by the old converter"));
        let r = copy_into(&Source::Dir(src.clone()), &dirs, &p, &c, &Hooks::default(), Kind::Br2);
        assert!(matches!(&r, Err(Failure::Io(e)) if e.contains("made by the old converter")), "{r:?}");
        assert!(!dirs.part.exists() && !dirs.root.exists());
        // converted data: one byte changed in the .part before the full check fails the verification, nothing installed
        std::fs::write(src.join("manifest.json"), converted_record(&src).to_json()).unwrap();
        let hooks = Hooks {
            slow_ms: 0,
            before_verify: Some(Box::new(|part: &Path| {
                let f = part.join("engine/INIT.DAT");
                let mut b = std::fs::read(&f).unwrap();
                b[0] ^= 1;
                std::fs::write(&f, b).unwrap();
            })),
        };
        assert_eq!(copy_into(&Source::Dir(src.clone()), &dirs, &p, &c, &hooks, Kind::Br2), Err(Failure::Verify("engine/INIT.DAT".into())));
        assert!(!dirs.part.exists() && !dirs.root.exists());
        // a cancelled copy leaves nothing either
        c.store(true, Ordering::Relaxed);
        assert_eq!(copy_into(&Source::Dir(src.clone()), &dirs, &p, &c, &Hooks::default(), Kind::Br2), Err(Failure::Cancelled));
        assert!(!dirs.part.exists() && !dirs.root.exists());
        std::fs::remove_dir_all(&base).unwrap();
    }

    #[test]
    fn converted_data_is_checked_against_its_manifest() {
        let base = temp("conv");
        let src = base.join("conv");
        legacy_export(&src);
        let rec = converted_record(&src);
        let dirs = Dirs::at(&base.join("cfg"));
        // a list without one of the required files is refused before any copy
        let mut short = rec.clone();
        short.files.pop();
        std::fs::write(src.join("manifest.json"), short.to_json()).unwrap();
        assert!(inspect(&src, &dirs.root, Kind::Br2).unwrap_err().contains("does not list"));
        std::fs::write(src.join("manifest.json"), rec.to_json()).unwrap();
        let f = inspect(&src, &dirs.root, Kind::Br2).unwrap();
        assert!(f.what.contains("ISO") && f.warn.is_some());
        let (p, c) = (Mutex::new(Progress::default()), AtomicBool::new(false));
        // a source file that does not match its manifest's checksum (same size)
        std::fs::write(src.join("engine/tables.okt"), "engine/tables.okx").unwrap();
        assert!(matches!(copy_into(&Source::Dir(src.clone()), &dirs, &p, &c, &Hooks::default(), Kind::Br2), Err(Failure::Io(e)) if e.contains("damaged")));
        std::fs::write(src.join("engine/tables.okt"), "engine/tables.okt").unwrap();
        copy_into(&Source::Dir(src.clone()), &dirs, &p, &c, &Hooks::default(), Kind::Br2).unwrap();
        assert_eq!(manifest::check_dir(&dirs.root), Verdict::Ok);
        // only the listed files (and the manifest) were copied
        assert!(!dirs.root.join("audio/music/BR2_MC01_ch0.mp3").exists());
        std::fs::remove_dir_all(&base).unwrap();
    }

    /// A tiny install of the imported game in `root`: its required files and its manifest (sizes, CRCs).
    fn br1_data(root: &Path) {
        let mut files = Vec::new();
        for p in manifest::required_br1().into_iter().filter(|p| p != "manifest.json") {
            let f = root.join(&p);
            std::fs::create_dir_all(f.parent().unwrap()).unwrap();
            std::fs::write(&f, p.as_bytes()).unwrap();
            files.push(serde_json::json!({"path": p, "size": p.len(), "crc32": crate::base::hash::crc32(p.as_bytes())}));
        }
        let m = serde_json::json!({"game": manifest::BR1_GAME, "format": 4, "source": "bin", "xa": true, "cdda": true, "content_id": 1, "files": files});
        std::fs::write(root.join("manifest.json"), m.to_string()).unwrap();
    }

    #[test]
    fn copy_br1_data_into_its_own_folder() {
        let base = temp("br1");
        // a whole data folder (the main game's export at its top, the imported game's data in br1/)
        let src = base.join("game_data_copy");
        legacy_export(&src);
        br1_data(&src.join("br1"));
        let dirs = Dirs::at(&base.join("cfg"));
        let b = dirs.sub(manifest::BR1_DIR);
        let f = inspect(&src, &dirs.root, Kind::Br1).unwrap();
        assert!(f.what.contains("Bloody Roar 1") && f.warn.is_none(), "{f:?}");
        let (p, c) = (Mutex::new(Progress::default()), AtomicBool::new(false));
        copy_into(&Source::of(&src, Kind::Br1).unwrap(), &b, &p, &c, &Hooks::default(), Kind::Br1).unwrap();
        assert_eq!(manifest::check_dir_br1(&b.root), Br1Verdict::Ok);
        assert!(!b.part.exists() && !dirs.root.join("manifest.json").exists());
        // The main game's data is not the imported game's, nor the other way round
        assert!(inspect(&base.join("game_data_copy/engine/.."), &dirs.root, Kind::Br2).unwrap_err().contains("old converter"));
        std::fs::remove_dir_all(src.join("br1")).unwrap();
        assert!(inspect(&src, &dirs.root, Kind::Br1).unwrap_err().contains("No converted Bloody Roar 1 data"));
        assert!(inspect(&b.root, &dirs.root, Kind::Br1).unwrap_err().contains("game data folder itself"));
        // a damaged source file is caught on the way
        br1_data(&base.join("other"));
        std::fs::write(base.join("other/engine/tables.okt"), "engine/tables.okX").unwrap();
        let r = copy_into(&Source::of(&base.join("other"), Kind::Br1).unwrap(), &b, &p, &c, &Hooks::default(), Kind::Br1);
        assert!(matches!(r, Err(Failure::Io(e)) if e.contains("damaged")));
        assert_eq!(manifest::check_dir_br1(&b.root), Br1Verdict::Ok, "the installed one stays");
        std::fs::remove_dir_all(&base).unwrap();
    }

    /// (Engine addition) Quake III Arena's maps this game converted: copied from a folder (or its `q3` folder) into their
    /// own folder and checked there; okconv-q3's own command line output (no `game` in its manifest) is refused, as is
    /// a folder with neither paks nor a manifest. A broken .pk3 is refused by the converter's check.
    #[test]
    fn copy_q3_maps_into_their_own_folder() {
        let base = temp("q3");
        let src = base.join("game_data_copy");
        let q = src.join(extras::Q3_DIR);
        let mut files = Vec::new();
        for (p, b) in [("shaders.json", &b"{}"[..]), ("baseq3/maps/q3dm6.glb", b"glb"), ("baseq3/textures/a.png", b"png")] {
            std::fs::create_dir_all(q.join(p).parent().unwrap()).unwrap();
            std::fs::write(q.join(p), b).unwrap();
            files.push(serde_json::json!({"path": p, "size": b.len(), "crc32": crate::base::hash::crc32(b)}));
        }
        let m = serde_json::json!({"game": extras::Q3_GAME, "format": 1, "source": "pk3", "maps": [{"name": "q3dm6"}], "files": files});
        std::fs::write(q.join("manifest.json"), m.to_string()).unwrap();
        let dirs = Dirs::at(&base.join("cfg"));
        let d = dirs.sub(extras::Q3_DIR);
        let f = inspect(&src, &dirs.root, Kind::Q3).unwrap();
        assert!(f.what.contains("Quake III Arena maps (1 map)") && f.warn.is_none() && f.bytes > 8, "{f:?}");
        let (p, c) = (Mutex::new(Progress::default()), AtomicBool::new(false));
        copy_into(&Source::of(&src, Kind::Q3).unwrap(), &d, &p, &c, &Hooks::default(), Kind::Q3).unwrap();
        assert_eq!(extras::check_dir_q3(&d.root), (Q3Verdict::Ok, 2));
        assert!(!d.part.exists() && !dirs.root.join("manifest.json").exists());
        // only Q3DM17's map: a warning that no stage is added
        std::fs::remove_file(q.join("baseq3/maps/q3dm6.glb")).unwrap();
        let m =
            serde_json::json!({"game": extras::Q3_GAME, "format": 1, "files": [{"path": "shaders.json", "size": 2, "crc32": crate::base::hash::crc32(b"{}")}]});
        std::fs::write(q.join("manifest.json"), m.to_string()).unwrap();
        assert!(inspect(&q, &dirs.root, Kind::Q3).unwrap().warn.is_some_and(|w| w.contains("no stage")));
        // okconv-q3's own output: refused
        std::fs::write(q.join("manifest.json"), r#"{"mods": ["baseq3"], "paks": [], "maps": []}"#).unwrap();
        assert!(inspect(&q, &dirs.root, Kind::Q3).unwrap_err().contains("okconv-q3's own output"));
        // nothing usable; a .pk3 that is not a zip
        assert!(inspect(&base.join("cfg"), &dirs.root, Kind::Q3).is_err());
        std::fs::create_dir_all(base.join("paks")).unwrap();
        std::fs::write(base.join("paks/PAK0.PK3"), b"not a zip").unwrap();
        #[cfg(feature = "converter")]
        assert!(inspect(&base.join("paks"), &dirs.root, Kind::Q3).unwrap_err().contains(".pk3"));
        std::fs::remove_dir_all(&base).unwrap();
    }

    #[test]
    fn inspect_refuses_what_it_cannot_use() {
        let base = temp("inspect");
        let data = base.join("data");
        assert!(inspect(&base.join("nothing"), &data, Kind::Br2).is_err());
        std::fs::write(base.join("disc.cue"), "FILE").unwrap();
        assert!(inspect(&base.join("disc.cue"), &data, Kind::Br2).unwrap_err().contains("disc image"));
        std::fs::create_dir_all(base.join("empty")).unwrap();
        assert!(inspect(&base.join("empty"), &data, Kind::Br2).unwrap_err().contains("no manifest.json"));
        // an incomplete export: what is missing
        std::fs::write(base.join("empty/manifest.json"), r#"{"format":3,"content_id":1}"#).unwrap();
        assert!(inspect(&base.join("empty/manifest.json"), &data, Kind::Br2).unwrap_err().contains("missing"));
        // an old export
        std::fs::write(base.join("empty/manifest.json"), "{}").unwrap();
        assert!(inspect(&base.join("empty/manifest.json"), &data, Kind::Br2).unwrap_err().contains("made by the old converter"));
        std::fs::remove_dir_all(&base).unwrap();
    }
}
