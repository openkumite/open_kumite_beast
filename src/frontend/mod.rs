//! The front end (`Front`): boot, title, menus, selects, VS screen, ladder, story, ranking.
//!
//! State: serialised: `Front` is part of `GameState`, in `serial.rs`'s layout (field changes are `STATE_VERSION`
//!   changes)
//!
//! The front end: title and mode select (with the menu engine), the console character
//! select (used here for every mode), the arcade ladder, the VS screen
//! (with the announcer) and the routing after a fight.
//! Everything is drawn as PS1 primitives in the fixed 512×480 space from the CMN files: INIT.DAT (fonts,
//! face icons, menu sprites), USATITLE.DAT, SE00.DAT (face CLUTs), P00.DAT (beast portraits), SBxx.DAT.
//!
//! `mod.rs`: `Front` and its accessors; `screen.rs`: `Screen`, `Mode`, the stage / mode tables, `MatchSpec`, `Event`,
//! `Pads`; `tick.rs`: a frame (`tick`); `draw.rs`: its primitives; `check.rs`: the state checks, `sync_key`,
//! `describe`.

mod check;
pub(crate) mod data;
mod draw;
pub mod memcard;
pub mod menus;
pub(crate) mod movie;
pub mod options;
mod progress;
pub mod ranking;
pub(crate) mod resident;
mod screen;
mod select;
mod serial;
mod story_flow;
pub mod story_screens;
mod team;
mod team_arcade;
mod tick;
mod title;
mod vs;

use crate::fight::hud;
use crate::frontend::options::Settings;
use crate::output::sound::Queue;

pub use data::{FrontData, load};
use movie::MoviePlay;
pub use resident::{Resident, Tim};
pub use screen::{Event, LoadSpec, MatchSpec, Mode, Pads, TeamSpec, stage_name};
use screen::{MODES, Screen, Text, mode_half, mode_rows, stage_label};
use std::sync::Arc;
use story_flow::StoryScreen;
pub use team_arcade::{TEAM_ARCADE_MAX_STAGES, TEAM_ARCADE_MIN_STAGES};

/// The run under way (engine grouping of `Front`'s fields): the mode, the sides, the ladder and the arcade /
/// survival scores.
#[derive(Clone)]
struct Run {
    mode: Mode,
    humans: [bool; 2],
    /// P.E2 win streaks and the champion flags (M.B4 & 0x1000) of the two sides
    streak: [u16; 2],
    /// P.E0: the best streak of this game (the VS RANKING value)
    best: [u16; 2],
    champ: [bool; 2],
    /// a side that dropped out (lost challenger match): its small GAME OVER in the next fight (state 0x900)
    dropped: [bool; 2],
    /// ladder, M.A2 = fight index, continues used (F.EC)
    ladder_o: [u8; 11],
    ladder_s: [u8; 11],
    a2: i32,
    continues: u32,
    /// M.A0 arcade rank (CPU level, bit 8 = after a continue)
    rank: u16,
    /// opt.21: L1+L2+R1+R2 held when the mode was chosen (strongest CPU)
    shoulders: bool,
    /// the human's costume of the last select (kept for the next ladder fights)
    human_costume: [u8; 2],
    /// fighting time of this run (frames of round state 4; the arcade ranking value)
    play_time: u32,
    /// a NAME ENTRY waiting behind the ending message
    pending_entry: Option<crate::frontend::ranking::Entry>,
    /// the arcade ending's movie (ENDING1 after the extra stage, else ENDING2)
    ending_movie: usize,
    /// engine addition (the import S10, `br1_boss`): the ARCADE run's extra imported boss: 0 not reached, 1 its
    /// fight is the current one, 2 fought
    br1_boss: u8,
    /// engine addition (TEAM ARCADE, simulation v26): the run's ladder (`team_arcade::TeamLadder`), from TEAM ARCADE's
    /// start
    team_ladder: Option<team_arcade::TeamLadder>,
}

/// The save (engine grouping): the options and unlocks, the memory card and its dialog, and the debug / engine
/// unlocks that open the select without touching the save.
#[derive(Clone)]
pub struct Save {
    /// options, CUSTOM block and unlocks (opt), saved on exit of the menus and at endings
    pub settings: Settings,
    /// save+0x410: opt changed since the last card load / save (the auto-save runs only then)
    pub dirty: bool,
    /// the MEMORY CARD dialog (over the MEMORY CARD menu, or the auto-save)
    card: Option<crate::frontend::memcard::CardDialog>,
    /// MEMORY CARD SLOT 1 (the save; the app stores it after a step that wrote it)
    pub memory_card: crate::frontend::options::Card,
    /// debug (F11, `SessionStart::unlock`): everything selectable (characters, fourth costumes, movies, pictures)
    /// without touching the save
    unlock_override: bool,
    /// engine default (simulation v5, unless `SessionStart::locked`): every character and fourth costume selectable
    /// without touching the save, and the stage select has ST9..STC too (`stage_count`); movies, pictures and
    /// CUSTOM items keep the save's progression. A state file without it reads false (the game's own rule, as it ran)
    all_open: bool,
}

/// The VRAM loads (engine grouping).
#[derive(Clone)]
struct Gfx {
    /// the TIMs loaded into VRAM (the image is the presentation's: `VramView`)
    vram: Resident,
    /// +1 on every rewrite of the VRAM (`VramView` rebuilds and the app uploads the image when it differs from the
    /// one last drawn; a new `VramView` after a load or a rewind starts with none)
    vram_gen: u32,
    /// the loads under a movie / picture, put back after
    vram_saved: Option<Resident>,
}

/// The front end's state. It serialises with the fields of `gfx`, `save` and `run` flat among the others, as before
/// those groups (`serial`).
#[derive(Clone)]
pub struct Front {
    /// the static CMN files and banks: a shared handle, not state (`session::Assets::front`; `Front::rebind`); not
    /// serialised
    data: Arc<FrontData>,
    gfx: Gfx,
    pub save: Save,
    run: Run,
    screen: Screen,
    pads: Pads,
    frame: u32,
    menu_cursor: usize,
    /// next P00 emblem picture of the menu frame (cycles 0..10)
    emblem: usize,
    /// opt.1B / opt.1C: select start cursors
    last: [u8; 2],
    /// M.98 and opt.40 bit 31 (RANDOM chosen); M.98 a stage number (an extra stage's too: `row_stage`), the select's
    /// cursor a row (`Select::stage`)
    stage: u8,
    stage_random: bool,
    rng: u32,
    /// main's `s3`: title visits since the logos last restarted the loop (mode 2); the 4th timeout skips the demo
    title_visits: u8,
    /// the attract ranking's camera over the demo's stage (reset, mode 0x1E)
    rank_cam: crate::fight::camera::Camera,
    movie: Option<MoviePlay>,
    story: Option<crate::frontend::story_screens::Run>,
    story_screen: Option<StoryScreen>,
    /// an event raised outside the frame's return path (mode start)
    pending_event: Option<Event>,
    /// story pages / pictures already fetched (their names; the bytes stay in the app's `session::FileCache`)
    file_cache: std::collections::BTreeSet<String>,
    /// the engine's preferences (owned by the app, mirrored here for the GRAPHICS screen; engine addition): not
    /// serialised, so machines with other preferences hash the same; `rebind` sets it again (the open GRAPHICS
    /// screen's, which is state as it decides the rows the cursor visits, else the app's)
    prefs: crate::frontend::options::Prefs,
    /// the session's simulation version (`GameState::sim_version`, given at each tick and after a load): which
    /// mode menu rows there are (`mode_rows`); not serialised
    sim_version: u32,
    /// the session's browser menus (`GameState::browser`, given after a load): `mode_rows`, the GRAPHICS rows; not
    /// serialised
    browser: bool,
    /// the session's NETWORK row on the mode menu (`GameState::network`, given after a load; engine addition):
    /// `mode_rows`; not serialised
    network: bool,
    /// the session's extra stages (bit k = `fight::stage::extra::STAGES[k]`, `SessionStart::extras`, given after a
    /// load; engine addition): the stage select's rows after the main game's stages (`stage_count`); not serialised
    extras: u8,
    /// the session's Android menus (`GameState::android`, given after a load; engine addition): the GRAPHICS rows;
    /// not serialised
    android: bool,
    /// the session has the imported game's data (`SessionStart::br1`, `GameState::br1`, given after a load; engine addition,
    /// the import S9): the select's imported row (`br1_row`) and the stage select's imported stages
    /// (`br1_stages`); not serialised
    br1: bool,
    /// pad edges and auto-repeat of pads 3 / 4 (engine addition, the team battle: `FrameInput::ext`; only its screens
    /// read them)
    ext: Pads,
    /// the team battle (engine addition): the pad that chose TEAM BATTLE, the select's last cursors, the stage row
    team_port: u8,
    team_last: [u8; 4],
    team_stage: u8,
    /// the result screen's row last chosen (REMATCH / CHARACTER SELECT / TITLE)
    team_result_row: u8,
    /// a netplay session (`GameState::netplay`, given after a load; engine addition): no TEAM BATTLE row; not serialised
    netplay: bool,
}

impl Front {
    /// The front end at boot; its sound banks are already in the mixer (`Audio::load_front_banks`).
    pub fn new(data: Arc<FrontData>, audio: &mut Queue, start: &crate::session::SessionStart) -> Self {
        use crate::session::Entry;
        let memory_card = crate::frontend::options::Card::new(start.card.clone(), start.card_fault);
        let mut f = Self {
            data,
            gfx: Gfx { vram: Resident::default(), vram_gen: 1, vram_saved: None },
            save: Save {
                settings: Settings::load(&memory_card),
                dirty: false,
                card: None,
                memory_card,
                unlock_override: start.unlock,
                // simulation v4 and older: the save's progression only (`--unlock` was the way to open everything)
                all_open: crate::fight::version::Gates::of(start.sim_version).all_open && !start.locked,
            },
            run: Run {
                mode: Mode::Arcade,
                humans: [true, false],
                streak: [0; 2],
                best: [0; 2],
                champ: [false; 2],
                dropped: [false; 2],
                ladder_o: [0; 11],
                ladder_s: [0; 11],
                a2: 0,
                continues: 0,
                rank: 0,
                shoulders: false,
                human_costume: [0; 2],
                play_time: 0,
                pending_entry: None,
                ending_movie: 3,
                br1_boss: 0,
                team_ladder: None,
            },
            rank_cam: crate::fight::camera::Camera::default(),
            movie: None,
            story: None,
            story_screen: None,
            pending_event: None,
            file_cache: Default::default(),
            prefs: crate::frontend::options::Prefs::default(),
            sim_version: start.sim_version,
            browser: start.browser,
            network: start.network,
            extras: start.extras(),
            android: start.android,
            br1: start.br1.is_some(),
            ext: Pads::default(),
            team_port: 0,
            team_last: [0, 1, 6, 3],
            team_stage: 0,
            team_result_row: 0,
            netplay: start.netplay,
            screen: Screen::Title { menu: false, timer: 360 },
            pads: Pads::default(),
            frame: 0,
            menu_cursor: 0,
            emblem: 0,
            last: [0, 2],
            stage: 0,
            stage_random: true,
            rng: 0x1234_5678 ^ (start.seed as u32),
            title_visits: 0,
        };
        f.boot_vram();
        f.apply_sound(audio);
        // main mode 3: the logos and the opening movie (scripted test runs start at the title)
        let intro = match start.entry {
            Entry::Intro => true,
            Entry::NameTest { intro, .. } | Entry::Epilogue { intro, .. } => intro,
            Entry::Title | Entry::DirectFight { .. } | Entry::Versus | Entry::TeamArcade { .. } => false,
        };
        if intro {
            f.enter_logos();
        } else {
            f.enter_title(audio);
        }
        match start.entry {
            // engine addition (a netplay session of the NETWORK menu): VS's character select at once, as the mode
            // menu's VS starts it (both players human)
            Entry::Versus => f.start_mode(Mode::Versus, 0, audio),
            // debug OKB_TEAM_ARCADE (engine addition, simulation v26): a TEAM ARCADE run at a stage's loading screen
            Entry::TeamArcade { stage, chr, partner } if f.gates().team_arcade => f.debug_team_arcade(stage, chr, partner, audio),
            // debug: character c's arcade epilogue (then the ending movie)
            Entry::Epilogue { chr, .. } => {
                f.story_screen = Some(StoryScreen::Epilogue(crate::frontend::story_screens::Epilogue::new(chr)));
                f.screen = Screen::Story;
            }
            // debug: NAME ENTRY for YUGO, "mode:value": 0 = arcade, clear time in frames (default 7323 = 02'02"05),
            // 3 = survival wins
            Entry::NameTest { mode: m, value: val, .. } => {
                let (table, rec) = if m == 3 {
                    (2, crate::frontend::options::RankRec { name: [b' '; 3], chr: 0, value: 0, c: 0, e: val as i16 })
                } else {
                    (0, crate::frontend::options::RankRec { name: [b' '; 3], chr: 0, value: val, c: 0, e: 9 })
                };
                f.run.mode = if m == 3 { Mode::Survival } else { Mode::Arcade };
                let e = f.rank_entry(table, m != 3, rec);
                f.boot_vram();
                f.pending_event = f.enter_name(e, audio);
            }
            _ => {}
        }
        f
    }

    /// After deserialising: the static data the state refers to: the CMN files and banks, and the story page's
    /// STY file parsed again. Err for a movie number out of range, or a fetched file the state reads that is not
    /// in `files` (`needed_files`: the caller can fetch them and try again).
    pub fn rebind(&mut self, data: &Arc<FrontData>, files: &crate::session::FileCache, prefs: crate::frontend::options::Prefs) -> Result<(), String> {
        self.check()?;
        self.data = data.clone();
        // the GRAPHICS screen keeps `prefs` equal to its edited copy (`set_prefs`, ApplyGraphics)
        self.prefs = match &self.screen {
            Screen::Menu { menu, .. } if menu.kind == menus::Kind::Graphics => menu.g,
            _ => prefs,
        };
        if self.run.ending_movie >= movie::COUNT {
            return Err(format!("ending movie {}", self.run.ending_movie));
        }
        if let Screen::Movie { n, .. } = self.screen
            && n >= movie::COUNT
        {
            return Err(format!("movie {n}"));
        }
        if let Some(name) = self.needed_files().into_iter().find(|n| files.get(n).is_none()) {
            return Err(format!("front end: {name} not in the store"));
        }
        if let Some(StoryScreen::Pages(p)) = &mut self.story_screen {
            p.rebind(files)?;
        }
        Ok(())
    }

    /// The fetched files a loaded state reads: the story page's STY file and, for the presentation (`VramView`),
    /// those of the resident TIMs (also the ones saved over a movie) and the playing movie's stream. The movie
    /// number must be in range.
    pub fn needed_files(&self) -> Vec<String> {
        let mut v: Vec<String> = self.gfx.vram.files().chain(self.gfx.vram_saved.iter().flat_map(|r| r.files())).map(String::from).collect();
        if let Some(StoryScreen::Pages(p)) = &self.story_screen {
            v.extend(p.file_used());
        }
        if let Some((n, _)) = self.movie_playing() {
            v.push(movie::files(n)[0].clone());
        }
        v.sort();
        v.dedup();
        v
    }

    /// The app's preferences changed (start, F5, Alt+Enter, a window going fullscreen): an open GRAPHICS screen
    /// shows them (engine addition).
    pub fn set_prefs(&mut self, p: crate::frontend::options::Prefs) {
        self.prefs = p;
        if let Screen::Menu { menu, .. } = &mut self.screen
            && menu.kind == menus::Kind::Graphics
        {
            menu.g = p;
        }
    }

    /// The app's preferences as last given (`set_prefs`) or edited on the GRAPHICS screen.
    pub fn prefs(&self) -> crate::frontend::options::Prefs {
        self.prefs
    }

    /// SOUND OPTIONS values → the mixer.
    fn apply_sound(&self, audio: &mut Queue) {
        let s = &self.save.settings;
        audio.set_options(s.master, s.bgm_volume, s.output != 0);
    }

    /// Selectable characters (opt.4C) and fourth costumes (opt.50).
    fn chars(&self) -> u32 {
        if self.save.unlock_override || self.save.all_open { 0x7FF } else { self.save.settings.characters }
    }

    fn costumes4(&self) -> u32 {
        if self.save.unlock_override || self.save.all_open { 0x7FF } else { self.save.settings.costume4 }
    }

    /// Stages of the stage select before RANDOM (the game's 9; ST9..STC too when `all_open`, and after them the
    /// extra stages the session has the files of, each from its simulation version, `extra_rows`, then the imported game's
    /// when the session has its data, `br1_stages`; not the debug unlock, which an older session may have used with the
    /// game's select).
    fn stage_count(&self) -> u8 {
        self.random_count() + self.port_rows().len() as u8
    }

    /// The stages RANDOM draws from: the select's rows but the extra stage's (engine addition: chosen on
    /// purpose only, so RANDOM draws the same with or without its files).
    fn random_count(&self) -> u8 {
        if self.save.all_open { 13 } else { 9 }
    }

    /// The extra stages on the stage select, in their rows' order after ST9..STC (`stage::extra::STAGES`: Q3DM17 from
    /// v10, Q3DM6 from v12, each when the session has its files).
    fn extra_rows(&self) -> impl Iterator<Item = &'static crate::fight::stage::extra::Extra> + use<> {
        let (open, extras, g) = (self.save.all_open, self.extras, self.gates());
        crate::fight::stage::extra::STAGES.iter().filter(move |e| open && extras & 1 << e.index() != 0 && g.reaches(e.sim))
    }

    /// Engine addition (the import S9, simulation v19): the imported game's eleven stages on the stage select after the
    /// extra stages, when the session has the imported game's data (`SessionStart::br1`) and the select is the open one.
    fn br1_stages(&self) -> bool {
        self.br1 && self.save.all_open && self.gates().br1_select
    }

    /// The stage numbers of the select's rows after `random_count`: the extra stages', then the imported game's.
    fn port_rows(&self) -> Vec<u8> {
        let br1 = if self.br1_stages() { &crate::fight::stage::br1::STAGES[..] } else { &[] };
        self.extra_rows().map(|e| e.stage).chain(br1.iter().map(|s| s.stage)).collect()
    }

    /// The stage of the select's row `row` (below `stage_count`): the main game's stage of that number, then the extra stages'
    /// (engine addition: Q3DM6 is row 13 when the session has no Q3DM17), then the imported game's.
    fn row_stage(&self, row: u8) -> u8 {
        let n = self.random_count();
        if row < n { row } else { self.port_rows().get((row - n) as usize).copied().unwrap_or(row) }
    }

    /// The select's row of stage `stage`; `stage_count` (RANDOM's) when the select does not have it.
    fn stage_row(&self, stage: u8) -> u8 {
        let n = self.random_count();
        if stage < n {
            return stage;
        }
        self.port_rows().iter().position(|&s| s == stage).map_or(self.stage_count(), |k| n + k as u8)
    }

    /// Engine addition (the import S9, simulation v19): the modes whose matches may hold imported fighters (`roster::
    /// BR1_FIRST..`): VS, CUSTOM VS HUMAN and PRACTICE, from simulation v20 (their CPU AI, the import S10) also ARCADE,
    /// SURVIVAL, WATCH, CUSTOM VS COMPUTER and VS COMPUTER ALL MEMBERS (not STORY), when the session has the imported game's
    /// data and the select is the open one.
    fn br1_modes(&self) -> bool {
        self.br1_stages()
            && match self.run.mode {
                Mode::Versus | Mode::Custom(0) | Mode::Custom(3) => true,
                Mode::Arcade | Mode::Survival | Mode::Watch | Mode::Custom(1) | Mode::Custom(2) => self.gates().br1_cpu_modes,
                _ => false,
            }
    }

    /// The select's row of imported fighters under the main game's grid, in `br1_modes` (from v20 also ARCADE's
    /// NORMAL select's third row: a user decision).
    fn br1_row(&self) -> bool {
        self.br1_modes()
    }

    /// Engine addition (the import S10, simulation v20): an ARCADE run with the imported game's data (`br1_modes`) ends with
    /// the imported game's Uriko, "URIKO (BR1)", as an extra boss after the main game's last fight (a user decision).
    fn has_br1_boss(&self) -> bool {
        self.run.mode == Mode::Arcade && self.br1_modes()
    }

    /// The select's row `row`: its name and half width (RANDOM from `stage_count` on).
    fn row_label(&self, row: u8) -> (&'static str, i32) {
        stage_label((row < self.stage_count()).then(|| self.row_stage(row)))
    }

    /// The selectable characters and fourth costumes (bit c = character c; tests, nettest).
    pub fn selectable(&self) -> (u32, u32) {
        (self.chars(), self.costumes4())
    }

    /// CONTROLLER CONFIG. button maps of both players.
    pub fn buttons(&self) -> [[u16; 8]; 2] {
        self.save.settings.buttons
    }

    /// BGM SELECT = ARCADE (stage themes; other continue / game-over tracks).
    pub fn arcade_bgm(&self) -> bool {
        self.save.settings.bgm_select == 1
    }

    /// INIT.DAT at boot (at their fixed VRAM places) and the face CLUTs of SE00.DAT at (0,480).
    fn boot_vram(&mut self) {
        let at: [(usize, (usize, usize)); 22] = [
            (0, (960, 0)),
            (1, (896, 0)),
            (2, (496, 480)),
            (3, (480, 480)),
            (4, (464, 480)),
            (5, (368, 480)),
            (6, (352, 496)),
            (16, (464, 496)),
            (17, (496, 496)),
            (18, (480, 496)),
            (19, (448, 496)),
            (20, (384, 496)),
            (21, (400, 496)),
            (22, (416, 496)),
            (23, (432, 496)),
            (24, (832, 256)),
            (25, (512, 256)),
            (26, (448, 480)),
            (27, (432, 480)),
            (8, (896, 448)),
            (9, (928, 384)),
            (7, (832, 448)),
        ];
        // (8, 9 and 7 go over the bottom of entry 24's page: menu buttons, arrows, cursors)
        for (k, pos) in at {
            if let Some(Some(t)) = self.data.init.get(k) {
                let clut = if k == 7 { Some((352, 480)) } else { None };
                self.gfx.vram.load(Tim::Init(k as u8), t, clut, Some(pos));
            }
        }
        self.gfx.vram.load(Tim::Se00, &self.data.se00, None, Some((0, 480)));
        self.gfx.vram_gen = self.gfx.vram_gen.wrapping_add(1);
    }

    fn rand(&mut self) -> u32 {
        crate::base::libc_rand::rand(&mut self.rng) as u32
    }

    /// `play(0x400, id)`: slot = id >> 6 (0 COMMON_2, 1 CHASEL_1, 2 TITLE, ENDING or CHASEL_2), tone = id & 63.
    fn play(&self, audio: &mut Queue, id: u16, delay: u32) {
        let slot = (id >> 6) as usize;
        let bank = match slot {
            0 => 0,
            1 => 1,
            _ if matches!(self.screen, Screen::Title { .. }) => 2,
            _ if matches!(self.screen, Screen::NameEntry(_) | Screen::NameFade { .. }) => 4,
            _ => 3,
        };
        let (name, _, vols) = &self.data.banks[bank];
        let tone = (id & 63) as usize;
        let vol = vols.get(tone).copied().unwrap_or(0x1FC0);
        audio.play_tone_in(&format!("fe:{name}"), tone, vol, 2, delay);
    }

    /// Fighting frames of the last match (the arcade clear time adds them up).
    /// The fight's status-label inputs (M.9C / M.9B per mode).
    pub fn label_info(&mut self) -> hud::LabelInfo {
        let dropped = std::mem::take(&mut self.run.dropped);
        let (m9c, m9b) = self.run.mode.m9c_9b();
        let ladder_len = match self.run.mode {
            Mode::Survival => 0x63,
            Mode::Custom(2) => 11,
            _ => 8,
        };
        hud::LabelInfo {
            m9c,
            m9b,
            fight: self.run.a2,
            ladder_len,
            ladder_done: self.run.mode == Mode::Arcade && self.run.a2 > ladder_len,
            play_time: self.run.play_time,
            shoulders: self.run.shoulders,
            streak: self.run.streak,
            champ: self.run.champ,
            dropped,
        }
    }

    pub fn add_play_time(&mut self, frames: u32) {
        self.run.play_time = self.run.play_time.saturating_add(frames);
    }

    /// The fight data is loaded.
    pub fn loaded(&mut self) {
        if let Screen::Vs { ready, .. } | Screen::DemoLoad { ready, .. } | Screen::TeamLoad { ready, .. } = &mut self.screen {
            *ready = true;
        }
    }

    /// The match the VS / demo-load screen (or the team battle the team loading screen) waits for (asked for and not
    /// `loaded` yet); None while its `Event::Load` is still to go out (engine addition: what a state put back by the
    /// debug tools asks for again).
    pub fn load_wanted(&self) -> Option<LoadSpec> {
        match self.screen {
            Screen::Vs { ready: false, spec, .. } | Screen::DemoLoad { ready: false, spec } if !matches!(self.pending_event, Some(Event::Load(_))) => {
                Some(LoadSpec::Match(spec))
            }
            Screen::TeamLoad { ready: false, spec, .. } => Some(LoadSpec::Team(spec)),
            _ => None,
        }
    }

    /// A COMMON_2 / front-end sound (`play(0x400, id)`).
    pub fn play_se(&self, audio: &mut Queue, id: u16) {
        self.play(audio, id, 0);
    }

    /// The attract demo's fight is starting or running.
    pub fn demo_running(&self) -> bool {
        self.run.mode == Mode::Demo
    }

    /// The attract demo runs (the app ends it on a key press, a KO or after 30 s).
    pub fn demo(&self) -> bool {
        self.run.mode == Mode::Demo && matches!(self.screen, Screen::Fight(_))
    }

    /// The mode's name (a replay's header; engine addition): its banner's (`Mode::banner`), the engine's own words for the
    /// demo and the team modes.
    pub fn mode_name(&self) -> &'static str {
        match self.run.mode {
            Mode::Arcade | Mode::Versus | Mode::Watch | Mode::Survival | Mode::Story | Mode::Custom(_) => self.run.mode.banner().0,
            Mode::Demo => "DEMO",
            Mode::Team => "TEAM BATTLE",
            Mode::TeamArcade => "TEAM ARCADE",
        }
    }

    /// A fetched file the state refers to (the app keeps its bytes): one of the story cache, the story page's,
    /// one a resident TIM comes from (`VramView` replays it), or the playing movie's (`VramView` plays it).
    pub fn file_cached(&self, name: &str) -> bool {
        self.file_cache.contains(name)
            || self.gfx.vram.uses(name)
            || self.gfx.vram_saved.as_ref().is_some_and(|v| v.uses(name))
            || matches!(&self.story_screen, Some(StoryScreen::Pages(p)) if p.file_used().as_deref() == Some(name))
            || self.movie_playing().is_some_and(|(n, _)| movie::files(n).iter().any(|f| f == name))
    }

    /// The TIMs in VRAM (`VramView`).
    pub fn resident(&self) -> &Resident {
        &self.gfx.vram
    }

    /// `Gfx::vram_gen`: +1 on every rewrite of the VRAM (`VramView`).
    pub fn vram_gen(&self) -> u32 {
        self.gfx.vram_gen
    }

    /// The movie playing: its number and frame counter (`VramView`).
    pub fn movie_playing(&self) -> Option<(usize, u32)> {
        match (self.screen, &self.movie) {
            (Screen::Movie { n, .. }, Some(mp)) => Some((n, mp.t)),
            _ => None,
        }
    }

    /// Debug: every character and the fourth costumes selectable.
    pub fn unlock_all(&mut self) {
        self.save.unlock_override = true;
    }

    /// Front-end frames since boot (screenshots).
    pub fn frame_no(&self) -> u32 {
        self.frame
    }

    pub fn in_fight(&self) -> bool {
        matches!(self.screen, Screen::Fight(_))
    }

    /// The session's simulation version (`session::rebind` after a load; `tick` gives it every frame).
    pub fn set_sim_version(&mut self, v: u32) {
        self.sim_version = v;
    }

    /// The rules of the session's simulation version (`fight::version`).
    pub(crate) fn gates(&self) -> crate::fight::version::Gates {
        crate::fight::version::Gates::of(self.sim_version)
    }

    /// The session's browser menus (`session::rebind` after a load).
    pub fn set_browser(&mut self, b: bool) {
        self.browser = b;
    }

    /// The session's NETWORK row (`GameState::network`; `session::rebind` after a load).
    pub fn set_network(&mut self, n: bool) {
        self.network = n;
    }

    /// The session's extra stages (`GameState::extra_stage`, `GameState::extra_q3dm6`; `session::rebind` after a
    /// load).
    pub fn set_extras(&mut self, e: u8) {
        self.extras = e;
    }

    /// The session's Android menus (`GameState::android`; `session::rebind` after a load).
    pub fn set_android(&mut self, a: bool) {
        self.android = a;
    }

    /// The session has the imported game's data (`GameState::br1`; `session::rebind` after a load).
    pub fn set_br1(&mut self, b: bool) {
        self.br1 = b;
    }

    /// A netplay session (`GameState::netplay`; `session::rebind` after a load).
    pub fn set_netplay(&mut self, n: bool) {
        self.netplay = n;
    }

    /// The team battle's CPU level (engine addition): OPTIONS' DIFFICULTY at the arcade ladder's middle fight (the CPU level's
    /// rank table, row DIFFICULTY, column 4: NORMAL 0x70, near the 0x80 of the headless team battles; the first fight's
    /// column is 0 on most rows, CPUs that hardly act).
    pub fn team_cpu_level(&self) -> u16 {
        progress::rank(self.save.settings.difficulty, 4) as u16 * 16
    }

    /// The `MODES` item under the mode menu's cursor.
    fn mode_item(&self) -> usize {
        let rows = mode_rows(self.gates(), self.browser, self.network, self.netplay);
        rows[self.menu_cursor.min(rows.len() - 1)]
    }
}
