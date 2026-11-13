//! The whole logic state (`GameState`) and a step's input, output and static data.
//!
//! State: serialised: `GameState` itself (field changes are `STATE_VERSION` changes)
//!
//! The whole logic state (`GameState`) and what crosses a step's boundary: its input (`FrameInput`, the debug
//! `Cheat`s), its output (`Out`, `Request`) and the static data it reads (`Assets`, the app's `AssetStore`).

use super::fight::Fight;
use super::start::{FightDebug, FileCache, SIM_VERSION, browser_build};
use crate::fight::fight_data::Loaded;
use crate::frontend::{Front, FrontData, MatchSpec};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

/// Everything `step` reads and writes. The memory card is the front end's
/// (`frontend::Save::memory_card`; a direct fight has none). Static data is not part of it (skipped by serde, put
/// back by `rebind`); `save_state` / `load_state` / `hash` work on its postcard bytes.
#[derive(Clone, Serialize, Deserialize)]
pub struct GameState {
    /// steps since the session started
    pub frame: u32,
    /// the front end (None in a direct --fight session)
    pub front: Option<Front>,
    pub fight: Option<Fight>,
    /// the attract demo's fight kept for its stage behind the attract ranking (drawn, never stepped)
    pub backdrop: Option<Fight>,
    /// the music playing (None: stopped); its PCM is the app's, which seeks to `Music::frames` after a load
    pub music: Option<Music>,
    /// the XA volume set by the logic (0x7F full, the pause's 0x30; added after save-state file format 1's first
    /// files, which load with the default)
    #[serde(default = "full_volume")]
    pub music_volume: u8,
    /// the match last asked for (`frontend::Event::Load`): its sides
    pub spec: Option<MatchSpec>,
    /// that match is loaded and waits for `frontend::Event::Fight`
    pub pending: bool,
    /// files asked for and not fetched yet (`FrameInput::FETCHED` hands them to `Front::fetched`); one fetch is in
    /// flight at a time: the front end waits on the screen that asked (a movie, a picture, a story page) and asks
    /// again only after `fetched`, so a new `Event::Fetch` never overtakes a pending one
    pub fetching: Option<Vec<String>>,
    /// the debug setup of every fight of the session (`SessionStart::fight`)
    pub debug: FightDebug,
    /// the simulation version this session runs (`SIM_VERSION`, a replay's recorded one): logic that changed
    /// branches on its rules (`fight::version::Gates::of(s.sim_version)`); save-state files (CBOR) written before it existed load as
    /// version 1 (postcard snapshots are refused by `STATE_VERSION` instead)
    #[serde(default = "first_sim_version")]
    pub sim_version: u32,
    /// the browser build's menus (`SessionStart::browser`)
    #[serde(default = "browser_build")]
    pub browser: bool,
    /// NETWORK on the mode menu (`SessionStart::network`)
    #[serde(default)]
    pub network: bool,
    /// the Android build's menus (`SessionStart::android`)
    #[serde(default)]
    pub android: bool,
    /// the extra stage Q3DM17 on the stage select (`SessionStart::extra_stage`)
    #[serde(default)]
    pub extra_stage: bool,
    /// the extra stage Q3DM6 on the stage select (`SessionStart::extra_q3dm6`)
    #[serde(default)]
    pub extra_q3dm6: bool,
    /// the session has the imported game's data (`SessionStart::br1` is some): the front end offers its fighters and stages
    /// (engine addition, the import S9, from simulation v19)
    #[serde(default)]
    pub br1: bool,
    /// the team battle (engine addition, simulation v22): four fighters on one stage, stepped in
    /// place of `fight` (its data in `Assets::fight`); added after save-state file format 1's first files
    #[serde(default)]
    pub team: Option<Box<crate::fight::team::TeamFight>>,
    /// the team battle last asked for (`frontend::Event::LoadTeam`, engine addition): its sides; `pending` says it is
    /// loaded (`spec` is None meanwhile)
    #[serde(default)]
    pub team_spec: Option<crate::frontend::TeamSpec>,
    /// a netplay session (`SessionStart::netplay`): no TEAM BATTLE on the mode menu
    #[serde(default)]
    pub netplay: bool,
    /// the team battle's pause → CONTROLLER CONFIG. (engine addition, T6, simulation v24: `TeamConfig`); added after
    /// save-state file format 1's first files
    #[serde(default)]
    pub team_config: Option<TeamConfig>,
}

impl GameState {
    /// The extra stages on the stage select (bit k = `fight::stage::extra::STAGES[k]`, as `SessionStart::extras`).
    pub fn extras(&self) -> u8 {
        self.extra_stage as u8 | (self.extra_q3dm6 as u8) << 1
    }
}

fn first_sim_version() -> u32 {
    1
}

impl Default for GameState {
    fn default() -> Self {
        Self {
            frame: 0,
            front: None,
            fight: None,
            backdrop: None,
            music: None,
            music_volume: full_volume(),
            spec: None,
            pending: false,
            fetching: None,
            debug: FightDebug::default(),
            sim_version: SIM_VERSION,
            browser: browser_build(),
            network: false,
            android: false,
            extra_stage: false,
            extra_q3dm6: false,
            br1: false,
            team: None,
            team_spec: None,
            netplay: false,
            team_config: None,
        }
    }
}

/// The button-mapping screen over a paused team battle (engine addition, T6: as a match's `Fight::config`): the
/// menu, driven by pads 1 / 2 (it edits P1 / P2's maps; pads 3 / 4 keep the default map), and their edges.
#[derive(Clone, Serialize, Deserialize)]
pub struct TeamConfig {
    pub menu: crate::frontend::menus::Menu,
    pub pads: crate::frontend::Pads,
}

/// The music playing: an XA track, from the fight's data, the front end's or loaded by the app
/// (`Request::Music`), and how far in it is in logic time.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Music {
    pub track: u8,
    pub looped: bool,
    /// steps since it started (a track that is not looped is silent past its end)
    pub frames: u32,
}

/// One frame's input.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct FrameInput {
    /// the pads as read (keyboard, gamepads)
    pub pad: [u16; 2],
    /// the scripted pads (debug OKB_SCRIPT / OKB_SCRIPT2 in a fight, OKB_FESCRIPT in the front end), where
    /// `flags` has `FORCED[i]`
    pub forced: [u16; 2],
    pub flags: u8,
    /// pads 3 / 4 as read (gamepads only; engine addition, the team battle, simulation v22: only the team battle's
    /// screens and fight read them)
    pub ext: [u16; 2],
    /// the scripted pads 3 / 4 (debug OKB_SCRIPT3 / OKB_SCRIPT4, OKB_FESCRIPT3 / OKB_FESCRIPT4), where `flags` has
    /// `FORCED_EXT[i]`
    pub forced_ext: [u16; 2],
}

impl FrameInput {
    /// pad i is scripted: in a fight `forced[i]` drives fighter i after the CPU masking (the CPU's side too; pad 1's
    /// also the WATCH display pad), in the front end it replaces the pad
    pub const FORCED: [u8; 2] = [1, 2];
    /// pad 3 / 4 is scripted (`forced_ext`; replay FORMAT 2 only)
    pub const FORCED_EXT: [u8; 2] = [0x40, 0x80];
    /// the match asked for is in the app's store (`Front::loaded`)
    pub const LOADED: u8 = 4;
    /// the files asked for are in the app's store (`Front::fetched`)
    pub const FETCHED: u8 = 8;
    /// the app stored the card written by the last step; `CARD_FAILED`: that failed (`Card::stored`)
    pub const CARD_STORED: u8 = 0x10;
    pub const CARD_FAILED: u8 = 0x20;
    /// every flag above but `FORCED_EXT` (a FORMAT 1 replay's input stream holds no other bits)
    pub const ALL: u8 = 0x3F;

    pub fn forced(&self, i: usize) -> Option<u16> {
        (self.flags & Self::FORCED[i] != 0).then_some(self.forced[i])
    }

    /// Pad `k` of four (0 / 1 `pad`, 2 / 3 `ext`), the scripted one where forced.
    pub fn pad4(&self, k: usize) -> u16 {
        match k {
            0 | 1 => self.forced(k).unwrap_or(self.pad[k]),
            _ if self.flags & Self::FORCED_EXT[k - 2] != 0 => self.forced_ext[k - 2],
            _ => self.ext[k - 2],
        }
    }

    /// The input uses pads 3 / 4 (a replay needs FORMAT 2 for it).
    pub fn uses_ext(&self) -> bool {
        self.ext != [0; 2] || self.forced_ext != [0; 2] || self.flags & !Self::ALL != 0
    }
}

/// A debug mutation of the state (engine additions), applied by `step`: the key ones at its start, the env ones where
/// the app applied them before. Replays record them as sparse `(frame, Cheat)` events.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Cheat {
    /// F2 / F3: fighter i at 1 HP (the next hit is a KO)
    OneHp(usize),
    /// F4: the round timer down to 1 s
    Timer,
    /// F6: both beast gauges full
    Gauges,
    /// `]` / `[`: P1's next / previous move
    NextMove,
    PrevMove,
    /// B / N: fighter i's beast form toggled
    Beast(usize),
    /// C: player 2 human / CPU
    Cpu,
    /// F11: every character and costume selectable
    Unlock,
    /// OKB_P2DMG=d@frame (at that fight frame): P2's damage set to d, after the scripted pads
    P2Dmg(i16),
    /// OKB_FORCEMOVE=frame:move (at that fight frame): P1's move request, before the fight's tick
    ForceMove(i16),
    /// OKB_BREAK=side (at fight frame 330): that wall broken as on a match-deciding KO, before the fight's tick
    Break(usize),
}

/// What the app must do after a step.
#[derive(Clone, Debug)]
pub enum Request {
    /// load this match into `AssetStore::fight` (after the VS screen has been drawn), then set `FrameInput::LOADED`
    Load(MatchSpec),
    /// the loaded match started (`GameState::fight`, when its data was there): build its view
    Fight,
    /// the fight is over: its data and view dropped, or kept for the attract ranking (`GameState::backdrop`); how
    /// it ended, the rounds each side won and whether it was the attract demo (the replay recorder's match entry)
    FightOver { backdrop: bool, outcome: crate::fight::sim::Outcome, rounds: [u8; 2], demo: bool },
    /// play XA track `.0`, looped when `.1`; None = stop (done through the sound queue already)
    Music(Option<(u8, bool)>),
    /// load these files into `AssetStore::files`, then set `FrameInput::FETCHED`
    Fetch(Vec<String>),
    /// GRAPHICS (engine addition): apply and save these preferences
    Prefs(crate::frontend::options::Prefs),
    /// EXIT GAME (engine addition): quit the app
    Exit,
    /// REPLAY (engine addition, v2): open the replay list (the app's; the session waits on the mode menu meanwhile)
    Replays,
    /// NETWORK (engine addition): open the app's NETWORK menu (the session waits on the mode menu meanwhile); in a
    /// netplay session started from it, leave the session
    Network,
    /// CHANGE GAME DATA (engine addition, v9): open the app's setup screen (the session waits on OPTIONS meanwhile)
    ChangeData,
    /// TEAM BATTLE (engine addition, simulation v22): load this team battle into `AssetStore::fight`
    /// (`fight_data::load_team`), then set `FrameInput::LOADED`
    LoadTeam(crate::frontend::TeamSpec),
    /// the team battle is over (its data and view dropped): how it ended (`Winner(team)` or `Quit`) and the rounds
    /// each team won (the replay recorder's match entry)
    TeamOver { outcome: crate::fight::sim::Outcome, rounds: [u8; 4] },
}

/// A step's output.
#[derive(Default)]
pub struct Out {
    /// the sound calls in the order made (the app applies them right after the step)
    pub audio: crate::output::sound::Queue,
    pub requests: Vec<Request>,
    /// the memory card was written: the app stores `frontend::Save::memory_card` and reports it in the next input
    pub card_dirty: bool,
    /// the DualShock motors after the fight's tick
    pub vib: Option<[[(u8, u8); 2]; 2]>,
}

impl Out {
    /// `marks`: keep the debug OKB_SOUNDLOG frame marks (`Audio::logging`).
    pub fn new(marks: bool) -> Self {
        Self { audio: crate::output::sound::Queue::new(marks), ..Default::default() }
    }
}

/// The static data a step reads.
#[derive(Clone, Copy)]
pub struct Assets<'a> {
    pub front: Option<&'a Arc<FrontData>>,
    /// the fight's data, or before it starts the loaded match
    pub fight: Option<&'a Loaded>,
    /// the backdrop fight's data (never stepped; `rebind` only)
    pub backdrop: Option<&'a Loaded>,
    pub files: &'a FileCache,
    /// the app's preferences (OPTIONS → GRAPHICS shows them; engine addition)
    pub prefs: crate::frontend::options::Prefs,
}

/// The app's static data: what `Assets` lends to the steps, plus the backdrop fight's data (drawn only). Shared
/// handles: the rollback test keeps the data each logged step saw (`rollback::Held`).
#[derive(Clone, Default)]
pub struct AssetStore {
    pub front: Option<Arc<FrontData>>,
    pub fight: Option<Arc<Loaded>>,
    pub backdrop: Option<Arc<Loaded>>,
    pub files: FileCache,
}

impl AssetStore {
    pub fn assets(&self, prefs: crate::frontend::options::Prefs) -> Assets<'_> {
        Assets { front: self.front.as_ref(), fight: self.fight.as_deref(), backdrop: self.backdrop.as_deref(), files: &self.files, prefs }
    }

    /// After a step: the fetched files the state no longer refers to (not in the story cache, not waited for) go.
    pub fn prune(&mut self, s: &GameState) {
        self.files.retain(|n, _| s.front.as_ref().is_some_and(|f| f.file_cached(n)) || s.fetching.as_ref().is_some_and(|w| w.contains(n)));
    }
}

fn full_volume() -> u8 {
    0x7F
}
