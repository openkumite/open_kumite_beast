//! The front end's screens and their vocabulary: `Screen`, `Text`, `Mode`, `MatchSpec`, `Event`, `Pads`.
//!
//! State: serialised: part of `GameState` (field changes are `STATE_VERSION` changes)
//!
//! The front end's screens and their vocabulary: `Screen` (the state of the screen up), `Text`, `Mode`, the stage and
//! mode-menu tables (the banner), `MatchSpec`, `Event`
//! (the requests to the app) and `Pads` (pad edges and auto-repeat).

use super::menus::Menu;
use super::movie::After;
use super::select::Select;
use crate::assets::game_tables;
use crate::assets::roster::SELECTABLE;
use crate::assets::text;
use crate::assets::text::Label::{self, Game, Port};
use crate::fight::version::Gates;
use serde::{Deserialize, Serialize};

/// Stage names of the select (the text list `stages`, menu-font text; no preview picture) and the half
/// widths used to centre them, RANDOM after the stages. ST9..STC (`fight_data::STAGE_CODES` 9..12) have
/// no entry in the game (the game's select never reaches them); the engine's open select (`Save::all_open`) shows
/// them as EXTRA1..EXTRA4, centred by their menu-font width as the table's are, and after them the engine's extra stages
/// (`fight::stage::extra`) by their codes, Q3DM17 and Q3DM6, those the session has the files of (`PORT_STAGES`); then
/// the imported game's eleven stages (stage numbers 15..25, `fight::stage::br1::STAGES`; engine addition, the import S9)
/// by the imported game's own names (its text list `stages` by the VS select's index k, which the stage setup makes G.94 k, 9 for
/// 8 and 10 for 9; G.94 8, Uriko's, is not on the imported game's select and has no name there: its arcade's FINAL STAGE,
/// the text `final_stage`).
const PORT_STAGES: [&str; 6] = ["EXTRA1", "EXTRA2", "EXTRA3", "EXTRA4", "Q3DM17", "Q3DM6"];
/// The stage numbers `stage_name` names: 0..8, the engine's 9..14, the imported game's 15..25.
pub(super) const STAGE_COUNT: usize = 26;
const RANDOM_HALF: i32 = 48;

/// A stage's name as the select shows it (the replay list; engine addition).
pub fn stage_name(stage: u8) -> &'static str {
    match stage {
        0..=8 => text::item("stages", stage as usize),
        9..=14 => PORT_STAGES[stage as usize - 9],
        15..=22 => text::br1_item("stages", stage as usize - 15),
        23 => text::br1("final_stage"),
        24 | 25 => text::br1_item("stages", stage as usize - 16),
        _ => "",
    }
}

/// A select row's stage (None: RANDOM): its name and half width.
pub(super) fn stage_label(stage: Option<u8>) -> (&'static str, i32) {
    let Some(s) = stage else { return (text::item("stages", 9), RANDOM_HALF) };
    let name = stage_name(s);
    match game_tables::get().stage_half.get(s as usize) {
        Some(&h) => (name, h),
        None => (name, crate::output::font::fonts().menu_width(name) / 2),
    }
}

/// Mode select items (the text list `modes`) and their half widths; REPLAY, EXIT GAME and NETWORK are engine
/// additions (REPLAY opens the app's replay list; EXIT GAME quits the app, not in the browser, where a page cannot
/// close itself; NETWORK opens the app's NETWORK menu), their half widths the menu font's text widths / 2 as the
/// table's.
pub(super) const MODES: [Label; 11] = [
    Game("modes.0"),
    Game("modes.1"),
    Game("modes.2"),
    Game("modes.3"),
    Game("modes.4"),
    Game("modes.5"),
    Game("replay"),
    Game("modes.6"),
    Port("EXIT GAME"),
    Port("NETWORK"),
    Port("TEAM BATTLE"),
];
/// `MODES` index of REPLAY, of EXIT GAME, of NETWORK and of TEAM BATTLE (engine addition, simulation v22: the team
/// battle).
pub(super) const REPLAY_ITEM: usize = 6;
pub(super) const EXIT_ITEM: usize = 8;
pub(super) const NETWORK_ITEM: usize = 9;
pub(super) const TEAM_ITEM: usize = 10;
/// The half width of `MODES` item `i`: the game's seven (ARCADE .. CUSTOM, OPTIONS) from the table `menu.mode_half`,
/// the engine's rows' their menu-font text widths / 2, as the table's.
pub(super) fn mode_half(i: usize) -> i32 {
    let t = &game_tables::get().mode_half;
    match i {
        0..=5 => t[i],
        7 => t[6],
        REPLAY_ITEM => 48,
        EXIT_ITEM => 64,
        NETWORK_ITEM => 56,
        _ => 84,
    }
}
/// The mode spinner's rows (`MODES` indices; `Front::menu_cursor` is a row): simulation v1 has no REPLAY row
/// (`session::SIM_VERSION` 2 added it), so a v1 replay's cursor numbers and moves stay as they were recorded. A
/// session started with `SessionStart::network` has NETWORK after REPLAY from v8, so the game's rows keep their
/// order, the engine's REPLAY and NETWORK sit between CUSTOM and OPTIONS and EXIT GAME stays last (the wrap from
/// ARCADE up still reaches EXIT GAME, then OPTIONS); a session without it (older replays, `sim`, the command line's
/// netplay) or before v8 has the rows it had.
const ROWS_V1: [usize; 8] = [0, 1, 2, 3, 4, 5, 7, 8];
const ROWS: [usize; 9] = [0, 1, 2, 3, 4, 5, 6, 7, 8];
const ROWS_NET: [usize; 10] = [0, 1, 2, 3, 4, 5, 6, 9, 7, 8];
/// From simulation v22 (`fight::version::v::TEAM`) TEAM BATTLE follows CUSTOM, outside netplay (`SessionStart::netplay`):
/// the game's rows before it keep their numbers and the wrap from ARCADE up still reaches EXIT GAME, then OPTIONS.
const ROWS_TEAM: [usize; 10] = [0, 1, 2, 3, 4, 5, 10, 6, 7, 8];
const ROWS_TEAM_NET: [usize; 11] = [0, 1, 2, 3, 4, 5, 10, 6, 9, 7, 8];

/// The mode spinner's rows under the rules `g` (EXIT GAME, the last, not in the browser's menus; NETWORK
/// when the session has it, from v8: it follows REPLAY; TEAM BATTLE from v22 outside a netplay session: it follows
/// CUSTOM).
pub(super) fn mode_rows(g: Gates, browser: bool, network: bool, netplay: bool) -> &'static [usize] {
    let team = g.team && !netplay;
    let rows: &'static [usize] = if !g.replay_row {
        &ROWS_V1
    } else if g.network_row && network {
        if team { &ROWS_TEAM_NET } else { &ROWS_NET }
    } else if team {
        &ROWS_TEAM
    } else {
        &ROWS
    };
    if browser { &rows[..rows.len() - 1] } else { rows }
}

/// Scrolling mode banner (the text list `screen_title`, widths `screen.title_widths`): ARCADE, VERSUS, STORY,
/// SURVIVAL, WATCH.
const BANNER: [usize; 5] = [10, 11, 12, 15, 14];

/// Game modes (M.9C).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Mode {
    Arcade,
    Versus,
    Watch,
    Survival,
    Story,
    /// CUSTOM GAME START: 0 VS HUMAN, 1 VS COMPUTER, 2 VS COMPUTER ALL MEMBERS (11-fight ladder), 3 PRACTICE
    Custom(u8),
    /// the attract demo of the title
    Demo,
    /// TEAM BATTLE (engine addition, simulation v22): its own screens (`frontend::team`)
    Team,
    /// TEAM ARCADE (engine addition, simulation v26): the team battle's co-op ladder (`frontend::team_arcade`)
    TeamArcade,
}

impl Mode {
    pub(super) fn banner(self) -> (&'static str, i32) {
        let k = match self {
            Mode::Custom(c) => 20 + c.min(3) as usize,
            Mode::Arcade => BANNER[0],
            Mode::Versus => BANNER[1],
            Mode::Story => BANNER[2],
            Mode::Survival => BANNER[3],
            Mode::Watch | Mode::Demo | Mode::Team | Mode::TeamArcade => BANNER[4],
        };
        (text::item("screen_title", k), game_tables::get().title_width(k))
    }

    /// The argument of the select (fight loader): 2 STORY, 3 SURVIVAL / CUSTOM vs all
    /// members (the human/beast picture of P01.DAT), else 1 (0 for ARCADE: the same VS layout).
    pub(super) fn select_arg(self) -> u8 {
        match self {
            Mode::Arcade => 0,
            Mode::Story => 2,
            Mode::Survival | Mode::Custom(2) => 3,
            _ => 1,
        }
    }

    /// (M.9C, M.9B) of the mode.
    pub(super) fn m9c_9b(self) -> (u8, u8) {
        match self {
            Mode::Arcade => (0, 0),
            Mode::Versus => (1, 2),
            Mode::Watch | Mode::Demo => (2, 3),
            // engine addition: the team battle as VS (its own select reads neither)
            Mode::Team | Mode::TeamArcade => (1, 2),
            Mode::Survival => (3, 0),
            Mode::Story => (4, 1),
            Mode::Custom(0) => (5, 2),
            Mode::Custom(1) => (5, 1),
            Mode::Custom(2) => (5, 0),
            Mode::Custom(_) => (5, 4),
        }
    }

    pub(super) fn ladder(self) -> bool {
        matches!(self, Mode::Arcade | Mode::Survival | Mode::Story | Mode::Custom(2))
    }
}

/// What the fight loader needs: characters, costumes, stage, who is the CPU.
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub struct MatchSpec {
    pub chr: [u8; 2],
    pub costume: [u8; 2],
    pub stage: u8,
    pub cpu: [bool; 2],
}

impl MatchSpec {
    /// The ids index the roster, the costumes and the stages (a decoded state: `session::load_state`).
    pub fn check(&self) -> Result<(), String> {
        let ok = self.chr.iter().all(|&c| (c as usize) < SELECTABLE) && self.costume.iter().all(|&c| c < 4) && (self.stage as usize) < STAGE_COUNT;
        if ok { Ok(()) } else { Err(format!("match {self:?}")) }
    }
}

/// What the team battle's loader needs (engine addition, simulation v22): the sub-mode, the
/// four fighters (index k = P(k + 1)), their costumes, the stage, who is a CPU.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TeamSpec {
    pub mode: crate::fight::team::TeamMode,
    pub chr: [u8; 4],
    pub costume: [u8; 4],
    pub stage: u8,
    pub cpu: [bool; 4],
}

impl TeamSpec {
    /// The ids index the select's fighters (the imported game's row too, T5), the costumes and the team stages (the big
    /// ones too; a decoded state or replay header: whether its session may have them is the header's check).
    pub fn check(&self) -> Result<(), String> {
        let ok = self.chr.iter().all(|&c| (c as usize) < SELECTABLE)
            && self.costume.iter().all(|&c| c < 4)
            && crate::fight::stage::team_stage(self.stage as usize, Gates::of(u32::MAX));
        if ok { Ok(()) } else { Err(format!("team battle {self:?}")) }
    }
}

/// Requests to the application.
#[derive(Clone, Serialize, Deserialize)]
pub enum Event {
    /// start loading this fight (the VS screen stays up meanwhile)
    Load(MatchSpec),
    /// the loaded fight can start
    Fight,
    /// play XA track `.0` (None = stop); `.1` = the music request's a1: loop (1) or play once and stop (0)
    Music(Option<(u8, bool)>),
    /// load these asset files and hand them to `Front::fetched`
    Fetch(Vec<String>),
    /// EXIT GAME (engine addition): quit the app
    Exit,
    /// REPLAY (engine addition, simulation v2): open the app's replay list
    Replays,
    /// NETWORK (engine addition, `SessionStart::network`): open the app's NETWORK menu (in a netplay session: leave it)
    Network,
    /// GRAPHICS (engine addition): apply and save these preferences
    Prefs(crate::frontend::options::Prefs),
    /// CHANGE GAME DATA (engine addition, v9): open the app's setup screen (OPTIONS stays up under it)
    ChangeData,
    /// TEAM BATTLE (engine addition, simulation v22): start loading this team battle (its loading screen stays up
    /// meanwhile); `Fight` starts it
    LoadTeam(TeamSpec),
}

/// A load the front end waits for (`Front::load_wanted`): a match or a team battle.
#[derive(Clone, Copy, Debug)]
pub enum LoadSpec {
    Match(MatchSpec),
    Team(TeamSpec),
}

/// Per-port pad words: held, new presses, auto-repeat (the press, then after 16 unchanged frames every 2nd).
/// Pad edges and auto-repeat of both ports (menus).
#[derive(Clone, Default, Serialize, Deserialize)]
pub struct Pads {
    pub held: [u16; 2],
    pub press: [u16; 2],
    pub rep: [u16; 2],
    cnt: [u32; 2],
}

impl Pads {
    pub fn update(&mut self, raw: [u16; 2]) {
        for p in 0..2 {
            let prev = self.held[p];
            self.press[p] = raw[p] & !prev;
            if raw[p] != prev {
                self.cnt[p] = 0;
                self.rep[p] = self.press[p];
            } else {
                self.cnt[p] += 1;
                self.rep[p] = if self.cnt[p] >= 16 && self.cnt[p].is_multiple_of(2) { raw[p] } else { 0 };
            }
            self.held[p] = raw[p];
        }
    }
    /// Pads `held` from frames the front end did not tick (engine addition, the team battle's end): no edge and no
    /// auto-repeat from them, so a button held across the end is not a press on the next screen.
    pub fn hold(&mut self, held: [u16; 2]) {
        *self = Self { held, ..Self::default() };
    }
    pub fn any_press(&self) -> u16 {
        self.press[0] | self.press[1]
    }
    pub fn any_rep(&self) -> u16 {
        self.rep[0] | self.rep[1]
    }
}

/// A message screen's text.
#[derive(Clone, Copy, Serialize, Deserialize)]
pub(super) enum Text {
    Congratulations,
    GameOver,
}

impl Text {
    pub(super) fn str(self) -> &'static str {
        match self {
            Text::Congratulations => text::item("screen_title", 2),
            Text::GameOver => text::get("game_over"),
        }
    }
}

#[derive(Clone, Copy, Serialize, Deserialize)]
pub(super) enum Screen {
    /// title task: `menu` = state 2 (mode select), else state 1 (PRESS START)
    Title {
        menu: bool,
        timer: i32,
    },
    /// the title timed out (state 0xF0): the 33-frame fade to black (fade (1, 8)), then the demo or the logos
    TitleFade {
        k: i32,
    },
    /// a boot logo (n = 3 SCEA, 1 HUDSON): 240 frames, `k` = frame
    Logo {
        n: u8,
        k: i32,
    },
    /// the attract ranking: frames k = 0..1199 after the demo
    Ranking {
        k: i32,
    },
    /// its exit fade (33 frames) over the frozen frame `at`, then the title or the logos
    RankFade {
        k: i32,
        to_title: bool,
        at: i32,
    },
    Select(Select),
    Vs {
        t: u32,
        ready: bool,
        spec: MatchSpec,
    },
    /// the attract demo's fight is loading (no VS screen)
    DemoLoad {
        ready: bool,
        spec: MatchSpec,
    },
    Fight(MatchSpec),
    /// a text screen, then back to the title (or the CUSTOM menu)
    Message {
        t: i32,
        text: Text,
        to_custom: bool,
    },
    /// STORY CONTINUE → NO (page player state 2): GAME OVER for `t` = 900 / 420 … 0 frames, then (`k`) the
    /// 33-frame fade to black and the title
    StoryGameOver {
        t: i32,
        k: Option<i32>,
    },
    /// OPTIONS / SOUND OPTIONS / CUSTOM; `parent` = the OPTIONS menu under SOUND OPTIONS
    Menu {
        menu: Menu,
        parent: Option<Menu>,
    },
    /// NAME ENTRY and the ranking
    NameEntry(crate::frontend::ranking::Entry),
    /// its exit (states 0xF0 / 0xF1): the 33-frame fade (fade (1, 8)), then the title
    NameFade {
        e: crate::frontend::ranking::Entry,
        k: i32,
    },
    /// a movie (loading until `Front::movie` is set), then `then`
    Movie {
        n: usize,
        then: After,
    },
    /// STORY mode / ending stills (`Front::story_screen`)
    Story,
    /// MOVIES AND PICTURES → a picture of /STY/PIC (`ready` once in VRAM)
    Picture {
        n: usize,
        ready: bool,
        menu: Menu,
        parent: Option<Menu>,
    },
    /// the auto-save on the way to the title (the card dialog (1)): `Save::card` on black
    AutoSave,
    /// TEAM BATTLE (engine addition, simulation v22, `frontend::team`): 2 VS 2 / FREE FOR ALL / EXIT
    TeamMode {
        cursor: u8,
        banner_x: i32,
    },
    /// the 4-player select
    TeamSelect(super::team::TeamSel),
    /// the team stage select over the chosen select (`row`: a `fight::stage::TEAM_STAGES` index, its length = RANDOM)
    TeamStage {
        sel: super::team::TeamSel,
        row: u8,
    },
    /// the team battle is loading (`Event::LoadTeam`); `t` frames up
    TeamLoad {
        t: u32,
        ready: bool,
        spec: TeamSpec,
    },
    /// the team battle runs (`GameState::team`)
    TeamFight(TeamSpec),
    /// its result: the team (2 VS 2) or player (FREE FOR ALL) that won, the rounds by team, REMATCH / CHARACTER
    /// SELECT / TITLE
    TeamResult {
        spec: TeamSpec,
        winner: Option<u8>,
        rounds: [u8; 4],
        cursor: u8,
        banner_x: i32,
    },
    /// TEAM ARCADE's clear card (engine addition, A4; `frontend::team_arcade`), `t` frames up, then CONGRATULATIONS
    TeamArcadeClear {
        t: u32,
    },
}

#[cfg(test)]
mod mode_rows_tests {
    use super::*;

    /// The rows follow the session (its version, its browser menus), never the running build: a replay or a
    /// netplay peer of the other kind visits the same rows.
    #[test]
    fn rows_of_the_session() {
        assert_eq!(
            [
                mode_rows(Gates::of(3), false, false, false).len(),
                mode_rows(Gates::of(3), true, false, false).len(),
                mode_rows(Gates::of(1), false, false, false).len(),
                mode_rows(Gates::of(1), true, false, false).len()
            ],
            [9, 8, 8, 7]
        );
        assert_eq!((mode_rows(Gates::of(3), false, false, false).last(), mode_rows(Gates::of(3), true, false, false).last()), (Some(&EXIT_ITEM), Some(&7)));
        // NETWORK after REPLAY; v1 has neither
        assert_eq!(mode_rows(Gates::of(8), false, true, false), [0, 1, 2, 3, 4, 5, REPLAY_ITEM, NETWORK_ITEM, 7, EXIT_ITEM]);
        assert_eq!(mode_rows(Gates::of(8), true, true, false), [0, 1, 2, 3, 4, 5, REPLAY_ITEM, NETWORK_ITEM, 7]);
        assert_eq!(mode_rows(Gates::of(1), false, true, false), ROWS_V1);
        // before v8 the flag changes nothing
        assert_eq!(mode_rows(Gates::of(7), false, true, false), ROWS);
        assert_eq!(mode_rows(Gates::of(7), true, true, false), &ROWS[..8]);
        // the game's rows keep their order, and the wrap from ARCADE up reaches the same rows (desktop: EXIT GAME,
        // then OPTIONS; the browser: OPTIONS)
        for (b, up) in [(false, 2), (true, 1)] {
            let (a, n) = (mode_rows(Gates::of(8), b, false, false), mode_rows(Gates::of(8), b, true, false));
            assert_eq!(a.iter().filter(|&&k| k != NETWORK_ITEM).collect::<Vec<_>>(), n.iter().filter(|&&k| k != NETWORK_ITEM).collect::<Vec<_>>());
            assert_eq!(a[a.len() - up..], n[n.len() - up..]);
        }
    }

    /// TEAM BATTLE (v22) follows CUSTOM, not in netplay: the rows before it and the wrap from ARCADE up are those of
    /// v21 (the routes that reach STORY by DOWN, OPTIONS by UP stay the same).
    #[test]
    fn team_battle_row() {
        for (browser, network) in [(false, false), (false, true), (true, false), (true, true)] {
            let (old, new) = (mode_rows(Gates::of(21), browser, network, false), mode_rows(Gates::of(22), browser, network, false));
            assert_eq!(new.iter().filter(|&&k| k != TEAM_ITEM).collect::<Vec<_>>(), old.iter().collect::<Vec<_>>());
            assert_eq!(new[6], TEAM_ITEM);
            assert_eq!((new[..6] == old[..6], new[new.len() - 2..] == old[old.len() - 2..]), (true, true));
            assert_eq!(mode_rows(Gates::of(22), browser, network, true), old);
        }
    }

    /// The engine's rows are centred by their text width as the game's table centres its own (data-gated: the fonts
    /// and REPLAY, the game's word, of the test data).
    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn port_rows_half_widths() {
        if crate::formats::tables::export_tables().is_none() {
            eprintln!("skipped: no okb_export");
            return;
        }
        let f = crate::output::font::fonts();
        for k in [REPLAY_ITEM, EXIT_ITEM, NETWORK_ITEM, TEAM_ITEM] {
            assert_eq!(mode_half(k), f.menu_width(MODES[k].text()) / 2, "{}", MODES[k].text());
        }
    }

    /// With the test data's text (data-gated, both games) every name and stage name shown is the one the engine showed
    /// before the text tables (its constants, by a CRC-32 per group of the names joined by newlines: no game text here):
    /// the imported game's `names` in its program image's order and its `stages` by the VS select's index map to the right cells.
    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn names_as_before() {
        use crate::assets::roster::{BR1_FIRST, SELECTABLE, beast, name};
        if crate::formats::tables::export_tables().is_none() || crate::formats::tables::br1_export_tables().is_none() {
            eprintln!("skipped: no okb_export (both games)");
            return;
        }
        let crc = |v: Vec<&str>| crate::base::hash::crc32(v.join("\n").as_bytes());
        let br1 = BR1_FIRST..SELECTABLE as u8;
        assert_eq!(crc((0..BR1_FIRST).map(name).collect()), 0x3AFE_1855, "the main game's names");
        assert_eq!(crc((0..BR1_FIRST).map(beast).collect()), 0x5644_3A63, "the main game's beasts");
        assert_eq!(crc(br1.clone().map(name).collect()), 0x564A_43EB, "the imported game's names");
        assert_eq!(crc(br1.map(beast).collect()), 0x0F33_8AE2, "the imported game's beasts");
        let mut stages: Vec<&str> = (0..15).map(stage_name).collect();
        stages.push(stage_label(None).0);
        assert_eq!(crc(stages), 0x8F3B_75AC, "the main game's and the port's stages, RANDOM");
        assert_eq!(crc((15..STAGE_COUNT as u8).map(stage_name).collect()), 0x31A0_35D6, "the imported game's stages");
        let own: Vec<&str> = crate::fight::stage::br1::STAGES.iter().map(|s| s.name()).collect();
        assert_eq!(crc(own), 0x495C_26ED, "the imported game's stages by their fighters");
    }
}
