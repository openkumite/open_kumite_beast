//! The fight HUD and the status labels.
//!
//! State: serialised: part of `GameState` (field changes are `STATE_VERSION` changes)
//!
//! The fight HUD and the status labels: the HUD drawer (objects built at a match's first round).
//! Everything is drawn as PS1 primitives in the game's fixed 512×480 HUD space (interlaced layout) from the
//! textures of CMN/INIT.DAT; all HUD primitives are opaque, textured ones raw (texel 0 transparent).
//! The HUD draws named sprites of the sprite atlas (`output/sprite.rs`) with palettes picked from small tables;
//! the primitives live in `output::prim`, the fonts in `output::font`.

use crate::assets::roster::br2_name;
use crate::assets::text;
use crate::fight::fighter::{F5C, F58, Fighter};
use crate::fight::round::Message;
use crate::output::font::{FONT_PAGE, FONT_PALS, Fonts, HUD_PAGE, HUD_PALS, LABEL_PALS, OK_PAGE, pal};
use crate::output::layout::{Anchor, Ax, tag};
use crate::output::prim::{Prim, f4, g4, spr};
use crate::output::sprite::{Page, Pal, PalCycle, Sprite};
use serde::{Deserialize, Serialize};

/// Side `p`'s HUD anchor (engine addition): P1 at the left edge of the safe area, P2 at the right, `y` the top or
/// bottom edge.
pub fn side_anchor(p: usize, y: Ax) -> Anchor {
    Anchor::hud(if p == 0 { Ax::Start } else { Ax::End }, y)
}

/// Centred sprite.
fn spr_c(cx: i32, cy: i32, w: i32, h: i32, s: Sprite, pal: Pal) -> Prim {
    spr(cx - w / 2, cy - h / 2, w, h, s, pal)
}

// ------------------------------------------------------------------ status labels

/// What the fight's status labels need from the front end (M.9C / M.9B, the ladder, the win streaks).
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize)]
pub struct LabelInfo {
    /// M.9C: 0 arcade, 1 versus, 2 watch, 3 survival, 4 story, 5 custom
    pub m9c: u8,
    /// M.9B: 0 1P joinable, 1 1P, 2 two humans, 3 CPU vs CPU (watch, demo), 4 practice
    pub m9b: u8,
    /// M.A2: fights won in the ladder; opt.10 the ladder length; M.A4 ≥ opt.10: the ladder is cleared
    pub fight: i32,
    pub ladder_len: i32,
    pub ladder_done: bool,
    /// the human's play time before this fight (P.F0) and opt.21 (strongest CPU: time in palette 6)
    pub play_time: u32,
    pub shoulders: bool,
    /// P.E2 win streaks and the champion flag (M.B4 & 0x1000)
    pub streak: [u16; 2],
    pub champ: [bool; 2],
    /// the side dropped out of the last fight: small GAME OVER for 180 frames
    pub dropped: [bool; 2],
}

/// A side nobody plays: PLEASE WAIT / the mode label / PRESS START ↔ AWAITING CHALLENGER.
/// 9B = 2 / 3 draw nothing.
pub fn idle_label(f: &Fonts, out: &mut Vec<Prim>, li: &LabelInfo, side: usize, round_state: u8, dc: u32) {
    if !matches!(li.m9b, 0 | 1 | 4) || (dc >> 4) & 3 == 0 {
        return;
    }
    let base = if side == 1 { 432 } else { 80 };
    let wait = match round_state {
        5..=11 | 16 => true,
        12..=14 => li.ladder_done,
        _ => false,
    } || (li.m9c == 0 && li.ladder_done);
    let (s, dx, pal) = if wait {
        (text::get("please_wait"), 0x21, pal::ORANGE)
    } else if (1..=5).contains(&li.m9c) {
        match li.m9c {
            1 => (text::get("v_s_mode"), 0x1B, pal::YELLOW),
            2 => (text::get("watch_mode"), 0x1E, pal::YELLOW),
            3 => (text::get("survival_mode"), 0x27, pal::YELLOW),
            4 => (text::get("story_mode"), 0x1E, pal::YELLOW),
            _ if li.m9b == 4 => (text::get("practice_mode"), 0x27, pal::YELLOW),
            _ => (text::get("custom_mode"), 0x21, pal::YELLOW),
        }
    } else if dc & 0x40 != 0 {
        (text::get("awaiting_challenger"), 0x39, pal::ORANGE)
    } else {
        (text::get("press_start"), 0x21, pal::YELLOW)
    };
    let s0 = out.len();
    f.small_text(out, s, base - dx, 32, pal);
    tag(&mut out[s0..], side_anchor(side, Ax::Start));
}

/// The fight's per-side labels (by the round flow's input state) at y 32, font 0.
/// `dc` = the flip counter; `play` = this fight's playing frames; `replay` = a replay is playing.
#[allow(clippy::too_many_arguments)]
/// `continue_side` = the loser during the continue countdown (round state 0xF); `frame` = the fight's frame.
pub fn status_labels(
    f: &Fonts,
    li: &LabelInfo,
    cpu: [bool; 2],
    round_state: u8,
    hud_hidden: bool,
    replay: bool,
    play: u32,
    dc: u32,
    continue_side: Option<usize>,
    frame: u64,
) -> Vec<Prim> {
    let mut out = Vec::new();
    let blink_on = (dc >> 4) & 3 != 0;
    let phase = dc & 0x40 != 0;
    let both_cpu = cpu[0] && cpu[1];
    let one_human = cpu[0] != cpu[1];
    if li.m9b == 3 {
        return out; // watch / demo
    }
    for side in 0..2 {
        let base = if side == 1 { 432 } else { 80 };
        // state 0x900: a dropped side's small GAME OVER, steady, for 180 frames
        if li.dropped[side] && frame < 180 {
            f.small_text(&mut out, text::get("game_over"), base - 0x1B, 32, pal::ORANGE);
            continue;
        }
        // state 0x600: the continue countdown (arcade): PRESS START only over the loser, a2 = 1
        if continue_side == Some(side) && round_state == 0xF {
            if li.m9c == 0 && blink_on && !phase {
                f.small_text(&mut out, text::get("press_start"), base - 0x21, 32, pal::YELLOW);
            }
            continue;
        }
        // the countdown ran out (round state 0x10): the side is dropped (0x800 → 0x900), small GAME OVER
        if continue_side == Some(side) && round_state == 0x10 {
            f.small_text(&mut out, text::get("game_over"), base - 0x1B, 32, pal::ORANGE);
            continue;
        }
        if cpu[side] {
            // state 0: the CPU side; 9B = 0 only while a human plays the other side
            if !both_cpu {
                idle_label(f, &mut out, li, side, round_state, dc);
            }
            continue;
        }
        // state 0x300: a human side, 9B = 0 / 2 / 3 only
        if !matches!(li.m9b, 0 | 2) {
            continue;
        }
        if cpu[1 - side] {
            let n = li.fight + 1;
            let s = match li.m9c {
                0 if n == 9 => text::get("special_stage").to_string(),
                0 if n >= li.ladder_len => text::get("final_stage").to_string(),
                0 => text::fill(text::get("stage_n"), &[&n]),
                3 | 4 if n >= li.ladder_len => text::get("final_fight").to_string(),
                _ => text::fill(text::get("fight_n"), &[&n]),
            };
            f.small_text(&mut out, &s, if side == 1 { 512 - 0x92 } else { 0x0E }, 32, pal::GREEN);
            // the play time (one human vs the CPU, HUD shown, no replay)
            if li.m9b == 0 && one_human && !hud_hidden && !replay {
                let t = li.play_time + play;
                let (s, cs) = (t / 60, (t % 60) * 100 / 60);
                let (min, sec) = (s / 60, s % 60);
                let pal = if li.shoulders { pal::RED } else { pal::CYAN };
                if min == 0 {
                    f.small_mono(&mut out, &format!("{sec:2}\"{cs:02}"), base + 0x20, 32, pal);
                } else {
                    let (min, sec, cs) = if min >= 100 { (99, 59, 99) } else { (min, sec, cs) };
                    f.small_mono(&mut out, &format!("{min:2}'{sec:02}\"{cs:02}"), base + 8, 32, pal);
                }
            }
        } else {
            let k = if li.streak[side] + li.streak[1 - side] == 0 { u16::from(li.champ[side]) } else { li.streak[side] };
            if k != 0 {
                // CHAMPION n WIN(S), opposite phase to CHALLENGER
                if blink_on && !phase && li.streak[side] > 0 {
                    let w = li.streak[side].min(999);
                    let s = if w == 1 { text::get("champion_1_win").to_string() } else { text::fill(text::get("champion_n_wins"), &[&w]) };
                    let len = s.len() as i32;
                    f.small_text(&mut out, &s, base - 3 * len, 32, pal::ORANGE);
                }
            } else if phase {
                f.small_text(&mut out, text::get("challenger"), if side == 1 { 512 - 0x6E } else { 0x32 }, 32, pal::ORANGE);
            }
        }
    }
    // engine addition: each side's labels (left / right half) move with its side of the HUD
    for p in &mut out {
        p.at = side_anchor(usize::from(p.xy[0][0] >= 256.0), Ax::Start);
    }
    out
}

// ------------------------------------------------------------------ HUD state

/// Where `Hud::bar` puts a side's parts (engine addition, the team battle's HUD): y offsets in HUD space of the top part
/// (the health bar, its frame, the round lamps), the combo counter and the bottom part (the beast gauge, its label
/// and the pop-ups). A match's are all 0.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct BarAt {
    pub hp: i32,
    pub combo: i32,
    pub gauge: i32,
}

/// One side's HUD state (the HUD's per-side object): the red bar, the gauge label, the pop-ups, the combo counter.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub(crate) struct PState {
    draining: bool,  // +6
    shown: i32,      // +8 red bar displayed damage
    target: i32,     // +C
    gauge_last: i16, // +12
    gstate: i32,     // +14 (−1 = label still in its build look)
    pop_scale: i32,  // +18
    pop_timer: i32,  // +1A (−1 inactive)
    pop_frame: i32,  // frames since the pop-up started (BEAST clut track)
    ok_frame: Option<i32>,
    label_frame: i32,
    // combo
    combo_pos: [i32; 3],
    combo_vel: [i32; 3],
    combo_shown: i32,
    combo_timer: i32,
    combo_active: bool,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct Hud {
    cnt: i32,
    s: [PState; 2],
    /// primitives of this frame in HUD space, in drawing order (back to front): the draw list `frame` builds
    /// from the state, not state (a loaded state has none until its next step)
    #[serde(skip)]
    pub prims: Vec<Prim>,
}

// the HUD's block (`assets::game_tables`, the table `hud.data`): the P2 names' x (i16 by character at +0) and the
// sprites' texture rectangles (u, v, w, h bytes) by offset; their pages are the code's
const NAME_X2: usize = 0;
const TIMER_BACK: usize = 0x120;
/// timer digits 0..9 (16×24 cells, 15 texels wide) and the cell shown for an untimed round
const DIGITS: usize = 0x13C;
const NO_TIME: usize = 0x164;
const LAMP_EMPTY: usize = 0x1F4;
const LAMP_WON: usize = 0x1F8;
const BEAST_POPUP: usize = 0x268;
const GAUGE_FULL: usize = 0x274;
const COMBO_HITS: usize = 0x288;
/// the gauge label: the full gauge's two looks, the beast form's 4 frames, the human form's 4 frames
const LABEL_FULL: usize = 0x304;
const LABEL_BEAST: usize = 0x33C;
const LABEL_HUMAN: usize = 0x37C;
/// the OK burst: 5 frames of 48×40
const OK_BURST: usize = 0x3A4;

/// The HUD block's sprite rectangle `k` of the run at `off` on `page`.
fn rect(page: Page, off: usize, k: usize) -> Sprite {
    let h = &crate::assets::game_tables::get().hud;
    let b = |i: usize| h.get(off + 4 * k + i).copied().unwrap_or(0) as i32;
    Sprite::new(page, b(0), b(1), b(2), b(3))
}

/// Character `c`'s P2 name x (`NAME_X2`).
fn name_x2(c: usize) -> i32 {
    let h = &crate::assets::game_tables::get().hud;
    let o = NAME_X2 + 2 * c;
    h.get(o..o + 2).map_or(0, |b| i16::from_le_bytes([b[0], b[1]]) as i32)
}

/// Frame piece `k` (`hud.frame`): corners TL TR BL BR, top and bottom edges, right and left sides.
fn frame_piece(k: usize) -> Sprite {
    let [u, v, w, h] = crate::assets::game_tables::get().frame[k].map(i32::from);
    Sprite::new(HUD_PAGE, u, v, w, h)
}

// the per-frame palette choices (rows of the palette blocks)
/// timer digits under 10 s: flash by `cnt % 3`; under 20 s orange, else white
const TIMER_FLASH: [u16; 3] = [pal::BLUE, pal::YELLOW, pal::RED];
/// the BEAST pop-up after its first 20 frames: one row per 3 frames (`hud.popup`: (row, frames) pairs, the first the
/// 20 frames' row 0)
fn popup_pal(k: usize) -> u16 {
    crate::assets::game_tables::get().popup[2 * (1 + k.min(5))]
}
/// the full gauge: label rows 8..15, one per 4 frames (0x7A1D | (cnt·16 & 0x1C0))
const GAUGE_CYCLE: PalCycle = PalCycle { first: 8, n: 8, shift: 2 };
/// label rows: the human / beast labels 0..3 / 4..7 with their frames, the full gauge's label 4, the OK 7
const LABEL_HUMAN_PAL: u16 = 0;
const LABEL_BEAST_PAL: u16 = 4;
const LABEL_FULL_PAL: u16 = 4;
const OK_PAL: u16 = 7;
/// the gauge frame by gauge state (0 filling, 1 full, 2 beast); the HP frame, timer back and combo use row 0
const FRAME_PALS: [u16; 3] = [4, 0, 5];
/// the round lamps: row 1, row 2 (glint) for the last 10 of every 127 frames
const LAMP_PAL: u16 = 1;
const LAMP_GLINT_PAL: u16 = 2;

impl PState {
    /// The side's object as the HUD builder makes it for fighter `f`.
    pub(crate) fn fresh(f: &Fighter) -> Self {
        PState { shown: f.o_ac as i32, target: f.o_ac as i32, gauge_last: f.gauge, gstate: -1, pop_timer: -1, ..Default::default() }
    }
}

fn gauge_state(f: &Fighter) -> i32 {
    if f.form & 0x10 != 0 {
        2
    } else if f.gauge >= 0x1E00 {
        1
    } else {
        0
    }
}

impl Hud {
    pub fn new() -> Self {
        Self { cnt: 0, s: Default::default(), prims: Vec::new() }
    }

    /// The HUD builder at the first round of a match: objects rebuilt, slide-in restarts.
    pub fn build(&mut self, fs: &[Fighter; 2]) {
        self.cnt = 0;
        for p in 0..2 {
            self.s[p] = PState::fresh(&fs[p]);
        }
    }

    /// One drawn frame of the HUD. `round_idx` = match +0x99, `need` = rounds to win, `lost` = +0xE4 per fighter,
    /// `timer` = +0x94, `frozen` = +0x82; `names`: a side's name in place of its character's (engine addition: an imported
    /// fighter's own, `roster::br1_name`, where its face holds its main-game proxy's character).
    #[allow(clippy::too_many_arguments)]
    pub fn frame(&mut self, fs: &[Fighter; 2], round_idx: i8, need: u8, lost: [u8; 2], timer: i16, frozen: bool, names: [Option<&str>; 2]) {
        self.cnt += 1;
        let cnt = self.cnt;
        let slide = |dy: i32| if cnt <= 32 { (dy * (0x1000 - cnt * 128)) >> 12 } else { 0 };
        let (top, bot) = (slide(-128), slide(128));
        let mut back: Vec<Prim> = Vec::new(); // ot[0x80E] (top HUD), drawn after the scene
        let mut front: Vec<Prim> = Vec::new(); // ot[4] (bottom HUD)
        back.extend(Self::timer(cnt, timer, top));
        // ---- names
        if cnt >= 33 {
            for p in 0..2 {
                let c = (fs[p].chr as usize).min(10);
                let f = crate::output::font::fonts();
                // a name in place of the table's (engine addition) ends where the character's would (P2's right-aligned)
                let name = names[p].unwrap_or(br2_name(c));
                let x = if p == 0 { 34 } else { name_x2(c) + f.width(&f.small, br2_name(c)) - f.width(&f.small, name) };
                let mut tmp = Vec::new();
                f.text(&mut tmp, &f.small, name, x, 80, pal::GREY);
                tag(&mut tmp, side_anchor(p, Ax::Start));
                back.extend(tmp);
            }
        }
        for p in 0..2 {
            // the round lamps of P1's side count P2's rounds lost (its wins), and the other way round
            let lamps = if p == 0 { lost[1] } else { lost[0] };
            Self::bar(&mut self.s[p], p, &fs[p], &fs[1 - p], cnt, (top, bot), round_idx, need, lamps, frozen, BarAt::default(), (&mut back, &mut front));
        }
        self.prims = back;
        self.prims.extend(front);
    }

    /// The timer of `frame` (`timer` frames left, −1 untimed) at the top centre, `top` the slide-in offset.
    pub(crate) fn timer(cnt: i32, timer: i16, top: i32) -> [Prim; 3] {
        let secs = if timer < 0 { -1 } else { (timer as i32 / 60).min(99) };
        let row = if secs < 10 {
            TIMER_FLASH[(cnt % 3) as usize]
        } else if secs < 20 {
            pal::ORANGE
        } else {
            pal::WHITE
        };
        let dp = FONT_PALS.row(row);
        let (d1, d2) = if secs < 0 {
            (rect(FONT_PAGE, NO_TIME, 0), rect(FONT_PAGE, NO_TIME, 0))
        } else {
            (rect(FONT_PAGE, DIGITS, (secs / 10) as usize), rect(FONT_PAGE, DIGITS, (secs % 10) as usize))
        };
        let timer_prims = [
            spr_c(256, 66 + top, 40, 32, rect(HUD_PAGE, TIMER_BACK, 0), HUD_PALS.row(0)),
            spr_c(247, 66 + top, 16, 24, d1, dp),
            spr_c(265, 66 + top, 16, 24, d2, dp),
        ];
        timer_prims.map(|p| p.at(Anchor::hud(Ax::Center, Ax::Start)))
    }

    /// One side's part of `frame`: side `p` (0 left, 1 right) shows fighter `f` (its opponent `o`): the
    /// BEAST pop-up and OK, the health bar, the beast gauge and its label, their frames and backs, `lamps` round lamps
    /// lit of `need`, the combo counter; into `back` (the top HUD, ot[0x80E]) and `front` (the bottom HUD, ot[4]) at
    /// `top` / `bot` (the slide-in offsets), tagged to the side's window edge; `at` moves the parts (the team battle's
    /// layout, engine addition; a match's is `BarAt::default()`).
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn bar(
        st: &mut PState,
        p: usize,
        f: &Fighter,
        o: &Fighter,
        cnt: i32,
        (top, bot): (i32, i32),
        round_idx: i8,
        need: u8,
        lamps: u8,
        frozen: bool,
        at: BarAt,
        (back, front): (&mut Vec<Prim>, &mut Vec<Prim>),
    ) {
        let (bl, fl) = (back.len(), front.len());
        // ---- BEAST pop-up and OK
        if f.gauge >= 0x1E00 && st.gauge_last < 0x1E00 && st.pop_timer < 0 && f.form == 0 {
            st.pop_scale = 0x1000;
            st.pop_timer = 120;
            st.pop_frame = 0;
        }
        if st.pop_timer >= 0 {
            let (rx, dx) = if p == 0 { (125, -200) } else { (323, 200) };
            let x = rx + ((dx * st.pop_scale) >> 12);
            let k = st.pop_frame;
            let pc = if k < 20 { 0 } else { popup_pal(((k - 20) / 3) as usize) };
            front.push(spr(x, 383 + bot, 64, 24, rect(HUD_PAGE, BEAST_POPUP, 0), LABEL_PALS.row(pc)));
            if st.pop_timer > 80 {
                st.pop_scale = (st.pop_scale * 425) >> 9;
            }
            if st.pop_timer == 100 {
                st.ok_frame = Some(0);
            }
            if st.pop_timer < 40 {
                st.pop_scale += 500;
            }
            st.pop_timer -= 1;
            st.pop_frame += 1;
        }
        // ---- HP: G4, red and blue segments
        let (ax, w0) = if p == 0 { (229, -192) } else { (283, 192) };
        if cnt >= 33 && round_idx >= 0 {
            let b0 = f.o_b0.max(1) as i32;
            let ac = f.o_ac as i32;
            let mut hp_end = ax;
            let mut bar = None;
            if ac >= 0 {
                let w = w0 * (b0 - ac) / b0;
                let w = if w == 0 { if p == 0 { -1 } else { 1 } } else { w };
                hp_end = ax + w;
                bar = Some(g4(ax, 56, w, 13, [255, 240, 32], [160, 96, 0]));
            }
            if f.flags[F58] & 0x8000 == 0 && o.flags[F5C] & 0x40 == 0 {
                st.draining = true;
                st.target = ac;
            }
            if st.draining {
                if st.target < st.shown {
                    st.shown = ac;
                } else {
                    st.shown += 2;
                    if st.shown >= st.target {
                        st.shown = st.target;
                        st.draining = false;
                    }
                }
            }
            let red_end = ax + w0 * (b0 - st.shown) / b0;
            let blue_end = ax + w0 * (b0 - f.o_aa as i32) / b0;
            let mut segs = Vec::new();
            if ac > 0 {
                if blue_end != red_end {
                    let col = if f.form & 0x10 != 0 && cnt & 1 == 0 { [220, 220, 255] } else { [64, 96, 160] };
                    segs.push(f4(red_end, 56, blue_end - red_end, 13, col));
                }
                segs.push(f4(hp_end, 56, red_end - hp_end, 13, [255, 0, 0]));
            }
            // painter's order: blue, red, then the bar
            back.extend(segs);
            back.extend(bar);
        }
        // ---- beast gauge
        let (gx, gw) = if p == 0 { (37, 90) } else { (475, -90) };
        let full = f.gauge >= 0x1E00 && f.form & 0x10 == 0;
        st.gauge_last = f.gauge;
        if cnt >= 33 {
            if full {
                front.push(spr(gx, 425 + bot, gw * f.gauge as i32 / 0x3C00, 13, rect(HUD_PAGE, GAUGE_FULL, 0), LABEL_PALS.row(GAUGE_CYCLE.row(cnt))));
            } else {
                let range = if f.form & 0x10 != 0 { 0x3C00 } else { 0x1E00 };
                let w = gw * f.gauge.max(0) as i32 / range;
                let (t, b) = if f.form & 0x10 != 0 { ([255, 240, 32], [160, 96, 0]) } else { ([32, 240, 255], [0, 64, 192]) };
                front.push(g4(gx, 425 + bot, w, 13, t, b));
            }
        }
        // ---- label (state-dependent animation) and OK
        let gs = gauge_state(f);
        if gs != st.gstate && st.gstate >= 0 || st.gstate < 0 && gs != 1 && cnt > 1 {
            st.gstate = gs;
            st.label_frame = 0;
        }
        let lf = st.label_frame;
        let label = match st.gstate {
            0 => {
                let k = ((lf / 3) % 4) as usize;
                let r = if p == 0 { (140, 423) } else { (358, 423) };
                spr(r.0, r.1 + bot, 16, 16, rect(HUD_PAGE, LABEL_HUMAN, k), LABEL_PALS.row(LABEL_HUMAN_PAL + k as u16))
            }
            2 => {
                let k = ((lf / 3) % 4) as usize;
                let r = if p == 0 { (140, 423) } else { (358, 423) };
                spr(r.0, r.1 + bot, 16, 16, rect(HUD_PAGE, LABEL_BEAST, k), LABEL_PALS.row(LABEL_BEAST_PAL + k as u16))
            }
            _ => {
                let r = if p == 0 { (136, 420) } else { (346, 420) };
                spr(r.0, r.1 + bot, 31, 23, rect(HUD_PAGE, LABEL_FULL, ((lf / 10) % 2) as usize), LABEL_PALS.row(LABEL_FULL_PAL))
            }
        };
        st.label_frame += 1;
        if let Some(k) = st.ok_frame {
            let (cx, cy) = if p == 0 { (160, 430) } else { (350, 430) };
            front.push(spr_c(cx, cy + bot, 48, 40, rect(OK_PAGE, OK_BURST, (k / 4).min(4) as usize), LABEL_PALS.row(OK_PAL)));
            st.ok_frame = if k + 1 >= 20 { None } else { Some(k + 1) };
        } else if st.pop_timer < 100 {
            front.push(label);
        }
        // ---- frames and backs
        let frame = |x0: i32, x1: i32, xe: i32, we: i32, y: i32, pal: Pal, out: &mut Vec<Prim>| {
            let at = [(x0, y, 8), (x1, y, 8), (x0, y + 16, 8), (x1, y + 16, 8), (xe, y, we), (xe, y + 16, we), (x1, y + 8, 8), (x0, y + 8, 8)];
            for ((x, y, w), s) in at.into_iter().zip((0..8).map(frame_piece)) {
                out.push(spr(x, y, w, 8, s, pal));
            }
        };
        let bc = HUD_PALS.row(FRAME_PALS[gs as usize]);
        let mut fr = Vec::new();
        if p == 0 {
            frame(32, 226, 40, 186, 51 + top, HUD_PALS.row(0), &mut fr);
        } else {
            frame(278, 472, 286, 186, 51 + top, HUD_PALS.row(0), &mut fr);
        }
        let mut fb = Vec::new();
        if p == 0 {
            frame(32, 124, 40, 84, 420 + bot, bc, &mut fb);
        } else {
            frame(380, 472, 388, 84, 420 + bot, bc, &mut fb);
        }
        // backs, then the frames, go under everything of their slot: the edge tiles' inner rows are opaque
        // black, and the game shows the full 13-line bar over them (checked against recorded frames)
        let hp_back = if p == 0 { f4(35, 54 + top, 196, 18, [2, 2, 9]) } else { f4(281, 54 + top, 196, 18, [2, 2, 9]) };
        let g_back = if p == 0 { f4(35, 423 + bot, 94, 18, [2, 2, 9]) } else { f4(383, 423 + bot, 94, 18, [2, 2, 9]) };
        let (nb, nf) = (1 + fr.len(), 1 + fb.len());
        back.splice(0..0, std::iter::once(hp_back).chain(fr));
        front.splice(0..0, std::iter::once(g_back).chain(fb));
        // ---- round lamps (left group = P2's rounds lost = P1's wins)
        if round_idx >= 0 {
            let glint = HUD_PALS.row(if cnt % 127 >= 117 { LAMP_GLINT_PAL } else { LAMP_PAL });
            let (xs, e4) = (if p == 0 { [205, 185, 165, 145] } else { [296, 316, 336, 356] }, lamps);
            for i in 0..(need as usize).min(4) {
                let lamp = rect(HUD_PAGE, if (i as u8) < e4 { LAMP_WON } else { LAMP_EMPTY }, 0);
                back.push(spr(xs[i], 78 + top, 16, 16, lamp, glint));
            }
        }
        // ---- combo counter
        let combo = back.len();
        let hits = f.o_dd as i32;
        if f.o_dd == 0 {
            st.combo_active = false;
        } else if hits >= 2 && o.flags[F58] & 0x8000 != 0 {
            let (x0, v) = if p == 0 { (-31, 8) } else { (430, -8) };
            if !st.combo_active {
                st.combo_pos = [x0; 3];
                st.combo_vel = [v; 3];
                st.combo_timer = 80;
                st.combo_shown = 2;
                st.combo_active = true;
            }
            if st.combo_shown < hits {
                st.combo_pos[0] = x0;
                st.combo_pos[1] = x0;
                st.combo_vel[0] = v;
                st.combo_vel[1] = v;
                st.combo_timer = 120;
                st.combo_shown = hits;
            }
        }
        st.combo_shown = st.combo_shown.min(99);
        if !frozen && st.combo_timer > 0 {
            st.combo_timer -= 1;
        }
        if st.combo_timer > 0 {
            for i in 0..3 {
                st.combo_pos[i] += st.combo_vel[i];
                st.combo_vel[i] = st.combo_vel[i] * 3800 / 4096;
            }
            let (n, pos) = (st.combo_shown, st.combo_pos);
            let mut tmp = Vec::new();
            if n >= 10 {
                let d = char::from(b'0' + (n / 10) as u8);
                crate::output::font::fonts().text(&mut tmp, &crate::output::font::fonts().digits, &d.to_string(), pos[0], 94, pal::ORANGE);
            }
            let d = char::from(b'0' + (n % 10) as u8);
            crate::output::font::fonts().text(&mut tmp, &crate::output::font::fonts().digits, &d.to_string(), pos[1] + 16, 94, pal::ORANGE);
            back.extend(tmp);
            back.push(spr(pos[2] + 36, 114, 32, 24, rect(HUD_PAGE, COMBO_HITS, 0), HUD_PALS.row(0)));
        }
        if at != BarAt::default() {
            let mv = |ps: &mut [Prim], dy: i32| ps.iter_mut().for_each(|q| q.xy.iter_mut().for_each(|v| v[1] += dy as f32));
            mv(&mut back[..nb], at.hp);
            mv(&mut back[bl + nb..combo], at.hp);
            mv(&mut back[combo..], at.combo);
            mv(&mut front[..nf], at.gauge);
            mv(&mut front[fl + nf..], at.gauge);
        }
        // engine addition: this side's groups move with its window edge (top: frame, HP, lamps, combo; bottom:
        // gauge, label, pop-ups)
        tag(&mut back[..nb], side_anchor(p, Ax::Start));
        tag(&mut back[bl + nb..], side_anchor(p, Ax::Start));
        tag(&mut front[..nf], side_anchor(p, Ax::End));
        tag(&mut front[fl + nf..], side_anchor(p, Ax::End));
    }

    /// A round message in the big font (`round_message`).
    pub fn message(&self, m: Message) -> Vec<Prim> {
        round_message(m)
    }
}

/// A round message in the big font: `(text, palette, y)` from the round flow's message.
pub fn round_message(m: Message) -> Vec<Prim> {
    let mut out = Vec::new();
    let text = m.text();
    if text.is_empty() {
        return out;
    }
    let pal = match m {
        Message::Round(_) | Message::FinalRound | Message::GetReady | Message::TimeOver => pal::BIG_YELLOW,
        Message::YouLose | Message::GameOver => pal::BIG_BLUE,
        _ => pal::BIG_ORANGE,
    };
    let y = if matches!(m, Message::Round(_) | Message::FinalRound | Message::SuddenDeath | Message::GetReady | Message::Fight) {
        3 * 480 / 8
    } else {
        480 / 2 - 0x10
    };
    let font = &crate::output::font::fonts().big;
    if m == Message::Challenger {
        // message 0: W/2 − 0xE7, H/2 − 0xC (style 8, the tall font with −4)
        crate::output::font::fonts().text(&mut out, font, &text, 256 - 0xE7, 480 / 2 - 0xC, pal);
        return out;
    }
    if matches!(m, Message::Continue(_)) {
        // message 21: fixed at (W/2 − 0x79, 3H/4 − 0x30)
        crate::output::font::fonts().text(&mut out, font, &text, 256 - 0x79, 3 * 480 / 4 - 0x30, pal);
        return out;
    }
    // message 0x16 GAME OVER: the fixed x W/2 − 0x63, not centred by width
    let x = if m == Message::GameOver { 256 - 0x63 } else { 256 - crate::output::font::fonts().width(font, &text) / 2 };
    crate::output::font::fonts().text(&mut out, font, &text, x, y, pal);
    out
}

impl Default for Hud {
    fn default() -> Self {
        Self::new()
    }
}
