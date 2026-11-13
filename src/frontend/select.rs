//! The character select: the console select and CHARACTER SELECT = NORMAL.
//!
//! State: serialised: part of `GameState` (field changes are `STATE_VERSION` changes)

use super::Tim;
use super::story_flow::StoryScreen;
use super::vs::vs_band;
use super::{Event, Front, MatchSpec, Mode, Screen};
use crate::assets::roster::br2_name;
use crate::assets::text;
use crate::fight::hud;
use crate::fight::input::pad::{CIRCLE, CROSS, DOWN, L1, LEFT, R1, RIGHT, SQUARE, START, TRIANGLE, UP};
use crate::frontend::menus;
use crate::output::layout::{Anchor, Ax, mirror_tiles, tag};
use crate::output::prim::{self, Prim, clut, ft4};
use crate::output::sound::Queue;
use serde::{Deserialize, Serialize};

/// The select's tables (`assets::game_tables`): the face icon cells (top-left corners), the cursor neighbours (up,
/// right, down, left), the name plates' x of the name and of the beast type (P2 + 0x140), the STORY select's x offsets
/// and the NORMAL select's face order.
use crate::assets::game_tables;
/// Engine addition (the import S9): the centre the plates are placed around (the "BEAST" label's, 0x48 + 45 / 2), for
/// the names the table has no entry for.
const PLATE_CX: i32 = 94;

/// One player of the select (the record at B+0x28+0x30·p).
#[derive(Clone, Copy, Default, Serialize, Deserialize)]
pub(super) struct Player {
    pub(super) cur: u8,
    /// 0 choosing, 1 chosen (waiting for the other), 2 done
    state: u8,
    flash: i32,
    cpu: bool,
    costume: u8,
}

impl Player {
    pub(super) fn costume(&self) -> u8 {
        self.costume
    }
}

#[derive(Clone, Copy, Serialize, Deserialize)]
pub(super) struct Select {
    pub(super) p: [Player; 2],
    pub(super) stage: u8,
    banner_x: i32,
    cycle: u8,
    /// CHARACTER SELECT = NORMAL in ARCADE: the maw screen
    normal: bool,
    /// frames since the select opened
    t: i32,
    /// NORMAL: frame 0 of the exit timeline (X, the frame after all players are done; -1 before)
    exit_x: i32,
    /// NORMAL: the countdown (ctx+0x14, frames)
    timer: i32,
    /// NORMAL, per player: the confirm frame (a CPU side: its human's; -1 none, `KEPT` chosen before a restart),
    /// the start of the cursor CLUT cycle and the last cursor move (that side's portrait and emblem are not drawn)
    confirm_t: [i32; 2],
    cyc0: [i32; 2],
    moved_t: [i32; 2],
    /// NORMAL: the spark bursts of intro frame 10 and of the exit X+30
    sparks: [Option<Burst>; 2],
    /// NORMAL: the CPU side's pad joined during the exit (sp+0x3C / 0x3D): the select restarts at X+81
    rejoin: Option<usize>,
}

impl Select {
    /// Per player the cursor, state, costume and CPU flag (a replay's sync hash: `Front::sync_key`).
    pub(super) fn sync_key(&self) -> [u32; 2] {
        self.p.map(|p| p.cur as u32 | (p.state as u32) << 8 | (p.costume as u32) << 16 | (p.cpu as u32) << 24)
    }
}

/// A spark burst: 100 particles (x, y in 480-line units, vx, vy, size) spawned at `t0`.
#[derive(Clone, Copy, Serialize, Deserialize)]
struct Burst {
    t0: i32,
    #[serde(with = "crate::base::serde_util::big_array")]
    p: [(i16, i16, i8, i8, u8); 100],
}

/// NORMAL select: the first frame of state 6 (inputs, the cursor CLUT cycle) and the exit timeline's length
/// (X+82: the routine returns).
const NORMAL_INPUT: i32 = 63;
const NORMAL_EXIT: i32 = 82;
/// NORMAL: `confirm_t` of a player whose choice is kept through a join restart (its band holds (400|416,500)).
const KEPT: i32 = -100;
/// NORMAL select face cells (240-line coordinates, the NORMAL select's primitives; the face order left to right: the table
/// `select.normal_order`).
const NFACE: [(i32, i32); 11] =
    [(100, 160), (132, 192), (356, 160), (196, 192), (228, 160), (260, 192), (292, 160), (324, 192), (164, 160), (388, 192), (68, 192)];

/// Engine addition (the import S10, simulation v20; a user decision): the NORMAL select with the imported game's data has
/// a third row of the four imported faces (240-line y `NBR1_Y`, in the columns of cells 1 3 5 7 as the QUICK
/// select's row), the main game's two rows `NROW_UP` higher, the countdown `NCOUNT_UP` (480 lines) higher; the faces
/// at `NBR1_FACES` and the portraits at (`NBR1_PORTRAIT` + 128 · side, 0) in VRAM, outside the select's own pages.
const NROW_UP: i32 = 24;
const NBR1_Y: i32 = 200;
const NBR1_X: [i32; 4] = [132, 196, 260, 324];
const NCOUNT_UP: i32 = 24;
const NBR1_FACES: (usize, usize) = (0, 0);
const NBR1_PORTRAIT: usize = 256;

/// NORMAL select: cell `c`'s face's top-left corner (240-line coordinates; `br1`: with the imported row).
fn nface(c: u8, br1: bool) -> (i32, i32) {
    match c.checked_sub(crate::assets::roster::BR1_FIRST) {
        Some(k) => (NBR1_X[k as usize % 4], NBR1_Y),
        None => {
            let (x, y) = NFACE[c as usize % 11];
            (x, if br1 { y - NROW_UP } else { y })
        }
    }
}

impl Front {
    /// Fight-loader state 0/1: the select for the current mode.
    pub(super) fn enter_select(&mut self, audio: &mut Queue) {
        let opp = self.opponent();
        let p = std::array::from_fn(|i| {
            let cpu = !self.run.humans[i] && self.run.mode != Mode::Watch;
            Player { cur: if cpu { opp } else { self.last[i] }, state: if cpu { 2 } else { 0 }, flash: 0, cpu, costume: 0 }
        });
        // RANDOM (the row after the stages) when opt.40 bit 31, else the last stage (M.98)
        let stage = if self.stage_random { self.stage_count() } else { self.stage_row(self.stage) };
        log::info!("front: select ({:?}, fight {})", self.run.mode, self.run.a2);
        let normal = self.run.mode == Mode::Arcade && self.save.settings.char_select == 0;
        let cyc0 = [NORMAL_INPUT; 2];
        self.screen = Screen::Select(Select {
            p,
            stage,
            banner_x: 0,
            cycle: 0,
            normal,
            t: 0,
            exit_x: -1,
            timer: 5999,
            confirm_t: [-1; 2],
            cyc0,
            moved_t: [-1; 2],
            sparks: [None; 2],
            rejoin: None,
        });
        if normal {
            // the SEL00 pages (as the VS screen) and the band background entry 0x20 at (768,0)
            for (k, at) in [(0x31usize, (768, 256)), (0x32, (640, 256)), (0x30, (0, 480)), (0x34, (704, 0)), (0x35, (704, 256))] {
                if let Some(Some(t)) = self.data.sel00.get(k) {
                    self.gfx.vram.load(Tim::Sel00(k as u8), t, None, Some(at));
                }
            }
            if let Some(Some(t)) = self.data.sel00.get(0x20) {
                self.gfx.vram.load(Tim::Sel00(0x20), t, Some((384, 496)), Some((768, 0)));
            }
            self.gfx.vram_gen = self.gfx.vram_gen.wrapping_add(1);
        }
        // engine addition (the import S9): the imported row's faces (15-bit, `assets::br1_art`) at (640,160); on the
        // NORMAL select (S10) at `NBR1_FACES`, outside its own pages
        if self.br1_row() {
            self.gfx.vram.load(Tim::Br1Faces, &self.data.br1.faces, None, Some(if normal { NBR1_FACES } else { (640, 160) }));
            self.gfx.vram_gen = self.gfx.vram_gen.wrapping_add(1);
        }
        for i in 0..2 {
            self.upload_portrait(i, p[i].cur);
        }
        // character select music: track 0x1F (BGM SELECT CONSOLE) / 1 (ARCADE), looped (a1 = 1)
        let (t, m) = if self.arcade_bgm() { (1, &self.data.select_music_arcade) } else { (0x1F, &self.data.select_music) };
        if let Some(m) = m {
            audio.play_music(t, m.clone(), true);
        }
    }

    /// The hovered character's beast picture (P00 entry c, 4-bit 256×256) → (512,0) for P1 / (576,0) for P2.
    fn upload_portrait(&mut self, side: usize, c: u8) {
        if matches!(self.screen, Screen::Select(Select { normal: true, .. })) {
            // engine addition (the import S10): an imported cell's portrait (15-bit 128×160) at (`NBR1_PORTRAIT`
            // + 128·side, 0)
            if let Some(k) = c.checked_sub(crate::assets::roster::BR1_FIRST) {
                if let Some(t) = self.data.br1.select.get(k as usize) {
                    self.gfx.vram.load(Tim::Br1Select(k), t, None, Some((NBR1_PORTRAIT + 128 * side, 0)));
                    self.gfx.vram_gen = self.gfx.vram_gen.wrapping_add(1);
                }
                return;
            }
            // SEL01 portrait → (512 + 96·side, 0) with its CLUT at (0, 501 + side); the beast emblem
            // (SEL00 entry c, 4-bit 96×96) → (832 + 32·side, 0), its CLUT resident at (464, emblem_row(c))
            if let Some(Some(t)) = self.data.sel01.get(0x10 + c as usize) {
                self.gfx.vram.load(Tim::Sel01(c.wrapping_add(0x10)), t, Some((0, 501 + side)), Some((512 + 96 * side, 0)));
            }
            if let Some(Some(t)) = self.data.sel00.get(c as usize) {
                self.gfx.vram.load(Tim::Sel00(c), t, Some((464, emblem_row(c) as usize)), Some((832 + 32 * side, 0)));
            }
            self.gfx.vram_gen = self.gfx.vram_gen.wrapping_add(1);
            return;
        }
        // engine addition (the import S9): an imported cell's portrait (15-bit 128×160) at (640 + 128·side, 0)
        if let Some(k) = c.checked_sub(crate::assets::roster::BR1_FIRST) {
            if let Some(t) = self.data.br1.select.get(k as usize) {
                self.gfx.vram.load(Tim::Br1Select(k), t, None, Some((640 + 128 * side, 0)));
                self.gfx.vram_gen = self.gfx.vram_gen.wrapping_add(1);
            }
            return;
        }
        if self.run.mode.select_arg() >= 2 {
            // only the human side's pictures (P01, 4-bit, no CLUT): beast c → (512,0), human c + 11
            // (Stun: 22) → (640,0)
            if let Screen::Select(sel) = &self.screen
                && sel.p[side].cpu
            {
                return;
            }
            let h = if c == 8 { 22 } else { c as usize + 11 };
            for (k, at) in [(c as usize, (512, 0)), (h, (640, 0))] {
                if let Some(Some(t)) = self.data.p01.get(k) {
                    self.gfx.vram.load(Tim::P01(k as u8), t, None, Some(at));
                }
            }
            self.gfx.vram_gen = self.gfx.vram_gen.wrapping_add(1);
            return;
        }
        if let Some(Some(t)) = self.data.p00.get(c as usize) {
            self.gfx.vram.load(Tim::P00(c), t, None, Some((512 + 64 * side, 0)));
            self.gfx.vram_gen = self.gfx.vram_gen.wrapping_add(1);
        }
    }

    /// A 100-particle spark burst, 5 `rand()` per particle.
    fn burst(&mut self, t0: i32) -> Burst {
        let mut p = [(0, 0, 0, 0, 0); 100];
        for q in p.iter_mut() {
            let x = (self.rand() & 0x1FF) as i16;
            let y = ((self.rand() & 0x1F) + 0xF5) as i16;
            let vx = (((self.rand() & 7) as i32 - 4) / 2) as i8;
            let vy = (((self.rand() & 0xF) as i32 - 4) / 2) as i8;
            let size = ((self.rand() & 7) + 2) as u8;
            *q = (x, y, vx, vy, size);
        }
        Burst { t0, p }
    }

    pub(super) fn tick_select(&mut self, audio: &mut Queue) -> Option<Event> {
        let (chars, costumes4, stages) = (self.chars(), self.costumes4(), self.stage_count());
        // engine addition (the import S9): the imported row's cells, and the extra stages' rows (an imported
        // fighter is refused there: `fight_data::load_match`)
        let br1 = self.br1_row();
        let avail = if br1 { chars | 0xF << crate::assets::roster::BR1_FIRST } else { chars };
        let extra_rows = self.random_count()..self.random_count() + self.extra_rows().count() as u8;
        // NORMAL: spark bursts at intro frame 10 and at exit X+30 (timelines)
        let spawn = match &self.screen {
            Screen::Select(s) if s.normal && (s.t + 1 == 10 || (s.exit_x >= 0 && s.t + 1 == s.exit_x + 30)) => Some(s.t + 1),
            _ => None,
        };
        let burst = spawn.map(|t0| self.burst(t0));
        let Screen::Select(sel) = &mut self.screen else { return None };
        if let Some(b) = burst {
            sel.sparks[(sel.exit_x >= 0) as usize] = Some(b);
        }
        let mut sounds: Vec<(u16, u32)> = Vec::new();
        let mut uploads = Vec::new();
        let watch = self.run.mode == Mode::Watch;
        sel.t += 1;
        let t = sel.t;
        if self.frame & 3 == 0 {
            sel.cycle = (sel.cycle + 1) % 6;
        }
        sel.banner_x -= 4;
        let bw = self.run.mode.banner().1;
        while sel.banner_x < -bw {
            sel.banner_x += bw;
        }
        if sel.normal {
            // a chosen player is done once the other is CPU or has chosen too; all done → state 7, the
            // exit timeline from the next frame (X). The countdown runs in states 3 and 6, 5999 again at state 5.
            for i in 0..2 {
                let o = sel.p[1 - i];
                if sel.p[i].state == 1 && (o.cpu || o.state >= 1) {
                    sel.p[i].state = 2;
                }
            }
            if sel.exit_x < 0 && sel.p.iter().all(|p| p.state == 2) {
                sel.exit_x = t + 1;
            }
            if sel.exit_x < 0 {
                sel.timer = if t == NORMAL_INPUT - 1 { 5999 } else { (sel.timer - 1).max(0) };
            }
            // START on the CPU side's pad joins (status 0x700): that player's last character, the
            // countdown and that cursor's CLUT cycle restart; SE 0x41 now and 0x0C the next frame
            if sel.exit_x < 0 && t != NORMAL_INPUT - 1 {
                for i in 0..2 {
                    if sel.p[i].cpu && !sel.p[1 - i].cpu && self.pads.press[i] & START != 0 {
                        let cur = self.last[i];
                        sel.p[i] = Player { cur, ..Player::default() };
                        sel.confirm_t[i] = -1;
                        sel.cyc0[i] = t.max(NORMAL_INPUT);
                        sel.timer = 5999;
                        self.run.humans[i] = true;
                        sounds.extend([(0x41, 0), (0x0C, 1)]);
                        uploads.push((i, cur));
                    }
                }
            }
            // state 8 (from the state-7 frame X-1): a join only marks the restart (SE 0x0C the next
            // frame); state 9 (X+81) re-inits the select: the joiner chooses from its last character, the
            // other player stays chosen (state 1), the intro runs again from the next frame and this one is blank
            if sel.exit_x >= 0 && t < sel.exit_x + 81 && sel.rejoin.is_none() {
                for i in 0..2 {
                    if sel.p[i].cpu && !sel.p[1 - i].cpu && self.pads.press[i] & START != 0 {
                        sel.rejoin = Some(i);
                        sounds.push((0x0C, 1));
                    }
                }
            }
            if let (Some(i), true) = (sel.rejoin, t == sel.exit_x + 81) {
                let cur = self.last[i];
                self.run.humans[i] = true;
                sel.p[i] = Player { cur, ..Player::default() };
                sel.p[1 - i].state = 1;
                let mut confirm_t = [KEPT; 2];
                confirm_t[i] = -1;
                *sel = Select { t: -1, exit_x: -1, timer: 5999, confirm_t, cyc0: [NORMAL_INPUT; 2], moved_t: [-1; 2], sparks: [None; 2], rejoin: None, ..*sel };
                uploads.push((i, cur));
            }
        }
        let t = sel.t;
        let intro = sel.normal && t < NORMAL_INPUT;
        // cancel: △ (only before the first ladder fight, or in VS; NORMAL: in state 6, SE 0x204 then 0x0D; the
        // other selects: 0x0D, then state 0xF0 sends 0x204 — both stop the music)
        let cancel_ok = if sel.normal { !intro && sel.exit_x < 0 && self.run.a2 == 0 } else { self.run.a2 == 0 || self.run.mode == Mode::Versus };
        if cancel_ok && self.pads.any_press() & TRIANGLE != 0 {
            audio.stop_music();
            self.play(audio, 0x0D, 0);
            if matches!(self.run.mode, Mode::Custom(_)) {
                self.open_menu(menus::Kind::Custom, audio);
                return None;
            }
            self.enter_title(audio);
            if let Screen::Title { menu, .. } = &mut self.screen {
                *menu = true;
            }
            return None;
        }
        // NORMAL: the countdown ran out — every player still choosing confirms with □
        let expired = sel.normal && !intro && sel.exit_x < 0 && sel.timer == 0;
        for i in 0..2 {
            let pl = &mut sel.p[i];
            if pl.flash > 0 {
                pl.flash -= 1;
            }
            if pl.cpu || pl.state != 0 || intro {
                continue;
            }
            // directions / shoulders auto-repeat, START and the face buttons on the press
            let (mut rep, mut press) = (self.pads.rep[i], self.pads.press[i]);
            if watch && i == 1 && sel.p[0].state != 0 {
                rep |= self.pads.rep[0];
                press |= self.pads.press[0];
            }
            let pl = &mut sel.p[i];
            let dir = [UP, RIGHT, DOWN, LEFT].iter().position(|&b| rep & b != 0);
            if let Some(d) = dir {
                let n = step(pl.cur, d, avail, br1);
                if n != pl.cur {
                    pl.cur = n;
                    sounds.push((0x40, 0));
                    uploads.push((i, n));
                    sel.moved_t[i] = t;
                }
            }
            let pl = &mut sel.p[i];
            if expired {
                // the cursor steps on in grid order to an available cell (an imported cell stays: engine addition)
                let br1_cur = crate::assets::roster::is_br1(pl.cur);
                let mut o = game_tables::get().normal_order.iter().position(|&c| c == pl.cur).unwrap_or(0);
                for _ in (0..11).filter(|_| !br1_cur) {
                    if chars >> game_tables::get().normal_order[o] & 1 != 0 {
                        break;
                    }
                    o = (o + 1) % 11;
                }
                if game_tables::get().normal_order[o] != pl.cur && !br1_cur {
                    pl.cur = game_tables::get().normal_order[o];
                    uploads.push((i, pl.cur));
                }
                press |= SQUARE;
            }
            let start_ok = costumes4 >> pl.cur & 1 != 0;
            if press & (SQUARE | CROSS | CIRCLE) != 0 || (start_ok && press & START != 0) {
                pl.costume = if press & SQUARE != 0 {
                    0
                } else if press & CROSS != 0 {
                    1
                } else if press & CIRCLE != 0 {
                    2
                } else {
                    3
                };
                pl.state = 1;
                pl.flash = 8;
                sounds.push((0x41, 0));
                // the same character can't wear the same costume twice: as at the fight setup, a
                // costume already taken by the other (human) player becomes 0, or 1 when it was 0
                let (cur, cos) = (pl.cur, pl.costume);
                let o = sel.p[1 - i];
                if !o.cpu && o.state > 0 && o.cur == cur && o.costume == cos {
                    sel.p[i].costume = if cos != 0 { 0 } else { 1 };
                }
                // NORMAL: a CPU side is revealed (and flashes) with its human's confirm
                sel.confirm_t[i] = t;
                if o.cpu {
                    sel.confirm_t[1 - i] = t;
                }
            }
        }
        let done = if sel.normal {
            sel.exit_x >= 0 && t >= sel.exit_x + NORMAL_EXIT
        } else {
            // a chosen player is done when the other is not choosing
            for i in 0..2 {
                let o = sel.p[1 - i];
                if sel.p[i].state == 1 && (o.state > 0 || (o.cpu && o.flash == 0)) {
                    sel.p[i].state = 2;
                }
            }
            // stage select (L1 / R1 by anyone) when M.9B != 0 and not story (rows 0..9, RANDOM 9; the
            // open select's rows 0..13, RANDOM 13)
            if !self.run.mode.ladder() {
                // engine addition (the import S9): with a cursor on an imported fighter the extra stages' rows are
                // passed over, and one chosen before turns RANDOM
                let refused = |row: u8, sel: &Select| br1 && extra_rows.contains(&row) && sel.p.iter().any(|p| crate::assets::roster::is_br1(p.cur));
                let rep = self.pads.any_rep();
                if rep & (L1 | R1) != 0 {
                    let d = if rep & L1 != 0 { stages } else { 1 };
                    sel.stage = (sel.stage + d) % (stages + 1);
                    while refused(sel.stage, sel) {
                        sel.stage = (sel.stage + d) % (stages + 1);
                    }
                    sounds.push((0x40, 0));
                }
                if refused(sel.stage, sel) {
                    sel.stage = stages;
                }
            }
            sel.p.iter().all(|p| p.state == 2) && sel.p.iter().all(|p| p.flash == 0)
        };
        let (p, stage) = (sel.p, sel.stage);
        for (id, delay) in sounds {
            self.play(audio, id, delay);
        }
        for (i, c) in uploads {
            self.upload_portrait(i, c);
        }
        if !done {
            return None;
        }
        // leave (0xF0): remember the choices, resolve RANDOM
        self.stage_random = stage == stages;
        if stage != stages {
            self.stage = self.row_stage(stage);
        }
        for i in 0..2 {
            if !p[i].cpu {
                self.last[i] = p[i].cur;
                self.run.human_costume[i] = p[i].costume;
            }
        }
        if self.run.mode == Mode::Story {
            // the prologue, then the page player (0)
            let h = if self.run.humans[0] { 0 } else { 1 };
            self.story = Some(crate::frontend::story_screens::Run::new(p[h].cur as usize));
            self.story_screen = Some(StoryScreen::Prologue(crate::frontend::story_screens::Prologue::new()));
            self.screen = Screen::Story;
            audio.stop_music();
            return None;
        }
        let mut spec =
            MatchSpec { chr: [p[0].cur, p[1].cur], costume: [p[0].costume, p[1].costume], stage: 0, cpu: [!self.run.humans[0], !self.run.humans[1]] };
        self.finish_spec(&mut spec);
        Some(self.enter_vs(spec, audio))
    }

    /// Character select (primitive order as captured from the console select in 480i).
    pub(super) fn draw_select(&self, out: &mut Vec<Prim>, sel: &Select) {
        if sel.normal {
            self.draw_select_normal(out, sel);
            return;
        }
        let arg = self.run.mode.select_arg();
        let story = arg == 2;
        // backdrop: the menu frame (style 0x18): blue lower half, red upper half, additive bottom bar, help line;
        // STORY style 0x19 (colour set): red lower half, blue upper half, red bar
        let (lo, up0, up1, tint) = if story { ([128, 0, 0], [0, 0, 128], [0, 0, 0], [32, 0, 0]) } else { ([0, 0, 128], [0, 0, 0], [128, 0, 0], [0, 0, 32]) };
        out.push(prim::g4(0, 280, 512, 200, lo, [0, 0, 0]));
        out.push(prim::g4(0, 0, 512, 280, up0, up1));
        // engine addition (the import S9): with the imported row in the bar's place, the bar and the help line at
        // the top of the screen
        let br1 = self.br1_row();
        let hy = if br1 { 0 } else { 424 };
        let mut bar = prim::f4(0, hy, 512, 24, tint);
        bar.semi = true;
        bar.tpage = 0x20;
        out.push(bar);
        // help line: layout 0x10 "EXIT △" falls into layout 0x40 "OK □×○"; after the
        // first ladder fight (M.A2 ≠ 0, not VS) the style is 0x48: no EXIT, as △ cannot cancel there
        let bc = clut(352, 488);
        let exit = self.run.a2 == 0 || self.run.mode == Mode::Versus;
        for (x, u) in [(248, 16), (232, 32), (216, 48), (328, 0)].into_iter().filter(|&(x, _)| exit || x != 328) {
            out.push(ft4(x, hy + 4, 16, 16, (u, 216, 16, 16), 0x1E, bc));
        }
        crate::output::font::fonts().menu_small(out, text::get("ok"), 192, hy + 8, 0);
        if exit {
            crate::output::font::fonts().menu_small(out, text::get("exit"), 288, hy + 8, 0);
        }
        // arg 2/3: the human side's beast (512,0) then human (640,0), both mirrored (x0 > x1)
        let h = sel.p[0].cpu as usize;
        if arg >= 2 {
            let c = sel.p[h].cur as usize;
            let cl = clut(352, if sel.p[h].flash > 0 { 493 } else { 486 });
            let bx = 320 - game_tables::get().story_beast_x[c];
            let (bw, by) = if c == 8 { (255, 25) } else { (200, 80) };
            for (x, y, w, uw, tp) in [(bx, by, bw, 255, 0x08), (128, 25, 191, 191, 0x0A)] {
                let mut p = ft4(x, y, w, 280 - y, (0, 0, uw, 255), tp, cl);
                for k in 0..4 {
                    p.xy[k][0] = (2 * x + w) as f32 - p.xy[k][0];
                }
                out.push(p);
            }
        }
        // big beast portraits (P1 mirrored; Stun bigger); the confirm flash uses CLUT (352,493)
        for i in (0..2).rev().filter(|_| arg < 2) {
            let pl = &sel.p[i];
            // engine addition (the import S9): an imported cell's portrait (15-bit 128×160 at (640 + 128·i, 0)) at
            // its own aspect in the beast picture's height
            if crate::assets::roster::is_br1(pl.cur) {
                let mut p = ft4(if i == 0 { 0 } else { 512 - 128 }, 80, 128, 200, (0, 0, 127, 159), 0x10A + 2 * i as u16, 0).at(side_anchor(i));
                if i == 0 {
                    for k in 0..4 {
                        p.xy[k][0] = 128.0 - p.xy[k][0];
                    }
                }
                out.push(p);
                continue;
            }
            let cl = clut(352, if pl.flash > 0 { 493 } else { 486 });
            let (w, y0) = if pl.cur == 8 { (255, 25) } else { (200, 80) };
            // engine addition: the portraits keep to the window edges (decision 4)
            let mut p = ft4(if i == 0 { 0 } else { 512 - w }, y0, w, 280 - y0, (0, 0, 255, 255), 0x08 + i as u16, cl).at(side_anchor(i));
            if i == 0 {
                for k in 0..4 {
                    p.xy[k][0] = w as f32 - p.xy[k][0];
                }
            }
            out.push(p);
        }
        // face icons (INIT 24, 8-bit, CLUT row 480 + c from SE00)
        for c in (0..11usize).rev() {
            if self.chars() >> c & 1 != 0 {
                let (x, y) = game_tables::get().faces[c];
                out.push(ft4(x, y, 56, 63, ((c as i32 & 3) * 64, (c as i32 >> 2) * 64 + 1, 56, 63), 0x9D, clut(0, 480 + c as u16)));
            }
        }
        // engine addition (the import S9): the imported row (15-bit, at (640,160): `assets::br1_art`)
        for k in (0..4).rev().filter(|_| br1) {
            let (x, y) = BR1_FACE[k];
            out.push(ft4(x, y, 56, 63, (56 * k as i32, 160, 56, 63), 0x10A, 0));
        }
        // cursor / label kinds: human 1, COM 2 only with M.9B != 0 outside STORY, else none
        let (m9c, m9b) = self.run.mode.m9c_9b();
        let com = m9b != 0 && m9c != 4;
        let shown = [!sel.p[0].cpu || (com && ((!sel.p[1].cpu && sel.p[1].state > 0) || sel.p[1].cpu)), !sel.p[1].cpu || (com && sel.p[0].state > 0)];
        // cursors: 50 % frames from (896,0), CLUT cycling (448,485..490) until chosen
        for i in (0..2).filter(|&i| shown[i]) {
            let pl = &sel.p[i];
            let (x, y) = face(pl.cur);
            let row = if pl.state > 0 { 491 } else { 485 + sel.cycle as u16 };
            let mut p = ft4(x, y, 56, 63, (if i == 0 { 0x40 } else { 0x78 }, 0xC0, 56, 63), 0x0E, clut(448, row));
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
        if story {
            // the name, then the text `s_story` (pen kept after the name), the chapter title
            // tall, CLEAR (story bit c)
            let c = sel.p[h].cur as usize;
            crate::output::font::fonts().menu_text(out, &format!("{}{}", br2_name(c), text::get("s_story")), 188 - game_tables::get().story_name_x[c], 216, 5);
            crate::output::font::fonts().menu_text_tall(out, text::item("story_title", c), 256 - game_tables::get().story_title_x[c], 240, 0);
            if self.save.settings.cleared >> c & 1 != 0 {
                crate::output::font::fonts().menu_small(out, text::get("clear"), 384, 60, 3);
            }
        }
        // name plates: name (pal 5), "BEAST", beast type (arg 3: the human side's only, no CLEAR)
        for i in (0..2).filter(|&i| arg < 2 || (arg == 3 && i == h)) {
            let c = sel.p[i].cur as usize;
            let dx = 0x140 * i as i32;
            let s0 = out.len();
            let f = crate::output::font::fonts();
            // an imported cell's (engine addition) centred under "BEAST" as the table's are, kept 8 px inside the screen
            let (name, beast) = (crate::assets::roster::name(c as u8), crate::assets::roster::beast(c as u8));
            let fit = |w: i32| (PLATE_CX - w / 2 + dx).min(512 - 8 - w).max(8) - dx;
            let (nx, bx) = game_tables::get().plates.get(c).copied().unwrap_or((fit(f.menu_width(name)), fit(f.menu_width(beast))));
            f.menu_text(out, name, nx + dx, 236, 5);
            f.menu_small(out, text::get("beast"), 0x48 + dx, 252, 0);
            f.menu_text(out, beast, bx + dx, 261, 5);
            if arg < 2 && c < 11 && self.save.settings.cleared >> (c + 16) & 1 != 0 {
                crate::output::font::fonts().menu_small(out, text::get("clear"), if i == 0 { 8 } else { 512 - 0x38 }, 60, 3);
            }
            // with the portrait (arg < 2) at its window edge
            if arg < 2 {
                tag(&mut out[s0..], side_anchor(i));
            }
        }
        // scrolling mode banner (pal 7, blended) and the title
        let (name, bw) = self.run.mode.banner();
        crate::frontend::menus::banner(crate::output::font::fonts(), out, name, bw, sel.banner_x, 40, 7);
        crate::output::font::fonts().menu_text(out, text::get("character_select"), 128, 40, 0);
        // 1P / 2P / CPU labels (centred 30×22)
        for i in (0..2).filter(|&i| shown[i]) {
            let pl = &sel.p[i];
            let (x, y) = face(pl.cur);
            let (cx, cy) = if i == 0 { (x + 0x24, y + 0x0F) } else { (x + 0x12, y + 0x31) };
            let v = if pl.cpu {
                0xE9
            } else if i == 0 {
                0xB9
            } else {
                0xD1
            };
            let mut p = ft4(cx - 15, cy - 11, 30, 22, (0xB1, v, 30, 22), 0x0E, clut(448, 491));
            p.semi = true;
            out.push(p);
        }
        if !self.run.mode.ladder() {
            let (name, half) = self.row_label(sel.stage);
            crate::output::font::fonts().menu_text(out, text::get("stage"), 216, 136, 4);
            crate::output::font::fonts().menu_text(out, text::get("select"), 208, 152, 4);
            crate::output::font::fonts().menu_text(out, name, 256 - half, 176, 3);
            crate::output::font::fonts().menu_text(out, "<L1", 200, 200, 4);
            crate::output::font::fonts().menu_text(out, "R1>", 272, 200, 4);
        }
    }

    /// CHARACTER SELECT = NORMAL, from the game's primitives (240-line coordinates, doubled):
    /// the beast maw — jaws (INIT 25 at (512,256), CLUT (448,480+)) and teeth strips (CLUT (480,506+)) — closes
    /// over the screen, flashes and opens onto the select; after the choice the background flashes, the faces
    /// and the title leave, the jaws close, and they open again onto the VS screen's background.
    fn draw_select_normal(&self, out: &mut Vec<Prim>, sel: &Select) {
        let t = sel.t;
        // engine addition (the import S10): the imported row (`nface`)
        let br1 = self.br1_row();
        // the restart frame of a join during the exit draws nothing
        if t < 0 {
            return;
        }
        // frames into the exit timeline, -1 before X
        let d = if sel.exit_x >= 0 && t >= sel.exit_x { t - sel.exit_x } else { -1 };
        // a CPU side stays hidden until its human confirms
        let shown = |i: usize| !sel.p[i].cpu || (sel.confirm_t[i] >= 0 && t >= sel.confirm_t[i]);
        let mut v: Vec<Prim> = Vec::new();
        // a quad from corners (TL, TR, BL, BR) with uv rectangle (u0, v0)-(u1, v1)
        let quad = |xy: [[i32; 2]; 4], uv: (i32, i32, i32, i32), tp: u16, cl: u16| -> Prim {
            let mut p = ft4(0, 0, 1, 1, (0, 0, 1, 1), tp, cl);
            p.xy = xy.map(|[x, y]| [x as f32, y as f32]);
            p.uv = [[uv.0 as f32, uv.1 as f32], [uv.2 as f32, uv.1 as f32], [uv.0 as f32, uv.3 as f32], [uv.2 as f32, uv.3 as f32]];
            p
        };
        let lerp_tab = |tab: &[i32], k: i32| tab[(k.max(0) as usize).min(tab.len() - 1)];
        // ---- background bands (ot 22): per side 8 strips scrolling up 6 px a frame until that side's confirm, then
        // one quad flashing (384,500..508) → (400|416,496..500), v 64 while waiting for the other, 128
        // when done. Exit: shrinking to y 120 from X+22 with (400|416,501..508), (432,496..504) from
        // X+30; from X+55 the VS background bands, pairs from the middle out (ctx flags 0x80 / 0x100 / 0x200 / 0x400
        // set by the exit timeline at X+55 / 58 / 60 / 62: prims +0x578 / +0x618 / +0x6B8 / +0x758).
        const SHRINK: [(i32, i32); 11] =
            [(56, 184), (61, 178), (67, 172), (73, 166), (79, 161), (85, 155), (91, 149), (96, 143), (102, 137), (108, 131), (114, 126)];
        let b0 = v.len();
        if d >= 55 {
            for k in 0..8 {
                if d >= [55, 58, 60, 62][(2 * k as i32 - 7).unsigned_abs() as usize / 2] {
                    vs_band(&mut v, k);
                }
            }
        } else if t >= 30 {
            for s in 0..2 {
                let (x, c0) = (256 * s as i32, sel.confirm_t[s]);
                if c0 == -1 || t < c0 {
                    for k in 0..8 {
                        let top = 41 + (55 + 17 * k - 6 * (t - 50)).rem_euclid(136);
                        v.push(ft4(x, top, 256 + 2 * s as i32, 17, (64 * s as i32, 8 * k, 64, 8), 0x0C, clut(384, 500)));
                    }
                    continue;
                }
                let (y0, y1) = if d >= 33 {
                    (120, 120)
                } else if d >= 22 {
                    SHRINK[(d - 22) as usize]
                } else {
                    (50, 190)
                };
                if y1 <= y0 {
                    continue;
                }
                let cx = if s == 0 { 400 } else { 416 };
                let cl = if d >= 30 {
                    clut(432, 496 + (d - 30).min(8) as u16)
                } else if d >= 22 {
                    clut(cx, 501 + (d - 22) as u16)
                } else if t - c0 < 9 {
                    clut(384, 500 + (t - c0) as u16)
                } else {
                    clut(cx, 496 + (t - c0 - 9).min(4) as u16)
                };
                let vv = if sel.p[s].state == 1 { 64 } else { 128 };
                v.push(ft4(x, y0, 256, y1 - y0, (64 * s as i32, vv, 63, 64), 0x0C, cl));
            }
        }
        // engine addition: the bands (mirrored halves) repeat out to the window edges, the VS bands also above and below
        // (the maw's right edge, x 510..512, is black in the game: its copies mirror about 510)
        mirror_tiles(&mut v, b0, [if d >= 55 { 512.0 } else { 510.0 }, 240.0], [2, if d >= 55 { 1 } else { 0 }]);
        // ---- portraits and beast emblems (ot 20); not drawn on the frame the cursor moved (the upload); a CPU's
        // portrait from the frame after the reveal, never its emblem; the emblems leave at X+55
        if t >= 31 {
            for s in 0..2 {
                let cpu = sel.p[s].cpu;
                if sel.moved_t[s] == t || (cpu && !(sel.confirm_t[s] >= 0 && t > sel.confirm_t[s])) {
                    continue;
                }
                // engine addition (the import S10): an imported cell's portrait at its own aspect in the picture's
                // height (P1 mirrored, as on the QUICK select), no emblem
                if crate::assets::roster::is_br1(sel.p[s].cur) {
                    let x0 = if s == 0 { 47 } else { 302 };
                    let mut p = ft4(x0, 56, 163, 127, (128 * s as i32, 0, 127, 159), 0x104, 0);
                    if s == 0 {
                        for k in 0..4 {
                            p.xy[k][0] = (2 * x0 + 163) as f32 - p.xy[k][0];
                        }
                    }
                    v.push(p);
                    continue;
                }
                if s == 0 {
                    v.push(ft4(33, 56, 191, 127, (0, 0, 191, 255), 0x88, clut(0, 501)));
                } else {
                    v.push(quad([[416, 56], [288, 56], [416, 183], [288, 183]], (0, 0, 127, 255), 0x8A, clut(0, 502)));
                    v.push(quad([[480, 56], [416, 56], [480, 183], [416, 183]], (192, 0, 255, 255), 0x88, clut(0, 502)));
                }
                if cpu || d >= 55 {
                    continue;
                }
                let ec = clut(464, emblem_row(sel.p[s].cur));
                if s == 0 {
                    v.push(ft4(1, 108, 95, 40, (0, 0, 95, 80), 0x0D, ec));
                    v.push(ft4(1, 148, 95, 8, (0, 80, 95, 16), 0x0D, ec));
                } else {
                    v.push(quad([[511, 108], [416, 108], [511, 148], [416, 148]], (128, 0, 223, 80), 0x0D, ec));
                    v.push(ft4(417, 148, 95, 8, (128, 80, 95, 16), 0x0D, ec));
                }
            }
        }
        // ---- the jaws (ot 18): positions per frame of the intro; on the exit they close (X+21..X+32), pulse their
        // CLUTs (X+34..X+54) and open off screen (X+56..X+67) with the lower teeth (the right ones drifting down)
        const UP_IN: [i32; 9] = [-47, -31, -15, 0, 16, 32, 48, 64, 80];
        const UP_OUT: [i32; 13] = [80, 72, 63, 55, 46, 38, 29, 21, 12, 4, -4, -13, -21];
        const LOW_OUT: [i32; 13] = [80, 86, 92, 98, 104, 110, 117, 123, 129, 135, 141, 147, 153];
        const TU_IN: [i32; 8] = [8, 18, 29, 39, 49, 60, 70, 80];
        const TU_OUT: [i32; 13] = [83, 76, 69, 62, 56, 49, 41, 35, 28, 21, 14, 7, 0];
        const UP_CLOSE: [i32; 12] = [-15, -9, -3, 2, 8, 15, 21, 27, 33, 39, 45, 51];
        const LOW_CLOSE: [i32; 12] = [147, 141, 135, 129, 123, 117, 110, 104, 98, 92, 86, 80];
        const TU_CLOSE: [i32; 12] = [8, 15, 22, 29, 35, 42, 49, 56, 63, 70, 76, 83];
        const UP_OPEN: [i32; 12] = [30, 9, -12, -33, -55, -76, -97, -118, -140, -161, -182, -203];
        const LOW_OPEN: [i32; 12] = [101, 123, 144, 165, 187, 208, 229, 251, 272, 293, 315, 336];
        const DRIFT: [i32; 13] = [0, 31, 61, 92, 124, 154, 185, 216, 247, 278, 308, 339, 370];
        let flash = |k: i32| [0, 1, 2, 3, 4, 3, 2, 1, 0][(k.clamp(0, 8)) as usize] as u16;
        // (upper jaw y, lower jaw y, upper / lower teeth edge, teeth outer edge, right lower teeth drift, CLUT step)
        let (up, low, teeth_up, teeth_low, ttop, drift, crow): (i32, i32, i32, i32, i32, i32, u16) = if d >= 56 {
            let k = d - 55;
            let tu = lerp_tab(&TU_OUT, k - 1);
            (
                lerp_tab(&UP_OPEN, k - 1),
                lerp_tab(&LOW_OPEN, k - 1),
                tu,
                256 - tu,
                if k <= 4 {
                    -2
                } else if k <= 7 {
                    -1
                } else {
                    0
                },
                lerp_tab(&DRIFT, k - 1),
                4,
            )
        } else if d >= 33 {
            let crow = if (34..55).contains(&d) { [3, 2, 1, 0, 1, 2, 3][((d - 34) / 3) as usize] } else { 4 };
            (51, 80, 83, 173, -2, 0, crow)
        } else if d >= 21 {
            let k = d - 21;
            let tu = TU_CLOSE[k as usize];
            (
                UP_CLOSE[k as usize],
                LOW_CLOSE[k as usize],
                tu,
                256 - tu,
                if k < 4 {
                    -8
                } else if k < 10 {
                    -9
                } else {
                    -10
                },
                0,
                4,
            )
        } else if t >= 37 {
            let ramp = if t >= 44 { 4 } else { ((t - 36).max(0) / 2) as u16 };
            (-21, 153, 0, 999, -8, 0, ramp)
        } else if t <= 8 {
            (
                lerp_tab(&UP_IN, t),
                208 - 16 * t,
                if t >= 1 { lerp_tab(&TU_IN, t - 1) } else { 0 },
                if t >= 1 { 248 - (TU_IN[(t - 1) as usize] - 8) } else { 999 },
                -8,
                0,
                0,
            )
        } else if t < 24 {
            (80, 80, 83, 173, -8, 0, if (12..20).contains(&t) { flash(t - 11) } else { 0 })
        } else {
            (lerp_tab(&UP_OUT, t - 24), lerp_tab(&LOW_OUT, t - 24), lerp_tab(&TU_OUT, t - 24), 999, -8, 0, 0)
        };
        let (jc, tc) = (clut(448, 480 + crow), clut(480, 506 + crow));
        // the teeth strips first, the jaws over them (the OT-18 chain order)
        let j0 = v.len();
        if teeth_up > ttop.max(0) {
            v.push(quad([[0, ttop], [256, ttop], [0, teeth_up], [256, teeth_up]], (0, 224, 255, 255), 0x1A, tc));
            v.push(quad([[510, ttop], [254, ttop], [510, teeth_up], [254, teeth_up]], (0, 224, 255, 255), 0x1A, tc));
        }
        let tb = 256 - ttop;
        if teeth_low < tb.min(256) {
            v.push(quad([[0, tb], [256, tb], [0, teeth_low], [256, teeth_low]], (0, 224, 255, 255), 0x1A, tc));
            v.push(quad([[510, tb + drift], [254, tb + drift], [510, teeth_low + drift], [254, teeth_low + drift]], (0, 224, 255, 255), 0x1A, tc));
        }
        // upper jaw: left half as is, right half mirrored; lower jaw flipped vertically
        v.push(quad([[0, up], [256, up], [0, up + 95], [256, up + 95]], (0, 0, 255, 191), 0x18, jc));
        v.push(quad([[512, up], [256, up], [512, up + 95], [256, up + 95]], (0, 0, 255, 191), 0x18, jc));
        v.push(quad([[0, low + 95], [256, low + 95], [0, low], [256, low]], (0, 0, 255, 191), 0x18, jc));
        v.push(quad([[512, low + 95], [256, low + 95], [512, low], [256, low]], (0, 0, 255, 191), 0x18, jc));
        mirror_tiles(&mut v, j0, [510.0, 240.0], [2, 0]);
        // ---- faces (ot 17): rising in one after another, dropping out from X+11
        let chars = self.chars();
        for (o, &c) in game_tables::get().normal_order.iter().enumerate() {
            if chars >> c & 1 == 0 {
                continue;
            }
            let (x, ty) = nface(c, br1);
            let speed = if NFACE[c as usize].1 == 160 { 10 } else { 6 };
            let start = 25 + 2 * o as i32;
            let mut y = if t <= start { 240 } else { (240 - speed * (t - start)).max(ty) };
            if d >= 11 {
                y = (ty + speed * (d - 10)).min(240);
            }
            if y < 240 {
                let c = c as i32;
                v.push(ft4(x, y, 56, 31, ((c & 3) * 64, (c >> 2) * 64, 56, 63), 0x9D, clut(0, 480 + c as u16)));
            }
        }
        // engine addition (the import S10): the imported row after them (15-bit, at `NBR1_FACES`)
        for k in (0..4u8).filter(|_| br1) {
            let (x, ty) = nface(crate::assets::roster::BR1_FIRST + k, true);
            let start = 25 + 2 * (11 + k as i32);
            let mut y = if t <= start { 240 } else { (240 - 6 * (t - start)).max(ty) };
            if d >= 11 {
                y = (ty + 6 * (d - 10)).min(240);
            }
            if y < 240 {
                v.push(ft4(x, y, 56, 31, (56 * k as i32, 0, 56, 63), 0x100, 0));
            }
        }
        // the imported sides' names in the logos' place (480-line coordinates, drawn after the rest)
        let mut br1_names: Vec<(i32, i32, &str)> = Vec::new();
        // ---- plates, name logos, title, cursors (ot 16)
        const TITLE_IN: [i32; 17] = [-20, -14, -7, -1, 5, 11, 17, 23, 30, 36, 42, 48, 55, 61, 67, 73, 80];
        const LOGO: [(i32, i32, i32, i32, u16); 11] = [
            (0, 160, 104, 48, 0x1C),
            (0, 96, 112, 48, 0x1A),
            (104, 160, 112, 48, 0x1C),
            (144, 48, 111, 48, 0x1A),
            (0, 48, 144, 48, 0x1A),
            (0, 0, 200, 48, 0x1A),
            (0, 208, 176, 47, 0x1C),
            (112, 96, 136, 48, 0x1A),
            (0, 144, 104, 48, 0x1A),
            (104, 144, 112, 48, 0x1A),
            (0, 112, 208, 48, 0x1C),
        ];
        if t >= 10 {
            // plates and logos drop to the VS screen's y from X+21 (12 frames: 36 → 183, 32 → 180)
            let ramp = ((t - 11).max(0) / 3).min(4) as u16;
            let k = (d - 20).clamp(0, 12);
            let (py, ly) = (36 + 49 * k / 4, 32 + 37 * k / 3);
            for side in 0..2 {
                if !shown(side) {
                    continue;
                }
                let cx = if side == 0 { 128 } else { 384 };
                let row = if side == 0 { 496 + ramp } else { 501 + ramp };
                v.push(ft4(cx - 96, py, 192, 16, (0, 192, 196, 32), 0x1A, clut(480, row)));
                // the left logo's CLUT turns (480,505) with state 6
                let lrow = if side == 0 && t >= NORMAL_INPUT { 505 } else { row };
                // engine addition (the import S10): an imported fighter has no name logo: its name in the menu font
                if crate::assets::roster::is_br1(sel.p[side].cur) {
                    br1_names.push((cx, 2 * ly + 8, crate::assets::roster::name(sel.p[side].cur)));
                    continue;
                }
                let (u, vv, w, h, tp) = LOGO[(sel.p[side].cur as usize).min(10)];
                v.push(ft4(cx - w / 2, ly, w, h / 2, (u, vv, w, h), tp, clut(480, lrow)));
            }
            // the title drops in, and leaves upwards along the same steps from X+11
            let ty = if d >= 11 { lerp_tab(&TITLE_IN, 16 - (d - 10)) } else { lerp_tab(&TITLE_IN, t - 10) };
            let row = match t {
                ..27 => 507,
                27..33 => 508,
                33..39 => 509,
                39..45 => 510,
                _ => 511,
            };
            if ty > -20 {
                v.push(ft4(214, ty, 88, 20, (72, 0, 88, 40), 0x1C, clut(496, row)));
            }
        }
        // cursors and labels (opaque): fade-in (496,497..500) from 50, the cycle (448,485..490) from 63 (or the
        // join), (496,496) once chosen, a CPU's (432,487) from its reveal; gone at X+10. "1P" top-right of the
        // cell for slot 0, "2P" / "COM" bottom-left for slot 1 (uv)
        if t >= 50 && d < 10 {
            for i in 0..2 {
                if !shown(i) {
                    continue;
                }
                let pl = &sel.p[i];
                let (x, y) = nface(pl.cur, br1);
                let cl = if pl.cpu {
                    clut(432, 487)
                } else if pl.state > 0 {
                    clut(496, 496)
                } else if t < NORMAL_INPUT {
                    clut(496, 497 + ((t - 51).max(0) / 3).min(3) as u16)
                } else {
                    clut(448, 485 + ((t - sel.cyc0[i]).max(0) / 3 % 6) as u16)
                };
                v.push(ft4(x, y, 56, 31, (if i == 0 && !pl.cpu { 64 } else { 120 }, 192, 56, 63), 0x0E, cl));
                let (lx, ly) = if i == 0 { (x + 25, y + 1) } else { (x, y + 20) };
                let (lv, lh) = if pl.cpu {
                    (232, 23)
                } else if i == 0 {
                    (184, 24)
                } else {
                    (208, 24)
                };
                v.push(ft4(lx, ly, 32, 12, (176, lv, 32, lh), 0x0E, cl));
            }
        }
        // ---- sparks (ot 16 after the rest): 25 frames, opaque, page (768,256); uv (0,0) on the spawn
        // frame, then 8 frames of 7×7 from u 176 (3 each), CLUT (496,502..505) (6 each); y and h halved
        for b in sel.sparks.iter().flatten() {
            let a = t - b.t0;
            if !(0..25).contains(&a) {
                continue;
            }
            let uv = if a == 0 { (0, 0, 0, 0) } else { (176 + 8 * ((a - 1) / 3), 248, 7, 7) };
            let cl = clut(496, 502 + ((a - 1).max(0) / 6) as u16);
            for &(x, y, vx, vy, size) in &b.p {
                let (x, y) = (x as i32 + vx as i32 * (a + 1), y as i32 + vy as i32 * (a + 1));
                v.push(ft4(x, y / 2, size as i32, size as i32 / 2, uv, 0x1C, cl));
            }
        }
        // ---- with ctx flag 0x80 (X+55): the VS mark (prim +0xB68) and the stage banner of the VS screen,
        // both ot 15, the banner drawn first
        if d >= 55 {
            if let Some((text, w)) = self.vs_banner() {
                crate::output::font::fonts().menu_text(&mut v, &text, (512 - w) / 2, 15, 4);
            }
            v.push(ft4(220, 126, 72, 32, (0, 0, 72, 64), 0x1C, clut(496, 502)));
        }
        for mut p in v {
            for k in 0..4 {
                p.xy[k][1] *= 2.0;
            }
            out.push(p);
        }
        for (cx, y, name) in br1_names {
            let f = crate::output::font::fonts();
            f.menu_text(out, name, cx - f.menu_width(name) / 2, y, 0);
        }
        // ---- the countdown (ot 0): from state 5 until X+9, big font, 3 chars right-aligned
        if t >= NORMAL_INPUT - 1 && d < 10 {
            crate::output::font::fonts().big_text(out, &format!("{:>3}", sel.timer / 60), 218, if br1 { 240 - NCOUNT_UP } else { 240 }, 14);
        }
        // ---- states 3 and 6 (the side labels (P, M, 0)): the CPU side's PRESS START / AWAITING CHALLENGER,
        // as in the fight (M.9B = 0, round state 0)
        if t != NORMAL_INPUT - 1 && d < 0 {
            let li = hud::LabelInfo { ladder_len: 8, ..Default::default() };
            for side in (0..2).filter(|&i| sel.p[i].cpu && !sel.p[1 - i].cpu) {
                hud::idle_label(crate::output::font::fonts(), out, &li, side, 0, self.frame);
            }
        }
    }
}

/// Engine addition: side `i`'s big portrait (and its plate) at its window edge.
fn side_anchor(i: usize) -> Anchor {
    Anchor::new(if i == 0 { Ax::Start } else { Ax::End }, Ax::Center)
}

/// NORMAL select: the emblem CLUT row of character `c` (by grid cell: (464,506) for the first cell,
/// then (464,496..505)).
fn emblem_row(c: u8) -> u16 {
    match game_tables::get().normal_order.iter().position(|&n| n == c).unwrap_or(1) {
        0 => 506,
        o => 495 + o as u16,
    }
}

/// Step the select cursor from `c` in direction `dir`, skipping characters that are not available;
/// at an edge the direction turns (dir − 1) & 3, all the way round = stay. `br1`: the imported row is there
/// (`neigh`).
pub(super) fn step(c: u8, dir: usize, avail: u32, br1: bool) -> u8 {
    let ok = |n: u8| avail >> n & 1 != 0;
    let mut d = dir;
    let mut n = neigh(c, d, br1);
    for _ in 0..64 {
        if ok(n) {
            return n;
        }
        let m = neigh(n, d, br1);
        if m == n {
            d = (d + 3) & 3;
            if d == dir {
                return c;
            }
            n = neigh(n, d, br1);
        } else {
            n = m;
        }
    }
    c
}

/// Engine addition (the import S9): the imported row's cells (`roster::BR1_FIRST..`) under the bottom row, in the
/// columns of cells 1, 3, 5, 7, and their neighbours (up, right, down, left; the row wraps as the main game's do).
const BR1_FACE: [(i32, i32); 4] = [(141, 417), (199, 417), (257, 417), (315, 417)];
const BR1_NEIGH: [[u8; 4]; 4] = [[1, 12, 11, 14], [3, 13, 12, 11], [5, 14, 13, 12], [7, 11, 14, 13]];

/// The cursor neighbour of cell `c` in direction `d` (`select.neighbours`); with the imported row (`br1`) the bottom
/// row's DOWN leads into it (to the cell below, or the nearest at the ends).
fn neigh(c: u8, d: usize, br1: bool) -> u8 {
    if let Some(k) = c.checked_sub(crate::assets::roster::BR1_FIRST) {
        return BR1_NEIGH[k as usize % 4][d];
    }
    match (br1, d, c) {
        (true, 2, 10 | 1) => 11,
        (true, 2, 3) => 12,
        (true, 2, 5) => 13,
        (true, 2, 7 | 9) => 14,
        _ => game_tables::get().neighbours[c as usize][d],
    }
}

/// Cell `c`'s face icon's top-left corner (`select.faces`, `BR1_FACE`).
pub(super) fn face(c: u8) -> (i32, i32) {
    match c.checked_sub(crate::assets::roster::BR1_FIRST) {
        Some(k) => BR1_FACE[k as usize % 4],
        None => game_tables::get().faces[c as usize],
    }
}
