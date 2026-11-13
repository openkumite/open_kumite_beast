//! The setup's QUAKE III ARENA MAPS on desktop and Android: Quake III Arena's `.pk3` files converted into the Quake
//! III install's `.part` folder (engine addition, feature `converter`; the extra stages Q3DM17 / Q3DM6).
//!
//! `paks_in` finds the paks a pick names: a folder with `.pk3` files (any case: the game's `baseq3`), else that
//! folder's `baseq3` (the game's folder), else a `.pk3` file itself; on Android a picked tree's (its top, or its
//! `baseq3` folder), each opened as a file descriptor. `inspect_q3` opens them (`imports::q3`: only their zip
//! directories are read) and lists their maps, so paks without any map are refused before anything is written, and
//! says when none of the maps is an extra stage. `convert_q3_into` runs the converter step by step on the job's
//! thread into the Quake III folder's `.part` (`platform::game_data::Dirs::sub(Q3_DIR)`), the cancel flag checked
//! between steps; then the full check (every file of the converter's summary read back), the quick check
//! (`assets::extras::check_dir_q3`) and the swap, as the imported game's disc's conversion does (`convert.rs`).

use super::convert::{FileAt, PartSink, verify};
use super::imports::q3;
use super::job::{Failure, Hooks};
use super::plan::{Kind, q3_error, q3_found};
use super::screen::{Found, Phase, Progress};
use crate::platform::game_data::Dirs;
use ok_br2_converter::ReadAt;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};

/// The paks a pick names (module header), by file name; Err (why) when it names none.
pub fn paks_in(p: &Path) -> Result<Vec<(String, Box<dyn ReadAt>)>, String> {
    #[cfg(target_os = "android")]
    if let Some(uri) = p.to_str().filter(|s| s.starts_with("content://") && s.contains("/tree/")) {
        let all = crate::platform::android::list_tree(uri)?;
        let pk3 = |r: &str| r.to_ascii_lowercase().ends_with(".pk3");
        for top in ["", "baseq3/"] {
            let docs: Vec<&(String, String, u64)> = all
                .iter()
                .filter(|(r, ..)| {
                    r.len() > top.len() && r.get(..top.len()).is_some_and(|h| h.eq_ignore_ascii_case(top)) && !r[top.len()..].contains('/') && pk3(r)
                })
                .collect();
            if docs.is_empty() {
                continue;
            }
            let mut paks: Vec<(String, Box<dyn ReadAt>)> = Vec::new();
            for (r, doc, _) in docs {
                let f = crate::platform::android::open_document(uri, doc)?;
                let at = FileAt::new(f).map_err(|e| format!("{r}: {e}"))?;
                paks.push((r[top.len()..].to_string(), Box::new(at)));
            }
            return Ok(paks);
        }
        return Err("No .pk3 files in that folder or in a baseq3 folder in it.".into());
    }
    let is_pk3 = |q: &Path| q.is_file() && q.extension().is_some_and(|e| e.eq_ignore_ascii_case("pk3"));
    let in_dir = |d: &Path| -> Vec<PathBuf> {
        let mut v: Vec<PathBuf> =
            std::fs::read_dir(d).map(|rd| rd.filter_map(|e| e.ok().map(|e| e.path())).filter(|q| is_pk3(q)).collect()).unwrap_or_default();
        v.sort();
        v
    };
    let files = if is_pk3(p) {
        vec![p.to_path_buf()]
    } else if p.is_dir() {
        let mut v = in_dir(p);
        if v.is_empty() {
            let base = std::fs::read_dir(p).ok().and_then(|rd| {
                rd.filter_map(|e| e.ok().map(|e| e.path())).find(|q| q.is_dir() && q.file_name().is_some_and(|n| n.eq_ignore_ascii_case("baseq3")))
            });
            v = base.map(|b| in_dir(&b)).unwrap_or_default();
        }
        v
    } else {
        Vec::new()
    };
    if files.is_empty() {
        return Err(format!("No .pk3 files in {} (or in a baseq3 folder in it).", p.display()));
    }
    let mut paks: Vec<(String, Box<dyn ReadAt>)> = Vec::new();
    for f in files {
        let name = f.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let at = std::fs::File::open(&f).and_then(FileAt::new).map_err(|e| format!("{}: {e}", f.display()))?;
        paks.push((name, Box::new(at)));
    }
    Ok(paks)
}

/// Checks the paks `paks` (`paks_in` of the pick `p`): what the confirmation shows, or why they cannot be converted.
pub fn inspect_q3(p: &Path, paks: Vec<(String, Box<dyn ReadAt>)>) -> Result<Found, String> {
    let paks = q3::open(paks).map_err(q3_error)?;
    let (what, warn, bytes) = q3_found(paks.names().len(), &paks.maps());
    #[cfg(target_os = "android")]
    if let Some(free) = crate::platform::android::free_space()
        && bytes + (64 << 20) > free
    {
        return Err(format!("Not enough free space: the Quake III Arena maps need about {} MB, {} MB are free.", (bytes >> 20) + 64, free >> 20));
    }
    Ok(Found { path: p.to_path_buf(), what, bytes, warn })
}

/// Converts the paks of the pick `src` into `dirs.part` (`Dirs::sub(Q3_DIR)`), checks it and installs it (module
/// header).
pub fn convert_q3_into(src: &Path, dirs: &Dirs, progress: &Mutex<Progress>, cancel: &AtomicBool, hooks: &Hooks) -> Result<(), Failure> {
    let set = |phase: Phase, done: u64, total: u64, file: &str| {
        if let Ok(mut p) = progress.lock() {
            *p = Progress { phase, done, total, file: file.to_string() };
        }
    };
    set(Phase::Reading, 0, 0, &src.display().to_string());
    let paks = q3::open(paks_in(src).map_err(Failure::Io)?).map_err(|e| Failure::Io(q3_error(e)))?;
    let _held = match dirs.lock().map_err(|e| Failure::Io(format!("game_data.lock: {e}")))? {
        Some(f) => f,
        None => return Err(Failure::Io("another copy of the game is installing its data: try again when it is done".into())),
    };
    dirs.fresh_part().map_err(|e| Failure::Io(format!("q3.part: {e}")))?;
    let r = (|| {
        let t = std::time::Instant::now();
        let mut sink = PartSink { root: dirs.part.clone(), full: false };
        let run = q3::Converter::new(paks).run(&mut sink, &mut |p| {
            set(Phase::Converting, p.done as u64, p.total as u64, &p.step);
            if hooks.slow_ms > 0 {
                std::thread::sleep(std::time::Duration::from_millis(hooks.slow_ms));
            }
            !cancel.load(Ordering::Relaxed)
        });
        let summary = match run {
            Ok(s) => s,
            Err(q3::Error::Cancelled) => return Err(Failure::Cancelled),
            Err(q3::Error::Sink(e)) if sink.full => return Err(Failure::NoSpace(e)),
            Err(e) => return Err(Failure::Io(e.to_string())),
        };
        log::info!(
            "setup: Quake III Arena converted ({} paks, {} maps) in {:.1} s: {} files",
            summary.paks.len(),
            summary.maps.len(),
            t.elapsed().as_secs_f64(),
            summary.files.len()
        );
        if let Some(f) = &hooks.before_verify {
            f(&dirs.part);
        }
        verify(dirs, &summary.files, progress, cancel, hooks)?;
        super::job::check_part(dirs, Kind::Q3)?;
        let total: u64 = summary.files.values().map(|f| f.0).sum();
        set(Phase::Installing, total, total, "");
        dirs.install().map_err(|e| Failure::Io(format!("q3: {e}")))
    })();
    if r.is_err() {
        dirs.discard_part();
    }
    r
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::assets::extras::{Q3Verdict, check_dir_q3};

    /// The real paks (OKB_Q3_PAKS, or the developer's copy; skipped without them): found in the baseq3 folder and in
    /// the game's folder, checked, converted into the Quake III folder and checked there; its maps hold both extra
    /// stages. Cancelled: nothing installed, no `.part` left. Writes about 270 MB: run with `--ignored`.
    #[test]
    #[ignore]
    fn convert_the_real_paks() {
        let Some(base) = crate::assets::extras::test_paks() else {
            eprintln!("skipped: no Quake III Arena paks (OKB_Q3_PAKS)");
            return;
        };
        assert!(paks_in(&base).is_ok() && paks_in(base.parent().unwrap()).is_ok());
        assert!(paks_in(&std::env::temp_dir().join("okb_no_such_paks")).is_err());
        let f = inspect_q3(&base, paks_in(&base).unwrap()).unwrap();
        assert!(f.warn.is_none() && f.what.contains("7 paks, 36 maps"), "{f:?}");
        let root = std::env::temp_dir().join(format!("okb_setup_q3_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let dirs = Dirs::at(&root).sub(crate::assets::extras::Q3_DIR);
        std::fs::create_dir_all(dirs.root.parent().unwrap()).unwrap();
        let (p, c) = (Mutex::new(Progress::default()), AtomicBool::new(true));
        assert_eq!(convert_q3_into(&base, &dirs, &p, &c, &Hooks::default()), Err(Failure::Cancelled));
        assert!(!dirs.root.exists() && !dirs.part.exists());
        c.store(false, Ordering::Relaxed);
        convert_q3_into(&base, &dirs, &p, &c, &Hooks::default()).unwrap();
        assert_eq!(check_dir_q3(&dirs.root), (Q3Verdict::Ok, 3));
        assert!(!dirs.part.exists());
        let _ = std::fs::remove_dir_all(&root);
    }
}
