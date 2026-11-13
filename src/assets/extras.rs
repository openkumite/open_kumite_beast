//! The extra stages' local files (engine addition): Quake III Arena's maps, where they lie and whether they are there.
//!
//! State: none
//!
//! The extra stages (`fight::stage::extra`) draw Quake III Arena's maps "The Longest Yard" (q3dm17) and "The Camping
//! Grounds" (q3dm6) from files converted from the player's own copy of Quake III: they are never part of the export
//! of the main game's disc, of git or of the web build. Their install is the folder `Q3_DIR` of the data folder
//! (the browser: its own OPFS folder `q3-<n>`, `platform::game_data::web::Q3`), written by the
//! setup screen from the `.pk3` files (the setup's `imports::q3`, the ok_q3_converter crate's engine data): every map
//! of the paks with the images they use and the shader table, in ok_q3_converter's layout (a map names its images
//! relative to itself), and a `manifest.json` naming the game (`Q3_GAME`), its `format` and every file with size and
//! CRC-32 (`quick_check_q3`). Older game data has each map's files copied into `extras/<map>/` in the same layout (the
//! copy script of earlier versions), still read when the install has no such map or is damaged (`source`). The
//! browser's start checks only the stages' maps, lightmaps and skies (`check_opfs_q3`, `light_files`); every file is
//! checked right after an install. `fixtures/q3_stage_files.txt` lists every file each stage reads. Without a stage's
//! files it is not offered (docs/extra_stage_q3dm17.md, docs/extra_stage_q3dm6.md).

use super::Assets;
use super::manifest::{Record, clip};
pub use crate::platform::game_data::Q3_DIR;

/// `manifest.json` `game` of a Quake III install (ok_q3_converter `runtime::GAME`).
pub const Q3_GAME: &str = "Quake III Arena";
/// The formats this build reads (ok_q3_converter `runtime::FORMAT_VERSION`).
pub const Q3_FORMAT_MIN: u32 = 1;
pub const Q3_FORMAT_MAX: u32 = 1;

/// An extra stage's files, under the Quake III install's folder (or the older `legacy` one, in the same layout): its
/// map (glTF with the map's images by relative name), the shader table (the converter's `shaders.json`) and its sky
/// box's six faces (`<sky>_<face>.png`; None: the map's own sky shader).
pub struct Files {
    pub legacy: &'static str,
    pub map: &'static str,
    pub shaders: &'static str,
    pub sky: Option<&'static str>,
}

/// The extra stages' files, in `fight::stage::extra::STAGES`' order. Q3DM17's own sky shader is plain black, so it
/// takes Quake III's farbox `env/space1`; Q3DM6 draws its own (a cloud layer).
pub const STAGES: [Files; 2] = [
    Files { legacy: "extras/q3dm17", map: "baseq3/maps/q3dm17.glb", shaders: "shaders.json", sky: Some("baseq3/env/space1") },
    Files { legacy: "extras/q3dm6", map: "baseq3/maps/q3dm6.glb", shaders: "shaders.json", sky: None },
];

/// What the Quake III install's quick check found.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Q3Verdict {
    /// no `q3/manifest.json`
    Absent,
    /// complete
    Ok,
    /// unusable (why): its maps are not read (the older `extras/` still are)
    Broken(String),
}

impl Q3Verdict {
    pub fn usable(&self) -> bool {
        *self == Q3Verdict::Ok
    }

    /// The setup screen's line about it.
    pub fn note(&self) -> String {
        match self {
            Q3Verdict::Absent => "Quake III Arena maps: not added (optional).".into(),
            Q3Verdict::Ok => "Quake III Arena maps: installed.".into(),
            Q3Verdict::Broken(e) => format!("Quake III Arena maps: damaged, not used ({e})."),
        }
    }
}

/// The files every Quake III install must have (which maps it has depends on the paks: `listed`).
pub fn required_q3() -> Vec<String> {
    ["manifest.json", "shaders.json"].map(String::from).into()
}

/// The manifest names Quake III Arena (`game`).
pub fn is_q3_manifest(json: &[u8]) -> bool {
    serde_json::from_slice::<serde_json::Value>(json).ok().and_then(|v| v.get("game")?.as_str().map(|g| g == Q3_GAME)).unwrap_or(false)
}

/// The quick check of a Quake III install: `manifest` the bytes of its `manifest.json` (None: no such file), `size` a
/// file's size under its folder (None: missing). Every listed file there with its size, `required_q3` among them.
pub fn quick_check_q3(manifest: Option<&[u8]>, size: &dyn Fn(&str) -> Option<u64>) -> Q3Verdict {
    check(manifest, size, false).0
}

/// The quick check (`light`: only the files `light_files` names) and the extra stages a usable install has
/// (`listed`; 0 otherwise), the manifest parsed once.
fn check(manifest: Option<&[u8]>, size: &dyn Fn(&str) -> Option<u64>, light: bool) -> (Q3Verdict, u8) {
    let Some(manifest) = manifest else { return (Q3Verdict::Absent, 0) };
    let broken = |e: String| (Q3Verdict::Broken(e), 0);
    let rec = match Record::parse(manifest) {
        None => return broken("its manifest.json is not JSON".into()),
        Some(Err(e)) => return broken(e),
        Some(Ok(r)) => r,
    };
    if !is_q3_manifest(manifest) {
        return broken("its manifest.json is not of Quake III Arena data".into());
    }
    match rec.format {
        None => return broken("its manifest.json has no format".into()),
        Some(v) if v < Q3_FORMAT_MIN => return broken(format!("made by an older converter (format {v}): convert it again")),
        Some(v) if v > Q3_FORMAT_MAX => return broken(format!("made by a newer version (format {v}): update the game")),
        _ => {}
    }
    let checked = if light { light_files(&rec) } else { Vec::new() };
    let (mut missing, mut wrong_size) = (Vec::new(), Vec::new());
    for f in rec.files.iter().filter(|f| !light || checked.contains(&f.path)) {
        match size(&f.path) {
            None => missing.push(clip(&f.path, 120)),
            Some(s) if s != f.size => wrong_size.push(clip(&f.path, 120)),
            Some(_) => {}
        }
    }
    for p in required_q3() {
        if !rec.files.iter().any(|f| f.path == p) && size(&p).is_none() {
            missing.push(p);
        }
    }
    if !missing.is_empty() || !wrong_size.is_empty() {
        missing.truncate(super::manifest::SHOWN);
        wrong_size.truncate(super::manifest::SHOWN);
        return broken(super::manifest::damage(&missing, &wrong_size));
    }
    (Q3Verdict::Ok, listed_in(&rec))
}

/// The extra stages (bit k = `STAGES[k]`) whose map and shader table a Quake III manifest lists.
pub fn listed(manifest: &[u8]) -> u8 {
    match Record::parse(manifest) {
        Some(Ok(rec)) => listed_in(&rec),
        _ => 0,
    }
}

fn listed_in(rec: &Record) -> u8 {
    let has = |p: &str| rec.files.iter().any(|f| f.path == p);
    STAGES.iter().enumerate().filter(|(_, f)| has(f.map) && has(f.shaders)).fold(0, |m, (k, _)| m | 1 << k)
}

/// The files a light check looks at (the browser's start: a size each is a call into its storage): the required
/// ones and, for each extra stage the manifest lists, its map, its lightmap and its sky box's faces. The rest (the
/// textures; a missing one draws grey) is checked in full only right after an install.
fn light_files(rec: &Record) -> Vec<String> {
    let mut v = required_q3();
    let mask = listed_in(rec);
    for (_, f) in STAGES.iter().enumerate().filter(|(k, _)| mask & 1 << k != 0) {
        v.push(f.map.to_string());
        v.push(format!("{}_lightmap.png", f.map.trim_end_matches(".glb")));
        if let Some(sky) = f.sky {
            v.extend(SKY_FACES.iter().map(|face| format!("{sky}_{face}.png")));
        }
    }
    v
}

/// The sky box faces' names (`fight::fight_data::load_arena`'s order).
const SKY_FACES: [&str; 6] = ["rt", "bk", "lf", "ft", "up", "dn"];

/// The quick check of the folder `q3` on disk (`<data folder>/q3`; desktop, Android), every file, and the extra
/// stages it has when usable.
#[cfg(not(target_arch = "wasm32"))]
pub fn check_dir_q3(q3: &std::path::Path) -> (Q3Verdict, u8) {
    let manifest = match super::manifest::read_json(&q3.join("manifest.json")) {
        Ok(m) => m,
        Err(e) => return (Q3Verdict::Broken(e), 0),
    };
    let size = |p: &str| std::fs::metadata(q3.join(p)).ok().filter(|m| m.is_file()).map(|m| m.len());
    check(manifest.as_deref(), &size, false)
}

/// The extra stages the Quake III install beside `assets` (`Assets::q3`) has, when it is usable (desktop, Android).
#[cfg(not(target_arch = "wasm32"))]
fn installed(assets: &Assets) -> u8 {
    assets.q3().map_or(0, |q3| check_dir_q3(q3.root()).1)
}

/// The quick check of the browser's Quake III folder `dir` in OPFS (None: none installed; `light`: `light_files`
/// only, the start's) and the extra stages it has when usable. A manifest there but unreadable or too large: Broken.
#[cfg(target_arch = "wasm32")]
pub async fn check_opfs_q3(dir: Option<&str>, light: bool) -> (Q3Verdict, u8) {
    use crate::platform::game_data::web;
    let Some(dir) = dir else { return (Q3Verdict::Absent, 0) };
    let Ok(root) = web::root().await else { return (Q3Verdict::Absent, 0) };
    let Ok(d) = web::subdir(&root, dir, false).await else { return (Q3Verdict::Broken("its folder is gone".into()), 0) };
    let m = match web::get(&d, "manifest.json").await {
        Err(_) => None,
        Ok(f) if f.size() > super::manifest::MAX_JSON as f64 => return (Q3Verdict::Broken("manifest.json is too large".into()), 0),
        Ok(f) => match web::read_blob(&f).await {
            Ok(b) => Some(b),
            Err(e) => return (Q3Verdict::Broken(format!("manifest.json: {e}")), 0),
        },
    };
    let rec = m.as_deref().and_then(Record::parse).and_then(Result::ok);
    // the light check's files hold the required ones (`light_files`)
    let paths = match &rec {
        Some(rec) if light => light_files(rec),
        Some(rec) => required_q3().into_iter().chain(rec.files.iter().map(|f| f.path.clone())).collect(),
        None => required_q3(),
    };
    let mut sizes = std::collections::HashMap::new();
    for p in paths {
        if !sizes.contains_key(&p)
            && let Ok(f) = web::get(&d, &p).await
        {
            sizes.insert(p, f.size() as u64);
        }
    }
    check(m.as_deref(), &|p| sizes.get(p).copied(), light)
}

/// The browser's Quake III install: its OPFS folder (`web::Q3`) and the extra stages it has (`listed`), when its light
/// check finds it usable (`Assets::set_q3_opfs` gives them to the game data's `Assets`).
#[cfg(target_arch = "wasm32")]
pub async fn opfs_q3_install() -> Option<(String, u8)> {
    let dir = crate::platform::game_data::web::Q3.pointer()?;
    let t = crate::perf_now();
    let (v, mask) = check_opfs_q3(Some(&dir), true).await;
    log::info!("game data {dir}: Quake III {v:?}, extra stages {mask:#b} ({:.0} ms)", (crate::perf_now() - t) * 1000.0);
    v.usable().then_some((dir, mask))
}

/// Where extra stage `k`'s files (`STAGES[k]`, paths under it) are read from: the Quake III install when it is usable
/// and lists them, else the older `extras/<map>/` folder when it has the map and the shader table (desktop,
/// Android); the browser: its OPFS install (`Assets::set_q3_opfs`), or over HTTP the older folder. None: no files.
pub fn source(assets: &Assets, k: usize) -> Option<Assets> {
    let f = STAGES.get(k)?;
    #[cfg(not(target_arch = "wasm32"))]
    {
        if installed(assets) & 1 << k != 0 {
            return assets.q3();
        }
        let dir = assets.root().join(f.legacy);
        [f.map, f.shaders].iter().all(|p| dir.join(p).is_file()).then(|| Assets::new(dir))
    }
    #[cfg(target_arch = "wasm32")]
    {
        if assets.q3_mask() & 1 << k != 0 {
            return assets.q3();
        }
        assets.legacy_http(f.legacy)
    }
}

/// The extra stages whose files `assets` has (bit k = `STAGES[k]`, `source`), none when debug `OKB_NO_EXTRAS` is set
/// (the golden baseline: the select as without the files). The browser: those of its OPFS install (none over HTTP).
pub fn present(assets: &Assets) -> u8 {
    #[cfg(not(target_arch = "wasm32"))]
    {
        if crate::base::env::var_os("NO_EXTRAS").is_some() {
            return 0;
        }
        let q3 = installed(assets);
        let legacy = |f: &Files| [f.map, f.shaders].iter().all(|p| assets.root().join(f.legacy).join(p).is_file());
        STAGES.iter().enumerate().filter(|(k, f)| q3 & 1 << k != 0 || legacy(f)).fold(0, |m, (k, _)| m | 1 << k)
    }
    #[cfg(target_arch = "wasm32")]
    {
        assets.q3_mask()
    }
}

/// Quake III Arena's paks for the tests that convert them: OKB_Q3_PAKS, else the developer's copy beside the project
/// (`../../19_games/PC/Quake 3/baseq3`); None when not there.
#[cfg(all(test, feature = "converter", not(target_arch = "wasm32")))]
pub(crate) fn test_paks() -> Option<std::path::PathBuf> {
    let p = crate::base::env::var_os("Q3_PAKS")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../19_games/PC/Quake 3/baseq3"));
    p.is_dir().then_some(p)
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use super::*;

    fn temp(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("okb_extras_{name}_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// A Quake III install in `dir` with `files` (path, bytes) and its manifest listing them.
    fn q3_install(dir: &std::path::Path, files: &[(&str, &[u8])]) {
        let mut list = Vec::new();
        for (p, b) in files {
            std::fs::create_dir_all(dir.join(p).parent().unwrap()).unwrap();
            std::fs::write(dir.join(p), b).unwrap();
            list.push(serde_json::json!({"path": p, "size": b.len(), "crc32": crate::base::hash::crc32(b)}));
        }
        let m = serde_json::json!({"game": Q3_GAME, "format": 1, "source": "pk3", "files": list});
        std::fs::write(dir.join("manifest.json"), m.to_string()).unwrap();
    }

    /// A legacy stage's files: both or none count, each on its own.
    #[test]
    fn legacy_needs_both_files() {
        let dir = temp("legacy");
        let a = Assets::new(&dir);
        let [q17, q6] = &STAGES;
        let at = |f: &Files, p: &str| dir.join(f.legacy).join(p);
        assert_eq!(present(&a), 0);
        std::fs::create_dir_all(at(q6, q6.map).parent().unwrap()).unwrap();
        std::fs::write(at(q6, q6.map), b"glb").unwrap();
        assert_eq!(present(&a), 0);
        std::fs::write(at(q6, q6.shaders), b"{}").unwrap();
        assert_eq!(present(&a), 2);
        assert_eq!(source(&a, 1).map(|s| s.root().to_path_buf()), Some(dir.join("extras/q3dm6")));
        std::fs::create_dir_all(at(q17, q17.map).parent().unwrap()).unwrap();
        std::fs::write(at(q17, q17.map), b"glb").unwrap();
        std::fs::write(at(q17, q17.shaders), b"{}").unwrap();
        assert_eq!(present(&a), 3);
        std::fs::remove_file(at(q6, q6.map)).unwrap();
        assert_eq!((present(&a), source(&a, 1).is_none()), (1, true));
        std::fs::remove_file(at(q17, q17.map)).unwrap();
        assert_eq!(present(&a), 0);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The q3 install alone, the legacy folder alone, both (the install wins), a broken install (the legacy folder).
    #[test]
    fn q3_install_first_then_legacy() {
        let dir = temp("q3");
        let a = Assets::new(&dir);
        let q3 = dir.join(Q3_DIR);
        // only Q3DM17's map in the install
        q3_install(&q3, &[("shaders.json", b"{}"), ("baseq3/maps/q3dm17.glb", b"glb"), ("baseq3/env/space1_ft.png", b"png")]);
        assert_eq!(check_dir_q3(&q3), (Q3Verdict::Ok, 1));
        assert_eq!(present(&a), 1);
        assert_eq!(source(&a, 0).map(|s| s.root().to_path_buf()), Some(q3.clone()));
        assert!(source(&a, 1).is_none());
        // Q3DM6 in the legacy folder: both
        std::fs::create_dir_all(dir.join("extras/q3dm6/baseq3/maps")).unwrap();
        std::fs::write(dir.join("extras/q3dm6/baseq3/maps/q3dm6.glb"), b"glb").unwrap();
        std::fs::write(dir.join("extras/q3dm6/shaders.json"), b"{}").unwrap();
        assert_eq!(present(&a), 3);
        assert_eq!(source(&a, 1).map(|s| s.root().to_path_buf()), Some(dir.join("extras/q3dm6")));
        // Q3DM17 in both: the install's
        std::fs::create_dir_all(dir.join("extras/q3dm17/baseq3/maps")).unwrap();
        std::fs::write(dir.join("extras/q3dm17/baseq3/maps/q3dm17.glb"), b"glb").unwrap();
        std::fs::write(dir.join("extras/q3dm17/shaders.json"), b"{}").unwrap();
        assert_eq!(source(&a, 0).map(|s| s.root().to_path_buf()), Some(q3.clone()));
        // a file of the install damaged: not used, the legacy folder is
        std::fs::write(q3.join("baseq3/env/space1_ft.png"), b"pn").unwrap();
        assert!(matches!(check_dir_q3(&q3), (Q3Verdict::Broken(e), 0) if e.contains("wrong size")));
        assert_eq!(source(&a, 0).map(|s| s.root().to_path_buf()), Some(dir.join("extras/q3dm17")));
        assert_eq!(present(&a), 3);
        std::fs::remove_dir_all(dir.join("extras")).unwrap();
        assert_eq!(present(&a), 0);
        // a broken manifest
        std::fs::write(q3.join("manifest.json"), b"{").unwrap();
        assert!(matches!(check_dir_q3(&q3), (Q3Verdict::Broken(_), 0)));
        assert_eq!(present(&a), 0);
        std::fs::remove_dir_all(&q3).unwrap();
        assert_eq!(check_dir_q3(&q3), (Q3Verdict::Absent, 0));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn quick_check_cases() {
        let size = |p: &str| match p {
            "manifest.json" | "shaders.json" => Some(2),
            "baseq3/maps/q3dm6.glb" => Some(3),
            _ => None,
        };
        let m = |game: &str, format: &str, files: &str| format!(r#"{{"game": "{game}", "format": {format}, "files": [{files}]}}"#).into_bytes();
        let sh = r#"{"path": "shaders.json", "size": 2, "crc32": 1}"#;
        let map = r#"{"path": "baseq3/maps/q3dm6.glb", "size": 3, "crc32": 2}"#;
        let ok = m(Q3_GAME, "1", &format!("{sh}, {map}"));
        assert_eq!(quick_check_q3(None, &size), Q3Verdict::Absent);
        assert_eq!(quick_check_q3(Some(&ok), &size), Q3Verdict::Ok);
        assert_eq!(listed(&ok), 2);
        let broken = |b: &[u8], what: &str| matches!(quick_check_q3(Some(b), &size), Q3Verdict::Broken(e) if e.contains(what));
        assert!(broken(b"[", "not JSON"));
        assert!(broken(&m("Bloody Roar (USA) SCUS-94199", "1", sh), "not of Quake III"));
        assert!(broken(&m(Q3_GAME, "2", sh), "newer"));
        assert!(broken(&m(Q3_GAME, "0", sh), "older"));
        assert!(broken(br#"{"game": "Quake III Arena", "files": []}"#, "no format"));
        assert!(broken(&m(Q3_GAME, "1", r#"{"path": "baseq3/x.png", "size": 1, "crc32": 3}"#), "missing baseq3/x.png"));
        assert!(broken(&m(Q3_GAME, "1", r#"{"path": "shaders.json", "size": 9, "crc32": 1}"#), "wrong size shaders.json"));
        assert!(broken(&m(Q3_GAME, "1", r#"{"path": "../x", "size": 1, "crc32": 3}"#), "broken file entry"));
        // shaders.json is required even when not listed (here: present, so fine) …
        assert_eq!(quick_check_q3(Some(&m(Q3_GAME, "1", "")), &size), Q3Verdict::Ok);
        // … and missing
        assert!(matches!(quick_check_q3(Some(&m(Q3_GAME, "1", "")), &|_| None), Q3Verdict::Broken(e) if e.contains("shaders.json")));
        // the light check (the browser's start): a stage's map, lightmap and sky, not its textures
        let files = r#"{"path": "shaders.json", "size": 2, "crc32": 1}, {"path": "baseq3/maps/q3dm17.glb", "size": 3, "crc32": 2},
            {"path": "baseq3/maps/q3dm17_lightmap.png", "size": 4, "crc32": 3}, {"path": "baseq3/env/space1_up.png", "size": 5, "crc32": 4},
            {"path": "baseq3/textures/x.png", "size": 6, "crc32": 5}"#;
        let m17 = m(Q3_GAME, "1", files);
        let sizes = |gone: &'static str| {
            move |p: &str| {
                let n = [
                    "manifest.json",
                    "shaders.json",
                    "baseq3/maps/q3dm17.glb",
                    "baseq3/maps/q3dm17_lightmap.png",
                    "baseq3/env/space1_up.png",
                    "baseq3/textures/x.png",
                ]
                .iter()
                .position(|q| *q == p && *q != gone)?;
                Some([2, 2, 3, 4, 5, 6][n])
            }
        };
        assert_eq!(check(Some(&m17), &sizes(""), true), (Q3Verdict::Ok, 1));
        assert_eq!(check(Some(&m17), &sizes("baseq3/textures/x.png"), true), (Q3Verdict::Ok, 1));
        assert!(matches!(check(Some(&m17), &sizes("baseq3/textures/x.png"), false), (Q3Verdict::Broken(_), 0)));
        for gone in ["baseq3/maps/q3dm17_lightmap.png", "baseq3/env/space1_up.png", "baseq3/maps/q3dm17.glb", "shaders.json"] {
            assert!(matches!(check(Some(&m17), &sizes(gone), true), (Q3Verdict::Broken(e), 0) if e.contains(gone)), "{gone}");
        }
        assert!(Q3Verdict::Ok.usable() && !Q3Verdict::Absent.usable() && Q3Verdict::Broken("x".into()).note().contains("damaged"));
    }

    /// End to end (the real paks, `test_paks`; about 270 MB written: run with `--ignored`): `ok_q3_converter`
    /// converts them into a data folder's `q3/`, every file each stage reads (`fixtures/q3_stage_files.txt`) is there,
    /// the full check passes and both stages are offered from it.
    #[cfg(feature = "converter")]
    #[test]
    #[ignore]
    fn the_real_paks_hold_both_stages() {
        let Some(base) = test_paks() else {
            eprintln!("skipped: no Quake III Arena paks (OKB_Q3_PAKS)");
            return;
        };
        struct Dir(std::path::PathBuf);
        impl ok_q3_converter::runtime::Sink for Dir {
            fn put(&mut self, path: &str, data: &[u8]) -> Result<(), String> {
                let to = self.0.join(path);
                std::fs::create_dir_all(to.parent().unwrap()).map_err(|e| e.to_string())?;
                std::fs::write(to, data).map_err(|e| e.to_string())
            }
        }
        let mut paks: Vec<(String, Box<dyn ok_q3_converter::vfs::ReadSeek>)> = Vec::new();
        for e in std::fs::read_dir(&base).unwrap().flatten().filter(|e| e.path().extension().is_some_and(|x| x.eq_ignore_ascii_case("pk3"))) {
            let f = std::io::BufReader::new(std::fs::File::open(e.path()).unwrap());
            paks.push((e.file_name().to_string_lossy().into_owned(), Box::new(f)));
        }
        let dir = temp("e2e");
        let q3 = dir.join(Q3_DIR);
        let paks = ok_q3_converter::runtime::Paks::open(paks).unwrap();
        ok_q3_converter::runtime::Converter::new(paks).run(&mut Dir(q3.clone()), &mut |_| true).unwrap();
        assert_eq!(check_dir_q3(&q3), (Q3Verdict::Ok, 3));
        let list = include_str!("fixtures/q3_stage_files.txt");
        let wanted: Vec<&str> = list.lines().filter(|l| !l.is_empty() && !l.starts_with('#') && !l.starts_with('[')).collect();
        assert_eq!(wanted.len(), 222);
        let missing: Vec<&&str> = wanted.iter().filter(|p| !q3.join(p).is_file()).collect();
        assert!(missing.is_empty(), "{missing:?}");
        assert_eq!(present(&Assets::new(&dir)), 3);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
