//! The team battle's screens: the mode choice, the 4-player select, the stage select, the loading screen, the result.
//!
//! Engine addition (the team battle T4); drawn with the console select's and the list menus' primitives
//! State: serialised: `TeamSel` and the team screens are part of `Front` (`Screen`; field changes are `STATE_VERSION`
//!   changes)
//!
//! TEAM BATTLE on the mode menu (from simulation v22, not in netplay) → `Screen::TeamMode` (2 VS 2 / FREE FOR ALL) →
//! `Screen::TeamSelect` (four cursors on the console select's grid; the pad that chose the mode plays P1's or P2's
//! slot, the other pads join with START, the empty slots are CPUs whose fighters the first human picks in slot order
//! once every human has chosen) → `Screen::TeamStage` (the team stages, `fight::stage::TEAM_STAGES`, and RANDOM) →
//! `Screen::TeamLoad` (`Event::LoadTeam`, then `Event::Fight`) → `Screen::TeamFight` → `Screen::TeamResult` (REMATCH
//! / CHARACTER SELECT / TITLE). Pads 3 / 4 are `Front::ext` (the session's `FrameInput::ext`).

use super::screen::TEAM_ITEM;
use super::select::{face, step};
use super::{Event, Front, Mode, Screen, TeamSpec};
use crate::assets::roster::{beast, name};
use crate::assets::text;
use crate::assets::text::Label::{self, Game, Port};
use crate::fight::input::pad::{CIRCLE, CROSS, DOWN, L1, LEFT, R1, RIGHT, SELECT, SQUARE, START, TRIANGLE, UP};
use crate::fight::team::{N, TeamMode};
use crate::output::font::{Fonts, pal};
use crate::output::layout::{Anchor, Ax};
use crate::output::prim::{self, Prim, clut, ft4};
use crate::output::sound::Queue;
use serde::{Deserialize, Serialize};

/// The mode choice's rows (to simulation v25).
const MODE_ROWS: [Label; 3] = [Port("2 VS 2"), Port("FREE FOR ALL"), Game("exit")];
/// The mode choice's rows from simulation v26 (TEAM ARCADE, `team_arcade`).
const MODE_ROWS_ARCADE: [Label; 4] = [Port("2 VS 2"), Port("FREE FOR ALL"), Port("TEAM ARCADE"), Game("exit")];
/// The TEAM ARCADE row's index (`MODE_ROWS_ARCADE`).
pub(super) const ARCADE_ROW: u8 = 2;
/// The result screen's rows.
const RESULT_ROWS: [Label; 3] = [Port("REMATCH"), Game("character_select"), Port("TITLE")];
/// Frames the loading screen stays up at least.
const LOAD_MIN: u32 = 90;
/// A decoded team screen's banner offset (it scrolls left from 0 by 4 a frame, wrapping by the banner's width).
pub(super) const BANNER_X: std::ops::RangeInclusive<i32> = -4096..=0;
/// The list menus' colour set of the team screens (`menus::frame`: CUSTOM's green).
pub(super) const STYLE: usize = 1;

/// One player of the team select.
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize)]
pub(super) struct TeamPlayer {
    pub(super) cur: u8,
    /// 0 choosing (a CPU: not reached yet or picked by the first human), 1 chosen
    pub(super) state: u8,
    /// a pad plays this slot (joined with START), else a CPU
    pub(super) human: bool,
    pub(super) costume: u8,
    /// the confirm flash (frames left)
    pub(super) flash: i32,
}

/// The team select (`Screen::TeamSelect`, `Screen::TeamStage`).
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub(super) struct TeamSel {
    pub(super) mode: TeamMode,
    pub(super) p: [TeamPlayer; N],
    /// frames since it opened
    t: i32,
    /// the cursors' CLUT cycle
    cycle: u8,
    banner_x: i32,
}

impl TeamSel {
    /// A select just opened with these players.
    pub(super) fn new(mode: TeamMode, p: [TeamPlayer; N]) -> TeamSel {
        TeamSel { mode, p, t: 0, cycle: 0, banner_x: 0 }
    }

    /// Per player the cursor, state, costume and human flag (a replay's sync hash).
    pub(super) fn sync_key(&self) -> [u32; N] {
        self.p.map(|p| p.cur as u32 | (p.state as u32) << 8 | (p.costume as u32) << 16 | (p.human as u32) << 24)
    }

    /// The first human slot (it picks the CPUs' fighters).
    fn first_human(&self) -> Option<usize> {
        (0..N).find(|&k| self.p[k].human)
    }

    /// The CPU slot being picked: the first one not chosen, once every human has chosen.
    fn cpu_turn(&self) -> Option<usize> {
        if self.p.iter().any(|p| p.human && p.state == 0) {
            return None;
        }
        (0..N).find(|&k| !self.p[k].human && self.p[k].state == 0)
    }

    /// A decoded select's cursors and costumes are the select's cells (with the imported row's, T5) and costumes.
    pub(super) fn check(&self) -> Result<(), String> {
        let ok = self.p.iter().all(|p| (p.cur as usize) < crate::assets::roster::SELECTABLE && p.costume < 4 && p.state < 2 && (0..=8).contains(&p.flash))
            && self.cycle < 6
            && BANNER_X.contains(&self.banner_x);
        if ok { Ok(()) } else { Err("front end: team select".into()) }
    }
}

/// The team battle's spec from the select: the chosen costumes, a duplicate's changed to the next one no earlier
/// fighter of its character wears (an imported fighter's three, T5: a fourth
/// duplicate keeps its own).
pub(super) fn team_spec(sel: &TeamSel, stage: u8) -> TeamSpec {
    team_spec_of(sel.mode, sel.p.map(|p| p.cur), sel.p.map(|p| p.costume), stage, sel.p.map(|p| !p.human))
}

/// `team_spec` from the fighters, their chosen costumes, the stage and the CPUs (TEAM ARCADE: the ladder's battles).
pub(super) fn team_spec_of(mode: TeamMode, chr: [u8; N], mut costume: [u8; N], stage: u8, cpu: [bool; N]) -> TeamSpec {
    for k in 1..N {
        let n = if crate::assets::roster::is_br1(chr[k]) { 3 } else { 4 };
        for _ in 0..n {
            if !(0..k).any(|j| chr[j] == chr[k] && costume[j] == costume[k]) {
                break;
            }
            costume[k] = (costume[k] + 1) % n;
        }
    }
    TeamSpec { mode, chr, costume, stage, cpu }
}

/// Fighter `k`'s colour (`fight::team::hud::colour`: 2 VS 2 the team's, FREE FOR ALL the player's) as a fill.
fn tint(mode: TeamMode, k: usize) -> [u8; 3] {
    [[96, 0, 0], [0, 16, 112], [0, 80, 0], [88, 72, 0]][mode.team(k)]
}

/// The mode's banner (its text's menu-font width + 64, as the game's banners repeat); TEAM ARCADE's with `arcade`.
fn banner_of(f: &Fonts, mode: Option<TeamMode>, arcade: bool) -> (&'static str, i32) {
    let s = match mode {
        _ if arcade => "TEAM ARCADE",
        Some(TeamMode::Vs2) => "2 VS 2",
        Some(TeamMode::Ffa) => "FREE FOR ALL",
        None => "TEAM BATTLE",
    };
    (s, f.menu_width(s) + 64)
}

/// The winner's line of the result: TEAM n WINS (2 VS 2), Pn WINS (FREE FOR ALL), or DRAW GAME (T7: no winner).
fn winner_text(mode: TeamMode, winner: Option<u8>) -> String {
    match (mode, winner) {
        (TeamMode::Vs2, Some(t)) => format!("TEAM {} WINS", t + 1),
        (TeamMode::Ffa, Some(k)) => format!("P{} WINS", k + 1),
        (_, None) => "DRAW GAME".to_string(),
    }
}

impl Front {
    /// Engine addition (T5, simulation v23): the team select's imported row (the QUICK select's third row) and BIG
    /// STAGE, when the session has the imported game's data.
    pub(super) fn team_br1(&self) -> bool {
        self.br1 && self.gates().team_br1
    }

    /// The team stage panel's stages before RANDOM: the enlarged main-game stages (`stage::TEAM_STAGES`), then from
    /// simulation v23 the big ones the session has the data of (`stage::TEAM_BIG_STAGES`: Q3DM6 with its files, BIG
    /// STAGE with the imported game's).
    pub(super) fn team_stages(&self) -> Vec<u8> {
        let big_ok = self.gates().team_big_stages;
        let q3dm6 = big_ok && self.extras & 1 << crate::fight::stage::extra::Q3DM6.index() != 0;
        let big = |&s: &usize| if s == crate::fight::stage::extra::Q3DM6.stage as usize { q3dm6 } else { self.team_br1() };
        let bigs = crate::fight::stage::TEAM_BIG_STAGES.iter().filter(|s| big_ok && big(s));
        crate::fight::stage::TEAM_STAGES.iter().chain(bigs).map(|&s| s as u8).collect()
    }

    /// The four pads' new presses and auto-repeats (pads 1 / 2 `pads`, pads 3 / 4 `ext`).
    pub(super) fn press4(&self) -> ([u16; N], [u16; N]) {
        ([self.pads.press[0], self.pads.press[1], self.ext.press[0], self.ext.press[1]], [self.pads.rep[0], self.pads.rep[1], self.ext.rep[0], self.ext.rep[1]])
    }

    /// The mode choice's rows: TEAM ARCADE's from simulation v26 (`Gates::team_arcade`), else the three of v22.
    pub(super) fn team_mode_rows(&self) -> &'static [Label] {
        if self.gates().team_arcade { &MODE_ROWS_ARCADE } else { &MODE_ROWS }
    }

    /// TEAM BATTLE on the mode menu: the mode choice in the list menus' frame (the next emblem, the music stopped).
    pub(super) fn enter_team_mode(&mut self, port: usize, audio: &mut Queue) {
        self.next_emblem();
        audio.stop_music();
        self.run.mode = Mode::Team;
        self.run.team_ladder = None;
        self.team_port = port as u8;
        self.screen = Screen::TeamMode { cursor: 0, banner_x: 0 };
    }

    pub(super) fn tick_team_mode(&mut self, cursor: u8, banner_x: i32, audio: &mut Queue) -> Option<Event> {
        let (press, rep) = self.press4();
        let (any, any_rep) = (press.iter().fold(0, |a, p| a | p), rep.iter().fold(0, |a, p| a | p));
        let bw = banner_of(crate::output::font::fonts(), None, false).1;
        let mut banner_x = banner_x - 4;
        while banner_x < -bw {
            banner_x += bw;
        }
        let arcade = self.gates().team_arcade;
        let n = self.team_mode_rows().len() as u8;
        let exit = n - 1;
        let mut cursor = cursor;
        if any_rep & DOWN != 0 {
            cursor = (cursor + 1) % n;
            self.play(audio, 0x0A, 0);
        } else if any_rep & UP != 0 {
            cursor = (cursor + n - 1) % n;
            self.play(audio, 0x0A, 0);
        }
        self.screen = Screen::TeamMode { cursor, banner_x };
        // (v26) START on EXIT leaves too (to v25 it chose FREE FOR ALL)
        let exit_buttons = if arcade { SQUARE | CROSS | CIRCLE | START } else { SQUARE | CROSS | CIRCLE };
        if any & TRIANGLE != 0 || (any & exit_buttons != 0 && cursor == exit) {
            self.play(audio, 0x0D, 0);
            self.team_to_menu(audio);
            return None;
        }
        if any & (SQUARE | CROSS | CIRCLE | START) != 0 {
            self.play(audio, 0x0C, 0);
            let mut p = [TeamPlayer::default(); N];
            for (k, pl) in p.iter_mut().enumerate() {
                pl.cur = self.team_last[k];
            }
            // (v26) TEAM ARCADE: its run and ladder, then its select
            if arcade && cursor == ARCADE_ROW {
                self.begin_team_arcade(self.team_port as usize);
                self.enter_team_select(self.arcade_sel(p), audio);
                return None;
            }
            let mode = if cursor == 0 { TeamMode::Vs2 } else { TeamMode::Ffa };
            // the pad that chose the mode plays at once
            let port = (self.team_port as usize).min(N - 1);
            p[port].human = true;
            self.enter_team_select(TeamSel::new(mode, p), audio);
        }
        None
    }

    /// Back to the mode menu's TEAM BATTLE row (a TEAM ARCADE run ends: `end_team_arcade`).
    pub(super) fn team_to_menu(&mut self, audio: &mut Queue) {
        self.end_team_arcade();
        self.enter_title(audio);
        if let Screen::Title { menu, .. } = &mut self.screen {
            *menu = true;
        }
        let rows = super::mode_rows(self.gates(), self.browser, self.network, self.netplay);
        if let Some(r) = rows.iter().position(|&k| k == TEAM_ITEM) {
            self.menu_cursor = r;
        }
    }

    /// The select with the console select's music (track 0x1F, looped).
    pub(super) fn enter_team_select(&mut self, sel: TeamSel, audio: &mut Queue) {
        log::info!("front: team select ({:?})", sel.mode);
        self.screen = Screen::TeamSelect(sel);
        // (T5) the imported row's faces (15-bit, `assets::br1_art`) at (640,160), as the QUICK select's
        if self.team_br1() {
            self.gfx.vram.load(super::Tim::Br1Faces, &self.data.br1.faces, None, Some((640, 160)));
            self.gfx.vram_gen = self.gfx.vram_gen.wrapping_add(1);
        }
        if let Some(m) = &self.data.select_music {
            audio.play_music(0x1F, m.clone(), true);
        }
    }

    pub(super) fn tick_team_select(&mut self, audio: &mut Queue) -> Option<Event> {
        let (press, rep) = self.press4();
        let (chars, costumes4) = (self.chars(), self.costumes4());
        // (T5) the imported row's cells: only with the row (the save's character bits past the main game's 11 are
        // not trusted: a hand-edited save would let a random pick an imported fighter without its data)
        let br1 = self.team_br1();
        let first = crate::assets::roster::BR1_FIRST;
        let avail = (chars & ((1 << first) - 1)) | if br1 { 0xF << first } else { 0 };
        let Screen::TeamSelect(mut sel) = self.screen else { return None };
        let mut sounds: Vec<u16> = Vec::new();
        sel.t = sel.t.wrapping_add(1);
        if self.frame & 3 == 0 {
            sel.cycle = (sel.cycle + 1) % 6;
        }
        let bw = banner_of(crate::output::font::fonts(), Some(sel.mode), self.arcade_team().is_some()).1;
        sel.banner_x -= 4;
        while sel.banner_x < -bw {
            sel.banner_x += bw;
        }
        for pl in sel.p.iter_mut() {
            pl.flash = (pl.flash - 1).max(0);
        }
        // (TEAM ARCADE, v26) the humans' team: only its two slots are live, the enemies' are the ladder's
        let arcade = self.arcade_team();
        let live = |k: usize| arcade.is_none_or(|t| k & 1 == t);
        // a CPU slot's pad joins with START: it chooses from where its cursor is (TEAM ARCADE: the partner's slot, from
        // the first of its pads to press START, `TeamLadder::partner_ports`)
        let mut joined = [false; N];
        match arcade {
            None => {
                for k in 0..N {
                    if !sel.p[k].human && press[k] & START != 0 {
                        sel.p[k] = TeamPlayer { cur: sel.p[k].cur, human: true, ..Default::default() };
                        joined[k] = true;
                        sounds.push(0x41);
                    }
                }
            }
            Some(t) if !sel.p[t + 2].human => {
                if let Some(k) = self.arcade_join(press) {
                    sel.p[k] = TeamPlayer { cur: sel.p[k].cur, human: true, ..Default::default() };
                    joined[k] = true;
                    sounds.push(0x41);
                }
            }
            Some(_) => {}
        }
        // each slot's pad (TEAM ARCADE: the pads by slot, `TeamLadder::slot_port`)
        let ports: [usize; N] = self.team_slot_ports().map_or([0, 1, 2, 3], |s| std::array::from_fn(|k| s[k].map_or(k, |p| p as usize)));
        // △: a chosen human chooses again (the CPUs too); a human choosing leaves (its slot a CPU), the last one back to
        // the mode choice (TEAM ARCADE: the partner leaves, the starter ends the run)
        let backs: Vec<usize> = (0..N).filter(|&k| sel.p[k].human && !joined[k] && press[ports[k]] & TRIANGLE != 0).collect();
        for k in backs {
            sounds.push(0x0D);
            if sel.p[k].state != 0 {
                sel.p[k].state = 0;
            } else if arcade.is_some_and(|t| k == t + 2) {
                sel.p[k].human = false;
                if let Some(l) = &mut self.run.team_ladder {
                    l.partner_port = None;
                }
            } else if arcade.is_none() && sel.p.iter().filter(|p| p.human).count() > 1 {
                sel.p[k].human = false;
            } else {
                for id in sounds {
                    self.play(audio, id, 0);
                }
                audio.stop_music();
                self.team_last = sel.p.map(|p| p.cur);
                let cursor = if arcade.is_some() { ARCADE_ROW } else { (sel.mode == TeamMode::Ffa) as u8 };
                self.end_team_arcade();
                self.screen = Screen::TeamMode { cursor, banner_x: 0 };
                return None;
            }
            for (k, p) in (0..).zip(sel.p.iter_mut()) {
                if !p.human && live(k) {
                    p.state = 0;
                }
            }
        }
        // who moves which cursor: each human choosing its own, the first human the CPU slot whose turn it is
        let mut drive: Vec<(usize, usize)> = (0..N).filter(|&k| sel.p[k].human && sel.p[k].state == 0 && !joined[k]).map(|k| (k, ports[k])).collect();
        if let (Some(c), Some(h)) = (sel.cpu_turn(), sel.first_human()) {
            drive.push((c, ports[h]));
        }
        for (k, pad) in drive {
            let (r, p) = (rep[pad], press[pad]);
            if let Some(d) = [UP, RIGHT, DOWN, LEFT].iter().position(|&b| r & b != 0) {
                let n = step(sel.p[k].cur, d, avail, br1);
                if n != sel.p[k].cur {
                    sel.p[k].cur = n;
                    sounds.push(0x40);
                }
            }
            // SELECT on a CPU slot's turn: a random fighter (one of the selectable ones), chosen in its first costume
            if !sel.p[k].human && p & SELECT != 0 {
                let avail: Vec<u8> = (0..crate::assets::roster::SELECTABLE as u8).filter(|&c| avail >> c & 1 != 0).collect();
                if !avail.is_empty() {
                    let c = avail[self.rand() as usize % avail.len()];
                    sel.p[k] = TeamPlayer { cur: c, state: 1, human: false, costume: 0, flash: 8 };
                    sounds.push(0x41);
                }
                continue;
            }
            let start_ok = costumes4 >> sel.p[k].cur & 1 != 0 && sel.p[k].human;
            if p & (SQUARE | CROSS | CIRCLE) != 0 || (start_ok && p & START != 0) {
                let pl = &mut sel.p[k];
                pl.costume = if p & SQUARE != 0 {
                    0
                } else if p & CROSS != 0 {
                    1
                } else if p & CIRCLE != 0 {
                    2
                } else {
                    3
                };
                pl.state = 1;
                pl.flash = 8;
                sounds.push(0x41);
            }
        }
        for id in sounds {
            self.play(audio, id, 0);
        }
        let done = sel.p.iter().all(|p| p.state == 1 && p.flash == 0);
        self.team_last = sel.p.map(|p| p.cur);
        // TEAM ARCADE: no stage panel, the ladder's battle at once
        if done && let (Some(t), Some(l)) = (arcade, &mut self.run.team_ladder) {
            l.chr = [sel.p[t].cur, sel.p[t + 2].cur];
            l.costume = [sel.p[t].costume, sel.p[t + 2].costume];
            if let Some(spec) = self.team_arcade_spec() {
                return Some(self.enter_team_load(spec, audio));
            }
            // never: the arcade select is only open with a stage left (`check`: a run's select has `idx < len`); the
            // select stays as it is
            debug_assert!(false, "an arcade select with no stage left");
            self.screen = Screen::TeamSelect(sel);
            return None;
        }
        if done {
            self.screen = Screen::TeamStage { sel, row: self.team_stage };
        } else {
            self.screen = Screen::TeamSelect(sel);
        }
        None
    }

    /// The stage select: LEFT / RIGHT (or L1 / R1) by any human, a confirm by any human loads the battle, △ back to
    /// the select (every player choosing again).
    pub(super) fn tick_team_stage(&mut self, mut sel: TeamSel, row: u8, audio: &mut Queue) -> Option<Event> {
        let (press, rep) = self.press4();
        let humans = |v: [u16; N]| (0..N).filter(|&k| sel.p[k].human).fold(0, |a, k| a | v[k]);
        let (p, r) = (humans(press), humans(rep));
        let stages = self.team_stages();
        let n = stages.len() as u8 + 1;
        let mut row = row % n;
        sel.t = sel.t.wrapping_add(1);
        let bw = banner_of(crate::output::font::fonts(), Some(sel.mode), false).1;
        sel.banner_x -= 4;
        while sel.banner_x < -bw {
            sel.banner_x += bw;
        }
        if r & (RIGHT | R1) != 0 {
            row = (row + 1) % n;
            self.play(audio, 0x40, 0);
        } else if r & (LEFT | L1) != 0 {
            row = (row + n - 1) % n;
            self.play(audio, 0x40, 0);
        }
        self.team_stage = row;
        if p & TRIANGLE != 0 {
            self.play(audio, 0x0D, 0);
            for pl in sel.p.iter_mut() {
                pl.state = 0;
            }
            self.screen = Screen::TeamSelect(sel);
            return None;
        }
        if p & (SQUARE | CROSS | CIRCLE | START) != 0 {
            self.play(audio, 0x41, 0);
            // RANDOM: one of the enlarged main-game stages (not the big ones: it draws the same with or without
            // their data, as the match's RANDOM)
            let stage = match stages.get(row as usize) {
                Some(&s) => s,
                None => crate::fight::stage::TEAM_STAGES[self.rand() as usize % crate::fight::stage::TEAM_STAGES.len()] as u8,
            };
            let spec = team_spec(&sel, stage);
            return Some(self.enter_team_load(spec, audio));
        }
        self.screen = Screen::TeamStage { sel, row };
        None
    }

    /// The loading screen: the battle asked for (`Event::LoadTeam`).
    pub(super) fn enter_team_load(&mut self, spec: TeamSpec, audio: &mut Queue) -> Event {
        audio.stop_music();
        log::info!("front: team battle {spec:?}");
        // (TEAM ARCADE) the humans' slots only: the enemies' pairs (the locked boss pair too) are no cell the select's
        // cursors could reach
        let ours = self.arcade_team();
        for k in 0..N {
            if ours.is_none_or(|t| k & 1 == t) {
                self.team_last[k] = spec.chr[k];
            }
        }
        self.screen = Screen::TeamLoad { t: 0, ready: false, spec };
        Event::LoadTeam(spec)
    }

    /// The loading screen until the battle is loaded and `LOAD_MIN` frames are up. (TEAM ARCADE) a partner pad's START
    /// turns a CPU partner into a human with the same fighter (`arcade_join`).
    pub(super) fn tick_team_load(&mut self, t: u32, ready: bool, mut spec: TeamSpec, audio: &mut Queue) -> Option<Event> {
        if self.arcade_team().is_some()
            && let Some(k) = self.arcade_join(self.press4().0)
        {
            spec.cpu[k] = false;
            self.play(audio, 0x41, 0);
        }
        if ready && t.saturating_add(1) >= LOAD_MIN {
            self.screen = Screen::TeamFight(spec);
            return Some(Event::Fight);
        }
        self.screen = Screen::TeamLoad { t: t.saturating_add(1), ready, spec };
        None
    }

    /// The team battle ended (`session::step`): its result screen, or after the pause's QUIT the mode menu (TEAM
    /// ARCADE: `team_arcade_over`). `held`: the four pads at its last frame (the front end is not ticked during the
    /// battle: a START that skipped the win pose, or buttons mashed at the KO, are not new presses on the result screen).
    pub fn team_over(&mut self, outcome: crate::fight::sim::Outcome, rounds: [u8; N], held: [u16; N], audio: &mut Queue) -> Option<Event> {
        let Screen::TeamFight(spec) = self.screen else { return None };
        self.pads.hold([held[0], held[1]]);
        self.ext.hold([held[2], held[3]]);
        audio.stop_music();
        if self.arcade_team().is_some() {
            return self.team_arcade_over(outcome, rounds, audio);
        }
        match outcome {
            crate::fight::sim::Outcome::Winner(w) => {
                log::info!("front: team battle over, team {w} won ({rounds:?})");
                self.next_emblem();
                self.screen = Screen::TeamResult { spec, winner: Some(w as u8), rounds, cursor: self.team_result_row, banner_x: 0 };
            }
            // (T7, simulation v25) DRAW GAME: the result without a winner
            crate::fight::sim::Outcome::Draw => {
                log::info!("front: team battle over, a draw ({rounds:?})");
                self.next_emblem();
                self.screen = Screen::TeamResult { spec, winner: None, rounds, cursor: self.team_result_row, banner_x: 0 };
            }
            _ => self.team_to_menu(audio),
        }
        None
    }

    pub(super) fn tick_team_result(
        &mut self,
        spec: TeamSpec,
        winner: Option<u8>,
        rounds: [u8; N],
        cursor: u8,
        banner_x: i32,
        audio: &mut Queue,
    ) -> Option<Event> {
        let (press, rep) = self.press4();
        let humans = |v: [u16; N]| (0..N).filter(|&k| !spec.cpu[k]).fold(v[0] | v[1], |a, k| a | v[k]);
        let (p, r) = (humans(press), humans(rep));
        let bw = banner_of(crate::output::font::fonts(), Some(spec.mode), false).1;
        let mut banner_x = banner_x - 4;
        while banner_x < -bw {
            banner_x += bw;
        }
        let n = RESULT_ROWS.len() as u8;
        let mut cursor = cursor;
        if r & DOWN != 0 {
            cursor = (cursor + 1) % n;
            self.play(audio, 0x0A, 0);
        } else if r & UP != 0 {
            cursor = (cursor + n - 1) % n;
            self.play(audio, 0x0A, 0);
        }
        self.screen = Screen::TeamResult { spec, winner, rounds, cursor, banner_x };
        // △ (the frame's EXIT): the mode menu, as TITLE
        if p & TRIANGLE != 0 {
            self.play(audio, 0x0D, 0);
            self.team_to_menu(audio);
            return None;
        }
        if p & (SQUARE | CROSS | CIRCLE | START) == 0 {
            return None;
        }
        self.play(audio, 0x0C, 0);
        self.team_result_row = cursor;
        match cursor {
            0 => Some(self.enter_team_load(spec, audio)),
            1 => {
                let p = std::array::from_fn(|k| TeamPlayer { cur: spec.chr[k], human: !spec.cpu[k], ..Default::default() });
                self.enter_team_select(TeamSel::new(spec.mode, p), audio);
                None
            }
            _ => {
                self.team_to_menu(audio);
                None
            }
        }
    }

    /// The mode choice in the list menus' frame.
    pub(super) fn draw_team_mode(&self, out: &mut Vec<Prim>, cursor: u8, banner_x: i32) {
        let f = crate::output::font::fonts();
        super::menus::frame(out, f, STYLE);
        let (name, bw) = banner_of(f, None, false);
        super::menus::banner(f, out, name, bw, banner_x, 48, 7);
        f.menu_text(out, name, 48, 48, 0);
        for (k, row) in self.team_mode_rows().iter().enumerate() {
            let y = 80 + 24 * k as i32;
            let hl = k == cursor as usize;
            if hl {
                f.menu_text(out, ">", 32, y, 0);
            }
            f.menu_text(out, row.text(), 48, y, if hl { 0 } else { 1 });
        }
    }

    /// The team select: the console select's backdrop, faces and title bar, a plate per player (P1 / P2 at the top, P3 /
    /// P4 under them, as the battle's HUD bars) and the four cursors; the stage select over it.
    pub(super) fn draw_team_select(&self, out: &mut Vec<Prim>, sel: &TeamSel, stage: Option<u8>) {
        let f = crate::output::font::fonts();
        // the console select's backdrop (the menu frame, style 0x18): blue lower half, red upper half, the bottom bar
        out.push(prim::g4(0, 280, 512, 200, [0, 0, 128], [0, 0, 0]));
        out.push(prim::g4(0, 0, 512, 280, [0, 0, 0], [128, 0, 0]));
        // (T5) with the imported row in the bar's place, the bar and the help line at the top (as the QUICK select's)
        let br1 = self.team_br1();
        let hy = if br1 { 0 } else { 424 };
        let mut bar = prim::f4(0, hy, 512, 24, [0, 0, 32]);
        bar.semi = true;
        bar.tpage = 0x20;
        out.push(bar);
        let bc = clut(352, 488);
        for (x, u) in [(248, 16), (232, 32), (216, 48), (328, 0)] {
            out.push(ft4(x, hy + 4, 16, 16, (u, 216, 16, 16), 0x1E, bc));
        }
        f.menu_small(out, text::get("ok"), 192, hy + 8, 0);
        f.menu_small(out, text::get("back"), 288, hy + 8, 0);
        // the face icons (INIT 24, CLUT row 480 + c from SE00)
        let chars = self.chars();
        for c in (0..11usize).rev().filter(|&c| chars >> c & 1 != 0) {
            let (x, y) = face(c as u8);
            out.push(ft4(x, y, 56, 63, ((c as i32 & 3) * 64, (c as i32 >> 2) * 64 + 1, 56, 63), 0x9D, clut(0, 480 + c as u16)));
        }
        // (T5) the imported row (15-bit, at (640,160))
        for k in (0..4u8).rev().filter(|_| br1) {
            let (x, y) = face(crate::assets::roster::BR1_FIRST + k);
            out.push(ft4(x, y, 56, 63, (56 * k as i32, 160, 56, 63), 0x10A, 0));
        }
        // the cursors: P1 / P3 the game's P1 frame, P2 / P4 its P2 frame, cycling until chosen; a CPU's only on its
        // turn or once chosen
        let turn = sel.cpu_turn();
        // (TEAM ARCADE) the enemies' slots: no cursor, "COM ?" (the ladder decides)
        let arcade = self.arcade_team();
        let live = |k: usize| arcade.is_none_or(|t| k & 1 == t);
        let shown = |k: usize| live(k) && (sel.p[k].human || sel.p[k].state > 0 || turn == Some(k));
        for k in (0..N).filter(|&k| shown(k)) {
            let pl = &sel.p[k];
            let (x, y) = face(pl.cur);
            let row = if pl.state > 0 { 491 } else { 485 + sel.cycle as u16 };
            let mut p = ft4(x, y, 56, 63, (if k & 1 == 0 { 0x40 } else { 0x78 }, 0xC0, 56, 63), 0x0E, clut(448, row));
            p.semi = true;
            out.push(p);
        }
        // the metal title bar: 4 × 128 px, the left half mirrored
        for (k, row) in [489u16, 490, 490, 489].iter().enumerate() {
            let x = 128 * k as i32;
            let mut p = ft4(x, 32, 128, 32, (128, 128, 127, 24), 0x1E, clut(352, *row)).at(Anchor::new(Ax::Stretch, Ax::Center));
            if k < 2 {
                for c in 0..4 {
                    p.uv[c][0] = 383.0 - p.uv[c][0];
                }
            }
            out.push(p);
        }
        let (bname, bw) = banner_of(f, Some(sel.mode), self.arcade_team().is_some());
        super::menus::banner(f, out, bname, bw, sel.banner_x, 40, 7);
        f.menu_text(out, "TEAM SELECT", 256 - f.menu_width("TEAM SELECT") / 2, 40, 0);
        // the plates
        for k in 0..N {
            let pl = &sel.p[k];
            let (x, y) = (if k & 1 == 0 { 16 } else { 272 }, if k < 2 { 76 } else { 176 });
            let mut bg = prim::f4(x, y, 224, 92, tint(sel.mode, k));
            bg.semi = true;
            bg.tpage = 0x20;
            out.push(bg);
            let col = crate::fight::team::hud::colour(sel.mode, k);
            let who = if pl.human { format!("{}P", k + 1) } else { "COM".to_string() };
            f.mid_text(out, &who, x + 8, y + 6, col);
            let flash = pl.flash > 0 && pl.flash & 2 != 0;
            if !live(k) {
                f.menu_text(out, "?", x + 72, y + 12, 5);
                continue;
            }
            if !shown(k) {
                if (self.frame >> 4) & 3 != 0 {
                    f.small_text(out, text::get("press_start"), x + 72, y + 18, pal::WHITE);
                }
                continue;
            }
            f.menu_text(out, name(pl.cur), x + 72, y + 12, if flash { 0 } else { 5 });
            f.menu_small(out, text::get("beast"), x + 72, y + 36, 0);
            f.menu_text(out, beast(pl.cur), x + 72, y + 46, 5);
            let state = if pl.state > 0 {
                text::get("ok")
            } else if pl.human {
                text::get("select")
            } else {
                "COM SELECT  SELECT: RANDOM"
            };
            f.small_text(out, state, x + 8, y + 70, if pl.state > 0 { pal::YELLOW } else { pal::WHITE });
        }
        // the four labels on the cursors' cells (corner by player), in the player's colour
        for k in (0..N).filter(|&k| shown(k)) {
            let (x, y) = face(sel.p[k].cur);
            let (lx, ly) = [(x + 2, y + 2), (x + 34, y + 2), (x + 2, y + 46), (x + 34, y + 46)][k];
            let who = if sel.p[k].human { format!("{}P", k + 1) } else { "CP".to_string() };
            f.small_text(out, &who, lx, ly, crate::fight::team::hud::colour(sel.mode, k));
        }
        if let Some(row) = stage {
            // an opaque panel over the plates, framed in the title bar's grey
            out.push(prim::f4(92, 124, 328, 104, [96, 96, 96]));
            out.push(prim::f4(96, 128, 320, 96, [0, 0, 0]));
            let name = self.team_stages().get(row as usize).map_or(crate::assets::text::item("stages", 9), |&s| super::stage_name(s));
            f.menu_text(out, "STAGE SELECT", 256 - f.menu_width("STAGE SELECT") / 2, 144, 4);
            f.menu_text(out, name, 256 - f.menu_width(name) / 2, 176, 3);
            f.menu_text(out, "<", 120, 176, 4);
            f.menu_text(out, ">", 384, 176, 4);
        }
    }

    /// The loading screen: the four fighters, the teams side by side (2 VS 2) or all four against each other.
    pub(super) fn draw_team_load(&self, out: &mut Vec<Prim>, spec: &TeamSpec) {
        let f = crate::output::font::fonts();
        super::menus::frame_with(out, f, STYLE, false);
        let (bname, _) = banner_of(f, Some(spec.mode), self.arcade_team().is_some());
        f.menu_text(out, bname, 256 - f.menu_width(bname) / 2, 64, 0);
        // (TEAM ARCADE) the ladder's stage
        if let Some(h) = self.team_arcade_header() {
            f.menu_text(out, &h, 256 - f.menu_width(&h) / 2, 100, 4);
        }
        for k in 0..N {
            let (x, y) = (if k & 1 == 0 { 64 } else { 288 }, if k < 2 { 160 } else { 240 });
            let who = format!("{}{}", if spec.cpu[k] { "COM " } else { "" }, name(spec.chr[k]));
            f.mid_text(out, &format!("P{}", k + 1), x, y, crate::fight::team::hud::colour(spec.mode, k));
            f.menu_text(out, &who, x + 40, y + 6, 1);
        }
        f.big_text_centered(out, text::item("modes", 1), 256, 196, pal::BIG_ORANGE);
        f.menu_text(out, super::stage_name(spec.stage), 256 - f.menu_width(super::stage_name(spec.stage)) / 2, 340, 4);
    }

    /// The result: the winner, every fighter with its team's rounds, the rows.
    pub(super) fn draw_team_result(&self, out: &mut Vec<Prim>, spec: &TeamSpec, winner: Option<u8>, rounds: &[u8; N], cursor: u8, banner_x: i32) {
        let f = crate::output::font::fonts();
        super::menus::frame(out, f, STYLE);
        let (bname, bw) = banner_of(f, Some(spec.mode), false);
        super::menus::banner(f, out, bname, bw, banner_x, 48, 7);
        f.menu_text(out, "RESULT", 48, 48, 0);
        let w = winner_text(spec.mode, winner);
        let pal = match winner {
            Some(t) => crate::fight::team::hud::colour(spec.mode, t as usize),
            None => pal::WHITE,
        };
        f.mid_text(out, &w, 256 - 8 * w.len() as i32, 96, pal);
        for k in 0..N {
            let y = 150 + 40 * k as i32;
            let team = spec.mode.team(k);
            let won = winner.is_some_and(|t| t as usize == team);
            f.mid_text(out, &format!("P{}", k + 1), 64, y - 4, crate::fight::team::hud::colour(spec.mode, k));
            f.menu_text(out, name(spec.chr[k]), 112, y, if won { 0 } else { 1 });
            if spec.cpu[k] {
                f.menu_small(out, "COM", 272, y + 4, 0);
            }
            f.menu_text(out, &format!("{}", rounds[team]), 336, y, if won { 0 } else { 1 });
        }
        for (k, row) in RESULT_ROWS.iter().enumerate() {
            let y = 324 + 24 * k as i32;
            let hl = k == cursor as usize;
            if hl {
                f.menu_text(out, ">", 32, y, 0);
            }
            f.menu_text(out, row.text(), 48, y, if hl { 0 } else { 1 });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fight::sim::Outcome;

    /// START held across the battle's end (it skipped the win pose) is not a press on the result screen: the screen
    /// stays until START is pressed again (then REMATCH, the first row).
    #[test]
    fn result_ignores_start_held_across_the_end() {
        let root = crate::base::env::var("EXPORT").unwrap_or_else(|_| concat!(env!("CARGO_MANIFEST_DIR"), "/../okb_export").into());
        if !std::path::Path::new(&root).join("manifest.json").exists() {
            eprintln!("skipped: no okb_export");
            return;
        }
        let data = pollster::block_on(crate::frontend::load(&crate::assets::Assets::new(root))).expect("front data");
        let mut audio = Queue::default();
        let mut f = Front::new(std::sync::Arc::new(data), &mut audio, &Default::default());
        let files = crate::session::FileCache::default();
        let spec = TeamSpec {
            mode: TeamMode::Ffa,
            chr: [0, 1, 2, 3],
            costume: [0; 4],
            stage: crate::fight::stage::TEAM_STAGES[0] as u8,
            cpu: [false, true, true, true],
        };
        // the last TeamLoad frame: no button held
        f.tick([0; 2], [0; 2], &mut audio, &files, crate::session::SIM_VERSION);
        f.screen = Screen::TeamFight(spec);
        f.team_over(Outcome::Winner(0), [2, 0, 0, 0], [START, 0, 0, 0], &mut audio);
        assert!(matches!(f.screen, Screen::TeamResult { cursor: 0, .. }));
        for pad in [START, START, 0] {
            assert!(f.tick([pad, 0], [0; 2], &mut audio, &files, crate::session::SIM_VERSION).is_none());
            assert!(matches!(f.screen, Screen::TeamResult { .. }));
        }
        assert!(matches!(f.tick([START, 0], [0; 2], &mut audio, &files, crate::session::SIM_VERSION), Some(Event::LoadTeam(s)) if s == spec));
    }
}
