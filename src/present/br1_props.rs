//! An imported stage's props: the models its stage code places and moves (engine addition, the import S8).
//!
//! State: presentation (reads the state, never writes it)
//!
//! Eight prop slots of 0xE bytes (+0 the type, +2..+0xC the type's words), each with its matrix block
//! (0xF0 bytes: +0x14..+0x18 position words, then up to four parts of 0x3C, part j's MATRIX at +0x1C +
//! 0x3C·j with its translation at +0x30 + 0x3C·j) and a rotation vector (0x20 bytes). Every slot draws
//! the stage's prop TMD (entry 0x19, loaded into each slot), object j with part j's matrix. The round start
//! places them at the first round's start only (the stage's case); the fades' reset stops them at that
//! round's fight start and at every later round's start: types 3 and 5 go on, type 4 ends
//! with +0xA = 0x800, the rest end. So in a fight only ZIN's slider (type 3) and VAK's four wanderers (type 5) are
//! left; the others belong to the imported game's own intro camera, which the engine does not
//! run (the main game's camera and round intro stand in).
//!
//! The engine steps them once per new fight frame (none while the match's frame stands, as the stage step skips them under
//! the pause G.7E), from the round flow it sees: a new round's start places them and stops them from the
//! second round on, the first round's fight start stops them; a rewind or a jump places them anew. Their random
//! numbers come from presentation copies of the imported game's two byte-table streams, never the fight's.
//!
//! Not yet: types 1 / 2 (the intro's birds), 4 (MIT's gate, effect task 0x1C), 6 (GAD's tower), 7 (FOX's intro).

use crate::fight::fight_data::Br1Props;
use crate::fight::stage::br1::Bounds;
use glam::{Mat3, Mat4, Vec3};

/// The most frames `Props::advance` steps through at once (more: placed anew, as for a rewind).
const MAX_GAP: u64 = 4096;

/// A part's matrix: the rotation (4.12) and the translation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Part {
    pub r: [[i32; 3]; 3],
    pub t: [i32; 3],
}

const IDENTITY: [[i32; 3]; 3] = [[0x1000, 0, 0], [0, 0x1000, 0], [0, 0, 0x1000]];

impl Default for Part {
    fn default() -> Self {
        Self { r: IDENTITY, t: [0; 3] }
    }
}

/// One prop slot: its record (+0 the type, `w[k]` the word at +2 + 2k), its rotation vector and its parts.
#[derive(Clone, Copy, Debug, Default)]
struct Slot {
    ty: u16,
    w: [i16; 6],
    rot: [i16; 3],
    parts: [Part; 4],
}

/// The imported game's byte-table streams for the props (presentation copies): stream A (the static tables' byte table,
/// index) and stream B (a copy of that table shuffled at the fight's load by
/// the shuffle with a stream A byte, index).
struct Streams {
    table: [u8; 256],
    b_table: [u8; 256],
    a: u8,
    b: u8,
}

impl Streams {
    fn new(table: [u8; 256]) -> Self {
        let mut s = Self { table, b_table: table, a: 0, b: 0 };
        // each byte k swapped with the one the table names at seed + k
        let seed = s.next_a() as usize;
        for k in 0..256 {
            let j = s.table[(seed + k) & 0xFF] as usize;
            s.b_table.swap(k, j);
        }
        s
    }
    fn next_a(&mut self) -> i32 {
        let v = self.table[self.a as usize];
        self.a = self.a.wrapping_add(1);
        v as i32
    }
    fn next_b(&mut self) -> i32 {
        let v = self.b_table[self.b as usize];
        self.b = self.b.wrapping_add(1);
        v as i32
    }
}

/// An imported stage's props (`g94` its stage byte, `half` the half sizes x / z of its ring record, the
/// stage's `Bounds::edge`).
pub struct Props {
    g94: u8,
    half: [i32; 2],
    flap5: [i16; 4],
    sin: Vec<i16>,
    slots: [Slot; 8],
    rnd: Streams,
    /// the fight frame last stepped and the round flow seen then (round, state)
    last: Option<(u64, u8, u8)>,
}

impl Props {
    /// The props of imported stage `b` with its static tables `t`.
    pub fn new(t: &Br1Props, b: &Bounds) -> Self {
        Self {
            g94: b.g94,
            half: [b.edge.w1 as i16 as i32, (b.edge.w1 >> 16) as i16 as i32],
            flap5: t.flap5,
            sin: t.sin.clone(),
            slots: Default::default(),
            rnd: Streams::new(b.rand),
            last: None,
        }
    }

    /// The game's sine (its quarter table, which RotMatrixZ reads too), a turn of 0x1000.
    fn rsin(&self, a: i32) -> i32 {
        let mut v = (a & 0x3FF) as usize;
        if a & 0x400 != 0 {
            v = 0x400 - v;
        }
        let r = self.sin[v] as i32;
        if a & 0x800 != 0 { -r } else { r }
    }
    fn rcos(&self, a: i32) -> i32 {
        self.rsin(a + 0x400)
    }

    /// RotMatrix: Rx · Ry · Rz of the angles `v`.
    fn rot_matrix(&self, v: [i16; 3]) -> [[i32; 3]; 3] {
        let [x, y, z] = v.map(|a| a as i32);
        let (sx, cx, sy, cy, sz, cz) = (self.rsin(x), self.rcos(x), self.rsin(y), self.rcos(y), self.rsin(z), self.rcos(z));
        [
            [(cy * cz) >> 12, -((cy * sz) >> 12), sy],
            [((cx * sz) >> 12) + ((((sy * cz) >> 12) * sx) >> 12), ((cx * cz) >> 12) - ((((sy * sz) >> 12) * sx) >> 12), -((cy * sx) >> 12)],
            [((sx * sz) >> 12) - ((((sy * cz) >> 12) * cx) >> 12), ((sx * cz) >> 12) + ((((sy * sz) >> 12) * cx) >> 12), (cy * cx) >> 12],
        ]
    }

    /// RotMatrixZ (a fresh matrix).
    fn rot_z(&self, a: i32) -> [[i32; 3]; 3] {
        let (s, c) = (self.rsin(a), self.rcos(a));
        [[c, -s, 0], [s, c, 0], [0, 0, 0x1000]]
    }

    /// The stage's props at a round's start (round 0 only; every slot's type cleared first).
    fn init(&mut self, round: u8) {
        if round > 0 {
            return;
        }
        for s in &mut self.slots {
            s.ty = 0;
        }
        match self.g94 {
            // ZIN: slot 0 type 3 at (0, 0, 0x1B00), moving out (+0xA 1)
            2 => {
                let s = &mut self.slots[0];
                s.ty = 3;
                s.parts[0] = Part { r: IDENTITY, t: [0, 0, 0x1B00] };
                (s.w[4], s.w[5]) = (1, 0);
            }
            // VAK: slots 0..3 type 5 at x −0x400 + 0x100k, y a stream A byte − 0x80, z 0xC00, standing
            // (+0xA 0), speed −0x10, the rotation 0
            5 => {
                for k in 0..4 {
                    let y = self.rnd.next_a() - 0x80;
                    let s = &mut self.slots[k];
                    s.ty = 5;
                    s.parts[0].t = [-0x400 + 0x100 * k as i32, y, 0xC00];
                    (s.w[4], s.w[5]) = (0, -0x10);
                    s.rot = [0; 3];
                }
            }
            // MIT's type 4, GAD's type 6: intro-only (`Props`' doc), not implemented
            _ => {}
        }
    }

    /// Every slot ends but types 3 and 5 (type 4 with +0xA 0x800).
    fn stop(&mut self) {
        for s in &mut self.slots {
            match s.ty {
                3 | 5 => {}
                4 => (s.ty, s.w[4]) = (0, 0x800),
                _ => s.ty = 0,
            }
        }
    }

    /// Type 3 (ZIN's slider): +0xC a pause in frames; else +0xA 1 moves it out (z + 8, y − 8) up to z
    /// 0x1F80, then pauses 180 frames and turns back; +0xA 0 moves it in (z − 8, y + 8) down to z 0x1400, then pauses
    /// 120 frames and turns out again.
    fn step3(s: &mut Slot) {
        if s.w[5] != 0 {
            s.w[5] -= 1;
            return;
        }
        let t = &mut s.parts[0].t;
        if s.w[4] != 0 {
            t[2] += 8;
            t[1] -= 8;
            if t[2] >= 0x1F80 {
                (s.w[5], s.w[4]) = (0xB4, 0);
            }
        } else {
            t[2] -= 8;
            t[1] += 8;
            if t[2] <= 0x1400 {
                (s.w[5], s.w[4]) = (0x78, 1);
            }
        }
    }

    /// Type 5 (VAK's wanderers): a stream B byte turns the pitch by ±0x10 (within ±0x100), the heading by
    /// ±0x40 and the speed +0xC by ±1 (within −0x20..0); the body's matrix RotMatrix(rotation), its step the matrix
    /// times (0, 0, speed) into +2..+6 and added to the position; inside the ring's half sizes + 0x100 it is put on
    /// the nearer edge, else kept within ±0xC00 (a clamp draws a stream B byte); its height within −0x100..0 (a clamp
    /// levels the pitch); the legs' angle from the leg table by +0xA (+1 a frame, & 3), parts 1 / 2 the body's
    /// matrix times Rz(±angle) at the body's point.
    fn step5(&mut self, k: usize) {
        let r = self.rnd.next_b();
        let mut s = self.slots[k];
        s.rot[0] = (s.rot[0] as i32 + if r & 4 != 0 { 0x10 } else { -0x10 }).clamp(-0x100, 0x100) as i16;
        s.rot[1] = s.rot[1].wrapping_add(if r & 1 != 0 { 0x40 } else { -0x40 });
        s.w[5] = (s.w[5] as i32 + if r & 2 != 0 { 1 } else { -1 }).clamp(-0x20, 0) as i16;
        let m = self.rot_matrix(s.rot);
        // ApplyMatrixSV of (0, 0, speed)
        let v = [0, 1, 2].map(|i| ((m[i][2] * s.w[5] as i32) >> 12).clamp(-0x8000, 0x7FFF) as i16);
        s.w[..3].copy_from_slice(&v);
        let t = &mut s.parts[0].t;
        for i in 0..3 {
            t[i] += v[i] as i32;
        }
        let (hx, hz) = (self.half[0] + 0x100, self.half[1] + 0x100);
        if t[0] < hx && -hx < t[0] && t[2] < hz && -hz < t[2] {
            if t[0].abs() >= t[2].abs() {
                t[0] = if t[0] < 0 { -hx } else { hx };
            } else {
                t[2] = if t[2] < 0 { -hz } else { hz };
            }
        } else {
            for i in [0, 2] {
                if t[i].abs() > 0xC00 {
                    t[i] = t[i].clamp(-0xC00, 0xC00);
                    self.rnd.next_b();
                }
            }
        }
        if t[1] < -0x100 {
            t[1] = -0x100;
            s.rot[0] = 0;
        } else if t[1] > 0 {
            t[1] = 0;
            s.rot[0] = 0;
        }
        let w = self.flap5[s.w[4] as usize & 3] as i32;
        s.w[4] = (s.w[4] + 1) & 3;
        let t = s.parts[0].t;
        s.parts[0].r = m;
        // RotTrans of the parts' offsets (+0x50 / +0x8C, never written: 0) and MulRotMatrix0
        s.parts[1] = Part { r: mul(&m, &self.rot_z(w)), t };
        s.parts[2] = Part { r: mul(&m, &self.rot_z(-w)), t };
        self.slots[k] = s;
    }

    /// Steps the props to fight frame `frame` in round `round` at round state `state` (`Props`' doc).
    pub fn advance(&mut self, frame: u64, round: u8, state: u8) {
        // frames forward are stepped one by one (sparse drawing, a stall, fast forward: up to `MAX_GAP`); the round flow
        // between is only known at both ends, its changes taken at the gap's first frame
        match self.last {
            Some((f, r, st)) if f <= frame && frame - f <= MAX_GAP && r == round => {
                // the first round's fight start
                if round == 0 && st < 4 && state >= 4 && frame > f {
                    self.stop();
                }
                for _ in f..frame {
                    self.step();
                }
            }
            Some((f, _, _)) if f <= frame && frame - f <= MAX_GAP => {
                // a new round's start (from the second round on the round start stops them)
                self.init(round);
                if round > 0 {
                    self.stop();
                }
                for _ in f..frame {
                    self.step();
                }
            }
            _ => {
                // a rewind or a jump: placed as the match's first round placed them, the streams from their start
                self.rnd = Streams::new(self.rnd.table);
                self.init(0);
                if round > 0 || state >= 4 {
                    self.stop();
                }
            }
        }
        self.last = Some((frame, round, state));
    }

    /// The stage step's slot loop: a frame of each slot's type.
    fn step(&mut self) {
        for k in 0..8 {
            match self.slots[k].ty {
                3 => Self::step3(&mut self.slots[k]),
                5 => self.step5(k),
                _ => {}
            }
        }
    }

    /// The parts to draw: (object j of the prop TMD, its matrix); type 5 only beyond the camera's distance from its
    /// pivot (the body's ground distance from G.B8 +0x28 / +0x2C against +0x3C; `pivot` / `dist` the
    /// main game's camera's).
    pub fn parts(&self, objects: usize, pivot: [i32; 3], dist: i32) -> Vec<(usize, Part)> {
        let mut out = Vec::new();
        for s in self.slots.iter().filter(|s| matches!(s.ty, 3 | 5)) {
            if s.ty == 5 {
                let t = s.parts[0].t;
                let (dx, dz) = ((pivot[0] - t[0]) as i64, (pivot[2] - t[2]) as i64);
                if (((dx * dx + dz * dz) as f64).sqrt() as i64) < dist as i64 {
                    continue;
                }
            }
            let n = if s.ty == 5 { 3 } else { 1 };
            out.extend((0..n.min(objects)).map(|j| (j, s.parts[j])));
        }
        out
    }
}

/// MulRotMatrix0: `m · a`, each element `>> 12`.
fn mul(m: &[[i32; 3]; 3], a: &[[i32; 3]; 3]) -> [[i32; 3]; 3] {
    std::array::from_fn(|i| std::array::from_fn(|j| (0..3).map(|k| m[i][k] * a[k][j]).sum::<i32>() >> 12))
}

/// A part's model matrix for the engine's scene: the stage's glTF axes (x, −y, −z) and its scale (game units / 300; the
/// mesh already in them by `stage_xf`).
pub fn model_matrix(p: &Part, stage_xf: Mat4) -> Mat4 {
    let c = [1.0f32, -1.0, -1.0];
    let r = Mat3::from_cols_array_2d(&std::array::from_fn(|col| std::array::from_fn(|row| c[row] * c[col] * p.r[row][col] as f32 / 4096.0)));
    let t = Vec3::new(p.t[0] as f32, -p.t[1] as f32, -p.t[2] as f32) / 300.0;
    Mat4::from_translation(t) * Mat4::from_mat3(r) * stage_xf
}

#[cfg(test)]
mod tests {
    use super::*;

    fn props(g94: u8) -> Props {
        // a sine table with sin(0x400) = 0x1000 and a linear ramp, the random table 0..255, half sizes 0x5C0
        let t = Br1Props { sin: (0..=0x400).map(|k| (k * 4) as i16).collect(), flap5: [0; 4], objects: Vec::new() };
        let b = Bounds {
            g94,
            hips: Default::default(),
            edge: crate::fight::br1::physics::Ring { w0: 0, w1: 0x05C0_05C0 },
            model: [0; 4],
            hp: [0; 4],
            pieces: 0,
            shadow: Default::default(),
            rand: std::array::from_fn(|k| k as u8),
        };
        Props::new(&t, &b)
    }

    /// ZIN's slider: out from z 0x1B00 by 8 a frame (y up by 8) to 0x1F80, 180 frames still, back to 0x1400, 120
    /// frames still, out again; it goes on after the fight start and in later rounds.
    #[test]
    fn zin_slider() {
        let mut p = props(2);
        p.advance(100, 0, 0);
        assert_eq!((p.slots[0].ty, p.slots[0].parts[0].t), (3, [0, 0, 0x1B00]));
        // out: (0x1F80 − 0x1B00) / 8 frames, past the fight start
        let mut f = 100;
        for _ in 0..(0x1F80 - 0x1B00) / 8 {
            f += 1;
            p.advance(f, 0, if f < 150 { 0 } else { 4 });
        }
        assert_eq!((p.slots[0].parts[0].t, p.slots[0].w[4], p.slots[0].w[5]), ([0, 0x1B00 - 0x1F80, 0x1F80], 0, 0xB4));
        for _ in 0..180 {
            f += 1;
            p.advance(f, 0, 4);
        }
        assert_eq!(p.slots[0].parts[0].t[2], 0x1F80);
        f += 1;
        p.advance(f, 0, 4);
        assert_eq!(p.slots[0].parts[0].t[2], 0x1F78);
        // the next round keeps it
        f += 1;
        p.advance(f, 1, 0);
        assert_eq!((p.slots[0].ty, p.slots[0].parts[0].t[2]), (3, 0x1F70));
        assert_eq!(p.parts(1, [0; 3], 0).len(), 1);
    }

    /// VAK's wanderers: four slots from x −0x400 step 0x100 at z 0xC00, kept outside the ring and within ±0xC00, their
    /// height within −0x100..0; three parts each, drawn only beyond the camera's distance.
    #[test]
    fn vak_wanderers() {
        let mut p = props(5);
        p.advance(10, 0, 0);
        let xs: Vec<i32> = p.slots[..4].iter().map(|s| s.parts[0].t[0]).collect();
        assert_eq!(xs, [-0x400, -0x300, -0x200, -0x100]);
        assert!(p.slots[..4].iter().all(|s| s.ty == 5 && s.parts[0].t[2] == 0xC00));
        for f in 11..2000 {
            p.advance(f, 0, if f < 50 { 0 } else { 4 });
            for s in &p.slots[..4] {
                let t = s.parts[0].t;
                assert!(t[0].abs() <= 0xC00 && t[2].abs() <= 0xC00 && (-0x100..=0).contains(&t[1]), "{f}: {t:?}");
                assert!(t[0].abs() >= 0x6C0 || t[2].abs() >= 0x6C0, "{f}: {t:?} inside the ring");
                assert!((-0x20..=0).contains(&s.w[5]));
            }
        }
        assert_eq!(p.parts(3, [0; 3], 0).len(), 12);
        assert!(p.parts(3, [0; 3], 0x7FFF).is_empty());
    }

    /// A gap forward (sparse drawing, fast forward) is stepped frame by frame: the same props as drawing every frame,
    /// across the fight start; a rewind places them anew.
    #[test]
    fn gaps_are_stepped() {
        for g94 in [2, 5] {
            let (mut every, mut sparse) = (props(g94), props(g94));
            for f in 100..=900 {
                every.advance(f, 0, if f < 300 { 0 } else { 4 });
            }
            sparse.advance(100, 0, 0);
            sparse.advance(900, 0, 4);
            let pos = |p: &Props| p.slots.map(|s| (s.ty, s.parts[0].t));
            assert_eq!(pos(&sparse), pos(&every), "stage {g94}");
            sparse.advance(500, 0, 4);
            assert_eq!(pos(&sparse), pos(&props_placed(g94)), "stage {g94}: a rewind");
        }
    }

    /// The props as the first round places them.
    fn props_placed(g94: u8) -> Props {
        let mut p = props(g94);
        p.advance(0, 0, 4);
        p
    }
}
