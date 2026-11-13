//! What the setup copies and how it describes a source (a folder, or a disc image the converter takes): shared by
//! the desktop / Android job (`job.rs`, `convert.rs`) and the browser's (`web.rs`, `worker.rs`) (engine addition).

use crate::assets::extras::{self, Q3Verdict};
use crate::assets::manifest::{self, Br1Verdict, Record, Verdict};
use std::path::Path;

/// What an install is for: the main game's data, or an optional one beside it, in its own folder of the data
/// folder with its own check and converter: the imported game's disc's (`assets::manifest::BR1_DIR`) or Quake III
/// Arena's maps (`assets::extras::Q3_DIR`, the extra stages; engine addition).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Kind {
    #[default]
    Br2,
    Br1,
    Q3,
}

impl Kind {
    /// The optional installs, in the order the setup's rows offer them.
    pub const EXTRAS: [Kind; 2] = [Kind::Br1, Kind::Q3];

    /// An optional install's folder in the data folder (`platform::game_data::Dirs::sub`); None for the main game's,
    /// which is the data folder itself.
    pub fn dir(self) -> Option<&'static str> {
        match self {
            Kind::Br2 => None,
            Kind::Br1 => Some(manifest::BR1_DIR),
            Kind::Q3 => Some(extras::Q3_DIR),
        }
    }

    /// What the setup's rows name (ADD …, REMOVE …).
    pub fn label(self) -> &'static str {
        match self {
            Kind::Br2 => "GAME DATA",
            Kind::Br1 => "BLOODY ROAR 1 DISC",
            Kind::Q3 => "QUAKE III ARENA MAPS",
        }
    }

    /// Its data in a sentence.
    pub fn data(self) -> &'static str {
        match self {
            Kind::Br2 => "game data",
            Kind::Br1 => "Bloody Roar 1 data",
            Kind::Q3 => "Quake III Arena maps",
        }
    }
}

/// What a Quake III copy takes: the files its manifest lists (with `extras::required_q3`), manifest.json last.
pub fn items_q3(manifest_json: &[u8]) -> Result<Vec<Item>, String> {
    items_extra(manifest_json, extras::required_q3())
}

/// A checked folder of converted Quake III maps (`v`, its manifest `json`) as the confirmation shows it, or why it
/// cannot be used. A folder of okconv-q3's own command line output (no `game` in its manifest) is refused: its layout
/// is the same, but nothing says which files it has.
pub fn describe_q3(v: &Q3Verdict, json: Option<&[u8]>, name: &str) -> Result<(String, Option<String>), String> {
    if json.is_some_and(|j| !extras::is_q3_manifest(j)) {
        return Err(other_q3(name));
    }
    match v {
        Q3Verdict::Absent => return Err(format!("No converted Quake III Arena maps in {name} (no manifest.json of Quake III Arena).")),
        Q3Verdict::Broken(e) => return Err(format!("These Quake III Arena maps cannot be used: {e}.")),
        Q3Verdict::Ok => {}
    }
    let maps = json.and_then(|j| serde_json::from_slice::<serde_json::Value>(j).ok()).and_then(|m| m["maps"].as_array().map(Vec::len)).unwrap_or(0);
    let mask = json.map_or(0, extras::listed);
    Ok((format!("Converted Quake III Arena maps ({}).", count(maps, "map")), no_stage(mask)))
}

/// A folder whose manifest is not this game's Quake III data (okconv-q3's own command line output, other data).
pub fn other_q3(name: &str) -> String {
    format!("{name} has no converted Quake III data of this game (okconv-q3's own output?): convert Quake III Arena's .pk3 files instead (its baseq3 folder).")
}

/// `n` things ("1 map", "36 maps").
fn count(n: usize, what: &str) -> String {
    if n == 1 { format!("1 {what}") } else { format!("{n} {what}s") }
}

/// The warning when none of the maps is an extra stage (`mask`: `extras::listed`).
fn no_stage(mask: u8) -> Option<String> {
    (mask == 0).then(|| "None of its maps is an extra stage (Q3DM17 The Longest Yard, Q3DM6 The Camping Grounds): no stage is added.".to_string())
}

/// About what a conversion of Quake III Arena's paks writes (the real `baseq3` paks: 272.8 MB in 1406 files, every
/// map with its images; ok_q3_converter's engine data).
#[cfg(feature = "converter")]
pub const Q3_BYTES: u64 = 273_200_000;

/// Quake III Arena's `.pk3` files the converter opened (`paks` of them holding the maps `maps`), as the confirmation
/// shows them: what it is, a warning, about what the conversion writes.
#[cfg(feature = "converter")]
pub fn q3_found(paks: usize, maps: &[String]) -> (String, Option<String>, u64) {
    let mask = extras::STAGES.iter().enumerate().filter(|(_, f)| maps.iter().any(|m| f.map.ends_with(&format!("/{m}.glb")))).fold(0, |m, (k, _)| m | 1 << k);
    let what = format!("Quake III Arena's .pk3 files ({}, {}), converted here.", count(paks, "pak"), count(maps.len(), "map"));
    (what, no_stage(mask), Q3_BYTES)
}

/// What an optional install's copy takes: the files its manifest lists (they must hold `required`, e.g.
/// `manifest::required_br1`), manifest.json last.
pub fn items_extra(manifest_json: &[u8], required: Vec<String>) -> Result<Vec<Item>, String> {
    let rec = match Record::parse(manifest_json) {
        Some(Ok(r)) => r,
        Some(Err(e)) => return Err(e),
        None => return Err("manifest.json is not JSON".into()),
    };
    if let Some(p) = required.into_iter().find(|p| p != "manifest.json" && !rec.files.iter().any(|f| &f.path == p)) {
        return Err(format!("its manifest.json does not list {p}"));
    }
    let mut items: Vec<Item> =
        rec.files.iter().filter(|f| f.path != "manifest.json").map(|f| Item { path: f.path.clone(), size: f.size, crc32: Some(f.crc32) }).collect();
    items.push(Item { path: "manifest.json".into(), size: manifest_json.len() as u64, crc32: None });
    Ok(items)
}

/// A checked folder of the imported game as the confirmation shows it, or why it cannot be used.
pub fn describe_br1(v: &Br1Verdict, rec: Option<&Record>, name: &str) -> Result<(String, Option<String>), String> {
    match v {
        Br1Verdict::Absent => return Err(format!("No converted Bloody Roar 1 data in {name} (no manifest.json of Bloody Roar (USA)).")),
        Br1Verdict::Broken(e) => return Err(format!("This Bloody Roar 1 data cannot be used: {e}.")),
        Br1Verdict::Ok | Br1Verdict::NoMusic => {}
    }
    let from = match rec.map(|r| r.source.as_str()) {
        Some("bin") => " from a BIN/CUE disc image",
        Some("chd") => " from a CHD disc image",
        Some("iso") => " from an ISO disc image",
        _ => "",
    };
    let warn = (*v == Br1Verdict::NoMusic).then(|| "It has no music and no movie sound (an ISO image has no CD-XA audio).".to_string());
    Ok((format!("Converted Bloody Roar 1 data, format {}{from}.", rec.and_then(|r| r.format).unwrap_or(0)), warn))
}

/// One file to copy: its path, size and, from a converter's manifest, the CRC-32 it must have.
#[derive(Clone, Debug)]
pub struct Item {
    pub path: String,
    pub size: u64,
    pub crc32: Option<u32>,
}

/// What a copy takes, from the manifest's bytes: the converter's listed files, manifest.json last; and the manifest's
/// record (an old export, without a format, is refused: `Verdict::Legacy`).
pub fn items(manifest_json: &[u8]) -> Result<(Vec<Item>, Record), String> {
    let rec = match Record::parse(manifest_json) {
        Some(Ok(r)) => r,
        Some(Err(e)) => return Err(e),
        None => return Err("manifest.json is not JSON".into()),
    };
    if rec.format.is_none() {
        return Err(Verdict::Legacy.reason().unwrap_or_default());
    }
    // the converter's list must hold the engine's required set (else the copy would end missing it)
    if let Some(p) = manifest::required().into_iter().find(|p| p != "manifest.json" && !rec.files.iter().any(|f| &f.path == p)) {
        return Err(format!("its manifest.json does not list {p}"));
    }
    let mut items: Vec<Item> =
        rec.files.iter().filter(|f| f.path != "manifest.json").map(|f| Item { path: f.path.clone(), size: f.size, crc32: Some(f.crc32) }).collect();
    items.push(Item { path: "manifest.json".into(), size: manifest_json.len() as u64, crc32: None });
    Ok((items, rec))
}

/// A checked source as the confirmation shows it (what it is, a warning), or why it cannot be used (`name`: the
/// source as the player named it).
pub fn describe(v: &Verdict, rec: Option<&Record>, name: &str) -> Result<(String, Option<String>), String> {
    match v {
        Verdict::Missing => return Err(format!("No converted game data in {name} (no manifest.json).")),
        Verdict::Ok => {}
        v => return Err(format!("This game data cannot be used. {}", v.reason().unwrap_or_default())),
    }
    let Some(r) = rec else { return Ok(("Converted game data.".into(), None)) };
    let from = match r.source.as_str() {
        "bin" => " from a BIN/CUE disc image",
        "chd" => " from a CHD disc image",
        "iso" => " from an ISO disc image",
        _ => "",
    };
    let warn = (!r.xa).then(|| "It has no music and no movie sound (an ISO image has no CD-XA audio).".to_string());
    Ok((format!("Converted game data, format {}{from}.", r.format.unwrap_or(0)), warn))
}

/// Why a job ended without installing.
#[derive(Debug, PartialEq, Eq)]
pub enum Failure {
    Cancelled,
    /// the disk is full (writing this file)
    NoSpace(String),
    /// the copy's read-back did not match (this file)
    Verify(String),
    Io(String),
}

impl Failure {
    pub fn message(&self) -> String {
        match self {
            Failure::Cancelled => "Cancelled: the game data was not changed.".into(),
            Failure::NoSpace(f) => format!("Not enough free space (writing {f}): free some space and try again."),
            Failure::Verify(f) => format!("The installed game data could not be verified ({f} differs): try again, or use another source."),
            Failure::Io(e) => format!("The install failed: {e}"),
        }
    }
}

/// A disc image the converter opened, as the confirmation shows it (`source`: `bin`, `chd` or `iso`): what it is,
/// a warning, and about what the conversion writes (the space line; an ISO has no music).
#[cfg(feature = "converter")]
pub fn disc_found(source: &str) -> (&'static str, Option<String>, u64) {
    let what = match source {
        "chd" => "A CHD disc image of Bloody Roar II (USA), converted here.",
        "iso" => "An ISO disc image of Bloody Roar II (USA), converted here.",
        _ => "A BIN/CUE disc image of Bloody Roar II (USA), converted here.",
    };
    let iso = source == "iso";
    let warn = iso.then(|| "No music and no movie sound: an ISO image has no CD-XA audio (a BIN/CUE or CHD image has).".to_string());
    (what, warn, if iso { 300 << 20 } else { 420 << 20 })
}

/// An imported image the converter opened (`source`; `cdda`: its CD audio track is there), as the confirmation
/// shows it: what it is, a warning, about what the conversion writes.
#[cfg(feature = "converter")]
pub fn br1_disc_found(source: &str, cdda: bool) -> (&'static str, Option<String>, u64) {
    let what = match source {
        "chd" => "A CHD disc image of Bloody Roar (USA), converted here.",
        "iso" => "An ISO disc image of Bloody Roar (USA), converted here.",
        _ => "A BIN/CUE disc image of Bloody Roar (USA), converted here.",
    };
    let warn = if source == "iso" {
        Some("No music and no movie sound: an ISO image has no CD-XA audio (a BIN/CUE or CHD image has).".to_string())
    } else if !cdda {
        Some(if cfg!(target_os = "android") {
            "No CD audio track (Track 2) with it: Android opens only the chosen file; a CHD image of the disc has it.".to_string()
        } else {
            "No CD audio track (Track 2) with it: choose the .cue with both .bin files (or a CHD) to have it.".to_string()
        })
    } else {
        None
    };
    (what, warn, if source == "iso" { 210 << 20 } else { 440 << 20 })
}

/// What the player is told about an image the imported converter refuses.
#[cfg(feature = "converter")]
pub fn br1_disc_error(e: super::imports::br1::Error) -> String {
    use super::imports::br1::Error;
    match e {
        Error::WrongDisc { found } if found.contains("SCUS_94424") => "This is the Bloody Roar II disc: choose the Bloody Roar 1 (USA) disc here.".into(),
        Error::WrongDisc { found } => format!("This is not Bloody Roar (USA, SCUS-94199): found {found}."),
        Error::NotRaw2352(e) => format!("The image's data track has no raw 2352-byte sectors ({e}): use the BIN/CUE (or CHD) made from the disc."),
        Error::Io(e) => format!("The image cannot be read: {e}."),
        e => format!("Not a usable disc image: {e}."),
    }
}

/// What the player is told about Quake III Arena's paks the converter refuses.
#[cfg(feature = "converter")]
pub fn q3_error(e: super::imports::q3::Error) -> String {
    use super::imports::q3::Error;
    match e {
        Error::NoMaps => "These .pk3 files hold no maps: choose Quake III Arena's baseq3 folder.".into(),
        Error::Io(e) => format!("The .pk3 files cannot be read: {e}."),
        e => format!("Not usable .pk3 files: {e}."),
    }
}

/// What the player is told about an image the converter refuses.
#[cfg(feature = "converter")]
pub fn disc_error(e: ok_br2_converter::DiscError) -> String {
    use ok_br2_converter::DiscError;
    match e {
        DiscError::WrongDisc { found } if found.contains("SCUS_94199") => {
            "This is the Bloody Roar 1 disc: add it with ADD BLOODY ROAR 1 DISC once the game's data is installed (on the page after its install, or in OPTIONS, CHANGE GAME DATA).".into()
        }
        DiscError::WrongDisc { found } => format!("This is not Bloody Roar II (USA, SCUS-94424): found {found}."),
        DiscError::NotRaw2352(e) => format!("The image's data track has no raw 2352-byte sectors ({e}): use the BIN/CUE (or CHD) made from the disc."),
        DiscError::Format(e) => format!("Not a usable disc image: {e}."),
        DiscError::Io(e) => format!("The image cannot be read: {e}."),
    }
}

/// A test's (or OKB_SETUP_SLOW's) hooks into `copy_into` and `convert_into`.
#[derive(Default)]
#[cfg_attr(target_arch = "wasm32", allow(dead_code))]
pub struct Hooks {
    /// a pause after each copied file, or each conversion step and checked file (milliseconds; screenshots of the
    /// progress)
    pub slow_ms: u64,
    /// called with the `.part` folder before the full check (a test corrupts a byte there)
    pub before_verify: Option<Box<dyn Fn(&Path) + Send>>,
}
