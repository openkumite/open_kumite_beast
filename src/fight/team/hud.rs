//! The team battle's HUD (`TeamHud`, engine addition): four of the match's bars at full size, the timer and the round
//! lamps per team.
//!
//! Engine addition, after the match's HUD (`fight::hud`)
//! State: serialised: part of `TeamFight` (field changes are `STATE_VERSION` changes)
//!
//! Each fighter's bar is the match's side part (`Hud::bar`: health bar, beast gauge and label, pop-ups, combo
//! counter) with its parts moved together (`BarAt`): P1 / P2 at the top left / right, the gauge right under the
//! health bar; P3 / P4 at the bottom, the health bar right over the gauge. So in 2 VS 2 the left column is team 1
//! (P1 + P3) and the right one team 2 (P2 + P4). The names carry the player number in the team's colour (2 VS 2) or
//! the player's (FREE FOR ALL); the round lamps are the team's (2 VS 2: under P1 / P2 only); a fighter out of the
//! round is drawn greyed. The timer is the match's, at the top centre.

use super::{N, TeamMode, TeamRound};
use crate::assets::roster::br2_name;
use crate::fight::fighter::Fighter;
use crate::fight::hud::{BarAt, Hud, PState, side_anchor};
use crate::fight::side::Fid;
use crate::output::font::pal;
use crate::output::layout::{Ax, tag};
use crate::output::prim::{Prim, f4};
use crate::output::sprite::Blend;
use serde::{Deserialize, Serialize};

/// The font palette of fighter `k`'s name: the team's in 2 VS 2 (team 1 red, team 2 blue), the player's in FREE FOR
/// ALL (red, blue, green, yellow).
pub fn colour(mode: TeamMode, k: usize) -> u16 {
    [pal::RED, pal::BLUE, pal::GREEN, pal::YELLOW][mode.team(k)]
}

/// The darkening over a greyed fighter's bar (`grey`).
const GREY_SUB: u8 = 112;

/// Fighter `k`'s bar layout: the top row's gauge 320 lines up (under its health bar) and its combo counter 40 down
/// (under the gauge); the bottom row's health bar 317 lines down (over its gauge) and its combo counter 230 down (over
/// the health bar).
fn bar_at(k: usize) -> BarAt {
    if k < 2 { BarAt { hp: 0, combo: 40, gauge: -320 } } else { BarAt { hp: 317, combo: 230, gauge: 0 } }
}

#[derive(Clone, Default, Serialize, Deserialize)]
pub struct TeamHud {
    cnt: i32,
    s: [PState; N],
    /// this frame's primitives in HUD space, drawing order (not state, as `Hud::prims`)
    #[serde(skip)]
    pub prims: Vec<Prim>,
}

impl TeamHud {
    /// At the first round of a match (as `Hud::build`): the bars rebuilt, the slide-in restarts.
    pub fn build(&mut self, fs: [&Fighter; N]) {
        self.cnt = 0;
        for k in 0..N {
            self.s[k] = PState::fresh(fs[k]);
        }
    }

    /// One frame: fighter `k`'s bar against `opp[k]` (the fighter it hit last: its combo counter's victim); `names`: a
    /// imported fighter's own name (T5; its face holds its proxy's character).
    pub fn frame(&mut self, fs: [&Fighter; N], opp: &[usize; N], r: &TeamRound, mode: TeamMode, names: [Option<&str>; N]) {
        self.cnt += 1;
        let cnt = self.cnt;
        let slide = |dy: i32| if cnt <= 32 { (dy * (0x1000 - cnt * 128)) >> 12 } else { 0 };
        let (top, bot) = (slide(-128), slide(128));
        let mut back: Vec<Prim> = Hud::timer(cnt, r.timer, top).to_vec();
        let mut front: Vec<Prim> = Vec::new();
        let font = crate::output::font::fonts();
        for k in 0..N {
            let side = Fid::of(k).slot().index();
            // (each fighter in lists of its own: `Hud::bar` puts a side's frames first in them, under the side's parts)
            let (mut b, mut f) = (Vec::new(), Vec::new());
            if cnt >= 33 {
                // "P1 YUGO" under the health bar; P2 / P4 right-aligned to where the match's P2 name ends
                let name = format!("P{} {}", k + 1, names[k].unwrap_or(br2_name((fs[k].chr as usize).min(10))));
                let w = font.width(&font.small, &name);
                let x = if side == 0 { 34 } else { 478 - w };
                font.text(&mut b, &font.small, &name, x, if k < 2 { 80 } else { 397 }, colour(mode, k));
            }
            // the round lamps: 2 VS 2 the team's under P1 / P2, FREE FOR ALL each fighter's own
            let (lamps, need) = match mode {
                TeamMode::Vs2 if k >= 2 => (0, 0),
                _ => (r.wins[mode.team(k)], r.need),
            };
            let slide = if k < 2 { (top, top) } else { (bot, bot) };
            let o = fs[opp[k]];
            Hud::bar(&mut self.s[k], side, fs[k], o, cnt, slide, r.index, need, lamps, false, bar_at(k), (&mut b, &mut f));
            // the whole bar moves with its row's window edge (`Hud::bar` tags the gauge to the bottom one)
            let at = side_anchor(side, if k < 2 { Ax::Start } else { Ax::End });
            tag(&mut b, at);
            tag(&mut f, at);
            if r.out[k] {
                grey(&mut b);
                grey(&mut f);
            }
            back.append(&mut b);
            front.append(&mut f);
        }
        self.prims = back;
        self.prims.extend(front);
    }
}

/// Greyed: the untextured parts (the bars) in grey of half their brightness, then a subtract tile of `GREY_SUB` over
/// the whole part (the HUD's sprites are raw textures: their colour does not change them).
fn grey(ps: &mut Vec<Prim>) {
    let Some(at) = ps.first().map(|p| p.at) else { return };
    let (mut x0, mut y0, mut x1, mut y1) = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
    for p in ps.iter_mut() {
        for [x, y] in p.xy {
            (x0, y0, x1, y1) = (x0.min(x), y0.min(y), x1.max(x), y1.max(y));
        }
        if !p.textured {
            for c in &mut p.rgb {
                *c = [((c[0] as u16 + c[1] as u16 + c[2] as u16) / 6) as u8; 3];
            }
        }
    }
    let tile = f4(x0 as i32, y0 as i32, (x1 - x0) as i32, (y1 - y0) as i32, [GREY_SUB; 3]).blend(Blend::Sub);
    ps.push(tile.at(at));
}
