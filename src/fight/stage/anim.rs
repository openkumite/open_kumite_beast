//! Stage animation: the per-stage routines' moving objects, alternate frames and CLUT rows.
//!
//! State: serialised: part of `GameState` (field changes are `STATE_VERSION` changes)
//!
//! Stage animation: what the per-stage routines at the end of the stage draw (table
//! the routine table, called from the stage draw) turn, show on alternate frames or recolour with 16×1 VRAM copies of CLUT rows.
//!
//! From simulation version 6 the routine's counters (W+0x26 / +0x28 / +0x2A, `Counters`) are part of the fight's
//! state as in the game: `step` once per drawn frame (also while paused), skipped while the stage is not drawn
//! (M.9E, the Beast Drive darkening at 255), reset at every round start (the stage init table), and rewound with the wall
//! block by the KO replay. The CLUT rows the copies wrote (`Rows`) stay in VRAM for the whole match. Up to version 5
//! everything is a function of the match's frame counter (`View::Frame`), counted from the match start through
//! both; the ranking background still draws that way.

use crate::fight::physics::Trig;
use crate::formats::gltf_model::FLOOR;
use crate::formats::vram::Vram;
use glam::{Mat4, Vec3, Vec4};
use serde::{Deserialize, Serialize};

/// Per stage: the slot-2 objects with their own model (`stages/<ST>_obj<k>.glb`, ok_br2_converter::stages), drawn
/// with `object_xf`.
const OBJECTS: [&[u8]; 13] = [&[], &[6, 7, 9, 11, 13, 15, 17, 19, 21], &[9], &[], &[4], &[], &[6, 7], &[8, 9], &[], &[], &[], &[], &[]];

/// The palette block of VRAM the animated stage CLUTs live in: the stage's CLUT slots 32..47 at (256 + 16k, 480),
/// 16 rows each.
pub const PAL_X: u32 = 256;
pub const PAL_Y: u32 = 480;
pub const PAL_W: u32 = 256;
pub const PAL_H: u32 = 16;

/// The stage objects that have their own model.
pub fn objects(stage: usize) -> &'static [u8] {
    OBJECTS.get(stage).copied().unwrap_or(&[])
}

/// The stage's export comes apart (`<ST>_base.glb` and the objects): moved objects, or objects the game never draws
/// (ST1: the 0xFF entries of its object list; STC: its routine draws objects 6..8 only).
pub fn split(stage: usize) -> bool {
    !objects(stage).is_empty() || stage == 12
}

/// The stage routine's counters W+0x26 / +0x28 / +0x2A, each incremented before use. They lie in the wall block
/// that the KO replay's snapshot copies (W..W+0x194), so a replay rewinds them.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Counters(pub [u16; 3]);

impl Counters {
    /// The stage init table's entry (end of the wall reset, every round start). ST0, ST2, ST6 and
    /// ST9..STC reset only matrices (ST6's toggle W+0x26 is kept).
    pub fn reset(&mut self, stage: usize) {
        match stage {
            1 => self.0 = [0; 3],
            3 => self.0[0] = 0,
            4 | 5 => (self.0[0], self.0[1]) = (0, 0),
            7 | 8 => self.0[0] = 0,
            _ => {}
        }
    }

    /// The counters on drawn frame `n` (1 = the first) of a match never reset or frozen (simulation versions up
    /// to 5).
    fn at(stage: usize, n: u64) -> Self {
        let c = |step: u16| ctr(n, step);
        Self(match stage {
            1 => [c(5), c(2), c(1)],
            3 => [c(0x14), 0, 0],
            4 => [c(1), ((n / 4) as u16).wrapping_mul(0x400), 0],
            5 => [0, c(0x14), 0],
            6 => [(n & 1) as u16, 0, 0], // W.26 taken as 0 before frame 1
            7 => [c(0x17), 0, 0],
            _ => [0; 3],
        })
    }
}

/// A W counter that adds `step` before each use: its value on drawn frame `n` (1 = the first) of a match never reset
/// or frozen.
fn ctr(n: u64, step: u16) -> u16 {
    (n as u16).wrapping_mul(step)
}

/// The CLUT rows the stage routine's VRAM copies wrote last, as the source row (x, y) of the copy (None: the stage
/// file's own colours): ST3 row (0x120, 0x1E0) in [0]; ST5 rows (0x110, 0x1E0 + r) in [r]. VRAM is reloaded only
/// with the stage, so these hold over the rounds and through the KO replay.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Rows(pub [Option<(u16, u16)>; 4]);

/// One drawn frame of the routine's counters and CLUT copies (the routine table's entry; skipped by the caller while the
/// stage is not drawn).
pub fn step(stage: usize, c: &mut Counters, rows: &mut Rows) {
    let w = &mut c.0;
    match stage {
        // W.26 += 5, W.28 += 2, W.2A += 1
        1 => {
            w[0] = w[0].wrapping_add(5);
            w[1] = w[1].wrapping_add(2);
            w[2] = w[2].wrapping_add(1);
        }
        // W.26 += 0x14, k = (W.26 >> 8) & 0x1F; k 1..15 copy (0x120, 0x1E0 + k), 16..31
        // (0x130, 0x1E0 + k − 16) to (0x120, 0x1E0); k = 0 copies nothing
        3 => {
            w[0] = w[0].wrapping_add(0x14);
            let k = (w[0] >> 8) & 0x1F;
            match k {
                0 => {}
                1..=15 => rows.0[0] = Some((0x120, 0x1E0 + k)),
                _ => rows.0[0] = Some((0x130, 0x1E0 + k - 16)),
            }
        }
        // W.26 += 1; when (W.26 & 3) == 0, W.28 += 0x400
        4 => {
            w[0] = w[0].wrapping_add(1);
            if w[0] & 3 == 0 {
                w[1] = w[1].wrapping_add(0x400);
            }
        }
        // W.28 += 0x14, k = (W.28 >> 8) & 7; k 0..3 copy row 0x1E4 (dark) to 0x1E0 + k, 4..7 row
        // 0x1E5 (bright) to 0x1E0 + k − 4
        5 => {
            w[1] = w[1].wrapping_add(0x14);
            let k = ((w[1] >> 8) & 7) as usize;
            if k < 4 {
                rows.0[k] = Some((0x110, 0x1E4));
            } else {
                rows.0[k - 4] = Some((0x110, 0x1E5));
            }
        }
        // W.26 = (W.26 == 0)
        6 => w[0] = (w[0] == 0) as u16,
        // W.26 += 0x17
        7 => w[0] = w[0].wrapping_add(0x17),
        _ => {}
    }
}

/// What the stage routine shows on a drawn frame.
#[derive(Clone, Copy, Debug)]
pub enum View {
    /// simulation versions up to 5 and the ranking background: drawn frame `n` of a match never reset or frozen
    Frame(u64),
    /// the fight's state (version 6)
    State(Counters, Rows),
}

impl View {
    fn counters(&self, stage: usize) -> Counters {
        match *self {
            View::Frame(n) => Counters::at(stage, n),
            View::State(c, _) => c,
        }
    }

    /// The VRAM row whose colours CLUT `clut` of `stage` shows (None: not animated).
    pub fn clut_source(&self, stage: usize, clut: u16) -> Option<(u32, u32)> {
        let rows = match *self {
            View::Frame(n) => return clut_source(stage, clut, n),
            View::State(_, rows) => rows,
        };
        let own = |r: usize, x: u16| rows.0[r].map(|(x, y)| (x as u32, y as u32)).unwrap_or((x as u32, 0x1E0 + r as u32));
        match (stage, clut) {
            (3, 0x7812) => Some(own(0, 0x120)),
            (5, 0x7811 | 0x7851 | 0x7891 | 0x78D1) => Some(own(((clut as u32 >> 6) - 0x1E0) as usize, 0x110)),
            _ => None,
        }
    }
}

/// RotMatrixY(a) ([[c, 0, s], [0, 4096, 0], [−s, 0, c]] in game space) in glTF space (x, −y, −z).
fn rot_y(t: &Trig, a: u16) -> Mat4 {
    let (c, s) = (t.cos(a as i32) as f32 / 4096.0, t.sin(a as i32) as f32 / 4096.0);
    Mat4::from_cols(Vec4::new(c, 0.0, s, 0.0), Vec4::Y, Vec4::new(-s, 0.0, c, 0.0), Vec4::W)
}

/// A translation of game units in glTF space.
fn translation(x: i32, y: i32, z: i32) -> Mat4 {
    Mat4::from_translation(Vec3::new(x as f32, -y as f32, -z as f32) / 300.0)
}

/// The matrix object `k` of `stage` is drawn with under `view` (world = M.R·v + M.t; None: not drawn this frame).
/// `cam`: the camera pivot (M.BC → +0x4A, `Camera::pos`).
pub fn object_xf(stage: usize, k: u8, view: &View, cam: [i16; 3], t: &Trig) -> Option<Mat4> {
    let w = view.counters(stage).0;
    match (stage, k) {
        // W.26 → objects 6, 7; W.28 → 9, 11, 13, 15; W.2A → 17, 19, 21 (t = 0)
        (1, 6 | 7) => Some(rot_y(t, w[0])),
        (1, 9..=15) => Some(rot_y(t, w[1])),
        (1, _) => Some(rot_y(t, w[2])),
        // ST2's sky / ST7's skyline: W+0x74 with t = (cam.pos.x, 0, cam.pos.z)
        (2, 9) | (7, 8) => Some(translation(cam[0] as i32, 0, cam[2] as i32)),
        // RotMatrixY(W.28)
        (4, 4) => Some(rot_y(t, w[1])),
        // object 6 while W.26 is set, else 7
        (6, 6) => (w[0] != 0).then_some(Mat4::IDENTITY),
        (6, 7) => (w[0] == 0).then_some(Mat4::IDENTITY),
        // the beacon (t = (0, 0, −4518) set by the stage init at every round start) on even W.26 only
        (7, 9) => (w[0] & 1 == 0).then(|| translation(0, 0, -4518) * rot_y(t, w[0])),
        _ => Some(Mat4::IDENTITY),
    }
}

/// `sorts`: objects `first..` up to the stage's object count W+0xC.
const REST: u8 = 0xFF;

/// The per-stage routines of the routine table as their object-drawer calls, in call order: (OT entry offset from M.8,
/// first object or `FLOOR`, count or `REST`). Everything they move or skip per frame is `object_xf`.
const ROUTINES: [&[(u16, u8, u8)]; 13] = [
    // ST0
    &[(0x2048, FLOOR, 1), (0x2048, 4, 1), (0x2074, 5, REST)],
    // ST1 (6, 7 / 8..15 / 16..21 each turned by its own matrix)
    &[(0x2070, 5, 1), (0x2074, 4, 1), (0x2078, 22, 1), (0x2078, FLOOR, 1), (0x2070, 6, 2), (0x2070, 8, 8), (0x2074, 16, 6)],
    // ST2 (9 with the camera pivot)
    &[(0x2048, FLOOR, 1), (0x2048, 5, 1), (0x2074, 6, 3), (0x2074, 4, 1), (0x2074, 9, 1)],
    // ST3
    &[(0x2048, FLOOR, 1), (0x2048, 4, 1), (0x2074, 5, REST)],
    // ST4 (4 turned)
    &[(0x203C, 8, 1), (0x203C, 0, 4), (0x2078, 5, 3), (0x2078, FLOOR, 1), (0x2074, 4, 1)],
    // ST5
    &[(0x2048, FLOOR, 1), (0x2048, 5, 1), (0x2074, 6, 1), (0x2074, 4, 1)],
    // ST6: 6, 8, 9 or 7, 8, 9 on alternate frames
    &[(0x2074, FLOOR, 1), (0x2074, 5, 1), (0x2074, 6, 4), (0x2074, 4, 1)],
    // ST7 (9 on even counts, 8 with the camera pivot)
    &[(0x2048, FLOOR, 1), (0x2048, 4, 1), (0x2078, 10, 1), (0x2078, 5, 3), (0x2074, 9, 1), (0x2078, 8, 1)],
    // ST8
    &[(0x2048, FLOOR, 1), (0x2074, 4, 4)],
    // ST9
    &[(0x2074, 4, REST), (0x2074, FLOOR, 1)],
    // STA, STB (the floor when W+4 is set: there is one)
    &[(0x2074, 4, REST), (0x2074, FLOOR, 1)],
    &[(0x2074, 4, REST), (0x2074, FLOOR, 1)],
    // STC
    &[(0x2074, 6, 3)],
];

/// The units the stage routine links into the ordering table under `view`, in link order: (OT slot = entry
/// offset / 4, object or `FLOOR`, matrix). `count` = the stage's object count (W+0xC); `empty(k)`: object k is an
/// empty entry of the object list (0xFF: obj+0xC = 1, skipped by the object drawer). `cam`: as `object_xf`.
pub fn sorts(stage: usize, view: &View, count: u8, empty: &dyn Fn(u8) -> bool, cam: [i16; 3], t: &Trig) -> Vec<(u16, u8, Mat4)> {
    let mut out = Vec::new();
    for &(ot, first, num) in ROUTINES.get(stage).copied().unwrap_or(&[]) {
        if first == FLOOR {
            out.push((ot / 4, FLOOR, Mat4::IDENTITY));
            continue;
        }
        let end = if num == REST { count } else { first.saturating_add(num).min(count) };
        for k in first..end {
            if empty(k) {
                continue;
            }
            let xf = if objects(stage).contains(&k) { object_xf(stage, k, view, cam, t) } else { Some(Mat4::IDENTITY) };
            if let Some(xf) = xf {
                out.push((ot / 4, k, xf));
            }
        }
    }
    out
}

/// The stage darkening's packets (linked whenever the level changes, also at the match
/// start): the CLUT block (256, 480, 96×16) restored from its copy below and covered by a semi-transparent TILE,
/// which writes every entry with the STP bit clear (VRAM dumps of ST9..STC). So the stage's textured polygons have
/// no STP texels and a CLUT entry 0x8000 is colour 0 (transparent).
pub fn clear_stp(vram: &mut Vram) {
    for y in 480..496 {
        for x in 256..352 {
            vram.words[y * 1024 + x] &= 0x7FFF;
        }
    }
}

/// The CLUTs (tpage words of the polygons) whose VRAM row the stage routine overwrites: ST3 (288, 480), ST5
/// (272, 480..483).
pub fn cluts(stage: usize) -> &'static [u16] {
    match stage {
        3 => &[0x7812],
        5 => &[0x7811, 0x7851, 0x7891, 0x78D1],
        _ => &[],
    }
}

/// VRAM position (x, y) of a CLUT word.
pub fn clut_xy(clut: u16) -> (u32, u32) {
    ((clut as u32 & 0x3F) * 16, (clut as u32 >> 6) & 0x1FF)
}

/// The VRAM row whose colours CLUT `clut` of `stage` shows on frame `n`, after the routine's copies of this and the
/// earlier frames of a match never reset or frozen (None: not animated; `View::Frame`).
fn clut_source(stage: usize, clut: u16, n: u64) -> Option<(u32, u32)> {
    match (stage, clut) {
        // W.26 += 0x14, k = (W.26 >> 8) & 0x1F; k 1..15 copy (0x120, 0x1E0 + k), 16..31
        // (0x130, 0x1E0 + k − 16) to (0x120, 0x1E0); k = 0 copies nothing (the file's row until the first copy)
        (3, 0x7812) => {
            let k = (ctr(n, 0x14) as u32 >> 8) & 0x1F;
            Some(match k {
                0 if n * 0x14 < 0x100 => (0x120, 0x1E0),
                0 => (0x130, 0x1EF),
                1..=15 => (0x120, 0x1E0 + k),
                _ => (0x130, 0x1E0 + k - 16),
            })
        }
        // W.28 += 0x14, k = (W.28 >> 8) & 7; k 0..3 copy row 0x1E4 (dark) to 0x1E0 + k, 4..7 row 0x1E5
        // (bright) to 0x1E0 + k − 4; a row not copied to yet (first cycle) keeps the file's colours
        (5, 0x7811 | 0x7851 | 0x7891 | 0x78D1) => {
            let r = (clut as u32 >> 6) - 0x1E0;
            let k = (ctr(n, 0x14) as u32 >> 8) & 7;
            Some(if k >= r + 4 {
                (0x110, 0x1E5)
            } else if k >= r {
                (0x110, 0x1E4)
            } else if n * 0x14 >= 0x800 {
                (0x110, 0x1E5)
            } else {
                (0x110, 0x1E0 + r)
            })
        }
        _ => None,
    }
}

/// `ok_br2_converter::vram::rgba`: a VRAM colour as the stage PNGs hold it (0 = transparent).
fn rgba(c: u16) -> [u8; 4] {
    if c == 0 {
        return [0; 4];
    }
    let ch = |v: u16| {
        let v = ((v & 31) << 3) as u8;
        v | v >> 5
    };
    [ch(c), ch(c >> 5), ch(c >> 10), 255]
}

/// The palette block (`PAL_X`, `PAL_Y`, `PAL_W`×`PAL_H`) of the stage's VRAM as RGBA.
pub fn palette(vram: &Vram) -> Vec<u8> {
    let mut out = Vec::with_capacity((4 * PAL_W * PAL_H) as usize);
    for y in PAL_Y..PAL_Y + PAL_H {
        for x in PAL_X..PAL_X + PAL_W {
            out.extend(rgba(vram.words[(y * 1024 + x) as usize]));
        }
    }
    out
}

/// A stage image (`<ST>_tp<tpage>_cl<clut>`, a 256×256 page) whose CLUT `stage` animates: the CLUT and the page's
/// 4-bit texel indices, when the page in `vram` with that CLUT gives the image's colours (else None: drawn as is).
pub fn paletted(stage: usize, img: &crate::formats::gltf_model::Image, vram: &Vram) -> Option<(u16, Vec<u8>)> {
    page_indices(img, vram, |c| cluts(stage).contains(&c))
}

/// `paletted` for an image whose CLUT `want` accepts (the floor tiles: any CLUT of the palette block).
pub fn page_indices(img: &crate::formats::gltf_model::Image, vram: &Vram, want: impl Fn(u16) -> bool) -> Option<(u16, Vec<u8>)> {
    let (rest, cl) = img.name.rsplit_once("_cl")?;
    let clut = u16::from_str_radix(cl, 16).ok().filter(|&c| want(c))?;
    let tpage = u16::from_str_radix(rest.rsplit_once("_tp")?.1, 16).ok()?;
    if tpage >> 7 & 3 != 0 || (img.width, img.height) != (256, 256) {
        return None; // 4-bit pages only (all the animated ones)
    }
    let (px, py) = ((tpage as usize & 0xF) * 64, ((tpage as usize >> 4) & 1) * 256);
    let (cx, cy) = clut_xy(clut);
    let mut index = Vec::with_capacity(256 * 256);
    for v in 0..256 {
        for u in 0..256 {
            let w = vram.words[(py + v) * 1024 + px + u / 4];
            let k = (w >> ((u & 3) * 4)) & 15;
            let c = rgba(vram.words[(cy * 1024 + cx) as usize + k as usize]);
            if img.rgba[4 * (v * 256 + u)..4 * (v * 256 + u) + 4] != c {
                log::warn!("stage image {} differs from its VRAM page: drawn as is (its CLUT row neither animated nor by depth)", img.name);
                return None;
            }
            index.push(k as u8);
        }
    }
    Some((clut, index))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn st5_far_wall_lights() {
        // ST5: row 3 at phases 2, 3, 4, 5, 7, 0 (after the first cycle) = bright, dark, dark, dark,
        // bright, bright
        let at = |k: u64| (0..).map(|i| 1000 + i).find(|&n| ((n * 0x14) >> 8) & 7 == k).unwrap();
        let want = [(2, 0x1E5), (3, 0x1E4), (4, 0x1E4), (5, 0x1E4), (7, 0x1E5), (0, 0x1E5)];
        for (k, row) in want {
            assert_eq!(clut_source(5, 0x78D1, at(k)), Some((0x110, row)), "phase {k}");
        }
        // the first cycle: rows not copied to yet keep their own colours
        assert_eq!(clut_source(5, 0x7891, 1), Some((0x110, 0x1E2)));
        assert_eq!(clut_source(5, 0x7811, 1), Some((0x110, 0x1E4)));
    }

    #[test]
    fn st3_sea_phases() {
        assert_eq!(clut_source(3, 0x7812, 1), Some((0x120, 0x1E0)));
        assert_eq!(clut_source(3, 0x7812, 13), Some((0x120, 0x1E1)));
        // k = 16 at W.26 = 0x1000 (n = 205): slot 35 row 0; k = 0 again after the cycle: the last copy (row 15)
        assert_eq!(clut_source(3, 0x7812, 205), Some((0x130, 0x1E0)));
        assert_eq!(clut_source(3, 0x7812, 410), Some((0x130, 0x1EF)));
    }

    #[test]
    fn counters() {
        // ST1 RAM at recorded frame 1300: W.26 / 28 / 2A = 0xAAF / 0x446 / 0x223 (n = 547)
        assert_eq!((ctr(547, 5), ctr(547, 2), ctr(547, 1)), (0xAAF, 0x446, 0x223));
        // ST7: W.26 0x30B2 → 0x30C9 → 0x30E0, the beacon drawn on even values
        assert_eq!(ctr(0x30B2 / 0x17, 0x17), 0x30B2);
        // ST4: W.28 0x1800 → 0x1C00 every 4 frames
        assert_eq!(((24u64 / 4) as u16).wrapping_mul(0x400), 0x1800);
    }

    /// Stepped from a match start with no reset and no skipped frame, the state shows what the frame count gave up
    /// to simulation version 5 (counters, objects and CLUT rows), for every stage over two ST3 cycles.
    #[test]
    fn stepped_state_matches_the_frame_count() {
        let t = Trig::parse(&[0; 0x1004]);
        for stage in 0..13 {
            let (mut c, mut rows) = (Counters::default(), Rows::default());
            for n in 1..=820u64 {
                step(stage, &mut c, &mut rows);
                let (old, new) = (View::Frame(n), View::State(c, rows));
                for &k in objects(stage) {
                    assert_eq!(object_xf(stage, k, &old, [7, 0, 9], &t), object_xf(stage, k, &new, [7, 0, 9], &t), "ST{stage} obj {k} n {n}");
                }
                for &clut in cluts(stage) {
                    assert_eq!(old.clut_source(stage, clut), new.clut_source(stage, clut), "ST{stage} clut {clut:#x} n {n}");
                }
            }
        }
    }

    /// The counters restart at a round start (ST6's toggle and the CLUT rows do not), and a frame the
    /// stage is not drawn is not counted.
    #[test]
    fn round_start_reset() {
        let mut rows = Rows::default();
        for (stage, want) in [(1, [0; 3]), (3, [0, 9, 9]), (4, [0, 0, 9]), (5, [0, 0, 9]), (6, [9; 3]), (7, [0, 9, 9]), (2, [9; 3])] {
            let mut c = Counters([9; 3]);
            c.reset(stage);
            assert_eq!(c.0, want, "ST{stage}");
        }
        let mut c = Counters::default();
        for _ in 0..30 {
            step(5, &mut c, &mut rows);
        }
        let kept = rows;
        c.reset(5);
        step(5, &mut c, &mut rows);
        assert_eq!(c.0[1], 0x14);
        // k = 0 copies the dark row into row 0 again; rows 1..3 keep the earlier copies
        assert_eq!((rows.0[0], &rows.0[1..]), (Some((0x110, 0x1E4)), &kept.0[1..]));
    }

    #[test]
    fn routine_links() {
        let t = Trig::parse(&[0; 0x1004]);
        let units = |stage: usize, n: u64, count: u8, empty: &dyn Fn(u8) -> bool| -> Vec<(u16, u8)> {
            sorts(stage, &View::Frame(n), count, empty, [0; 3], &t).into_iter().map(|(slot, u, _)| (slot, u)).collect()
        };
        // ST9 (9 objects): 4..8 then the floor, all at M.8 + 0x2074 (slot 0x81D; the recorded OT walk: floor drawn first)
        let st9: Vec<(u16, u8)> = (4..9).map(|k| (0x81D, k)).chain([(0x81D, FLOOR)]).collect();
        assert_eq!(units(9, 1, 9, &|_| false), st9);
        // STC: objects 6..8 only
        assert_eq!(units(12, 1, 9, &|_| false), vec![(0x81D, 6), (0x81D, 7), (0x81D, 8)]);
        // ST1: the empty entries of its object list (8, 10, …, 20) are skipped
        let st1 = units(1, 1, 23, &|k| (8..=20).contains(&k) && k % 2 == 0);
        assert_eq!(st1.iter().filter(|u| u.1 != FLOOR).count(), 23 - 4 - 7);
        assert!(st1.iter().all(|u| !(8..=20).contains(&u.1) || u.1 % 2 == 1));
        // ST6: object 6 on odd frames, 7 on even ones, then 8, 9
        assert_eq!(units(6, 1, 10, &|_| false), vec![(0x81D, FLOOR), (0x81D, 5), (0x81D, 6), (0x81D, 8), (0x81D, 9), (0x81D, 4)]);
        assert_eq!(units(6, 2, 10, &|_| false)[2], (0x81D, 7));
        // ST4: objects 8 and 0..3 at 0x203C (slot 0x80F, behind the fighter shadows' 0x810)
        assert_eq!(&units(4, 1, 9, &|_| false)[..5], &[(0x80F, 8), (0x80F, 0), (0x80F, 1), (0x80F, 2), (0x80F, 3)]);
    }

    #[test]
    fn stp_cleared_in_the_stage_cluts() {
        let mut v = Vram::default();
        v.words[482 * 1024 + 321] = 0xBDEF;
        v.words[480 * 1024 + 288] = 0x8000;
        v.words[480 * 1024 + 352] = 0x8421;
        clear_stp(&mut v);
        assert_eq!((v.words[482 * 1024 + 321], v.words[480 * 1024 + 288], v.words[480 * 1024 + 352]), (0x3DEF, 0, 0x8421));
    }

    #[test]
    fn colours_as_the_pngs() {
        assert_eq!(rgba(0), [0; 4]);
        assert_eq!(rgba(0x7FFF), [255, 255, 255, 255]);
        assert_eq!(rgba(0x8000), [0, 0, 0, 255]);
        assert_eq!(rgba(16), [132, 0, 0, 255]);
    }
}
