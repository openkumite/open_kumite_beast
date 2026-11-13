//! The touch controls: the on-screen PS1 pad and the TOUCH CONTROLS page.
//!
//! Engine addition.
//!
//! The touch controls (engine addition, `docs/android.md`; switched on by the Android app only): the PS1 pad's
//! layout on screen, the d-pad (or, as an option, a floating stick) on the left, □ △ ○ × on the right, L1 / R1 above
//! them, START and OPT on their row towards the middle; outside fights moved clear of the menus' text and pictures
//! (`Layout::front`). Fingers (`Touch::event`) give raw pad bits (`bits`), OR-ed
//! into player 1's pad as the keyboard's are, so the session sees nothing new. Drawn as untextured PS1 quads (and
//! the small font for the labels) in window pixels, into `Renderer::screen`. The TOUCH CONTROLS page (`Menu`, OPT) edits the
//! `TouchPrefs` (`touch.json`), presentation only: they are not part of the game state.
#![cfg_attr(not(target_os = "android"), allow(dead_code))]

use crate::assets::text::Label::{self, Game, Port};
use crate::fight::input::pad::{CIRCLE, CROSS, DOWN, L1, LEFT, R1, RIGHT, SQUARE, TRIANGLE, UP};
use crate::frontend::Pads;
use crate::output::font::{Fonts, pal};
use crate::output::layout::{Clip, Fit, Viewport};
use crate::output::prim::Prim;
use crate::output::psx_quad::PsxQuad;
use serde::{Deserialize, Serialize};

/// START's raw pad bit.
const START: u16 = 0x800;

/// The page's choices (`touch.json`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct TouchPrefs {
    /// DIRECTION: the floating stick instead of the d-pad
    pub stick: bool,
    /// SIZE: index into `SIZES`
    pub size: u8,
    /// OPACITY: LOW (outlines only) / NORMAL (half-transparent) / HIGH (opaque)
    pub opacity: u8,
}

impl Default for TouchPrefs {
    fn default() -> Self {
        Self { stick: false, size: 1, opacity: 1 }
    }
}

/// SIZE: SMALL / NORMAL / LARGE (× the d-pad's radius).
const SIZES: [f32; 3] = [0.82, 1.0, 1.2];

impl TouchPrefs {
    /// The stored text (`platform::storage::read_touch_prefs`); defaults when missing or unreadable.
    pub fn from_stored(text: Option<String>) -> Self {
        text.and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default()
    }

    fn size(&self) -> f32 {
        SIZES[(self.size as usize).min(SIZES.len() - 1)]
    }

    fn opacity(&self) -> u8 {
        self.opacity.min(2)
    }
}

/// What a finger holds, fixed when it comes down.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Ctl {
    /// the d-pad / stick; the centre the direction is read from, kept while the finger is down (the d-pad's at the
    /// touch, also when the layout changes under it; the stick's where the finger came down, dragged along past the
    /// rim)
    Dir([f32; 2]),
    /// the face buttons, by where the finger is now
    Face,
    /// L1, R1 or START
    Button(u16),
    /// OPT: the TOUCH CONTROLS page, when the finger goes up on it
    Opt,
}

#[derive(Clone, Copy, Debug)]
struct Finger {
    id: i32,
    ctl: Ctl,
    at: [f32; 2],
}

/// Where the controls are in the window (pixels).
#[derive(Clone, Copy, Debug, PartialEq)]
struct Layout {
    /// the d-pad's centre and radius
    dpad: [f32; 2],
    r: f32,
    /// the face buttons' centre, a button's distance from it and its radius
    face: [f32; 2],
    d: f32,
    b: f32,
    /// L1, R1, START, OPT: centre x, y, half width, half height
    l1: [f32; 4],
    r1: [f32; 4],
    start: [f32; 4],
    opt: [f32; 4],
}

/// The face buttons: offset from the centre (in `d`), bit, symbol colour (the pad's: green △, red ○, blue ×,
/// pink □).
const FACE: [([f32; 2], u16, [f32; 3]); 4] = [
    ([0.0, -1.0], TRIANGLE, [60.0, 210.0, 150.0]),
    ([1.0, 0.0], CIRCLE, [230.0, 80.0, 80.0]),
    ([0.0, 1.0], CROSS, [100.0, 150.0, 250.0]),
    ([-1.0, 0.0], SQUARE, [225.0, 120.0, 205.0]),
];

/// The controls' band in the window's height: below the fight HUD's names and above its beast gauges.
const TOP: f32 = 0.2;
const BOTTOM: f32 = 0.84;

/// What the front end's screens leave free in the bottom corners, in the core's 512 × 480 units (measured on the
/// title, the mode select, the character selects, every OPTIONS page, CUSTOM, the replay list, the NETWORK menu with
/// its picker and the story pages, WIDE and CLASSIC; the VS screen, the memory card dialogs and the name entry by
/// their code): from `y` down a control on the left keeps left of `x` (on the right, right of it); nothing above the
/// first `y` (the VERSUS select's names reach 280 beside the core in WIDE). The steps: the arcade select's beast
/// emblem (to 307, from x 2), the list menus' cursors (from x 16 / to 488, rows to 412), the story subtitle box (360
/// to 440, its portrait from x 4, its arrow to 504 and y 448), the selects' portraits (to 447, x 85 to 443), the hint
/// line (x 192 to 341). The list menus' main-game logo (bottom right) is decor and may be covered.
const FE_LEFT: [[f32; 2]; 5] = [[285.0, 0.0], [312.0, 10.0], [360.0, 2.0], [440.0, 64.0], [450.0, 180.0]];
const FE_RIGHT: [[f32; 2]; 4] = [[285.0, 512.0], [312.0, 500.0], [360.0, 510.0], [450.0, 345.0]];
/// Beside the core (WIDE's sides, CLASSIC's bars) from y 80 to 228 there is only decor (beast art, backdrops): L1 /
/// OPT and START / R1 go there when the side is wide enough.
const FE_SIDE: [f32; 2] = [80.0, 228.0];

/// The front end's own layout only with controls of a usable size (of the window's height): the d-pad's radius, a
/// face button's radius (20:9 and 16:9 phones pass at every SIZE, with a cutout too); OPT's half height when it
/// moves in the fallback.
const FRONT_MIN: [f32; 3] = [0.045, 0.025, 0.025];

/// A control's drawn footprint (window pixels): a rectangle (x0, y0, x1, y1) or a disc.
#[derive(Clone, Copy, Debug)]
enum Shape {
    Rect([f32; 4]),
    Disc([f32; 2], f32),
}

/// The core's 512 × 480 units in the window (`Viewport`'s largest centred 4:3 rectangle, as the renderer places
/// it) and the front end's free corners in it.
struct Core {
    x0: f32,
    y0: f32,
    ux: f32,
    uy: f32,
}

impl Core {
    fn new(w: f32, h: f32) -> Self {
        let v = Viewport::new([w as u32, h as u32], Fit::Classic);
        Core { x0: v.core[0] as f32, y0: v.core[1] as f32, ux: v.unit[0], uy: v.unit[1] }
    }

    fn x(&self, u: f32) -> f32 {
        self.x0 + u * self.ux
    }

    fn y(&self, u: f32) -> f32 {
        self.y0 + u * self.uy
    }

    /// `s` clear of the front end's screens on the left (or the `right`) side (`FE_LEFT` / `FE_RIGHT`).
    fn clear(&self, s: Shape, right: bool) -> bool {
        let steps: &[[f32; 2]] = if right { &FE_RIGHT } else { &FE_LEFT };
        let (top, bottom) = match s {
            Shape::Rect(r) => (r[1], r[3]),
            Shape::Disc(c, r) => (c[1] - r, c[1] + r),
        };
        if top < self.y(steps[0][0]) {
            return false;
        }
        (0..steps.len()).all(|i| {
            let (y0, lim) = (self.y(steps[i][0]), self.x(steps[i][1]));
            let y1 = steps.get(i + 1).map_or(f32::MAX, |s| self.y(s[0]));
            if bottom <= y0 || top >= y1 {
                return true;
            }
            // the shape's reach across in this step's rows
            let (near, far) = match s {
                Shape::Rect(r) => (r[0], r[2]),
                Shape::Disc(c, r) => {
                    let dy = (c[1].clamp(y0, y1) - c[1]).abs();
                    let half = (r * r - dy * dy).max(0.0).sqrt();
                    (c[0] - half, c[0] + half)
                }
            };
            if right { near >= lim } else { far <= lim }
        })
    }

    /// The rectangle `r` beside the core in the rows of `FE_SIDE`, on the left (or the `right`).
    fn beside(&self, r: [f32; 4], right: bool) -> bool {
        let rows = r[1] >= self.y(FE_SIDE[0]) && r[3] <= self.y(FE_SIDE[1]);
        rows && if right { r[0] >= self.x(514.0) } else { r[2] <= self.x(-2.0) }
    }
}

/// The rectangle of a box (centre x, y, half width, half height).
fn rect(bx: [f32; 4]) -> [f32; 4] {
    [bx[0] - bx[2], bx[1] - bx[3], bx[0] + bx[2], bx[1] + bx[3]]
}

/// A box's hit area as a rectangle: grown by `pad` of its half height on every side (`inside`).
fn hit(bx: [f32; 4], pad: f32) -> [f32; 4] {
    let g = bx[3] * pad;
    rect([bx[0], bx[1], bx[2] + g, bx[3] + g])
}

/// Two rectangles without a shared point inside.
fn apart(a: [f32; 4], b: [f32; 4]) -> bool {
    a[2] <= b[0] || b[2] <= a[0] || a[3] <= b[1] || b[3] <= a[1]
}

impl Layout {
    /// For a `w` × `h` window with the safe `insets` (left, top, right, bottom) and the SIZE factor `s`: in fights.
    fn new(w: f32, h: f32, insets: [f32; 4], s: f32) -> Self {
        let [il, it, ir, ib] = insets;
        // the d-pad's radius: a fifth of the height, no more than a fifth of the width
        let r = (h * 0.2).min(w * 0.2) * s;
        let m = h * 0.06;
        let (bw, bh) = (r * 0.5, r * 0.2);
        // the controls between the fight HUD's rows, in CLASSIC and WIDE alike (the core spans the window's height):
        // the d-pad and the face buttons end above `BOTTOM` (the beast gauges start at 420 of 480), L1 / R1 start
        // below `TOP` (the names end at 94) as long as the SIZE leaves room; both within the safe insets
        let y = (h * BOTTOM).min(h - ib - m) - r;
        let y = y.max(it + m + 2.0 * bh + m * 0.5 + r).max(h * TOP + 2.0 * bh + m * 0.5 + r).min(h - ib - r);
        let dpad = [il + m + r, y];
        let face = [w - ir - m - r, y];
        let shoulder = y - r - m * 0.5 - bh;
        Layout {
            dpad,
            r,
            face,
            d: r * 0.64,
            b: r * 0.34,
            l1: [dpad[0], shoulder, bw, bh],
            r1: [face[0], shoulder, bw, bh],
            // on the shoulder row, towards the middle: clear of the bottom centre the menus and selects use
            start: [face[0] - r * 1.2, shoulder, r * 0.4, bh * 0.85],
            opt: [dpad[0] + r * 1.2, shoulder, r * 0.36, bh * 0.85],
        }
    }

    /// Outside fights: the d-pad and the face buttons in the bottom corners, each as large as the SIZE asks and the
    /// front end's screens leave room for (`FE_LEFT` / `FE_RIGHT`); L1 / OPT and START / R1 beside the core above
    /// them (`FE_SIDE`), else on the bottom row beside the d-pad and the face buttons, as large as fits there.
    fn front(w: f32, h: f32, insets: [f32; 4], s: f32) -> Self {
        let [il, _, ir, ib] = insets;
        let core = Core::new(w, h);
        let m = h * 0.015;
        let full = (h * 0.2).min(w * 0.2) * s;
        // the largest radius up to the SIZE's for which `ok` holds (a smaller control in the same corner lies inside
        // a larger one)
        let largest = |ok: &dyn Fn(f32) -> bool| {
            if ok(full) {
                return full;
            }
            let (mut lo, mut hi) = (0.0, full);
            for _ in 0..24 {
                let mid = (lo + hi) * 0.5;
                if ok(mid) {
                    lo = mid;
                } else {
                    hi = mid;
                }
            }
            lo
        };
        let z = [0.0; 4];
        // the face buttons reach d + b = 0.98 of their radius from their centre
        let at = |r: f32, rf: f32| Layout {
            dpad: [il + m + r, h - ib - m - r],
            r,
            face: [w - ir - m - rf * 0.98, h - ib - m - rf * 0.98],
            d: rf * 0.64,
            b: rf * 0.34,
            l1: z,
            r1: z,
            start: z,
            opt: z,
        };
        let r = largest(&|r| at(r, r).dir_shapes().iter().all(|&s| core.clear(s, false)));
        let rf = largest(&|rf| at(r, rf).face_shapes().iter().all(|&s| core.clear(s, true)));
        // too small to play with (a 4:3 tablet has no room beside the core): the fight's layout, see-through over the
        // menus, but OPT on the bottom line clear of them when it fits there at a usable size (`FRONT_MIN`)
        if r < h * FRONT_MIN[0] || rf * 0.34 < h * FRONT_MIN[1] {
            let mut l = Layout::new(w, h, insets, s);
            let opt = l.opt;
            for k in (0..11).map(|n| 1.0 - n as f32 * 0.05) {
                let (hw, hh) = (opt[2] * k, opt[3] * k);
                if hh < h * FRONT_MIN[2] {
                    break;
                }
                // close to the bottom edge: the line below the menus is narrow
                let bx = [il + m + hw, h - ib - m * 0.3 - hh, hw, hh];
                if core.clear(Shape::Rect(rect(bx)), false) {
                    l.opt = bx;
                    break;
                }
            }
            l.opt_apart(insets[1]);
            return l;
        }
        let mut l = at(r, rf);
        // a pair, the outer box first (L1 / R1 at the window's edge), at scale `k` of the SIZE's measure (both boxes
        // of the fight's L1 height): beside the core in a row (`way` 0) or a column with the inner box below (1),
        // or on the bottom row beside the cluster (2); `None` where it does not fit
        let pair = |right: bool, inner_w: f32, k: f32, way: usize| {
            let dir = if right { -1.0 } else { 1.0 };
            let (ow, iw, bh) = (full * 0.5 * k, full * inner_w * k, full * 0.2 * k);
            // apart by more than the boxes' grown hit areas (`event`)
            let g = bh * 1.2;
            let row = |from: f32, y: f32| ([from + dir * ow, y, ow, bh], [from + dir * (2.0 * ow + g + iw), y, iw, bh]);
            let edge = if right { w - ir - m } else { il + m };
            let side_bottom = core.y(FE_SIDE[1]);
            let (o, i) = match way {
                0 => row(edge, side_bottom - bh),
                1 => ([edge + dir * ow, side_bottom - 3.0 * bh - g, ow, bh], [edge + dir * iw, side_bottom - bh, iw, bh]),
                _ => row(if right { l.face[0] - rf * 0.98 - g } else { l.dpad[0] + r + g }, h - ib - m - bh),
            };
            let ok = |bx: [f32; 4]| if way < 2 { core.beside(rect(bx), right) } else { core.clear(Shape::Rect(rect(bx)), right) };
            (ok(o) && ok(i)).then_some((o, i))
        };
        // each side's largest; when both are beside the core, the same way and size on both (the smaller: a cutout
        // narrows one side)
        let own = |right: bool, inner_w: f32| {
            let mut ks = (0..15).map(|n| 1.0 - n as f32 * 0.05);
            ks.find_map(|k| (0..3).find_map(|way| Some((k, way, pair(right, inner_w, k, way)?))))
        };
        // (none fits in a window too small for the front end's corners: the fight's places)
        let fight = Layout::new(w, h, insets, s);
        let ((l1, opt), (r1, start)) = match (own(false, 0.36), own(true, 0.4)) {
            (Some((kl, wl, _)), Some((kr, wr, _))) if wl < 2 && wr < 2 => {
                let (k, way) = (kl.min(kr), wl.max(wr));
                (pair(false, 0.36, k, way).unwrap_or((fight.l1, fight.opt)), pair(true, 0.4, k, way).unwrap_or((fight.r1, fight.start)))
            }
            (left, right) => (left.map_or((fight.l1, fight.opt), |p| p.2), right.map_or((fight.r1, fight.start), |p| p.2)),
        };
        (l.l1, l.opt, l.r1, l.start) = (l1, opt, r1, start);
        l.opt_apart(insets[1]);
        l
    }

    /// The fight's boxes crowd a near-square or portrait window (OPT, tested first, on START, which then can't be
    /// pressed): OPT above L1 then, below the window's `top` inset.
    fn opt_apart(&mut self, top: f32) {
        if [(self.start, 0.6), (self.l1, 0.5), (self.r1, 0.5)].iter().any(|&(bx, pad)| !apart(hit(self.opt, 0.6), hit(bx, pad))) {
            let y = self.l1[1] - self.l1[3] * 1.5 - self.opt[3] * 1.6;
            self.opt = [self.l1[0], y.max(top + self.opt[3] * 1.6), self.opt[2], self.opt[3]];
        }
    }

    /// The d-pad's footprint (the cross, and the stick's base).
    fn dir_shapes(&self) -> [Shape; 3] {
        let ([x, y], r, a) = (self.dpad, self.r, self.r * 0.33);
        [Shape::Rect([x - r, y - a, x + r, y + a]), Shape::Rect([x - a, y - r, x + a, y + r]), Shape::Disc(self.dpad, r)]
    }

    /// The face buttons' footprint.
    fn face_shapes(&self) -> [Shape; 4] {
        [0, 1, 2, 3].map(|k| Shape::Disc(self.button(k), self.b))
    }

    fn button(&self, k: usize) -> [f32; 2] {
        let (o, _, _) = FACE[k];
        [self.face[0] + o[0] * self.d, self.face[1] + o[1] * self.d]
    }
}

/// `p` inside the box (centre x, y, half sizes) grown by `pad` of its half height.
fn inside(p: [f32; 2], bx: [f32; 4], pad: f32) -> bool {
    let g = bx[3] * pad;
    (p[0] - bx[0]).abs() <= bx[2] + g && (p[1] - bx[1]).abs() <= bx[3] + g
}

fn dist(a: [f32; 2], b: [f32; 2]) -> f32 {
    ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2)).sqrt()
}

/// The direction bits of `v` (window y down) beyond `dead`: eight equal sectors (a component past cos 67.5° of the
/// length counts).
fn dir_bits(v: [f32; 2], dead: f32) -> u16 {
    let len = (v[0] * v[0] + v[1] * v[1]).sqrt();
    if len <= dead {
        return 0;
    }
    let k = len * 0.383;
    let mut bits = 0;
    if v[0] < -k {
        bits |= LEFT;
    }
    if v[0] > k {
        bits |= RIGHT;
    }
    if v[1] < -k {
        bits |= UP;
    }
    if v[1] > k {
        bits |= DOWN;
    }
    bits
}

/// The touch controls: the fingers down and the layout of the window.
pub struct Touch {
    pub prefs: TouchPrefs,
    /// drawn (false after a key or gamepad event, true again at the next touch)
    pub shown: bool,
    /// OPT is offered (the front end, no replay list or playback up; set per frame by the app)
    pub opt: bool,
    /// a fight is on (set per frame by the app): the fight's layout, else the front end's (`Layout::front`)
    pub fight: bool,
    /// OPT was let go on: the app opens the page
    pub opt_tapped: bool,
    win: [f32; 2],
    insets: [f32; 4],
    fingers: Vec<Finger>,
}

impl Touch {
    pub fn new(prefs: TouchPrefs) -> Self {
        Self { prefs, shown: true, opt: false, fight: false, opt_tapped: false, win: [1280.0, 720.0], insets: [0.0; 4], fingers: Vec::new() }
    }

    /// The window's size in pixels.
    pub fn set_window(&mut self, w: u32, h: u32) {
        self.win = [w.max(1) as f32, h.max(1) as f32];
    }

    /// The window's safe insets (the display cutout), pixels.
    pub fn set_insets(&mut self, insets: [f32; 4]) {
        self.insets = insets.map(|v| v.max(0.0));
    }

    /// The safe insets last given (`set_insets`), pixels: the fight HUD keeps inside them too
    /// (`Renderer::set_hud_insets`).
    pub fn insets(&self) -> [f32; 4] {
        self.insets
    }

    fn layout(&self) -> Layout {
        let make = if self.fight { Layout::new } else { Layout::front };
        make(self.win[0], self.win[1], self.insets, self.prefs.size())
    }

    /// Every finger let go (focus lost: their ups go elsewhere).
    pub fn release_all(&mut self) {
        self.fingers.clear();
    }

    /// A finger event: `phase` 0 down, 1 move, 2 up, 3 cancel, at window pixel `p`.
    pub fn event(&mut self, phase: u8, id: i32, p: [f32; 2]) {
        self.shown = true;
        let l = self.layout();
        match phase {
            0 => {
                self.fingers.retain(|f| f.id != id);
                let ctl = if self.opt && inside(p, l.opt, 0.6) {
                    Ctl::Opt
                } else if inside(p, l.start, 0.6) {
                    Ctl::Button(START)
                } else if inside(p, l.l1, 0.5) {
                    Ctl::Button(L1)
                } else if inside(p, l.r1, 0.5) {
                    Ctl::Button(R1)
                } else if p[0] < self.win[0] * 0.45 {
                    // the d-pad answers anywhere on the left; the stick is centred where the finger came down
                    Ctl::Dir(if self.prefs.stick { p } else { l.dpad })
                } else if p[0] > self.win[0] * 0.55 {
                    Ctl::Face
                } else {
                    return;
                };
                self.fingers.push(Finger { id, ctl, at: p });
            }
            1 => {
                let stick = self.prefs.stick;
                if let Some(f) = self.fingers.iter_mut().find(|f| f.id == id) {
                    f.at = p;
                    // the stick's centre follows a finger that goes past the rim
                    if let (Ctl::Dir(c), true) = (&mut f.ctl, stick) {
                        let (v, n) = ([p[0] - c[0], p[1] - c[1]], dist(p, *c));
                        if n > l.r {
                            *c = [p[0] - v[0] / n * l.r, p[1] - v[1] / n * l.r];
                        }
                    }
                }
            }
            _ => {
                if let Some(k) = self.fingers.iter().position(|f| f.id == id) {
                    let f = self.fingers.remove(k);
                    if phase == 2 && f.ctl == Ctl::Opt && self.opt && inside(p, l.opt, 0.6) {
                        self.opt_tapped = true;
                    }
                }
            }
        }
    }

    /// The face buttons a finger at `p` presses: those within 0.8 `d` of it (between two, both), else the nearest
    /// one within reach.
    fn face_bits(l: &Layout, p: [f32; 2]) -> u16 {
        let mut bits = 0;
        let mut near = (f32::MAX, 0);
        for (k, &(_, bit, _)) in FACE.iter().enumerate() {
            let n = dist(p, l.button(k));
            if n < l.d * 0.8 {
                bits |= bit;
            }
            if n < near.0 {
                near = (n, bit);
            }
        }
        if bits == 0 && near.0 < l.d + l.b * 1.6 { near.1 } else { bits }
    }

    /// The raw pad bits the fingers hold.
    pub fn bits(&self) -> u16 {
        let l = self.layout();
        let mut bits = 0;
        for f in &self.fingers {
            bits |= match f.ctl {
                // from the centre held since the touch: a finger still on the d-pad when a fight starts or ends (the
                // layouts place it apart) keeps its direction
                Ctl::Dir(c) => dir_bits([f.at[0] - c[0], f.at[1] - c[1]], l.r * if self.prefs.stick { 0.22 } else { 0.18 }),
                Ctl::Face => Self::face_bits(&l, f.at),
                Ctl::Button(b) => b,
                Ctl::Opt => 0,
            };
        }
        bits
    }

    /// The controls as quads in clip space of the window (`Renderer::screen`); none while hidden.
    pub fn quads(&self, fonts: &Fonts) -> Vec<PsxQuad> {
        let mut q = Draw { out: Vec::new(), win: self.win, level: self.prefs.opacity() };
        if !self.shown {
            return q.out;
        }
        let l = self.layout();
        let held = self.bits();
        let lit = |bit: u16| held & bit != 0;
        // the direction: the d-pad's cross, or the stick's base and knob
        if self.prefs.stick {
            let finger = self.fingers.iter().find_map(|f| match f.ctl {
                Ctl::Dir(c) => Some((c, f.at)),
                _ => None,
            });
            let (c, at) = finger.unwrap_or((l.dpad, l.dpad));
            q.disc(c, l.r, IDLE);
            q.ring(c, l.r * 0.94, l.r, EDGE);
            let v = [at[0] - c[0], at[1] - c[1]];
            let n = dist(at, c).max(1e-3);
            let reach = n.min(l.r * 0.6);
            let knob = [c[0] + v[0] / n * reach, c[1] + v[1] / n * reach];
            q.disc(knob, l.r * 0.42, if finger.is_some() { LIT } else { EDGE });
        } else {
            let (c, a) = (l.dpad, l.r * 0.33);
            for (dir, bit) in [([0.0, -1.0], UP), ([1.0, 0.0], RIGHT), ([0.0, 1.0], DOWN), ([-1.0, 0.0], LEFT)] {
                // the arm from the centre square out to the rim, its arrow near the end
                let (x0, y0) = (c[0] + dir[0] * a, c[1] + dir[1] * a);
                let (x1, y1) = (c[0] + dir[0] * l.r, c[1] + dir[1] * l.r);
                let (px, py) = (dir[1].abs() * a, dir[0].abs() * a);
                q.rect([x0.min(x1) - px, y0.min(y1) - py, x0.max(x1) + px, y0.max(y1) + py], if lit(bit) { LIT } else { IDLE });
                let tip = [c[0] + dir[0] * l.r * 0.86, c[1] + dir[1] * l.r * 0.86];
                let base = [c[0] + dir[0] * l.r * 0.58, c[1] + dir[1] * l.r * 0.58];
                let side = [dir[1] * a * 0.55, dir[0] * a * 0.55];
                q.tri(tip, [base[0] + side[0], base[1] + side[1]], [base[0] - side[0], base[1] - side[1]], EDGE);
            }
            q.rect([c[0] - a, c[1] - a, c[0] + a, c[1] + a], IDLE);
        }
        // the face buttons
        for (k, &(_, bit, col)) in FACE.iter().enumerate() {
            let c = l.button(k);
            q.disc(c, l.b, if lit(bit) { LIT } else { IDLE });
            q.ring(c, l.b * 0.9, l.b, EDGE);
            let col = if lit(bit) { [255.0; 3] } else { col };
            let s = l.b * 0.48;
            let w = l.b * 0.11;
            match bit {
                TRIANGLE => {
                    let p = [[c[0], c[1] - s], [c[0] + s * 0.95, c[1] + s * 0.65], [c[0] - s * 0.95, c[1] + s * 0.65]];
                    for e in 0..3 {
                        q.seg(p[e], p[(e + 1) % 3], w, col);
                    }
                }
                CIRCLE => q.ring(c, s * 0.85, s * 0.85 + 2.0 * w, col),
                CROSS => {
                    q.seg([c[0] - s * 0.8, c[1] - s * 0.8], [c[0] + s * 0.8, c[1] + s * 0.8], w, col);
                    q.seg([c[0] - s * 0.8, c[1] + s * 0.8], [c[0] + s * 0.8, c[1] - s * 0.8], w, col);
                }
                _ => {
                    let (a, b) = ([c[0] - s * 0.75, c[1] - s * 0.75], [c[0] + s * 0.75, c[1] + s * 0.75]);
                    for (p0, p1) in [(a, [b[0], a[1]]), ([b[0], a[1]], b), (b, [a[0], b[1]]), ([a[0], b[1]], a)] {
                        q.seg(p0, p1, w, col);
                    }
                }
            }
        }
        // L1, R1, START (and OPT outside fights)
        let mut boxes = vec![(l.l1, "L1", lit(L1)), (l.r1, "R1", lit(R1)), (l.start, "START", lit(START))];
        if self.opt && !self.fight {
            let down = self.fingers.iter().any(|f| f.ctl == Ctl::Opt);
            boxes.push((l.opt, "OPT", down));
        }
        for (bx, label, on) in boxes {
            q.rect([bx[0] - bx[2], bx[1] - bx[3], bx[0] + bx[2], bx[1] + bx[3]], if on { LIT } else { IDLE });
            q.text(fonts, label, [bx[0], bx[1]], bx[3] * 1.1);
        }
        q.out
    }
}

/// The controls' colours: a fill at rest and held, the outlines and arrows.
const IDLE: [f32; 3] = [40.0, 40.0, 46.0];
const LIT: [f32; 3] = [190.0, 190.0, 200.0];
const EDGE: [f32; 3] = [170.0, 170.0, 180.0];

/// Building the controls' quads: window pixels → clip space; by OPACITY the fills are left out (LOW), half-transparent
/// (B/2 + F/2, NORMAL) or opaque (HIGH), the outlines and symbols half-transparent or (HIGH) opaque.
struct Draw {
    out: Vec<PsxQuad>,
    win: [f32; 2],
    level: u8,
}

impl Draw {
    fn clip(&self, p: [f32; 2]) -> [f32; 4] {
        [p[0] / self.win[0] * 2.0 - 1.0, 1.0 - p[1] / self.win[1] * 2.0, 0.0, 1.0]
    }

    /// A quad (corners TL TR BL BR as the PS1 orders them; a triangle repeats its last corner); `fill`: a button's
    /// face rather than an outline or symbol.
    fn quad(&mut self, p: [[f32; 2]; 4], rgb: [f32; 3], fill: bool) {
        if fill && self.level == 0 {
            return;
        }
        let c = rgb.map(|v| v.clamp(0.0, 255.0) as u8);
        self.out.push(PsxQuad {
            pos: p.map(|p| self.clip(p)),
            uv: [[0.0; 2]; 4],
            rgb: [c; 4],
            tpage: 0,
            clut: 0,
            textured: false,
            semi: self.level < 2,
            sheet: None,
            clip: Clip::No,
        });
    }

    fn rect(&mut self, r: [f32; 4], rgb: [f32; 3]) {
        self.quad([[r[0], r[1]], [r[2], r[1]], [r[0], r[3]], [r[2], r[3]]], rgb, true);
    }

    fn tri(&mut self, a: [f32; 2], b: [f32; 2], c: [f32; 2], rgb: [f32; 3]) {
        self.quad([a, b, c, c], rgb, false);
    }

    /// A line from `a` to `b`, `w` either side.
    fn seg(&mut self, a: [f32; 2], b: [f32; 2], w: f32, rgb: [f32; 3]) {
        let n = dist(a, b).max(1e-3);
        let o = [-(b[1] - a[1]) / n * w, (b[0] - a[0]) / n * w];
        self.quad([[a[0] + o[0], a[1] + o[1]], [b[0] + o[0], b[1] + o[1]], [a[0] - o[0], a[1] - o[1]], [b[0] - o[0], b[1] - o[1]]], rgb, false);
    }

    const SIDES: usize = 24;

    fn corner(c: [f32; 2], r: f32, k: usize) -> [f32; 2] {
        let a = k as f32 * std::f32::consts::TAU / Self::SIDES as f32;
        [c[0] + r * a.cos(), c[1] + r * a.sin()]
    }

    fn disc(&mut self, c: [f32; 2], r: f32, rgb: [f32; 3]) {
        for k in 0..Self::SIDES {
            self.quad([c, Self::corner(c, r, k), c, Self::corner(c, r, k + 1)], rgb, true);
        }
    }

    fn ring(&mut self, c: [f32; 2], r0: f32, r1: f32, rgb: [f32; 3]) {
        for k in 0..Self::SIDES {
            self.quad([Self::corner(c, r0, k), Self::corner(c, r1, k), Self::corner(c, r0, k + 1), Self::corner(c, r1, k + 1)], rgb, false);
        }
    }

    /// `s` in the small font, centred on `c`, `h` pixels high.
    fn text(&mut self, f: &Fonts, s: &str, c: [f32; 2], h: f32) {
        let mut p: Vec<Prim> = Vec::new();
        f.small_text(&mut p, s, 0, 0, pal::GREY);
        // the small font's cell: 16 units high in the 512 × 480 space
        let k = h / 16.0;
        let w = f.width(&f.small, s) as f32 * k;
        let o = [c[0] - w * 0.5, c[1] - h * 0.5];
        for g in p {
            let pos = g.xy.map(|[x, y]| self.clip([o[0] + x * k, o[1] + y * k]));
            self.out.push(PsxQuad {
                pos,
                uv: g.uv,
                rgb: g.rgb,
                tpage: g.tpage,
                clut: g.clut,
                textured: g.textured,
                semi: g.semi,
                sheet: g.sheet,
                clip: Clip::No,
            });
        }
    }
}

// ------------------------------------------------------------------ the TOUCH CONTROLS page

/// What the app must do after a frame of the page.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Cmd {
    None,
    Close,
    /// the choices changed: applied and saved
    Prefs(TouchPrefs),
}

/// □ × ○ confirm, as the list menus.
const CONFIRM: u16 = SQUARE | CROSS | CIRCLE;
/// Menu font palettes: highlighted, normal, orange (values), as the replay list's.
const HL: u16 = 0;
const NORMAL: u16 = 1;
const ORANGE: u16 = 3;
const TOP_Y: i32 = 96;
const ROW_H: i32 = 32;
/// (the game data is changed from OPTIONS → CHANGE GAME DATA, the setup screen)
const ROWS: [Label; 4] = [Port("DIRECTION"), Port("SIZE"), Port("OPACITY"), Game("exit")];
const LEVELS: [Label; 3] = [Game("small"), Game("normal"), Port("LARGE")];
const OPACITIES: [Label; 3] = [Port("LOW"), Game("normal"), Port("HIGH")];

/// The TOUCH CONTROLS page, in the list menus' style (`menus::frame`, the banner, the cursor `>`); the session
/// waits while it is up.
pub struct Menu {
    pub prefs: TouchPrefs,
    cursor: usize,
    banner_x: i32,
    pads: Pads,
}

impl Menu {
    pub fn new(prefs: TouchPrefs, held: [u16; 2]) -> Self {
        let mut pads = Pads::default();
        pads.update(held);
        Self { prefs, cursor: 0, banner_x: 0, pads }
    }

    /// The row's value changed by `d` (±1; a confirm counts as +1).
    fn change(&mut self, d: i32) -> Cmd {
        let step = |v: u8, n: usize| ((v as i32 + d).rem_euclid(n as i32)) as u8;
        match self.cursor {
            0 => self.prefs.stick = !self.prefs.stick,
            1 => self.prefs.size = step(self.prefs.size, LEVELS.len()),
            2 => self.prefs.opacity = step(self.prefs.opacity, OPACITIES.len()),
            _ => return Cmd::Close,
        }
        Cmd::Prefs(self.prefs)
    }

    /// One 60 Hz frame with both pads; the sounds (`play(0x400, id)`) go into `sounds`.
    pub fn tick(&mut self, raw: [u16; 2], sounds: &mut Vec<u16>) -> Cmd {
        self.pads.update(raw);
        self.banner_x -= 4;
        while self.banner_x < -BANNER_W {
            self.banner_x += BANNER_W;
        }
        let (press, rep) = (self.pads.any_press(), self.pads.any_rep());
        if press & TRIANGLE != 0 {
            sounds.push(0x0D);
            return Cmd::Close;
        }
        if rep & (UP | DOWN) != 0 {
            sounds.push(0x0A);
            self.cursor = if rep & DOWN != 0 { (self.cursor + 1) % ROWS.len() } else { (self.cursor + ROWS.len() - 1) % ROWS.len() };
        }
        if rep & (LEFT | RIGHT) != 0 && self.cursor < 3 {
            sounds.push(0x0B);
            return self.change(if rep & RIGHT != 0 { 1 } else { -1 });
        }
        if press & CONFIRM != 0 {
            sounds.push(if self.cursor < 3 { 0x0B } else { 0x0C });
            return self.change(1);
        }
        Cmd::None
    }

    /// A tap at (x, y) of the 512 × 480 space: the row under it chosen and changed.
    pub fn tap(&mut self, x: f32, y: f32, sounds: &mut Vec<u16>) -> Cmd {
        let row = ((y - (TOP_Y - 8) as f32) / ROW_H as f32).floor();
        if !(0.0..ROWS.len() as f32).contains(&row) || !(16.0..496.0).contains(&x) {
            return Cmd::None;
        }
        self.cursor = row as usize;
        sounds.push(if self.cursor < 3 { 0x0B } else { 0x0C });
        self.change(1)
    }

    pub fn draw(&self, f: &Fonts, out: &mut Vec<Prim>) {
        crate::frontend::menus::frame(out, f, 6);
        crate::frontend::menus::banner(f, out, "TOUCH CONTROLS", BANNER_W, self.banner_x, 48, 7);
        f.menu_text(out, "TOUCH CONTROLS", 48, 48, HL);
        for (k, label) in ROWS.iter().enumerate() {
            let y = TOP_Y + ROW_H * k as i32;
            let hl = k == self.cursor;
            if hl {
                f.menu_text(out, ">", 32, y, HL);
            }
            f.menu_text(out, label.text(), 48, y, if hl { HL } else { NORMAL });
            let value = match k {
                0 => Some(if self.prefs.stick { "STICK" } else { "D-PAD" }),
                1 => Some(LEVELS[(self.prefs.size as usize).min(2)].text()),
                2 => Some(OPACITIES[(self.prefs.opacity as usize).min(2)].text()),
                _ => None,
            };
            if let Some(v) = value {
                f.menu_text(out, v, 304, y, ORANGE);
            }
        }
        let hint = match self.cursor {
            0 => "THE STICK IS CENTRED WHERE THE FINGER COMES DOWN.",
            _ => "",
        };
        f.menu_small(out, hint, 48, TOP_Y + ROW_H * ROWS.len() as i32 + 16, NORMAL);
    }
}

/// The banner's period: the text width + 64, as the list menus'.
const BANNER_W: i32 = 288;

#[cfg(test)]
mod tests {
    use super::*;

    fn touch() -> Touch {
        let mut t = Touch::new(TouchPrefs::default());
        t.set_window(2400, 1080);
        t.fight = true;
        t
    }

    #[test]
    fn front_end_clear_of_the_menus() {
        // a phone (20:9) and a 16:9 one; every SIZE; with and without a cutout on the left: the front end's own
        // layout, all clear (a cutout narrows the side: only the 20:9 phone, where cutouts are, is asked to keep the
        // larger sizes with one)
        for (w, h) in [(2400.0, 1080.0), (1920.0, 1080.0)] {
            for s in SIZES {
                for il in [0.0, 120.0] {
                    let insets = [il, 0.0, 0.0, 0.0];
                    let l = Layout::front(w, h, insets, s);
                    let core = Core::new(w, h);
                    let at = format!("{w}x{h} {s} {il}");
                    assert_ne!(l, Layout::new(w, h, insets, s), "{at}");
                    assert!(l.r >= h * FRONT_MIN[0] && l.b >= h * FRONT_MIN[1], "{at}");
                    // the d-pad and the face buttons in the corners the screens leave free
                    assert!(l.dir_shapes().iter().all(|&s| core.clear(s, false)), "{at}");
                    assert!(l.face_shapes().iter().all(|&s| core.clear(s, true)), "{at}");
                    // L1 / OPT, START / R1 beside the core or in those corners, inside the window and the insets
                    let usable = il == 0.0 || w == 2400.0;
                    let boxes = [(l.l1, false), (l.opt, false), (l.r1, true), (l.start, true)];
                    for (bx, right) in boxes {
                        let r = rect(bx);
                        assert!(core.beside(r, right) || core.clear(Shape::Rect(r), right), "{at}: {bx:?}");
                        assert!(r[0] >= il && r[2] <= w && r[1] >= 0.0 && r[3] <= h, "{at}: {bx:?}");
                        // big enough to hit
                        assert!(!usable || (r[3] - r[1] >= h * 0.035 && r[2] - r[0] >= h * 0.06), "{at}: {bx:?}");
                    }
                    // the d-pad at least 0.11 of the height across, a face button 0.04
                    assert!(!usable || (l.r >= h * 0.055 && l.b >= h * 0.02), "{at}: {} {}", l.r, l.b);
                    // and no two controls overlap, nor the boxes' grown hit areas (`event`)
                    let e = l.d + l.b;
                    let mut all = vec![
                        [l.dpad[0] - l.r, l.dpad[1] - l.r, l.dpad[0] + l.r, l.dpad[1] + l.r],
                        [l.face[0] - e, l.face[1] - e, l.face[0] + e, l.face[1] + e],
                    ];
                    all.extend([hit(l.l1, 0.5), hit(l.opt, 0.6), hit(l.r1, 0.5), hit(l.start, 0.6)]);
                    for i in 0..all.len() {
                        for j in i + 1..all.len() {
                            assert!(apart(all[i], all[j]), "{at}: {:?} {:?}", all[i], all[j]);
                        }
                    }
                }
            }
        }
        // the 20:9 phone keeps the SIZE (as far as LARGE), the boxes beside the core, both sides alike even with a
        // cutout on one
        let l = Layout::front(2400.0, 1080.0, [0.0; 4], 1.0);
        assert!(l.r > 200.0 && l.l1[1] < 540.0 && l.r1[1] < 540.0);
        let l = Layout::front(2400.0, 1080.0, [120.0, 0.0, 0.0, 0.0], 1.0);
        assert!(l.l1[1] == l.r1[1] && l.opt[1] == l.start[1] && l.l1[2] == l.r1[2]);
    }

    #[test]
    fn front_end_falls_back_on_a_tablet() {
        // a 4:3 tablet (no room beside the core): the fight's layout at its sizes, see-through over the menus; only
        // OPT moves, to the bottom line clear of the menus, at a usable size and clear of the other controls' hit areas
        for s in SIZES {
            for il in [0.0, 120.0] {
                let (w, h, insets) = (2048.0, 1536.0, [il, 0.0, 0.0, 0.0]);
                let (l, f) = (Layout::front(w, h, insets, s), Layout::new(w, h, insets, s));
                let at = format!("{s} {il}");
                assert_eq!(Layout { opt: f.opt, ..l }, f, "{at}");
                let o = rect(l.opt);
                assert!(Core::new(w, h).clear(Shape::Rect(o), false), "{at}: {o:?}");
                assert!(o[3] - o[1] >= h * 2.0 * FRONT_MIN[2] && o[0] >= il && o[3] <= h, "{at}: {o:?}");
                let g = l.opt[3] * 0.6;
                assert!(o[1] - g > l.dpad[1] + l.r, "{at}: {o:?}");
            }
        }
    }

    #[test]
    fn front_end_buttons() {
        let mut t = touch();
        t.fight = false;
        t.opt = true;
        let l = t.layout();
        assert_ne!(l, Layout::new(2400.0, 1080.0, [0.0; 4], 1.0));
        t.event(0, 1, [l.dpad[0] + l.r * 0.8, l.dpad[1]]);
        t.event(0, 2, l.button(2));
        t.event(0, 3, [l.l1[0], l.l1[1]]);
        t.event(0, 4, [l.start[0], l.start[1]]);
        t.event(0, 5, [l.r1[0], l.r1[1]]);
        assert_eq!(t.bits(), RIGHT | CROSS | L1 | START | R1);
        t.event(0, 6, [l.opt[0], l.opt[1]]);
        t.event(2, 6, [l.opt[0], l.opt[1]]);
        assert!(t.opt_tapped);
        // a fight starting under a held d-pad: the finger keeps its direction; a new touch reads the fight's d-pad
        t.fight = true;
        assert_eq!(t.bits() & (UP | DOWN | LEFT | RIGHT), RIGHT);
        t.event(2, 1, [0.0; 2]);
        let f = t.layout();
        t.event(0, 1, [f.dpad[0], f.dpad[1] - f.r * 0.8]);
        assert_eq!(t.bits() & (UP | DOWN | LEFT | RIGHT), UP);
    }

    #[test]
    fn front_end_buttons_apart_on_narrow_windows() {
        // a foldable's inner screen in portrait, a near-square split screen: the fight's boxes crowd there, and OPT
        // (tested first) must not lie on START, L1 or R1
        for (w, h) in [(1812.0, 2176.0), (1080.0, 1200.0), (1080.0, 2400.0)] {
            for s in SIZES {
                let l = Layout::front(w, h, [0.0; 4], s);
                let o = hit(l.opt, 0.6);
                for (bx, pad) in [(l.start, 0.6), (l.l1, 0.5), (l.r1, 0.5)] {
                    assert!(apart(o, hit(bx, pad)), "{w}x{h} {s}: {:?} {bx:?}", l.opt);
                }
                assert!(o[1] >= 0.0, "{w}x{h} {s}: {o:?}");
            }
        }
    }

    #[test]
    fn dpad_sectors() {
        let t = touch();
        let l = t.layout();
        let at = |dx: f32, dy: f32| {
            let mut t = touch();
            t.event(0, 1, [l.dpad[0] + dx, l.dpad[1] + dy]);
            t.bits()
        };
        assert_eq!(at(0.0, 0.0), 0);
        assert_eq!(at(l.r * 0.8, 0.0), RIGHT);
        assert_eq!(at(-l.r * 0.8, 0.0), LEFT);
        assert_eq!(at(0.0, -l.r * 0.8), UP);
        assert_eq!(at(l.r * 0.5, l.r * 0.5), RIGHT | DOWN);
        assert_eq!(at(-l.r * 0.5, -l.r * 0.5), LEFT | UP);
        // a 20° tilt is still straight right
        assert_eq!(at(l.r * 0.8, l.r * 0.29), RIGHT);
    }

    #[test]
    fn clear_of_the_fight_hud() {
        // a phone (20:9), a 16:9 one and a 4:3 tablet; every SIZE; a cutout on the left
        for (w, h) in [(2400.0, 1080.0), (1920.0, 1080.0), (2048.0, 1536.0)] {
            for s in SIZES {
                let l = Layout::new(w, h, [120.0, 0.0, 0.0, 0.0], s);
                let top = l.l1[1] - l.l1[3];
                assert!(top >= h * TOP && l.r1[1] - l.r1[3] >= h * TOP, "{w}x{h} {s}: {top}");
                assert!(l.dpad[1] + l.r <= h * BOTTOM + 0.5 && l.face[1] + l.d + l.b <= h * BOTTOM + 0.5);
                assert!(l.dpad[0] - l.r >= 120.0 && l.face[0] + l.d + l.b <= w);
            }
        }
    }

    #[test]
    fn multitouch_and_slides() {
        let mut t = touch();
        let l = t.layout();
        t.event(0, 1, [l.dpad[0] - l.r * 0.7, l.dpad[1]]);
        t.event(0, 2, l.button(2));
        assert_eq!(t.bits(), LEFT | CROSS);
        // between × and ○: both
        let (x, o) = (l.button(2), l.button(1));
        t.event(1, 2, [(x[0] + o[0]) / 2.0, (x[1] + o[1]) / 2.0]);
        assert_eq!(t.bits(), LEFT | CROSS | CIRCLE);
        t.event(1, 2, l.button(0));
        assert_eq!(t.bits(), LEFT | TRIANGLE);
        t.event(2, 1, [0.0, 0.0]);
        assert_eq!(t.bits(), TRIANGLE);
        t.event(3, 2, [0.0, 0.0]);
        assert_eq!(t.bits(), 0);
    }

    #[test]
    fn stick_floats_and_buttons() {
        let mut t = touch();
        t.prefs.stick = true;
        // below L1 (the stick answers anywhere on the left but on the buttons there)
        t.event(0, 1, [300.0, 900.0]);
        assert_eq!(t.bits(), 0);
        t.event(1, 1, [300.0, 700.0]);
        assert_eq!(t.bits(), UP);
        let l = t.layout();
        t.event(0, 2, [l.start[0], l.start[1]]);
        t.event(0, 3, [l.r1[0], l.r1[1]]);
        assert_eq!(t.bits(), UP | START | R1);
        // OPT only when offered, and only on its release over it
        t.event(0, 4, [l.opt[0], l.opt[1]]);
        t.event(2, 4, [l.opt[0], l.opt[1]]);
        assert!(!t.opt_tapped);
        t.opt = true;
        t.event(0, 4, [l.opt[0], l.opt[1]]);
        assert_eq!(t.bits(), UP | START | R1);
        t.event(2, 4, [l.opt[0], l.opt[1]]);
        assert!(t.opt_tapped);
    }

    #[test]
    fn page_rows_and_prefs() {
        let mut m = Menu::new(TouchPrefs::default(), [0; 2]);
        let mut s = Vec::new();
        assert_eq!(m.tap(100.0, TOP_Y as f32, &mut s), Cmd::Prefs(TouchPrefs { stick: true, ..Default::default() }));
        assert_eq!(m.tap(100.0, (TOP_Y + ROW_H) as f32, &mut s), Cmd::Prefs(TouchPrefs { stick: true, size: 2, opacity: 1 }));
        assert_eq!(m.tap(100.0, (TOP_Y + 3 * ROW_H) as f32, &mut s), Cmd::Close);
        assert_eq!(m.tap(100.0, (TOP_Y + 4 * ROW_H) as f32, &mut s), Cmd::None);
        assert_eq!(m.tap(100.0, 20.0, &mut s), Cmd::None);
        let p = TouchPrefs { stick: true, size: 0, opacity: 2 };
        assert_eq!(TouchPrefs::from_stored(serde_json::to_string(&p).ok()), p);
        assert_eq!(TouchPrefs::from_stored(Some("{".into())), TouchPrefs::default());
    }
}
