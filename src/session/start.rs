//! What a session starts from: `SessionStart`, `SIM_VERSION`, the fetched files.
//!
//! State: `SessionStart`: a replay's header (CBOR); `FightDebug` is part of `GameState`
//!
//! What a session starts from: `SessionStart` (where it starts, the debug setup of its fights), the simulation
//! version it runs (`SIM_VERSION`) and a direct fight's data for the newest one (`load`), and the fetched files as
//! the logic sees them (`FileCache`, `Cached`). `frontend` may use it (ARCHITECTURE.md, Layers, rule 1).

use crate::assets::Assets;
use crate::fight::fight_data::{Loaded, direct_costume, load_match};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeSet, HashMap};
use std::sync::Arc;

/// The simulation's version, its changelog and the rules each version introduced: `fight::version`.
pub use crate::fight::version::SIM_VERSION;

/// The headless tools' SIM_VERSION=n (sim, fetest): None when unset, Err unless n is a version this build runs
/// (1..=`SIM_VERSION`).
pub fn sim_version_env() -> Result<Option<u32>, String> {
    let Ok(v) = std::env::var("SIM_VERSION") else { return Ok(None) };
    parse_sim_version("SIM_VERSION", &v).map(Some)
}

/// The value `v` of variable `k` as a simulation version 1..=`SIM_VERSION` (`sim_version_env`, the app's
/// OKB_SIM_VERSION).
pub fn parse_sim_version(k: &str, v: &str) -> Result<u32, String> {
    match v.parse() {
        Ok(n) if (1..=SIM_VERSION).contains(&n) => Ok(n),
        _ => Err(format!("{k}={v}: not a simulation version 1..={SIM_VERSION}")),
    }
}

/// Err unless the fight data `d` was built for simulation version `v` (its clips and skeleton math:
/// `Loaded::portable`).
pub(super) fn data_fits(d: &Loaded, v: u32) -> Result<(), String> {
    if d.portable() == crate::fight::version::Gates::of(v).portable_math {
        Ok(())
    } else {
        Err(format!("data for simulation version {}, the state runs {v}", d.sim_version()))
    }
}

/// A direct fight's data for the newest simulation version.
pub async fn load(assets: &Assets, p1: &str, p2: &str, stage: &str) -> Result<Loaded, String> {
    load_match(assets, [p1, p2], direct_costume(), stage, false, crate::session::SIM_VERSION).await
}

/// Everything a run starts from; a replay file's header holds it (CBOR: a field added later
/// takes its default in older files).
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct SessionStart {
    /// the simulation version the session runs (`SIM_VERSION`; a replay's recorded one); a replay file carries it
    /// in its envelope, so it is not part of the header's CBOR
    #[serde(skip)]
    pub sim_version: u32,
    /// the front end's random seed (the clock; debug OKB_SEED=n)
    pub seed: u64,
    /// the stored save file (save.json's bytes) put into MEMORY CARD SLOT 1, `None` = no file; copied by
    /// `Front::new` (the card then lives in `Front::save`)
    #[serde(with = "crate::base::serde_util::opt_blob")]
    pub card: Option<Vec<u8>>,
    /// debug: everything selectable (characters, fourth costumes, movies, pictures) without touching the save: F11
    /// (`Cheat::Unlock`) does it during a session; set at the start by `--unlock` / OKB_UNLOCK / `?unlock=1` up to
    /// simulation v4 (now no-ops: the characters and costumes are open by default) and by fetest's FE_UNLOCK
    pub unlock: bool,
    /// --locked / OKB_LOCKED / `?locked=1`: the game's own progression, the characters and fourth costumes the save
    /// unlocked, and its nine stages; otherwise (from simulation v5) all characters and costumes are selectable and
    /// the stage select has ST9..STC too, the save untouched. A file without it reads false, and runs as recorded:
    /// before v5 the game's own is the only one (`frontend::Save::all_open`)
    pub locked: bool,
    /// where the run starts
    pub entry: Entry,
    /// debug BR2_MCFAULT: a forced memory card result
    pub card_fault: Option<crate::frontend::memcard::Fault>,
    /// the debug setup of every fight of the session (`Fight::new`)
    pub fight: FightDebug,
    /// the browser build's menus (engine additions: no EXIT GAME, no WINDOW SIZE / VSYNC / FRAME CAP): the session's,
    /// not the running build's, so a replay or a netplay peer of the other kind runs the same menu rows (stage 7;
    /// a file without it takes the running build's, as it was recorded before)
    pub browser: bool,
    /// NETWORK on the mode menu (engine addition: the app's NETWORK menu, `frontend::screen::mode_rows`): set by the
    /// app for the sessions it starts when its build can play over a network (and so for fetest, which takes the
    /// app's start); a file without it (every replay before the menu) reads false and runs the rows it was recorded
    /// with; from simulation v8 (`mode_rows`)
    #[serde(default)]
    pub network: bool,
    /// the Android build's menus (engine addition: GRAPHICS without WINDOW, WINDOW SIZE, VSYNC and FRAME CAP): set by
    /// the app on Android, the session's as `browser` is (a file without it, every one recorded elsewhere, reads
    /// false: the rows it was recorded with)
    #[serde(default)]
    pub android: bool,
    /// the extra stage Q3DM17 on the stage select (engine addition, `fight::stage::extra`, from simulation v10): set by
    /// the app when its game data has the stage's files (`assets::extras`), in netplay only when both players have
    /// them (the joiner's Hello says so); a file without it (every replay before the stage) reads false and runs the
    /// rows it was recorded with. A session that picks the stage draws its map from the files (`fight_data::Loaded::arena`;
    /// without them the fight is the same, drawn without the map)
    #[serde(default)]
    pub extra_stage: bool,
    /// the extra stage Q3DM6 on the stage select (engine addition, `fight::stage::extra::Q3DM6`, from simulation v12): as
    /// `extra_stage` for its own files (a file without it reads false)
    #[serde(default)]
    pub extra_q3dm6: bool,
    /// The imported game's install's content id (engine addition, the import S7: `assets::manifest::br1_content_id`):
    /// set by the app when its game data has a usable one, in netplay only when both players have the same; a
    /// replay that has one plays only over the same data of the imported game (`replay::br1_fits`); a file without it reads
    /// None (every replay before the import)
    #[serde(default)]
    pub br1: Option<u32>,
    /// the game data's content id (engine addition: `assets::manifest::install_id`, the converter's id of the data the
    /// simulation reads): set by the app for the sessions it starts; a netplay peer whose data has another one is
    /// refused (`net::handshake::DATA_DIFFERS`), a replay recorded over other data plays with a warning
    /// (`replay::content_warning`). A file without it (every replay before it, the tools' starts) reads None, and it is
    /// not written when None: such a start keeps the bytes of the builds before
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content: Option<u32>,
    /// a netplay session (engine addition, simulation v22: set by the app's `netplay::connect` for both peers): no TEAM
    /// BATTLE on the mode menu (four pads are not sent; `frontend::screen::mode_rows`); a file without it reads false
    /// (not written when false: a local session's start keeps the bytes of the builds before)
    #[serde(default, skip_serializing_if = "is_false")]
    pub netplay: bool,
}

fn is_false(b: &bool) -> bool {
    !*b
}

impl Default for SessionStart {
    fn default() -> Self {
        Self {
            sim_version: SIM_VERSION,
            seed: 0,
            card: None,
            unlock: false,
            locked: false,
            entry: Entry::default(),
            card_fault: None,
            fight: FightDebug::default(),
            browser: browser_build(),
            network: false,
            android: false,
            extra_stage: false,
            extra_q3dm6: false,
            br1: None,
            content: None,
            netplay: false,
        }
    }
}

impl SessionStart {
    /// The extra stages on the stage select (bit k = `fight::stage::extra::STAGES[k]`: `extra_stage`, `extra_q3dm6`).
    pub fn extras(&self) -> u8 {
        self.extra_stage as u8 | (self.extra_q3dm6 as u8) << 1
    }

    /// Sets `extra_stage` and `extra_q3dm6` from `extras` (bits as `extras` gives them; others ignored).
    pub fn set_extras(&mut self, extras: u8) {
        self.extra_stage = extras & 1 != 0;
        self.extra_q3dm6 = extras & 2 != 0;
    }
}

/// This is the browser build (wasm32): the default of `SessionStart::browser`.
pub(crate) fn browser_build() -> bool {
    cfg!(target_arch = "wasm32")
}

/// Where a session starts.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Entry {
    /// main mode 3: the logos and the opening movie (a plain start; always in the browser)
    #[default]
    Intro,
    /// the title screen (scripted test runs: OKB_FESCRIPT, OKB_SCRIPT with the menus, OKB_NOINTRO)
    Title,
    /// --fight / OKB_SCRIPT without OKB_FESCRIPT / `?fight=1`: this match at once, no front end
    DirectFight { p1: String, p2: String, stage: String },
    /// debug OKB_NAMETEST=mode:value: NAME ENTRY for YUGO, mode 0 = arcade (clear time in frames), 3 = survival
    /// (wins); over the boot screen (`intro`: the logos, else the title)
    NameTest { intro: bool, mode: i32, value: i32 },
    /// debug OKB_EPILOGUE=c: character c's arcade epilogue (then the ending movie), over the boot screen
    Epilogue { intro: bool, chr: usize },
    /// VS's character select at once, both players human (engine addition: a netplay session started from the
    /// NETWORK menu)
    Versus,
    /// debug OKB_TEAM_ARCADE=n (engine addition, TEAM ARCADE A4, simulation v26): a pad-1 TEAM ARCADE run at stage n's
    /// (1 ..) loading screen, the humans' fighters `chr` (P1, P3), the partner pad 2 (`partner`) or a CPU
    TeamArcade { stage: u8, chr: [u8; 2], partner: bool },
}

/// Debug setup applied to every fight of the session as it starts (front-end fights then take the front end's
/// sides and rules over it).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct FightDebug {
    /// OKB_P2=human: player 2 is not the CPU (C toggles it in game)
    pub p2_human: bool,
    /// OKB_WATCH: CPU vs CPU
    pub watch: bool,
    /// OKB_MODEL=n (+ OKB_CFLAGS=hex, OKB_PRACTICE): CUSTOM rules with MODEL TYPE n, the custom flags, PRACTICE
    pub custom: Option<CustomTest>,
    /// OKB_CONT: the continue screen after a lost direct fight
    pub cont: bool,
    /// OKB_GAUGE=n: P1 starts with this beast gauge
    pub gauge: Option<i16>,
    /// OKB_P2DMG=d: P2 starts the first round with this much damage (a quick KO)
    pub p2dmg: Option<i16>,
}

/// OKB_MODEL's CUSTOM rules.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CustomTest {
    /// MODEL TYPE 1 big head, 2 kids, 3 big arms
    pub model: u8,
    pub flags: u32,
    pub practice: bool,
}

/// Fetched files by asset path (`frontend::Event::Fetch`), held by the app's `AssetStore`; shared with the state's
/// users (the movie playing), never copied.
pub type FileCache = HashMap<String, Arc<[u8]>>;

/// The story cache as the logic sees it: the files it holds (`names`, state) with their bytes from the app's store.
pub struct Cached<'a> {
    pub names: &'a BTreeSet<String>,
    pub files: &'a FileCache,
}

impl<'a> Cached<'a> {
    pub fn get(&self, name: &str) -> Option<&'a Arc<[u8]>> {
        if self.names.contains(name) { self.files.get(name) } else { None }
    }
}

/// `SIM_VERSION` (a serde default).
pub(crate) fn sim_version_now() -> u32 {
    SIM_VERSION
}
