//! The game data's install record and its checks (engine addition): which files the engine needs, the quick check at
//! every start and what the full check compares.
//!
//! State: none
//!
//! The data folder holds the converted export: `manifest.json` (from the converter `ok_br2_converter`) and the files
//! under it. The converter's manifest is the install's record: `format`, `source`, `xa`, `content_id` and
//! `files: [{path, size, crc32}]` (an old Python export has none of them and is refused, `Verdict::Legacy`).
//! `quick_check` takes that record, checks the format version, the content id, every listed file's size and the
//! engine's own required set (`required`), and gives a `Verdict`. The full check (CRC-32 of every listed file,
//! `base::hash`) runs only right after a copy or a conversion (`app::setup`).

use serde::{Deserialize, Serialize};

/// The data formats this build reads (the converter's manifest `format`; 2 adds the static tables
/// `engine/tables.okt`, which the engine reads instead of the program file: format 1 has none;
/// 3 the game's text and the front end's, the HUD's and the camera's tables). A record must also have its `content_id`
/// (`Verdict::NoContentId`: the converters before it wrote none).
pub const FORMAT_MIN: u32 = 3;
pub const FORMAT_MAX: u32 = 3;

/// The character codes (`roster::CHARACTERS`) and the stage codes (`fight::fight_data::STAGE_CODES`, which L1 cannot
/// name) whose files every install must have.
const STAGES: [&str; 13] = ["ST0", "ST1", "ST2", "ST3", "ST4", "ST5", "ST6", "ST7", "ST8", "ST9", "STA", "STB", "STC"];
/// The movies' streams (`frontend::menus::MOVIE_FILES`).
const MOVIES: [&str; 4] = ["USAOP", "BR", "ED1", "ED2"];

/// One file of an install: its path under the data folder ('/' separators), size and CRC-32 (IEEE).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileEntry {
    pub path: String,
    pub size: u64,
    pub crc32: u32,
}

/// What an install is: the format (None: an old Python export), where it came from (`bin`, `chd`, `iso`),
/// whether it has the CD-XA audio (music and movie sound; false from an ISO), its content id and its files.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Record {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub format: Option<u32>,
    #[serde(default)]
    pub source: String,
    #[serde(default)]
    pub xa: bool,
    /// the converter's id of the data the simulation reads (engine addition; the converter's `docs/content_id.md`): a
    /// session's start carries it (`SessionStart::content`), so that a netplay peer with other data is refused and a
    /// replay of other data warned of; None when the manifest has none (or not a number of 1..=u32::MAX, logged)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content_id: Option<u32>,
    #[serde(default)]
    pub files: Vec<FileEntry>,
}

impl Record {
    /// The record in a manifest's JSON (its other keys ignored); None when it is not a
    /// JSON object. A file entry's `crc32` may be a number or a hex string; a broken entry makes the record invalid
    /// (Err with the reason).
    pub fn parse(json: &[u8]) -> Option<Result<Record, String>> {
        Self::of(&serde_json::from_slice(json).ok()?)
    }

    /// As `parse`, from the parsed JSON.
    pub fn of(v: &serde_json::Value) -> Option<Result<Record, String>> {
        let o = v.as_object()?;
        let format = match o.get("format") {
            None => None,
            Some(f) => match f.as_u64().and_then(|f| u32::try_from(f).ok()) {
                Some(f) => Some(f),
                None => return Some(Err(format!("format {f} is not a version number"))),
            },
        };
        let mut files = Vec::new();
        for f in o.get("files").and_then(|f| f.as_array()).map(Vec::as_slice).unwrap_or_default() {
            let path = f.get("path").and_then(|p| p.as_str());
            let size = f.get("size").and_then(|s| s.as_u64());
            let crc32 = match f.get("crc32") {
                Some(serde_json::Value::String(s)) => u32::from_str_radix(s.trim_start_matches("0x"), 16).ok(),
                Some(n) => n.as_u64().and_then(|n| u32::try_from(n).ok()),
                None => None,
            };
            match (path, size, crc32) {
                (Some(p), Some(s), Some(c)) if safe_path(p) => files.push(FileEntry { path: p.to_string(), size: s, crc32: c }),
                _ => return Some(Err(format!("a broken file entry: {}", clip(&f.to_string(), 120)))),
            }
        }
        Some(Ok(Record {
            format,
            source: o.get("source").and_then(|s| s.as_str()).unwrap_or_default().to_string(),
            xa: o.get("xa").and_then(|x| x.as_bool()).unwrap_or(false),
            content_id: o.get("content_id").and_then(id_of),
            files,
        }))
    }

    #[cfg(test)]
    pub fn to_json(&self) -> String {
        serde_json::to_string_pretty(self).unwrap_or_default()
    }
}

/// A manifest's `content_id` value as an id: a number of 1..=u32::MAX; None otherwise (logged: a converter's bug; 0 is
/// the netplay Hello's "none").
fn id_of(id: &serde_json::Value) -> Option<u32> {
    let ok = id.as_u64().and_then(|id| u32::try_from(id).ok()).filter(|&id| id != 0);
    if ok.is_none() {
        log::warn!("manifest.json: content_id {} is not a content id (convert the game data again)", clip(&id.to_string(), 40));
    }
    ok
}

/// `s` cut to at most `n` characters ("…" at the cut): what a broken file shows stays a line or two.
pub fn clip(s: &str, n: usize) -> String {
    match s.char_indices().nth(n) {
        Some((k, _)) => format!("{}...", &s[..k]),
        None => s.to_string(),
    }
}

/// A relative path that stays inside the data folder: no root, no `..`, no backslash or drive, not empty.
pub fn safe_path(p: &str) -> bool {
    !p.is_empty() && !p.starts_with('/') && !p.contains('\\') && !p.contains(':') && p.split('/').all(|c| !c.is_empty() && c != "." && c != "..")
}

/// What the quick check found.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Verdict {
    /// a converted install of a format this build reads, complete
    Ok,
    /// an old Python export (no format): not usable (it has no `engine/tables.okt`)
    Legacy,
    /// no data folder or no manifest
    Missing,
    /// made by an older converter
    TooOld(u32),
    /// made by a newer converter
    TooNew(u32),
    /// made by a converter that wrote no content id (`Record::content_id`)
    NoContentId,
    /// a record that cannot be read
    Broken(String),
    /// files missing or of the wrong size (the first few of each)
    Damaged { missing: Vec<String>, wrong_size: Vec<String> },
}

impl Verdict {
    /// The data can be used as it is.
    pub fn usable(&self) -> bool {
        matches!(self, Verdict::Ok)
    }

    /// Why the data cannot be used (the setup screen's first line); None when it can.
    pub fn reason(&self) -> Option<String> {
        Some(match self {
            Verdict::Ok => return None,
            Verdict::Legacy => "The game data was made by the old converter: convert the game data again.".into(),
            Verdict::Missing => "No game data yet.".into(),
            Verdict::TooOld(v) => format!("The game data was made by an older converter (format {v}): convert it again."),
            Verdict::TooNew(v) => format!("The game data was made by a newer version (format {v}): update the game."),
            Verdict::NoContentId => "The game data was made by an older converter (no content id): convert it again.".into(),
            Verdict::Broken(e) => format!("The game data's record cannot be read: {e}."),
            Verdict::Damaged { missing, wrong_size } => format!("The game data is damaged: {}.", damage(missing, wrong_size)),
        })
    }
}

/// The files missing and of the wrong size, as `Verdict::reason`, `quick_check_br1` and `extras::quick_check_q3` name
/// them.
pub(crate) fn damage(missing: &[String], wrong_size: &[String]) -> String {
    let mut s = String::new();
    if !missing.is_empty() {
        s += &format!("missing {}", missing.join(", "));
    }
    if !wrong_size.is_empty() {
        s += &format!("{}wrong size {}", if missing.is_empty() { "" } else { "; " }, wrong_size.join(", "));
    }
    s
}

/// The files every install must have, whatever made it: the start-up's and the matches' required loads (the front
/// end's CMN files, the engine tables, each character's ACT / ANI / models, each stage's DAT / model) and the movies'
/// streams. Music and movie sound (`.xa` / `.mp3`) are not in it: an ISO install has none.
pub fn required() -> Vec<String> {
    let mut v: Vec<String> =
        ["manifest.json", "engine/INIT.DAT", "engine/tables.okt", "engine/index.json", "engine/CMN.ACT", "engine/eft/EFT.DAT"].map(String::from).into();
    v.extend(["INIT", "USATITLE", "SE00", "P00"].map(|n| format!("engine/cmn/{n}.DAT")));
    for c in super::roster::CHARACTERS {
        v.push(format!("engine/{c}.ACT"));
        v.push(format!("engine/eft/{c}EFT.DAT"));
        for k in 1..=4 {
            v.push(format!("engine/{c}{k}P.ANI"));
            v.push(format!("characters/{c}_{k}P_human.glb"));
            v.push(format!("characters/{c}_{k}P_beast.glb"));
        }
    }
    for s in STAGES {
        v.push(format!("stages/{s}.glb"));
        v.push(format!("engine/stage/{s}.DAT"));
    }
    v.extend(MOVIES.map(|m| format!("video/{m}.bsf")));
    v
}

/// The most a `manifest.json` is read of (a converted install's lists its ~2000 files in well under
/// 1 MiB); a larger file is not one (`Broken`).
pub const MAX_JSON: usize = 16 << 20;

/// At most this many paths of each kind in a `Damaged` verdict.
pub(crate) const SHOWN: usize = 4;

/// The quick check over a data folder: `manifest` the bytes of `manifest.json` (None: no such file), `size` a file's
/// size (None: missing).
pub fn quick_check(manifest: Option<&[u8]>, size: &dyn Fn(&str) -> Option<u64>) -> Verdict {
    let Some(manifest) = manifest else { return Verdict::Missing };
    let rec = match Record::parse(manifest) {
        None => return Verdict::Broken("manifest.json is not JSON".into()),
        Some(Err(e)) => return Verdict::Broken(e),
        Some(Ok(r)) => r,
    };
    match rec.format {
        // an old Python export: none of its files is what this build reads first (`engine/tables.okt`)
        None => return Verdict::Legacy,
        Some(v) if v < FORMAT_MIN => return Verdict::TooOld(v),
        Some(v) if v > FORMAT_MAX => return Verdict::TooNew(v),
        _ => {}
    }
    // without it a session could not tell other data from this (`install_id`)
    if rec.content_id.is_none() {
        return Verdict::NoContentId;
    }
    let (mut missing, mut wrong_size) = (Vec::new(), Vec::new());
    for f in &rec.files {
        match size(&f.path) {
            None => missing.push(clip(&f.path, 120)),
            Some(s) if s != f.size => wrong_size.push(clip(&f.path, 120)),
            Some(_) => {}
        }
    }
    for p in required() {
        if !rec.files.iter().any(|f| f.path == p) && size(&p).is_none() {
            missing.push(p);
        }
    }
    if missing.is_empty() && wrong_size.is_empty() {
        return Verdict::Ok;
    }
    missing.truncate(SHOWN);
    wrong_size.truncate(SHOWN);
    Verdict::Damaged { missing, wrong_size }
}

/// `quick_check` of the folder `root` on disk (desktop, Android).
#[cfg(not(target_arch = "wasm32"))]
pub fn check_dir(root: &std::path::Path) -> Verdict {
    let manifest = match read_json(&root.join("manifest.json")) {
        Ok(m) => m,
        Err(e) => return Verdict::Broken(e),
    };
    let size = |p: &str| std::fs::metadata(root.join(p)).ok().filter(|m| m.is_file()).map(|m| m.len());
    quick_check(manifest.as_deref(), &size)
}

/// A manifest's bytes, at most `MAX_JSON` of them (None: no such file).
#[cfg(not(target_arch = "wasm32"))]
pub fn read_json(p: &std::path::Path) -> Result<Option<Vec<u8>>, String> {
    if !p.is_file() {
        return Ok(None);
    }
    let name = p.file_name().unwrap_or_default().to_string_lossy();
    crate::platform::storage::read_capped(p, MAX_JSON).map(Some).map_err(|e| format!("{name}: {e}"))
}

/// `quick_check` of the browser's install folder `dir` in OPFS (the manifest, then the size of every file the check
/// asks about); Err when the browser cannot store files at all.
#[cfg(target_arch = "wasm32")]
pub async fn check_opfs(dir: Option<&str>) -> Result<Verdict, String> {
    use crate::platform::game_data::web;
    let root = web::root().await?;
    let Some(dir) = dir else { return Ok(Verdict::Missing) };
    let Ok(d) = web::subdir(&root, dir, false).await else { return Ok(Verdict::Missing) };
    let read = |p: &'static str| {
        let d = d.clone();
        async move {
            let f = web::get(&d, p).await.ok()?;
            if f.size() > (16 << 20) as f64 {
                return None;
            }
            web::read_blob(&f).await.ok()
        }
    };
    let m = read("manifest.json").await;
    let mut paths = required();
    if let Some(Ok(rec)) = m.as_deref().and_then(Record::parse) {
        paths.extend(rec.files.into_iter().map(|f| f.path));
    }
    let mut sizes = std::collections::HashMap::new();
    for p in paths {
        if !sizes.contains_key(&p)
            && let Ok(f) = web::get(&d, &p).await
        {
            sizes.insert(p, f.size() as u64);
        }
    }
    Ok(quick_check(m.as_deref(), &|p| sizes.get(p).copied()))
}

/// The imported game's install (engine addition): the optional second disc's data, in the folder `BR1_DIR` of the data
/// folder (desktop, Android; the browser keeps it in its own OPFS folder, `platform::game_data::web`). Its
/// `manifest.json` (from ok_br1_converter's `runtime::Converter`, through the setup's `imports::br1`) names the game
/// (`BR1_GAME`), its `format`, `source`, `xa`, `cdda` and every file with size and CRC-32; it is its own record. The
/// main game's record and `quick_check` never look at it: without it, or with it broken, the game is the plain main
/// game.
pub use crate::platform::game_data::BR1_DIR;
pub const BR1_GAME: &str = "Bloody Roar (USA) SCUS-94199";
/// The formats of an imported install this build reads (2 adds `engine/tables.okt` and `content_id`; 3 the clips'
/// ranges and no program file: the engine reads only the tables; 4 the game's text and the default buttons).
pub const BR1_FORMAT_MIN: u32 = 4;
pub const BR1_FORMAT_MAX: u32 = 4;
/// The fighters (ok_br1_converter `runtime::FIGHTERS`) and stages (its `chars::FIGHTERS`' `XX00` and the two CMN
/// stages) of an install of the imported game.
pub const BR1_FIGHTERS: [&str; 4] = ["MIT", "FOX", "GRE", "URI"];
const BR1_STAGES: [&str; 11] = ["EVE00", "FOX00", "GAD00", "GRE00", "MIT00", "RON00", "URI00", "VAK00", "ZIN00", "BIG00", "SMA00"];

/// What the imported quick check found.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Br1Verdict {
    /// no `br1/manifest.json`: the plain main game
    Absent,
    /// complete, with its music and movie sound
    Ok,
    /// complete, but from an ISO image (no CD-XA music, no movie sound)
    NoMusic,
    /// unusable (why): the game runs as the plain main game and the setup screen says so
    Broken(String),
}

impl Br1Verdict {
    pub fn usable(&self) -> bool {
        matches!(self, Br1Verdict::Ok | Br1Verdict::NoMusic)
    }

    /// The setup screen's line about it.
    pub fn note(&self) -> String {
        match self {
            Br1Verdict::Absent => "Bloody Roar 1 disc: not added (optional).".into(),
            Br1Verdict::Ok => "Bloody Roar 1 disc: installed.".into(),
            Br1Verdict::NoMusic => "Bloody Roar 1 disc: installed (from an ISO image: no music, no movie sound).".into(),
            Br1Verdict::Broken(e) => format!("Bloody Roar 1 disc: damaged, not used ({e})."),
        }
    }
}

/// The files every install of the imported game must have: the static tables, the shared CMN files, the four fighters' ANM / meshes /
/// costumes / models, every stage's DAT and model, the endings' and the intro's frames, the staff rolls, the epilogue's
/// picture.
pub fn required_br1() -> Vec<String> {
    let mut v: Vec<String> =
        ["manifest.json", "engine/tables.okt", "engine/CMN/CMN.ACT", "engine/CMN/EFFECT.DAT", "engine/CMN/INIT.DAT"].map(String::from).into();
    for c in BR1_FIGHTERS {
        v.push(format!("engine/{c}/{c}.ANM"));
        // the fighter file the game loads (`fight::br1::res`; the ANM differs from it in 2 bytes)
        v.push(format!("engine/{c}/{c}31.DAT"));
        v.push(format!("engine/{c}/{c}20.DAT"));
        v.push(format!("engine/{c}/{c}10.DAT"));
        v.push(format!("models/{c}_human.glb"));
        v.push(format!("models/{c}_beast.glb"));
    }
    for s in BR1_STAGES {
        let dir = if s.starts_with("BIG") || s.starts_with("SMA") { "CMN" } else { &s[..3] };
        v.push(format!("engine/{dir}/{s}.DAT"));
        v.push(format!("stages/{s}.glb"));
    }
    v.extend(["BR", "FOX", "GREG", "MITUKO"].map(|m| format!("movies/{m}.bsf")));
    // (the import S11) the staff rolls and Uriko's epilogue picture the endings play
    v.extend(["movies/MITUKOST.bsf", "movies/ALICESTF.bsf", "pictures/ART1/ART124.TIM"].map(String::from));
    v
}

/// The quick check of an imported install: `manifest` the bytes of its `manifest.json` (None: no such file),
/// `size` a file's size under its folder (None: missing).
pub fn quick_check_br1(manifest: Option<&[u8]>, size: &dyn Fn(&str) -> Option<u64>) -> Br1Verdict {
    let Some(manifest) = manifest else { return Br1Verdict::Absent };
    let rec = match Record::parse(manifest) {
        None => return Br1Verdict::Broken("its manifest.json is not JSON".into()),
        Some(Err(e)) => return Br1Verdict::Broken(e),
        Some(Ok(r)) => r,
    };
    if !is_br1_manifest(manifest) {
        return Br1Verdict::Broken("its manifest.json is not of Bloody Roar (USA) data".into());
    }
    match rec.format {
        None => return Br1Verdict::Broken("its manifest.json has no format".into()),
        Some(v) if v < BR1_FORMAT_MIN => return Br1Verdict::Broken(format!("made by an older converter (format {v}): convert it again")),
        Some(v) if v > BR1_FORMAT_MAX => return Br1Verdict::Broken(format!("made by a newer version (format {v}): update the game")),
        _ => {}
    }
    // without it the install would drop out of sessions, replays and netplay unseen (`br1_install_id`)
    if br1_content_id(manifest).is_none() {
        return Br1Verdict::Broken("its manifest.json has no content id: convert it again".into());
    }
    let (mut missing, mut wrong_size) = (Vec::new(), Vec::new());
    for f in &rec.files {
        match size(&f.path) {
            None => missing.push(clip(&f.path, 120)),
            Some(s) if s != f.size => wrong_size.push(clip(&f.path, 120)),
            Some(_) => {}
        }
    }
    for p in required_br1() {
        if !rec.files.iter().any(|f| f.path == p) && size(&p).is_none() {
            missing.push(p);
        }
    }
    if !missing.is_empty() || !wrong_size.is_empty() {
        missing.truncate(SHOWN);
        wrong_size.truncate(SHOWN);
        return Br1Verdict::Broken(damage(&missing, &wrong_size));
    }
    if rec.xa { Br1Verdict::Ok } else { Br1Verdict::NoMusic }
}

/// The imported game's install's content id (engine addition, the import S7): its `manifest.json`'s `content_id`, which
/// its converter computed over the disc's files the simulation reads (CRC-32 of the sorted "path:crc32\n" lines of the
/// static tables, CMN.ACT and the nine `XX31.DAT`), carried by a session's start (a replay's header, the netplay Hello) so that a
/// replay or a peer with other data of the imported game is refused. None when the manifest does not parse or has no such
/// number of 1..=u32::MAX (logged: a converter's bug; 0 is the netplay Hello's "none").
pub fn br1_content_id(manifest: &[u8]) -> Option<u32> {
    let v: serde_json::Value = serde_json::from_slice(manifest).ok()?;
    id_of(v.get("content_id")?)
}

/// The game data's content id (engine addition: `Record::content_id`), read through `assets` (a folder, the browser's
/// OPFS folder or HTTP); Err asking for a new conversion when its manifest has none, Err when it cannot be read.
pub async fn install_id(assets: &crate::assets::Assets) -> Result<u32, String> {
    let m = assets.load("manifest.json").await?;
    if m.len() > MAX_JSON {
        return Err("manifest.json is too large".into());
    }
    match Record::parse(&m) {
        Some(Ok(Record { content_id: Some(id), .. })) => Ok(id),
        Some(Ok(_)) => Err("manifest.json has no content id (convert the game data again)".into()),
        Some(Err(e)) => Err(format!("manifest.json: {e}")),
        None => Err("manifest.json is not JSON".into()),
    }
}

/// The content id of the imported game's install beside `assets` (`Assets::br1`) when it is usable (`check_dir_br1`);
/// None without one (the browser: the one its start found, `Assets::set_br1_opfs`; none for data over HTTP).
pub fn br1_install_id(assets: &crate::assets::Assets) -> Option<u32> {
    #[cfg(not(target_arch = "wasm32"))]
    {
        let dir = assets.br1()?.root().to_path_buf();
        if !check_dir_br1(&dir).usable() {
            return None;
        }
        br1_content_id(&std::fs::read(dir.join("manifest.json")).ok()?)
    }
    #[cfg(target_arch = "wasm32")]
    {
        assets.br1_opfs_id()
    }
}

/// The manifest names the second disc (`game`): the second disc's data, not the main game's.
pub fn is_br1_manifest(json: &[u8]) -> bool {
    serde_json::from_slice::<serde_json::Value>(json).ok().and_then(|v| v.get("game")?.as_str().map(|g| g == BR1_GAME)).unwrap_or(false)
}

/// `quick_check_br1` of the folder `br1` on disk (`<data folder>/br1`; desktop, Android).
#[cfg(not(target_arch = "wasm32"))]
pub fn check_dir_br1(br1: &std::path::Path) -> Br1Verdict {
    let manifest = match read_json(&br1.join("manifest.json")) {
        Ok(m) => m,
        Err(e) => return Br1Verdict::Broken(e),
    };
    let size = |p: &str| std::fs::metadata(br1.join(p)).ok().filter(|m| m.is_file()).map(|m| m.len());
    quick_check_br1(manifest.as_deref(), &size)
}

/// `quick_check_br1` of the browser's imported-game folder `dir` in OPFS (None: none installed).
#[cfg(target_arch = "wasm32")]
pub async fn check_opfs_br1(dir: Option<&str>) -> Br1Verdict {
    use crate::platform::game_data::web;
    let Some(dir) = dir else { return Br1Verdict::Absent };
    let Ok(root) = web::root().await else { return Br1Verdict::Absent };
    let Ok(d) = web::subdir(&root, dir, false).await else { return Br1Verdict::Broken("its folder is gone".into()) };
    let m = match web::get(&d, "manifest.json").await {
        Ok(f) if f.size() <= MAX_JSON as f64 => web::read_blob(&f).await.ok(),
        _ => None,
    };
    let mut paths = required_br1();
    if let Some(Ok(rec)) = m.as_deref().and_then(Record::parse) {
        paths.extend(rec.files.into_iter().map(|f| f.path));
    }
    let mut sizes = std::collections::HashMap::new();
    for p in paths {
        if !sizes.contains_key(&p)
            && let Ok(f) = web::get(&d, &p).await
        {
            sizes.insert(p, f.size() as u64);
        }
    }
    quick_check_br1(m.as_deref(), &|p| sizes.get(p).copied())
}

/// The browser's install of the imported game (engine addition, the import S9): its OPFS folder (`web::BR1`) and content id,
/// when its quick check finds it usable (`Assets::set_br1_opfs` gives them to the game data's `Assets`).
#[cfg(target_arch = "wasm32")]
pub async fn opfs_br1_install() -> Option<(String, u32)> {
    let dir = crate::platform::game_data::web::BR1.pointer()?;
    let v = check_opfs_br1(Some(&dir)).await;
    if !v.usable() {
        log::info!("game data {dir}: the imported game {v:?}");
        return None;
    }
    let id = br1_content_id(&crate::platform::game_data::web::read(&dir, "manifest.json").await.ok()?)?;
    log::info!("game data {dir}: the imported game {v:?}, content id {id:08x}");
    Some((dir, id))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn br1_manifest(xa: bool, files: &[(&str, u64)]) -> String {
        let list: Vec<String> = files.iter().map(|(p, s)| format!(r#"{{"path":"{p}","size":{s},"crc32":1}}"#)).collect();
        format!(r#"{{"game":"{BR1_GAME}","format":4,"source":"bin","xa":{xa},"cdda":true,"content_id":1,"files":[{}]}}"#, list.join(","))
    }

    /// The files an ending plays are required of an install (S11); the tables are, the program file is not (format 3).
    #[test]
    fn br1_required_files() {
        let req = required_br1();
        for f in ["movies/MITUKOST.bsf", "movies/ALICESTF.bsf", "pictures/ART1/ART124.TIM", "engine/tables.okt"] {
            assert!(req.iter().any(|r| r == f), "{f}");
        }
        assert!(!req.iter().any(|r| r.contains("SCUS")));
    }

    /// The manifest's `content_id` is the id, whatever its files; none without it, for 0, one past 32 bits or not a number.
    /// The install beside the engine (data-gated) has the imported game USA disc's id.
    #[test]
    fn br1_content_id_of_the_manifest() {
        let m = |id: &str| format!(r#"{{"game":"{BR1_GAME}","format":2,"source":"bin","xa":true,"content_id":{id},"files":[]}}"#);
        assert_eq!(br1_content_id(m("2245824082").as_bytes()), Some(0x85DC_8E52));
        assert_eq!(br1_content_id(m("0").as_bytes()), None);
        assert_eq!(br1_content_id(m("4294967296").as_bytes()), None);
        assert_eq!(br1_content_id(m("\"85DC8E52\"").as_bytes()), None);
        assert_eq!(br1_content_id(br1_manifest(true, &[("engine/CMN/CMN.ACT", 1)]).replace(r#""content_id":1,"#, "").as_bytes()), None);
        assert_eq!(br1_content_id(b"no json"), None);
        let dir = std::env::var("BR1_EXPORT").unwrap_or_else(|_| concat!(env!("CARGO_MANIFEST_DIR"), "/../okb_export/br1").into());
        let Ok(b) = std::fs::read(std::path::Path::new(&dir).join("manifest.json")) else {
            eprintln!("skipped: no okb_export/br1");
            return;
        };
        assert_eq!(br1_content_id(&b), Some(0x85DC_8E52));
    }

    #[test]
    fn the_br1_install() {
        let mut files: HashMap<String, u64> = required_br1().into_iter().map(|p| (p, 10)).collect();
        let check = |m: Option<&str>, files: &HashMap<String, u64>| quick_check_br1(m.map(str::as_bytes), &|p| files.get(p).copied());
        assert_eq!(check(None, &files), Br1Verdict::Absent);
        assert!(!Br1Verdict::Absent.usable() && Br1Verdict::NoMusic.usable());
        files.insert("audio/music/BGM00_ch0.xa".into(), 99);
        let m = br1_manifest(true, &[("audio/music/BGM00_ch0.xa", 99), ("engine/tables.okt", 10)]);
        assert_eq!(check(Some(&m), &files), Br1Verdict::Ok);
        assert_eq!(check(Some(&br1_manifest(false, &[])), &files), Br1Verdict::NoMusic);
        // formats 1 to 3 (with the program file, without the clips' ranges, without the text) are not read: convert again
        for v in [1, 2, 3] {
            let Br1Verdict::Broken(why) = check(Some(&br1_manifest(false, &[]).replace("\"format\":4", &format!("\"format\":{v}"))), &files) else { panic!() };
            assert!(why.contains("convert it again"), "{why}");
        }
        // no content id (sessions, replays and netplay would leave the install out): convert again
        let Br1Verdict::Broken(why) = check(Some(&m.replace(r#""content_id":1,"#, "")), &files) else { panic!() };
        assert!(why.contains("content id") && why.contains("convert it again"), "{why}");
        // a listed file of another size, a required file missing
        files.insert("audio/music/BGM00_ch0.xa".into(), 98);
        files.remove("models/URI_beast.glb");
        let Br1Verdict::Broken(why) = check(Some(&m), &files) else { panic!() };
        assert!(why.contains("URI_beast") && why.contains("BGM00_ch0"), "{why}");
        // The main game's manifest, no format, another format, not JSON
        assert!(matches!(check(Some(r#"{"game":"Bloody Roar II (USA) SCUS-94424","format":1}"#), &files), Br1Verdict::Broken(_)));
        assert!(matches!(check(Some(&format!(r#"{{"game":"{BR1_GAME}"}}"#)), &files), Br1Verdict::Broken(_)));
        let newer = br1_manifest(true, &[]).replace("\"format\":4", &format!("\"format\":{}", BR1_FORMAT_MAX + 1));
        let Br1Verdict::Broken(why) = check(Some(&newer), &files) else { panic!() };
        assert!(why.contains("newer"), "{why}");
        assert!(matches!(check(Some("x"), &files), Br1Verdict::Broken(_)));
        assert!(Br1Verdict::Broken("x".into()).note().contains("not used"));
        // The main game's own check never needs it
        assert!(!required().iter().any(|p| p.starts_with(BR1_DIR)));
    }

    fn check(manifest: Option<&str>, files: &HashMap<String, u64>) -> Verdict {
        quick_check(manifest.map(str::as_bytes), &|p| files.get(p).copied())
    }

    fn full_set() -> HashMap<String, u64> {
        required().into_iter().map(|p| (p, 10)).collect()
    }

    #[test]
    fn missing_without_a_manifest() {
        assert_eq!(check(None, &HashMap::new()), Verdict::Missing);
        assert!(Verdict::Missing.reason().is_some() && !Verdict::Missing.usable());
    }

    /// An old Python export (no format) is refused, complete or not, with its own reason.
    #[test]
    fn legacy_export_is_refused() {
        let mut files = full_set();
        let v = check(Some(r#"{"game":"x","stages":[]}"#), &files);
        assert_eq!(v, Verdict::Legacy);
        assert!(!v.usable() && v.reason().unwrap().contains("made by the old converter: convert the game data again"));
        files.remove("characters/YUG_1P_human.glb");
        assert_eq!(check(Some("{}"), &files), Verdict::Legacy);
    }

    #[test]
    fn versions_out_of_range() {
        let files = full_set();
        assert_eq!(check(Some(r#"{"format":0}"#), &files), Verdict::TooOld(0));
        // format 1 (the program file, no `engine/tables.okt`): convert again
        assert_eq!(check(Some(r#"{"format":1}"#), &files), Verdict::TooOld(1));
        assert!(Verdict::TooOld(1).reason().unwrap().contains("convert it again"));
        assert_eq!(check(Some(&format!(r#"{{"format":{}}}"#, FORMAT_MAX + 1)), &files), Verdict::TooNew(FORMAT_MAX + 1));
        // format 2 (no text) is not read, format 3 (`engine/tables.okt` with the text) is
        assert_eq!(check(Some(r#"{"format":2}"#), &files), Verdict::TooOld(2));
        let m = r#"{"format":3,"source":"iso","xa":false,"content_id":7,"files":[{"path":"engine/INIT.DAT","size":10,"crc32":1}]}"#;
        assert_eq!(check(Some(m), &files), Verdict::Ok);
        assert!(matches!(check(Some(r#"{"format":"one"}"#), &files), Verdict::Broken(_)));
        assert!(matches!(check(Some("not json"), &files), Verdict::Broken(_)));
    }

    /// A record without a content id (a converter before it), or with one that is not a number of 1..=u32::MAX, asks for
    /// a new conversion; `install_id` reads the id through the assets, Err without it.
    #[test]
    fn content_id_needed() {
        let files = full_set();
        let m = |id: &str| format!(r#"{{"format":3,"source":"iso","xa":false{id},"files":[]}}"#);
        assert_eq!(check(Some(&m(r#","content_id":4294967295"#)), &files), Verdict::Ok);
        for bad in ["", r#","content_id":0"#, r#","content_id":4294967296"#, r#","content_id":"5970af0c""#, r#","content_id":-1"#] {
            let v = check(Some(&m(bad)), &files);
            assert_eq!(v, Verdict::NoContentId, "{bad}");
            assert!(v.reason().unwrap().contains("convert it again"));
        }
        let rec = |id: &str| Record::parse(m(id).as_bytes()).unwrap().unwrap();
        assert_eq!(rec(r#","content_id":1500574340"#).content_id, Some(1_500_574_340));
        assert_eq!(rec("").content_id, None);
        #[cfg(not(target_arch = "wasm32"))]
        {
            let d = std::env::temp_dir().join(format!("okb_manifest_id_{}", std::process::id()));
            std::fs::create_dir_all(&d).unwrap();
            let a = crate::assets::Assets::new(&d);
            let id = || pollster::block_on(install_id(&a));
            assert!(id().is_err());
            std::fs::write(d.join("manifest.json"), m(r#","content_id":9"#)).unwrap();
            assert_eq!(id(), Ok(9));
            std::fs::write(d.join("manifest.json"), m("")).unwrap();
            assert!(id().unwrap_err().contains("convert the game data again"));
            std::fs::remove_dir_all(&d).unwrap();
        }
        // the data beside the engine (data-gated): the converter's id
        let dir = crate::base::env::var("EXPORT").unwrap_or_else(|_| concat!(env!("CARGO_MANIFEST_DIR"), "/../okb_export").into());
        let Ok(b) = std::fs::read(std::path::Path::new(&dir).join("manifest.json")) else {
            eprintln!("skipped: no okb_export");
            return;
        };
        assert!(Record::parse(&b).unwrap().unwrap().content_id.is_some(), "{dir}: no content id (convert it again)");
    }

    #[test]
    fn listed_files_and_sizes() {
        let mut files = full_set();
        files.insert("audio/music/BR2_MC01_ch0.xa".into(), 4624);
        let m = r#"{"format":3,"source":"bin","xa":true,"content_id":7,"files":[
            {"path":"audio/music/BR2_MC01_ch0.xa","size":4624,"crc32":3421780262},
            {"path":"engine/INIT.DAT","size":10,"crc32":"cbf43926"}]}"#;
        assert_eq!(check(Some(m), &files), Verdict::Ok);
        files.insert("engine/INIT.DAT".into(), 11);
        files.remove("audio/music/BR2_MC01_ch0.xa");
        assert_eq!(
            check(Some(m), &files),
            Verdict::Damaged { missing: vec!["audio/music/BR2_MC01_ch0.xa".into()], wrong_size: vec!["engine/INIT.DAT".into()] }
        );
        // a path out of the folder is a broken record
        let bad = r#"{"format":3,"files":[{"path":"../x","size":1,"crc32":1}]}"#;
        assert!(matches!(check(Some(bad), &files), Verdict::Broken(_)));
    }

    #[test]
    fn iso_install_without_audio() {
        // xa:false: no .xa / .mp3 listed, none required
        let files = full_set();
        let m = r#"{"format":3,"source":"iso","xa":false,"content_id":7,"files":[{"path":"engine/INIT.DAT","size":10,"crc32":1}]}"#;
        assert_eq!(check(Some(m), &files), Verdict::Ok);
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn an_oversized_manifest_is_broken() {
        let d = std::env::temp_dir().join(format!("okb_manifest_big_{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        std::fs::File::create(d.join("manifest.json")).unwrap().set_len(MAX_JSON as u64 + 1).unwrap();
        assert!(matches!(check_dir(&d), Verdict::Broken(_)));
        std::fs::remove_dir_all(&d).unwrap();
        assert_eq!(check_dir(&d), Verdict::Missing);
    }

    #[test]
    fn safe_paths() {
        assert!(safe_path("engine/INIT.DAT"));
        for p in ["", "/etc/x", "a/../b", "a//b", "C:/x", "a\\b", "./a"] {
            assert!(!safe_path(p), "{p}");
        }
    }

    /// The converted data beside the repo (`../okb_export`, `OKB_EXPORT`; skipped without it) passes, and its
    /// imported game's install too.
    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn the_export_passes() {
        let root = crate::base::env::var("EXPORT").unwrap_or_else(|_| concat!(env!("CARGO_MANIFEST_DIR"), "/../okb_export").into());
        let root = std::path::Path::new(&root);
        if !root.join("manifest.json").exists() {
            eprintln!("skipped: no okb_export");
            return;
        }
        assert_eq!(check_dir(root), Verdict::Ok);
        // and the imported game's install beside it
        if root.join(BR1_DIR).join("manifest.json").exists() {
            assert_eq!(check_dir_br1(&root.join(BR1_DIR)), Br1Verdict::Ok);
        }
    }
}
