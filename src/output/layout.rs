//! Window layout: where the 4:3 screen sits in the window, the 512×480 HUD space in clip space.
//!
//! Engine addition.
//! State: vocabulary: `Anchor` and friends are serialised where state embeds prims
//!
//! Window layout (engine addition): where the game's 4:3 screen sits in the window, the 3D view's aspect and the
//! mapping of the 512×480 HUD space to clip space. The renderer owns one `Viewport`, rebuilt on every resize and
//! fit change; all sizes are physical pixels. Prims carry an `Anchor` that places them in a window wider or taller
//! than 4:3 (Fill); with no extension (Classic, an exact 4:3 window) every anchor maps as the core does.

use crate::output::prim::Prim;
use crate::output::psx_quad::PsxQuad;
use serde::{Deserialize, Serialize};

/// The widest 3D view (decision 7): some stages' geometry ends beside a wider view, so a wider window shows the
/// scene in a centred 21:9 band with the ambient fill beside it.
pub const MAX_ASPECT_3D: f32 = 64.0 / 27.0;
/// The widest (or tallest) HUD safe area (decision 2): the HUD's edge anchors stop at 16:9.
pub const MAX_ASPECT_HUD: f32 = 16.0 / 9.0;
/// Design units beyond each side of the core that repeated backdrops (scrolling banners, mirrored bands) are drawn
/// out to: windows up to about 7:1. The window edge (Fill) or the core scissor (Classic) cuts the rest.
pub const REACH: i32 = 1024;

/// How the game's 4:3 screen fills the window.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Fit {
    /// The game's 4:3 picture in the centred 4:3 rectangle, black bars around it (viewport + scissor).
    Classic,
    /// The whole window is drawn: the 3D view widens (Hor+ / Vert+), the 2D screens extend to the edges.
    Fill,
}

/// Placement along one axis of the extended window.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Ax {
    /// the core's place; an untextured prim's edges on the core border move out to the window border (fades,
    /// backdrops, gradient halves and bars)
    #[default]
    Center,
    /// moved with the left / top edge
    Start,
    /// moved with the right / bottom edge
    End,
    /// the core's 0..512 (0..480) spread over the whole window
    Stretch,
}

/// Where a prim sits in a window wider or taller than 4:3.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Anchor {
    pub x: Ax,
    pub y: Ax,
    /// Start / End stop at the HUD safe area (`MAX_ASPECT_HUD`) instead of the window edge
    pub safe: bool,
    /// the scissor (`Clip`)
    pub clip: Clip,
    /// part of a fixed picture (title, logos, story pictures, movies) or of a screen with no extension above and
    /// below: the bars get a blurred, darkened copy of it (`ambient_split`)
    pub ambient: Ambient,
}

/// A prim's scissor in the window.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Clip {
    /// the drawn area
    #[default]
    No,
    /// the core 4:3 rectangle, as the game's drawing area clipped (slide-ins, scrolling text)
    Core,
    /// the window outside the core (empty in Classic and at 4:3): copies that extend a repeated backdrop or text,
    /// cut where they would reach into the core
    Outside,
}

/// Which bars around the marked prims get the ambient fill.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Ambient {
    #[default]
    No,
    /// all around the prims' bounding box
    Around,
    /// above and below it only (a screen that extends sideways by itself)
    TopBottom,
}

impl Anchor {
    pub const CORE: Anchor = Anchor { x: Ax::Center, y: Ax::Center, safe: false, clip: Clip::No, ambient: Ambient::No };
    pub const AMBIENT: Anchor = Anchor { ambient: Ambient::Around, ..Anchor::CORE };
    pub const CLIP: Anchor = Anchor { clip: Clip::Core, ..Anchor::CORE };

    pub const fn new(x: Ax, y: Ax) -> Self {
        Anchor { x, y, ..Anchor::CORE }
    }

    /// A HUD anchor: Start / End against the safe area.
    pub const fn hud(x: Ax, y: Ax) -> Self {
        Anchor { x, y, safe: true, ..Anchor::CORE }
    }
}

/// Anchor every prim of `prims`.
pub fn tag(prims: &mut [Prim], at: Anchor) {
    for p in prims {
        p.at = at;
    }
}

/// Extend a backdrop that fills the `size` rectangle from (0, 0) (the core, in the drawer's units) by mirrored
/// copies of `out[start..]`: `n` tiles on each side (x, y), every other one mirrored so the seams match. The copies
/// are drawn outside the core only (`Clip::Outside`).
pub fn mirror_tiles(out: &mut Vec<Prim>, start: usize, size: [f32; 2], n: [i32; 2]) {
    let end = out.len();
    let at = |v: f32, k: i32, s: f32| k as f32 * s + if k & 1 != 0 { s - v } else { v };
    for j in -n[1]..=n[1] {
        for i in -n[0]..=n[0] {
            if i == 0 && j == 0 {
                continue;
            }
            for q in start..end {
                let mut p = out[q];
                p.xy = p.xy.map(|[x, y]| [at(x, i, size[0]), at(y, j, size[1])]);
                p.at.clip = Clip::Outside;
                out.push(p);
            }
        }
    }
}

/// The renderer's ambient fill for one frame: after the quads before `at` are drawn, the window outside `rect`
/// (pixels x, y, w, h) gets a blurred, darkened copy of what `rect` shows, scaled to cover the window. `scene`:
/// those quads are the capped 3D view, drawn with `rect` as viewport and scissor.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Split {
    pub at: usize,
    pub rect: [u32; 4],
    pub scene: bool,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Viewport {
    /// window size (w, h)
    pub win: [u32; 2],
    /// the largest centred 4:3 rectangle (x, y, w, h)
    pub core: [u32; 4],
    /// window pixels per HUD design unit (x, y): the 512×480 space over `core`
    pub unit: [f32; 2],
    /// the window beyond `core` in design units (left, top, right, bottom); 0 in Classic
    pub ext: [f32; 4],
    /// the HUD safe area beyond `core` in design units (left, top, right, bottom): `ext` up to `MAX_ASPECT_HUD`
    pub safe: [f32; 4],
    pub fit: Fit,
    /// HUD SIZE (engine addition): prims with a HUD anchor (`Anchor::safe`) scaled by this about their anchor (the
    /// safe-area edge, or the core's centre line); 1 everywhere but the fight HUD (`with_hud`)
    pub hud: f32,
}

impl Viewport {
    pub fn new(win: [u32; 2], fit: Fit) -> Self {
        let [w, h] = win.map(|v| v.max(1));
        let (cw, ch) = if w * 3 >= h * 4 { (((h as u64 * 4 + 1) / 3) as u32, h) } else { (w, ((w as u64 * 3 + 2) / 4) as u32) };
        let (cw, ch) = (cw.clamp(1, w), ch.clamp(1, h));
        let core = [(w - cw) / 2, (h - ch) / 2, cw, ch];
        let unit = [ch as f32 * (4.0 / 3.0) / 512.0, ch as f32 / 480.0];
        let beyond = |sw: f32, sh: f32| {
            let (ex, ey) = (((sw / unit[0] - 512.0) / 2.0).max(0.0), ((sh / unit[1] - 480.0) / 2.0).max(0.0));
            [ex, ey, ex, ey]
        };
        let (ext, safe) = match fit {
            Fit::Classic => ([0.0; 4], [0.0; 4]),
            Fit::Fill => {
                let (wf, hf) = (w as f32, h as f32);
                (beyond(wf, hf), beyond(wf.min(hf * MAX_ASPECT_HUD), hf.min(wf * MAX_ASPECT_HUD)))
            }
        };
        Viewport { win, core, unit, ext, safe, fit, hud: 1.0 }
    }

    /// Android (engine addition): the HUD safe area out to the window's edges (as far as the 3D view's
    /// `MAX_ASPECT_3D` band) less the display cutout's safe `insets` (window pixels: left, top, right, bottom),
    /// instead of the centred 16:9 one: a 20:9 phone's HUD at its edges, clear of the camera. Vertically the band
    /// stays capped as in `new`. Classic unchanged.
    pub fn with_insets(self, insets: [f32; 4]) -> Self {
        if self.fit != Fit::Fill {
            return self;
        }
        let [bx, _, bw, _] = self.scene_rect().map(|v| v as f32);
        let ([w, h], [il, it, ir, ib]) = (self.win.map(|v| v as f32), insets.map(|v| v.max(0.0)));
        // the core's centre is the window's (`to_quads`): 256 / 240 units on each side of it
        let (cx, cy) = (w / 2.0, h / 2.0);
        // vertically still the centred band `new` caps (a window taller than `MAX_ASPECT_HUD`: multi-window)
        let hh = h.min(w * MAX_ASPECT_HUD) / 2.0;
        let (l, r, t, b) = (bx.max(il), (bx + bw).min(w - ir), it.max(cy - hh), (h - ib).min(cy + hh));
        let safe = [(cx - l) / self.unit[0] - 256.0, (cy - t) / self.unit[1] - 240.0, (r - cx) / self.unit[0] - 256.0, (b - cy) / self.unit[1] - 240.0];
        Viewport { safe, ..self }
    }

    /// This layout with the HUD anchors scaled by `s` (`hud_fit`).
    pub fn with_hud(self, s: f32) -> Self {
        Viewport { hud: s, ..self }
    }

    /// The drawn area in window pixels (x, y, w, h): `core` in Classic, the whole window in Fill.
    pub fn view_rect(&self) -> [u32; 4] {
        match self.fit {
            Fit::Classic => self.core,
            Fit::Fill => [0, 0, self.win[0], self.win[1]],
        }
    }

    /// Viewport and scissor of every pass: `Some(core)` in Classic.
    pub fn scissor(&self) -> Option<[u32; 4]> {
        (self.fit == Fit::Classic).then_some(self.core)
    }

    /// The 3D view in window pixels (x, y, w, h): the drawn area, narrowed to a centred `MAX_ASPECT_3D` band in a
    /// wider window.
    pub fn scene_rect(&self) -> [u32; 4] {
        let [x, y, w, h] = self.view_rect();
        let max = (h as f32 * MAX_ASPECT_3D).round() as u32;
        if self.fit == Fit::Fill && w > max { [x + (w - max) / 2, y, max, h] } else { [x, y, w, h] }
    }

    /// The 3D view is narrower than the window (`scene_rect`).
    pub fn scene_capped(&self) -> bool {
        self.scene_rect() != self.view_rect()
    }

    /// Width / height of the 3D view: exactly 4:3 in Classic.
    pub fn aspect3d(&self) -> f32 {
        match self.fit {
            Fit::Classic => 4.0 / 3.0,
            Fit::Fill => {
                let [_, _, w, h] = self.scene_rect();
                w as f32 / h.max(1) as f32
            }
        }
    }

    /// The 3D view's field (aspect_x, gain_y): Hor+ for wide views (aspect_x = the view's aspect, gain 1), Vert+
    /// for tall ones (aspect_x = 4/3, gain = aspect / (4/3) < 1), so the horizontal field is never narrower than
    /// the game's 4:3 one.
    pub fn fov3d(&self) -> [f32; 2] {
        let aspect = self.aspect3d();
        if aspect >= 4.0 / 3.0 { [aspect, 1.0] } else { [4.0 / 3.0, aspect / (4.0 / 3.0)] }
    }

    /// Clip-space scale (x, y) of view space for the GTE's H = `h` over the logical 320×240 screen: h/(120·aspect)
    /// and h/120 for aspect ≥ 4/3, h/160 and (h/120)·aspect/(4/3) below.
    pub fn proj_scale(&self, h: f32) -> [f32; 2] {
        let [ax, gy] = self.fov3d();
        [h / (120.0 * ax), h / 120.0 * gy]
    }

    /// The core 4:3 area's half-size in clip space of the drawn area (x, y): (1, 1) when the drawn area is 4:3.
    pub fn core_clip(&self) -> [f32; 2] {
        let aspect = match self.fit {
            Fit::Classic => 4.0 / 3.0,
            Fit::Fill => self.win[0] as f32 / self.win[1].max(1) as f32,
        };
        if aspect >= 4.0 / 3.0 { [(4.0 / 3.0) / aspect, 1.0] } else { [1.0, aspect / (4.0 / 3.0)] }
    }

    /// A HUD-space coordinate `v` on axis `k` (0 = x over 512, 1 = y over 480) placed by `a` (`untextured`: the
    /// Center edge rule applies).
    fn place(&self, v: f32, k: usize, a: Anchor, untextured: bool) -> f32 {
        let size = [512.0, 480.0][k];
        let (lo, hi) = (self.ext[k], self.ext[k + 2]);
        let (slo, shi) = if a.safe { (self.safe[k], self.safe[k + 2]) } else { (lo, hi) };
        let ax = [a.x, a.y][k];
        let p = match ax {
            Ax::Center if untextured && v <= 0.0 => v - lo,
            Ax::Center if untextured && v >= size => v + hi,
            Ax::Center => v,
            Ax::Start => v - slo,
            Ax::End => v + shi,
            Ax::Stretch => -lo + v * (size + lo + hi) / size,
        };
        if !a.safe || self.hud == 1.0 {
            return p;
        }
        let pivot = match ax {
            Ax::Start => -slo,
            Ax::End => size + shi,
            Ax::Center | Ax::Stretch => size / 2.0,
        };
        pivot + (p - pivot) * self.hud
    }
}

/// HUD SIZE (engine addition): `s`, but above 1 only as far as the HUD's rows (prims with a HUD anchor sharing a
/// vertical anchor) keep their side groups (Start / Center / End horizontally) from running into each other; at
/// least 1. A 4:3 window has no room between the life bars and the timer, a wider one the extra safe area.
pub fn hud_fit(prims: &[Prim], vp: &Viewport, s: f32) -> f32 {
    if s <= 1.0 {
        return s;
    }
    let (sl, sh) = (vp.safe[0], vp.safe[2]);
    let mut limit = s;
    for row in [Ax::Start, Ax::Center, Ax::End] {
        let xs = |ax: Ax| prims.iter().filter(move |p| p.at.safe && p.at.y == row && p.at.x == ax).flat_map(|p| p.xy.map(|[x, _]| x));
        let start = xs(Ax::Start).reduce(f32::max);
        let (c0, c1) = (xs(Ax::Center).reduce(f32::min), xs(Ax::Center).reduce(f32::max));
        let end = xs(Ax::End).reduce(f32::min);
        // the scaled groups (`Viewport::place`): Start x·s − sl, Center 256 + (x − 256)·s, End 512 + sh + (x − 512)·s
        let mut fit = |room: f32, span: f32| {
            if span > 0.0 {
                limit = limit.min(room / span);
            }
        };
        if let (Some(b), Some(c)) = (start, c0) {
            fit(256.0 + sl, b + 256.0 - c);
        }
        if let (Some(c), Some(e)) = (c1, end) {
            fit(256.0 + sh, c - 256.0 + 512.0 - e);
        }
        if let (Some(b), Some(e)) = (start, end) {
            fit(512.0 + sl + sh, b + 512.0 - e);
        }
    }
    limit.max(1.0)
}

/// HUD space (512×480) → clip-space quads of the drawn area, each prim placed by its anchor.
pub fn to_quads(prims: &[Prim], vp: &Viewport) -> Vec<PsxQuad> {
    let [sx, sy] = vp.core_clip();
    prims
        .iter()
        .map(|p| {
            let pos = p.xy.map(|[x, y]| {
                let (x, y) = (vp.place(x, 0, p.at, !p.textured), vp.place(y, 1, p.at, !p.textured));
                [(x - 256.0) / 256.0 * sx, (240.0 - y) / 240.0 * sy, 0.0, 1.0]
            });
            PsxQuad { pos, uv: p.uv, rgb: p.rgb, tpage: p.tpage, clut: p.clut, textured: p.textured, semi: p.semi, sheet: p.sheet, clip: p.at.clip }
        })
        .collect()
}

/// Quads in clip space of the drawn area (`to_quads`) → clip space of the capped 3D band (`scene_rect`), for HUD /
/// front-end quads drawn before a `scene` split (that pass has the band as viewport): same window pixels, the band
/// scissor cuts what lies beside it. No change when the view is not capped.
pub fn to_scene(quads: &mut [PsxQuad], vp: &Viewport) {
    if !vp.scene_capped() {
        return;
    }
    let ([vx, _, vw, _], [bx, _, bw, _]) = (vp.view_rect().map(|v| v as f32), vp.scene_rect().map(|v| v as f32));
    // x_band = ((x/w + 1)·vw + 2·(vx − bx)) / bw − 1, kept homogeneous in w
    let (s, o) = (vw / bw, (vw + 2.0 * (vx - bx)) / bw - 1.0);
    for q in quads {
        for p in &mut q.pos {
            p[0] = p[0] * s + p[3] * o;
        }
    }
}

/// The inverse of `to_scene` for a point: clip space of the capped 3D band (`scene_rect`, e.g. a projected point of
/// the 3D view) → clip space of the drawn area, where `to_quads` puts the HUD. No change when the view is not capped.
pub fn from_scene(p: [f32; 2], vp: &Viewport) -> [f32; 2] {
    if !vp.scene_capped() {
        return p;
    }
    let ([vx, _, vw, _], [bx, _, bw, _]) = (vp.view_rect().map(|v| v as f32), vp.scene_rect().map(|v| v as f32));
    let (s, o) = (vw / bw, (vw + 2.0 * (vx - bx)) / bw - 1.0);
    [(p[0] - o) / s, p[1]]
}

/// The ambient fill of a frame drawn from `prims` (1:1 with its quads): around the bounding box of the prims marked
/// `Anchor::ambient` (within the core; across the window for `Ambient::TopBottom`), applied after the last of them.
/// None in Classic, in a window with no extension (exactly 4:3, as Classic) or when the box fills the window.
pub fn ambient_split(prims: &[Prim], vp: &Viewport) -> Option<Split> {
    if vp.fit == Fit::Classic || vp.ext == [0.0; 4] {
        return None;
    }
    let last = prims.iter().rposition(|p| p.at.ambient != Ambient::No)?;
    let (mut x0, mut y0, mut x1, mut y1) = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
    for p in prims[..=last].iter().filter(|p| p.at.ambient != Ambient::No) {
        for [x, y] in p.xy {
            (x0, y0, x1, y1) = (x0.min(x), y0.min(y), x1.max(x), y1.max(y));
        }
    }
    let [cx, cy, cw, ch] = vp.core.map(|v| v as f32);
    let px = |v: f32| (cx + v.clamp(0.0, 512.0) * cw / 512.0).round() as u32;
    let py = |v: f32| (cy + v.clamp(0.0, 480.0) * ch / 480.0).round() as u32;
    let (l, t, r, b) = (px(x0), py(y0), px(x1), py(y1));
    let (l, r) = if prims[last].at.ambient == Ambient::TopBottom { (0, vp.win[0]) } else { (l, r) };
    let rect = [l, t, r.saturating_sub(l), b.saturating_sub(t)];
    (rect[2] > 0 && rect[3] > 0 && rect != [0, 0, vp.win[0], vp.win[1]]).then_some(Split { at: last + 1, rect, scene: false })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classic_core_and_scissor() {
        let vp = Viewport::new([1920, 1080], Fit::Classic);
        assert_eq!(vp.core, [240, 0, 1440, 1080]);
        assert_eq!(vp.view_rect(), vp.core);
        assert_eq!(vp.scissor(), Some(vp.core));
        assert_eq!(vp.aspect3d(), 4.0 / 3.0);
        assert_eq!(vp.core_clip(), [1.0, 1.0]);
        assert_eq!(vp.ext, [0.0; 4]);
        // tall: bars top and bottom
        assert_eq!(Viewport::new([720, 1280], Fit::Classic).core, [0, 370, 720, 540]);
    }

    #[test]
    fn fill_extends_the_core() {
        let vp = Viewport::new([1920, 1080], Fit::Fill);
        assert_eq!(vp.view_rect(), [0, 0, 1920, 1080]);
        assert_eq!(vp.scissor(), None);
        assert_eq!(vp.unit, [2.8125, 2.25]);
        // 1920 / 2.8125 = 682.67 units: 85.33 on each side
        assert!((vp.ext[0] - 85.333_336).abs() < 1e-3 && vp.ext[1] == 0.0 && vp.ext[0] == vp.ext[2]);
        assert_eq!(vp.core_clip(), [(4.0 / 3.0) / (1920.0 / 1080.0), 1.0]);
        // an exact 4:3 window is Classic
        let (f, c) = (Viewport::new([1024, 768], Fit::Fill), Viewport::new([1024, 768], Fit::Classic));
        assert_eq!((f.view_rect(), f.aspect3d(), f.core_clip(), f.ext), (c.view_rect(), c.aspect3d(), c.core_clip(), c.ext));
    }

    #[test]
    fn hor_plus_and_vert_plus() {
        let h = 360.0f32;
        // wide: the game's expressions (the 4:3 framing stays exact, the vertical field is the game's)
        let a = 2560.0f32 / 1080.0;
        assert_eq!(Viewport::new([2560, 1080], Fit::Fill).proj_scale(h), [h / (120.0 * a), h / 120.0]);
        assert_eq!(Viewport::new([1280, 720], Fit::Classic).proj_scale(h), [h / (120.0 * (4.0 / 3.0)), h / 120.0]);
        // tall: the 4:3 horizontal field, more shown above and below
        let vp = Viewport::new([720, 1280], Fit::Fill);
        let [kx, ky] = vp.proj_scale(h);
        assert_eq!(kx, h / 160.0);
        assert!((ky - h / 120.0 * (720.0 / 1280.0) / (4.0 / 3.0)).abs() < 1e-6);
        assert_eq!(vp.fov3d()[0], 4.0 / 3.0);
        assert!(vp.fov3d()[1] < 1.0);
    }

    #[test]
    fn hud_size_scales_about_the_anchor_and_fits() {
        let vp = Viewport::new([1920, 1080], Fit::Fill);
        let e = vp.safe[0];
        let top = |x: Ax| Anchor::hud(x, Ax::Start);
        // 1: as before; a scale moves a HUD prim about its safe edge (or the centre line), not a core one
        let big = vp.with_hud(1.25);
        assert_eq!(big.place(40.0, 0, top(Ax::Start), false), -e + 40.0 * 1.25);
        assert_eq!(big.place(472.0, 0, top(Ax::End), false), 512.0 + e - 40.0 * 1.25);
        assert_eq!(big.place(266.0, 0, top(Ax::Center), false), 256.0 + 10.0 * 1.25);
        assert_eq!(big.place(80.0, 1, top(Ax::Start), false), 80.0 * 1.25);
        assert_eq!(big.place(40.0, 0, Anchor::new(Ax::Start, Ax::Start), false), vp.place(40.0, 0, Anchor::new(Ax::Start, Ax::Start), false));
        // a top row as the fight's: life bars to 238 / from 274, the timer 236..276
        let row = [
            crate::output::prim::f4(20, 40, 218, 20, [0; 3]).at(top(Ax::Start)),
            crate::output::prim::f4(236, 40, 40, 32, [0; 3]).at(top(Ax::Center)),
            crate::output::prim::f4(274, 40, 218, 20, [0; 3]).at(top(Ax::End)),
        ];
        // no room in 4:3, (256 + e) / 258 in 16:9; smaller always
        assert_eq!(hud_fit(&row, &Viewport::new([1440, 1080], Fit::Classic), 1.25), 1.0);
        assert_eq!(hud_fit(&row, &vp, 1.25), 1.25);
        assert!((hud_fit(&row, &vp, 2.0) - (256.0 + e) / 258.0).abs() < 1e-5);
        assert_eq!(hud_fit(&row, &vp, 0.8), 0.8);
    }

    #[test]
    fn anchors_place_by_the_extension() {
        let vp = Viewport::new([1920, 1080], Fit::Fill);
        let e = vp.ext[0];
        let p = crate::output::prim::f4(10, 10, 20, 20, [0; 3]);
        // untextured, inside the core: unchanged; on the core border: out to the window border
        assert_eq!(vp.place(10.0, 0, Anchor::CORE, true), 10.0);
        assert_eq!(vp.place(0.0, 0, Anchor::CORE, true), -e);
        assert_eq!(vp.place(512.0, 0, Anchor::CORE, true), 512.0 + e);
        assert_eq!(vp.place(0.0, 0, Anchor::CORE, false), 0.0);
        assert_eq!(vp.place(10.0, 0, Anchor::new(Ax::Start, Ax::Center), false), 10.0 - e);
        assert_eq!(vp.place(10.0, 0, Anchor::new(Ax::End, Ax::Center), false), 10.0 + e);
        assert_eq!(vp.place(512.0, 0, Anchor::new(Ax::Stretch, Ax::Center), false), 512.0 + e);
        // 16:9 is the safe area's limit: the HUD reaches the edges; at 32:9 it stops at 16:9
        assert_eq!(vp.safe, vp.ext);
        let uw = Viewport::new([3840, 1080], Fit::Fill);
        assert!((uw.safe[0] - e).abs() < 1e-3 && uw.ext[0] > uw.safe[0]);
        // Classic and an exact 4:3 window: every anchor maps as the core does
        for vp in [Viewport::new([1920, 1080], Fit::Classic), Viewport::new([1024, 768], Fit::Fill)] {
            let a = Anchor { safe: true, clip: Clip::Core, ..Anchor::new(Ax::End, Ax::Stretch) };
            assert_eq!(to_quads(&[p.at(a)], &vp)[0].pos, to_quads(&[p], &vp)[0].pos);
        }
    }

    #[test]
    fn insets_take_the_hud_to_the_edges() {
        // a 20:9 phone: no insets, the window's edges (the 16:9 cap gone); the cutout's side keeps clear of it
        let vp = Viewport::new([2400, 1080], Fit::Fill);
        let e = vp.ext[0];
        assert!(vp.safe[0] < e);
        let a = vp.with_insets([0.0; 4]);
        assert!((a.safe[0] - e).abs() < 1e-3 && (a.safe[2] - e).abs() < 1e-3 && a.safe[1].abs() < 1e-3 && a.safe[3].abs() < 1e-3);
        let b = vp.with_insets([120.0, 0.0, 0.0, 0.0]);
        assert!((b.safe[0] - (e - 120.0 / vp.unit[0])).abs() < 1e-3 && (b.safe[2] - e).abs() < 1e-3);
        assert_eq!((b.ext, b.unit, b.core), (vp.ext, vp.unit, vp.core));
        // a top inset moves a top-anchored prim down by it
        let c = vp.with_insets([0.0, 54.0, 0.0, 0.0]);
        assert!((c.place(10.0, 1, Anchor::hud(Ax::Start, Ax::Start), false) - (10.0 + 54.0 / vp.unit[1])).abs() < 1e-3);
        // wider than the 3D band: its edges; Classic: unchanged
        let uw = Viewport::new([3840, 1080], Fit::Fill).with_insets([0.0; 4]);
        assert!((uw.safe[0] - (2560.0 / 2.0 / uw.unit[0] - 256.0)).abs() < 1e-3);
        let cl = Viewport::new([2400, 1080], Fit::Classic);
        assert_eq!(cl.with_insets([120.0, 0.0, 0.0, 0.0]), cl);
        // taller than 16:9 (multi-window): the top and bottom rows keep `new`'s cap; a deeper inset still counts
        let tall = Viewport::new([1080, 2400], Fit::Fill);
        let t = tall.with_insets([0.0; 4]);
        assert!(t.safe[1] < tall.ext[1] && (t.safe[1] - tall.safe[1]).abs() < 1e-3 && (t.safe[3] - tall.safe[3]).abs() < 1e-3);
        let t = tall.with_insets([0.0, 300.0, 0.0, 0.0]);
        assert!((t.safe[1] - ((1200.0 - 300.0) / tall.unit[1] - 240.0)).abs() < 1e-3);
    }

    #[test]
    fn scene_band_is_capped() {
        let vp = Viewport::new([3840, 1080], Fit::Fill);
        assert_eq!(vp.scene_rect(), [640, 0, 2560, 1080]);
        assert!(vp.scene_capped());
        assert!((vp.aspect3d() - MAX_ASPECT_3D).abs() < 1e-3);
        // the core keeps its place in the window
        assert_eq!(vp.core_clip(), [(4.0 / 3.0) / (3840.0 / 1080.0), 1.0]);
        for vp in [Viewport::new([2560, 1080], Fit::Fill), Viewport::new([720, 1280], Fit::Fill), Viewport::new([3840, 1080], Fit::Classic)] {
            assert!(!vp.scene_capped());
        }
        // window-space quads mapped into the band keep their window pixels
        let px = |x: f32, r: [u32; 4]| r[0] as f32 + (x + 1.0) / 2.0 * r[2] as f32;
        let vp = Viewport::new([3840, 1080], Fit::Fill);
        let mut q = to_quads(&[crate::output::prim::f4(128, 0, 256, 480, [0; 3]), crate::output::prim::f4(0, 0, 512, 480, [0; 3])], &vp);
        let before: Vec<[f32; 4]> = q.iter().map(|q| q.pos.map(|p| px(p[0], vp.view_rect()))).collect();
        assert_eq!([before[0][0], before[1][0], before[1][1]], [1560.0, 0.0, 3840.0]);
        to_scene(&mut q, &vp);
        for (q, b) in q.iter().zip(&before) {
            for (p, b) in q.pos.iter().zip(b) {
                assert!((px(p[0], vp.scene_rect()) - b).abs() < 1e-2);
            }
        }
    }

    #[test]
    fn ambient_split_and_mirror_tiles() {
        let mut v =
            vec![crate::output::prim::f4(0, 0, 512, 480, [0; 3]), crate::output::prim::ft4(0, 60, 512, 360, (0, 0, 256, 180), 0, 0).at(Anchor::AMBIENT)];
        v.push(crate::output::prim::f4(0, 0, 10, 10, [0; 3]));
        let vp = Viewport::new([1920, 1080], Fit::Fill);
        assert_eq!(ambient_split(&v, &vp), Some(Split { at: 2, rect: [240, 135, 1440, 810], scene: false }));
        assert_eq!(ambient_split(&v, &Viewport::new([1920, 1080], Fit::Classic)), None);
        // an exact 4:3 window is Classic: no fill, the letterbox stays black
        assert_eq!(ambient_split(&v, &Viewport::new([1024, 768], Fit::Fill)), None);
        // the window's bars only above and below for TopBottom
        v[1].at.ambient = Ambient::TopBottom;
        assert_eq!(ambient_split(&v, &Viewport::new([720, 1280], Fit::Fill)), Some(Split { at: 2, rect: [0, 438, 720, 405], scene: false }));
        // mirrored tiles: the left neighbour is the mirror image about x = 0
        let mut t = vec![crate::output::prim::ft4(0, 0, 256, 240, (0, 0, 255, 60), 0, 0)];
        mirror_tiles(&mut t, 0, [512.0, 240.0], [1, 0]);
        assert_eq!(t.len(), 3);
        assert_eq!(t[1].xy.map(|p| p[0]), [0.0, -256.0, 0.0, -256.0]);
        assert_eq!(t[2].xy.map(|p| p[0]), [1024.0, 768.0, 1024.0, 768.0]);
    }

    #[test]
    fn from_scene_inverts_to_scene() {
        // 32:9 is wider than the 3D band (capped), 16:9 not (both unchanged)
        for (win, capped) in [([3840, 1080], true), ([1920, 1080], false)] {
            let vp = Viewport::new(win, Fit::Fill);
            assert_eq!(vp.scene_capped(), capped);
            let mut q = to_quads(&[crate::output::prim::f4(-40, 30, 600, 200, [0; 3])], &vp);
            let before = q[0].pos;
            to_scene(&mut q, &vp);
            assert_eq!(q[0].pos == before, !capped);
            for (a, b) in q[0].pos.iter().zip(before) {
                let p = from_scene([a[0], a[1]], &vp);
                assert!((p[0] - b[0]).abs() < 1e-5 && (p[1] - b[1]).abs() < 1e-5, "{p:?} vs {b:?}");
            }
        }
    }
}
