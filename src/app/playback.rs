//! A replay playback's controls and overlay.
//!
//! Engine addition.
//!
//! A playback's controls and overlay (engine addition, game-state plan stage 6b). The pads do not drive a replayed
//! session, so they drive the playback (either pad; the keyboard's pad keys too): START pause, □ one frame,
//! LEFT / RIGHT speed (×0.25 … ×4), L1 / R1 five seconds back / on, L2 the match's start (TOP), UP / DOWN the round's
//! start / the next round, SELECT the HUD, ○ the input display, × the key help (shown the first 10 s and while paused), △ back to the replay list (`--replay`:
//! quit). The overlay: "SKIPPING n%" while a skip or seek runs, the speed / time / round line, both sides' pads, the
//! key help (menu fonts and icons, as the list menus draw them). Debug OKB_UISCRIPT / OKB_UISHOTS script and capture
//! the list and the playback by their own frame count (`App::ui_frame`); OKB_NOPLAYUI hides the overlay.

use super::App;
use super::replays::Target;
use crate::assets::text;
use crate::fight::input::pad::{CIRCLE, CROSS, DOWN, L1, L2, LEFT, R1, RIGHT, SELECT, SQUARE, START, TRIANGLE, UP};
use crate::fight::round::Message;
use crate::frontend::Pads;
use crate::output::font::Fonts;
use crate::output::layout::{Anchor, Ax, tag};
use crate::output::prim::{self, Prim};

/// The speeds: quarter steps per 60 Hz frame, and their names.
pub const SPEEDS: [(u32, &str); 5] = [(1, "X0.25"), (2, "X0.5"), (4, "X1"), (8, "X2"), (16, "X4")];
const NORMAL: usize = 2;
/// L1 / R1: seconds back / on.
const JUMP: u32 = 5 * 60;
/// UP within this many frames of a round's start: the round before.
const ROUND_GRACE: u32 = 2 * 60;
/// The key help's frames from the start (it covers the beast gauges).
const LEGEND_FRAMES: u32 = 10 * 60;

pub(super) struct Controls {
    pub paused: bool,
    /// `SPEEDS` index
    pub speed: usize,
    /// quarter steps owed
    sub: u32,
    pub hud: bool,
    pub inputs: bool,
    /// the key help pinned (×); it shows anyway while paused and for `legend_left` frames from the start
    pub legend: bool,
    legend_left: u32,
    pads: Pads,
}

impl Default for Controls {
    fn default() -> Self {
        Self { paused: false, speed: NORMAL, sub: 0, hud: true, inputs: true, legend: false, legend_left: LEGEND_FRAMES, pads: Pads::default() }
    }
}

/// A 60 Hz frame of the controls (`Controls::tick`): the pads' new presses and auto-repeats, the steps to run, a
/// change the sound follows.
pub(super) struct Tick {
    pub press: u16,
    pub rep: u16,
    pub steps: i32,
    pub sound: bool,
}

impl Controls {
    /// With the pads held as the playback starts: those are no presses until let go.
    pub fn new(held: [u16; 2]) -> Self {
        let mut c = Self::default();
        c.pads.update(held);
        c
    }

    /// One 60 Hz frame with the pads: pause, frame step, speed, the HUD, the input display and the key help as
    /// pressed; None on △ (back).
    pub fn tick(&mut self, pads: [u16; 2]) -> Option<Tick> {
        self.pads.update(pads);
        let (press, rep) = (self.pads.any_press(), self.pads.any_rep());
        if press & TRIANGLE != 0 {
            return None;
        }
        let mut sound = false;
        let mut step = 0;
        if press & START != 0 {
            self.paused = !self.paused;
            sound = true;
        }
        if press & SQUARE != 0 {
            sound |= !self.paused;
            self.paused = true;
            step = 1;
        }
        if rep & (LEFT | RIGHT) != 0 {
            let s = if rep & RIGHT != 0 { (self.speed + 1).min(SPEEDS.len() - 1) } else { self.speed.saturating_sub(1) };
            sound |= s != self.speed;
            self.speed = s;
        }
        if press & SELECT != 0 {
            self.hud = !self.hud;
        }
        if press & CIRCLE != 0 {
            self.inputs = !self.inputs;
        }
        if press & CROSS != 0 {
            self.legend = !self.legend_shown();
            self.legend_left = 0;
        }
        self.legend_left = self.legend_left.saturating_sub(1);
        let steps = if step > 0 { step } else { self.steps() };
        Some(Tick { press, rep, steps, sound })
    }

    /// The steps make no sound: paused (a frame step) or not at ×1.
    pub fn quiet(&self) -> bool {
        self.paused || self.speed != NORMAL
    }

    /// The key help is up.
    pub fn legend_shown(&self) -> bool {
        self.legend || self.paused || self.legend_left > 0
    }

    /// The steps of this 60 Hz frame at the speed (none while paused).
    fn steps(&mut self) -> i32 {
        if self.paused {
            return 0;
        }
        self.sub += SPEEDS[self.speed].0;
        let n = self.sub / 4;
        self.sub %= 4;
        n as i32
    }
}

impl App {
    /// The pads for the replay list and the playback controls: with debug OKB_UISCRIPT's at this UI frame, which
    /// counts on.
    pub(super) fn ui_pads(&mut self, pads: [u16; 2]) -> [u16; 2] {
        let s = self.env.uiscript.get(self.ui_frame as usize).copied().unwrap_or(0);
        self.ui_frame += 1;
        [pads[0] | s, pads[1]]
    }

    /// One 60 Hz frame of a playback's controls with the pads; the steps to run in it (none before its session
    /// exists: then only △ works).
    pub(super) fn playback_tick(&mut self, pads: [u16; 2]) -> i32 {
        let pads = self.ui_pads(pads);
        let Some(pb) = &mut self.player else { return 0 };
        let tick = if pb.started {
            pb.ctl.tick(pads)
        } else {
            // its session not there yet: △ only
            pb.ctl.pads.update(pads);
            if pb.ctl.pads.any_press() & TRIANGLE == 0 {
                return 0;
            }
            None
        };
        let Some(Tick { press, rep, steps, sound }) = tick else {
            if pb.from_list.is_some() {
                self.end_playback(pads);
            } else {
                pb.ended = true;
            }
            return 0;
        };
        let (frame, from) = (self.state.frame, pb.from());
        let seek = if rep & L1 != 0 {
            Some(Target::Frame(frame.saturating_sub(JUMP)))
        } else if rep & R1 != 0 {
            Some(Target::Frame(frame + JUMP))
        } else if press & L2 != 0 {
            Some(Target::Frame(from))
        } else if press & (UP | DOWN) != 0 {
            let cur = pb.rounds.iter().rposition(|r| r.1 <= frame);
            if press & UP != 0 {
                match cur {
                    Some(k) if frame - pb.rounds[k].1 < ROUND_GRACE && k > 0 => Some(Target::Frame(pb.rounds[k - 1].1)),
                    Some(k) => Some(Target::Frame(pb.rounds[k].1)),
                    None => Some(Target::Frame(from)),
                }
            } else {
                match cur.map_or(0, |k| k + 1) {
                    k if k < pb.rounds.len() => Some(Target::Frame(pb.rounds[k].1)),
                    // not seen yet: on until it starts (before the fight: until its first round)
                    _ => Some(Target::After(self.state.fight.as_ref().map_or(i8::MIN, |g| g.m.round.index))),
                }
            }
        } else {
            None
        };
        if sound {
            self.playback_sound();
        }
        if let Some(t) = seek
            && !self.player.as_ref().is_some_and(|pb| pb.stopped)
        {
            self.replay_seek(t);
        }
        steps
    }

    /// The playback's overlay (512×480 space): the skip's progress, the status line, the input display and the key
    /// help, as the controls show them.
    pub(super) fn playback_prims(&self, out: &mut Vec<Prim>) {
        let Some(pb) = &self.player else { return };
        if self.env.noplayui {
            return;
        }
        let f = crate::output::font::fonts();
        if pb.target.is_some() {
            let t = format!("SKIPPING {}%", pb.progress());
            let w = f.menu_width(&t);
            out.push(prim::f4(256 - w / 2 - 16, 220, w + 32, 40, [96, 96, 96]).blend(crate::output::sprite::Blend::Sub));
            f.menu_text(out, &t, 256 - w / 2, 232, 0);
            return;
        }
        let c = &pb.ctl;
        // the status: speed, PAUSED, the fight's time / its length, the round
        let h = &pb.p.replay.header;
        let base = h.fight_from.unwrap_or(h.from);
        let t = |n: u32| {
            let s = n.saturating_sub(base) / 60;
            format!("{}:{:02}", s / 60, s % 60)
        };
        let mut status = SPEEDS[c.speed].1.to_string();
        if c.paused {
            status += " PAUSED";
        }
        status += &format!("  {}/{}", t(self.state.frame), t(h.frames));
        if let Some(g) = &self.state.fight {
            let r = g.m.round.index;
            let m = if r < 0 { Message::SuddenDeath } else { Message::Round(r as i32 + 1) };
            status += &format!("  {}", m.text());
        }
        if pb.ended {
            status += "  END";
        }
        // under the HUD's names and the input displays
        let s0 = out.len();
        let w = 9 * status.len() as i32;
        out.push(prim::f4(256 - w / 2 - 6, 132, w + 12, 16, [96, 96, 96]).blend(crate::output::sprite::Blend::Sub));
        f.menu_small(out, &status, 256 - w / 2, 136, if c.paused { 3 } else { 0 });
        tag(&mut out[s0..], Anchor::new(Ax::Center, Ax::Start));
        if c.inputs {
            let cpu = self.state.fight.as_ref().map(|g| [g.m.cpu[0], g.m.cpu[1]]).or(self.state.spec.map(|m| m.cpu)).unwrap_or_default();
            for k in 0..2 {
                let pad = pb.last.forced(k).unwrap_or(pb.last.pad[k]);
                let s0 = out.len();
                input_display(f, out, if k == 0 { 12 } else { 512 - 12 - 132 }, 96, k, pad, cpu[k]);
                tag(&mut out[s0..], Anchor::new(if k == 0 { Ax::Start } else { Ax::End }, Ax::Start));
            }
        }
        if c.legend_shown() {
            let s0 = out.len();
            // below the beast gauges
            out.push(prim::f4(0, 438, 512, 32, [96, 96, 96]).blend(crate::output::sprite::Blend::Sub));
            let icon = |k: usize| text::item("cc_icons", k);
            legend(
                f,
                out,
                442,
                &[("", "START", text::get("pause")), (icon(0), "", "STEP"), ("", "</>", "SPEED"), ("", "L1", "-5S"), ("", "R1", "+5S"), ("", "L2", "TOP")],
            );
            legend(
                f,
                out,
                457,
                &[
                    ("", "UP/DN", text::get("round")),
                    ("", text::get("select"), text::get("hud")),
                    (icon(2), "", "INPUTS"),
                    (icon(1), "", "HELP"),
                    (icon(3), "", text::get("exit")),
                ],
            );
            tag(&mut out[s0..], Anchor::new(Ax::Center, Ax::End));
        }
    }
}

/// A side's pad at (x, y), 132 × 30: P1 / P2 (COM for a CPU side), the direction on a 3 × 3 pad, then □ × ○ △ L1 R1,
/// the held ones lit.
fn input_display(f: &Fonts, out: &mut Vec<Prim>, x: i32, y: i32, side: usize, pad: u16, cpu: bool) {
    out.push(prim::f4(x, y, 132, 32, [96, 96, 96]).blend(crate::output::sprite::Blend::Sub));
    f.menu_small(out, if side == 0 { "P1" } else { "P2" }, x + 4, y + 4, 0);
    if cpu {
        f.menu_small(out, "COM", x + 4, y + 18, 3);
        return;
    }
    // the direction: the cell of the held directions lit
    let dx = i32::from(pad & RIGHT != 0) - i32::from(pad & LEFT != 0);
    let dy = i32::from(pad & DOWN != 0) - i32::from(pad & UP != 0);
    for cy in -1..=1 {
        for cx in -1..=1 {
            let lit = (cx, cy) == (dx, dy);
            let rgb = if lit {
                [255, 200, 0]
            } else if (cx, cy) == (0, 0) {
                [40, 40, 40]
            } else {
                [72, 72, 72]
            };
            out.push(prim::f4(x + 32 + 8 * (cx + 1), y + 5 + 8 * (cy + 1), 7, 7, rgb));
        }
    }
    // the buttons (CONTROLLER CONFIG.'s icons): lit in palette 8, else dimmed
    let mut bx = x + 62;
    for (bit, icon) in [(SQUARE, 0), (CROSS, 1), (CIRCLE, 2), (TRIANGLE, 3)].map(|(b, k)| (b, text::item("cc_icons", k))) {
        f.menu_icons(out, icon, bx, y + 1, if pad & bit != 0 { 8 } else { 1 });
        bx += 16;
    }
    let mut bx = x + 62;
    for (bit, icon) in [(L1, 6), (R1, 4)].map(|(b, k)| (b, text::item("cc_icons", k))) {
        f.menu_icons(out, icon, bx, y + 16, if pad & bit != 0 { 8 } else { 1 });
        bx += 32;
    }
}

/// A line of the key help, centred: per entry the button (an icon, or its name in the small font) and what it does.
fn legend(f: &Fonts, out: &mut Vec<Prim>, y: i32, items: &[(&str, &str, &str)]) {
    let width = |(icon, key, what): &(&str, &str, &str)| {
        let icon = if icon.is_empty() { 0 } else { f.menu_width(icon) + 3 };
        let key = if key.is_empty() { 0 } else { 9 * (key.len() as i32 + 1) };
        icon + key + 9 * what.len() as i32 + 12
    };
    let mut x = 256 - items.iter().map(width).sum::<i32>() / 2;
    for it in items {
        let (icon, key, what) = *it;
        let x0 = x;
        if !icon.is_empty() {
            f.menu_icons(out, icon, x, y - 4, 8);
            x += f.menu_width(icon) + 3;
        }
        if !key.is_empty() {
            f.menu_small(out, key, x, y, 3);
            x += 9 * (key.len() as i32 + 1);
        }
        f.menu_small(out, what, x, y, 0);
        x = x0 + width(it);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The speeds give 1 step per 4 frames at ×0.25 up to 4 per frame at ×4; none while paused.
    #[test]
    fn speeds() {
        for (k, want) in [(0, 1), (1, 2), (2, 4), (3, 8), (4, 16)] {
            let mut c = Controls { speed: k, ..Default::default() };
            assert_eq!((0..4).map(|_| c.steps()).sum::<i32>(), want, "{}", SPEEDS[k].1);
        }
        let mut c = Controls { paused: true, ..Default::default() };
        assert_eq!(c.steps(), 0);
        assert!(c.quiet() && !Controls::default().quiet());
    }

    /// The button still held from the list's PLAY is no press: the first frames change nothing; let go and
    /// pressed again it works.
    #[test]
    fn held_confirm_is_no_press() {
        let held = [CROSS | SQUARE, CIRCLE];
        let mut c = Controls::new(held);
        let was = (c.paused, c.speed, c.hud, c.inputs, c.legend);
        for _ in 0..3 {
            let t = c.tick(held).expect("not back");
            assert_eq!((t.press, t.steps, t.sound), (0, 1, false));
        }
        assert_eq!((c.paused, c.speed, c.hud, c.inputs, c.legend), was);
        c.tick([0; 2]);
        let t = c.tick([SQUARE, CIRCLE]).expect("not back");
        assert_eq!((t.press, t.steps, c.paused, c.inputs), (SQUARE | CIRCLE, 1, true, false));
        assert!(Controls::new([TRIANGLE, 0]).tick([TRIANGLE, 0]).is_some() && Controls::default().tick([TRIANGLE, 0]).is_none());
    }
}
