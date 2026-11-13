//! The fight camera: its modes and the view it outputs.
//!
//! State: serialised: part of `GameState` (field changes are `STATE_VERSION` changes)
//!
//! The fight camera (its struct is serialised). Modes implemented: MOVE (3, the normal
//! fight camera: snap / follow / zoom_fit2), the scripted MODE0..2, WIN_1P/2P
//! (4/5), LOSE_1P/2P (6/7), THROW_1P/2P (8/9), REPLAY (0xA/0xB), the tracking shot
//! (0xC/0xD), the special-KO shots (0xE..0x11), the ranking turntable (0x1E) and the TV camera (0x20);
//! other modes keep the last state.
//!
//! Output: view = Rx(pitch)·Ry(−yaw)·(v − POS) + ZOM, projected with H = 360 on a logical
//! 320×240 screen (the screen-scale matrix makes the image resolution independent).
//! Game space: x right, y down, z forward; angles in 4096 units.

use crate::fight::physics::Trig;
use serde::{Deserialize, Serialize};

pub const MOVE: i16 = 3;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Camera {
    pub ang: [i16; 3],   // +0x28 pitch, yaw, roll
    pub angt: [i16; 3],  // +0x30
    pub ang_speed: i16,  // +0x38
    pub pos: [i16; 3],   // +0x4A pivot
    pub post: [i16; 3],  // +0x52
    pub pos_speed: i16,  // +0x5A
    pub zom: [i16; 3],   // +0x5C view-space offset (z = distance)
    pub zomt: [i16; 3],  // +0x64
    pub zom_speed: i16,  // +0x6C
    pub mode: i16,       // +0x76 (bit 0x80 = snap once, modes 0..2)
    pub timer: i16,      // +0x78
    pub snap: i16,       // +0x82 (<0: snap on the next MOVE update; −1 = P1 left, −2 = other side, −3 = keep side)
    pub shake: [i16; 2], // +0x86 horizontal, +0x84 vertical
    pub h: i16,          // projection distance
    pub win_frame: i16,  // +0x8C
    pub win_base: i16,
    pub win_side: bool,
    /// REPLAY orbit: radius (+0x72), direction (+0x70), angle (OFS)
    pub rp_r: i32,
    pub rp_dir: bool,
    pub rp_ofs: i32,
    /// stage number (M.98) for the REPLAY shot 2 pitch
    pub stage: u8,
    /// +0x3A look-at point (modes 1/2), +0x42 joint midpoint / POS origin, +0x80/+0x81 joints JT0 (F1) / JT1 (F2)
    pub look: [i16; 3],
    pub org: [i16; 3],
    pub jt: [i8; 2],
    /// +0x7E: the TV camera's preset (mode 0x20)
    pub preset: i16,
}

impl Default for Camera {
    fn default() -> Self {
        Self {
            ang: [0; 3],
            angt: [0; 3],
            ang_speed: 0,
            pos: [-1792, -768, -1792],
            post: [0; 3],
            pos_speed: 0,
            zom: [0; 3],
            zomt: [0; 3],
            zom_speed: 0,
            mode: MOVE,
            timer: 0,
            snap: -1,
            shake: [0; 2],
            h: 360,
            win_frame: 0,
            win_base: 0,
            win_side: false,
            rp_r: 0,
            rp_dir: false,
            rp_ofs: 0,
            stage: 0,
            look: [0; 3],
            org: [0; 3],
            jt: [4, 4],
            preset: 0,
        }
    }
}

/// What the camera reads from a fighter: root, chest (part 3) and pelvis (part 4) positions, yaw, flags.
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub struct Subject {
    pub root: [i32; 3],
    pub part3: [i32; 3],
    pub part4: [i32; 3],
    pub yaw: i16,
    pub yaw_c6: i16,
    pub f5c: u32,
    pub side_flag: bool, // state +0x100 (mirror)
    /// +0x74: angle to the opponent relative to the yaw
    pub o74: i16,
    /// skeleton node world positions (joint cameras, MODEL+0x34+j*0x50)
    pub bones: [[i32; 3]; 20],
    /// byte 0 bit 0x80: knocked out
    pub ko: bool,
    /// F68 (bit 0x2000: blown away, the special-KO tracking shot)
    pub f68: u32,
}

impl Subject {
    /// What the camera reads from fighter `f` (its block and model: the root, parts 3 / 4, the skeleton nodes).
    pub fn of(f: &crate::fight::fighter::Fighter) -> Self {
        use crate::fight::fighter::{F5C, F68};
        Subject {
            root: f.m_t,
            part3: f.chest,
            part4: f.hips,
            yaw: f.yaw,
            yaw_c6: f.o_c6,
            f5c: f.flags[F5C],
            side_flag: f.anim.mirror,
            o74: f.o_74,
            ko: f.o_00 & 0x80 != 0,
            f68: f.flags[F68],
            bones: std::array::from_fn(|k| f.parts.get(k).copied().unwrap_or(f.m_t)),
        }
    }
}

fn wrap12(v: i32) -> i32 {
    (v << 20) >> 20
}

fn isqrt(v: i64) -> i32 {
    if v <= 0 { 0 } else { (v as f64).sqrt() as i32 }
}

/// Move `p` toward `t` by at most `st`; returns the step applied.
fn step_s16(p: &mut i16, t: i16, st: i32) -> i32 {
    let d = t as i32 - *p as i32;
    if d == 0 {
        return 0;
    }
    let st = st.max(1);
    let s = if d.abs() > st { d.signum() * st } else { d };
    *p = (*p as i32 + s) as i16;
    s
}

/// The same on the 4096-unit circle (result masked to 0..4095).
fn step_angle(p: &mut i16, t: i16, st: i32) -> i32 {
    let d = wrap12(t as i32 - *p as i32);
    if d == 0 {
        return 0;
    }
    let st = st.max(1);
    let s = if d.abs() > st { d.signum() * st } else { d };
    *p = ((*p as i32 + s) & 0xFFF) as i16;
    s
}

/// Rx(a)·Ry(b) with the game's sine table, 4.12 fixed point.
fn rot_xy(t: &Trig, a: i32, b: i32) -> [[i32; 3]; 3] {
    let (sa, ca, sb, cb) = (t.sin(a), t.cos(a), t.sin(b), t.cos(b));
    [[cb, 0, sb], [(sa * sb) >> 12, ca, -((sa * cb) >> 12)], [-((sb * ca) >> 12), sa, (cb * ca) >> 12]]
}

fn apply(m: &[[i32; 3]; 3], v: [i32; 3]) -> [i32; 3] {
    [0, 1, 2].map(|i| (m[i][0] * v[0] + m[i][1] * v[1] + m[i][2] * v[2]) >> 12)
}

impl Camera {
    pub fn set_mode(&mut self, mode: i16) {
        self.mode = mode;
        self.timer = 0;
    }

    /// The point a fighter is followed by (x/z from the pelvis while airborne, y always the pelvis).
    fn sample(s: &Subject) -> [i32; 3] {
        let xz = if s.f5c & 0x4_0000 != 0 { s.part4 } else { s.root };
        [xz[0], s.part4[1], xz[2]]
    }

    /// Distance that fits both points on the 320×240 screen (→ zomt.z).
    fn zoom_fit2(&mut self, t: &Trig, p1: [i32; 3], p2: [i32; 3]) {
        self.zoom_fit_n(t, &[p1, p2]);
    }

    /// `zoom_fit2` over any number of points (engine addition, the team battle; two points give `zoom_fit2`'s values):
    /// the largest |x| and |y| and the nearest z in view space. No points: `zomt` is kept.
    pub(crate) fn zoom_fit_n(&mut self, t: &Trig, ps: &[[i32; 3]]) {
        if ps.is_empty() {
            return;
        }
        let r = rot_xy(t, self.ang[0] as i32, -(self.ang[1] as i32));
        let rel = |p: [i32; 3]| [0, 1, 2].map(|k| (p[k] as i16).wrapping_sub(self.pos[k]) as i32);
        let (mut ax, mut ay, mut near) = (0, 0, i32::MAX);
        for &p in ps {
            let v = apply(&r, rel(p));
            ax = ax.max(v[0].abs());
            ay = ay.max(v[1].abs());
            near = near.min(v[2]);
        }
        let zx = ((2 * ax + 0x180) * 360) / 320;
        let zy = ((2 * ay + 0x180) * 360) / 240;
        let z = zx.max(zy).max(0x4A4);
        self.zomt[2] = (z - near) as i16;
    }

    fn pivot_target(a: [i32; 3], b: [i32; 3], s1: &Subject, s2: &Subject) -> [i16; 3] {
        let mut p = [(a[0] + b[0]) / 2, (a[1] + b[1]) / 2, (a[2] + b[2]) / 2];
        if p[1] >= -255 && s1.f5c & 0x8_0000 == 0 && s2.f5c & 0x8_0000 == 0 {
            p[1] = -256;
        }
        p.map(|v| v as i16)
    }

    fn move_snap(&mut self, t: &Trig, s1: &Subject, s2: &Subject) {
        let (a, b) = (Self::sample(s1), Self::sample(s2));
        let ca = [a[0], a[1].min(-128), a[2]];
        let cb = [b[0], b[1].min(-128), b[2]];
        self.move_snap_on(t, a, b, Self::pivot_target(a, b, s1, s2), &[ca, cb], i16::MAX);
    }

    /// `move_snap` with the yaw from the points `a` → `b`, the pivot `post` and the zoom fitting `fit`, its distance at
    /// most `zmax` (the team battle's, `update_team`; a match passes its pair's and no limit).
    fn move_snap_on(&mut self, t: &Trig, a: [i32; 3], b: [i32; 3], post: [i16; 3], fit: &[[i32; 3]], zmax: i16) {
        self.pos = post;
        let (dx, dz) = (b[0] - a[0], b[2] - a[2]);
        if isqrt(dx as i64 * dx as i64 + dz as i64 * dz as i64) > 0x80 {
            let mut s = -t.atan2(dx, dz);
            match self.snap {
                -1 => {}
                -2 => s += 0x800,
                -3 => {
                    if wrap12(self.ang[1] as i32 - s).abs() > 0x400 {
                        s += 0x800;
                    }
                    s &= 0xFFF;
                }
                _ => {}
            }
            self.ang[1] = s as i16;
            self.angt[1] = s as i16;
            self.snap = 0;
        }
        self.ang[0] = 0;
        self.ang[2] = 0;
        self.zoom_fit_n(t, fit);
        self.zomt[2] = self.zomt[2].min(zmax);
        self.zom = self.zomt;
    }

    fn move_follow(&mut self, t: &Trig, s1: &Subject, s2: &Subject) {
        let (a, b) = (Self::sample(s1), Self::sample(s2));
        let ca = [a[0], a[1].min(-128), a[2]];
        let cb = [b[0], b[1].min(-128), b[2]];
        self.move_follow_on(t, a, b, Self::pivot_target(a, b, s1, s2), &[ca, cb], i16::MAX);
    }

    /// `move_follow` with the yaw from `a` → `b`, the pivot target `post` and the zoom fitting `fit` (at most `zmax`),
    /// as `move_snap_on`.
    fn move_follow_on(&mut self, t: &Trig, a: [i32; 3], b: [i32; 3], post: [i16; 3], fit: &[[i32; 3]], zmax: i16) {
        self.post = post;
        // pivot ≈ 1/16 of the way per frame (VectorNormal / SquareRoot0 form)
        let d = [0, 1, 2].map(|k| self.post[k] as i32 - self.pos[k] as i32);
        let l2 = d.iter().map(|&v| v as i64 * v as i64).sum::<i64>();
        if l2 > 0 {
            let l = isqrt(l2);
            if l > 0 {
                for k in 0..3 {
                    let n = (d[k] << 12) / l;
                    self.pos[k] = self.pos[k].wrapping_add(((n * l) >> 16) as i16);
                }
            }
        }
        // yaw: perpendicular to the fighters' line, never more than 90° away (no flip when they cross)
        let (dx, dz) = (b[0] - a[0], b[2] - a[2]);
        if isqrt(dx as i64 * dx as i64 + dz as i64 * dz as i64) > 0x80 {
            let mut s = -t.atan2(dx, dz);
            if wrap12(self.ang[1] as i32 - s).abs() > 0x400 {
                s += 0x800;
            }
            self.angt[1] = (s & 0xFFF) as i16;
            let st = (wrap12(self.ang[1] as i32 - self.angt[1] as i32).abs() >> 6).max(1);
            step_angle(&mut self.ang[1], self.angt[1], st);
        }
        // pitch eases to 0x5B (≈ 8° down)
        self.angt[0] = 0x5B;
        let st = ((self.ang[0] as i32 - 0x5B).abs() >> 5).max(1);
        step_angle(&mut self.ang[0], 0x5B, st);
        self.angt[2] = 0;
        self.ang[2] = 0;
        // distance: out by 1/4, in by 1/16 of the gap
        self.zoom_fit_n(t, fit);
        self.zomt[2] = self.zomt[2].min(zmax);
        let d = self.zom[2] as i32 - self.zomt[2] as i32;
        let st = if self.zom[2] < self.zomt[2] { (d.abs() << 14) >> 16 } else { (d.abs() << 12) >> 16 };
        step_s16(&mut self.zom[2], self.zomt[2], st);
    }

    /// lookat: (pitch, yaw) that put `pt` straight ahead of POS (not masked, as the game does).
    fn lookat(&self, t: &Trig, pt: [i32; 3]) -> (i16, i16) {
        let d = [0, 1, 2].map(|k| pt[k] - self.pos[k] as i32);
        let h = isqrt(d[0] as i64 * d[0] as i64 + d[2] as i64 * d[2] as i64);
        (t.atan2(h, d[1]) as i16, t.atan2(d[2], d[0]) as i16)
    }

    fn yclamp(p: [i32; 3], s: &Subject) -> [i32; 3] {
        if s.f5c & 0x8_0000 == 0 && p[1] >= -255 { [p[0], -256, p[2]] } else { p }
    }

    /// REPLAY_1P / 2P: four random shots of the final KO replay; `w` = the winner.
    fn replay(&mut self, t: &Trig, w: &Subject, o: &Subject, s: [&Subject; 2], r: i32) {
        // the game's two RNGs; here three successive values from the frame's random
        let mut seed = r as u32 ^ 0x9E37_79B9;
        let mut rnd = || crate::base::libc_rand::rand(&mut seed);
        let mid = |a: [i32; 3], b: [i32; 3]| [(a[0] + b[0]) / 2, ((a[1] + b[1]) / 2).min(-256), (a[2] + b[2]) / 2];
        if self.timer == 0 {
            self.h = 360;
            match rnd() & 3 {
                0 => {
                    let rp = &t.cam.random;
                    self.pos = [rp[(rnd() & 7) as usize][0], rp[(rnd() & 7) as usize][1], rp[(rnd() & 7) as usize][2]];
                    let m = mid(s[0].part4, s[1].part4);
                    self.post = m.map(|v| v as i16);
                    let (p, y) = self.lookat(t, m);
                    self.ang = [p, y, 0];
                    self.angt = self.ang;
                    self.zoom_fit2(t, [s[0].part4[0], s[0].part4[1].min(-256), s[0].part4[2]], [s[1].part4[0], s[1].part4[1].min(-256), s[1].part4[2]]);
                    self.zom = [0, 0, self.zomt[2]];
                    self.timer = 1;
                }
                1 => {
                    self.post = [w.part4[0] as i16, t.cam.heights[(rnd() & 3) as usize], w.part4[2] as i16];
                    self.rp_r = if rnd() & 1 != 0 { 768 } else { 1024 };
                    self.rp_dir = rnd() & 1 != 0;
                    self.rp_ofs = (rnd() & 1) << 4;
                    self.zom = [0; 3];
                    self.timer = 2;
                }
                2 => {
                    let sgn = if rnd() & 1 != 0 { 0x180 } else { -0x180 };
                    self.ang[1] = ((0x800 - w.yaw as i32 + sgn) & 0xFFF) as i16;
                    self.ang[0] = if self.stage == 6 { 0 } else { -0x100 };
                    self.ang[2] = 0;
                    self.zom = [0, 0, 0x200];
                    self.timer = 3;
                }
                _ => {
                    let a = o.yaw as i32 + o.o74 as i32 + 0x400 + if rnd() & 1 != 0 { 0x400 } else { 0xC00 };
                    let eye = [o.part4[0] + (t.cos(a) * 768) / 4096, -256, o.part4[2] + (t.sin(a) * 768) / 4096];
                    self.pos = eye.map(|v| v as i16);
                    let look = [w.part4[0], -256, w.part4[2]];
                    let (p, y) = self.lookat(t, look);
                    self.ang = [p, y, 0];
                    self.angt = self.ang;
                    self.zom = [0; 3];
                    self.timer = 4;
                }
            }
            return;
        }
        match self.timer {
            1 => {
                // look target ≈ 1/16 toward the midpoint, angles at 0x40 per frame, zoom fit (out /4, in /16)
                let m = mid(Self::yclamp(s[0].part4, s[0]), Self::yclamp(s[1].part4, s[1]));
                for k in 0..3 {
                    let d = m[k] - self.post[k] as i32;
                    self.post[k] = (self.post[k] as i32 + if d / 16 != 0 { d / 16 } else { d.signum() }) as i16;
                }
                let (p, y) = self.lookat(t, self.post.map(|v| v as i32));
                self.angt = [p, y, 0];
                step_angle(&mut self.ang[0], self.angt[0], 0x40);
                step_angle(&mut self.ang[1], self.angt[1], 0x40);
                self.zoom_fit2(t, s[0].part4, s[1].part4);
                let d = self.zom[2] as i32 - self.zomt[2] as i32;
                let st = if self.zom[2] < self.zomt[2] { d.abs() / 4 } else { d.abs() / 16 };
                step_s16(&mut self.zom[2], self.zomt[2], st);
            }
            2 => {
                self.rp_ofs += if self.rp_dir { -8 } else { 8 };
                let a = self.rp_ofs;
                self.pos =
                    [(self.post[0] as i32 + (t.cos(a) * self.rp_r) / 4096) as i16, self.post[1], (self.post[2] as i32 + (t.sin(a) * self.rp_r) / 4096) as i16];
                let (p, y) = self.lookat(t, Self::yclamp(w.part4, w));
                self.ang = [p, y, 0];
                self.angt = self.ang;
            }
            3 => self.pos = Self::yclamp(w.part4, w).map(|v| v as i16),
            _ => {
                let target = if o.ko { o.part4 } else { w.part4 };
                let (p, y) = self.lookat(t, target);
                self.angt = [p, y, 0];
                step_angle(&mut self.ang[0], self.angt[0], 0x40);
                step_angle(&mut self.ang[1], self.angt[1], 0x40);
            }
        }
    }

    /// The tracking shot of a blown-away loser (special-KO modes 0xF / 0x11): the pivot catches up with
    /// the pelvis midpoint every frame, the yaw keeps ≈ 0x380 off the perpendicular of the fighters' line.
    fn track(&mut self, t: &Trig, s1: &Subject, s2: &Subject) {
        let (a, b) = (s1.part4, s2.part4);
        self.post = [(a[0] + b[0]) / 2, (a[1] + b[1]) / 2, (a[2] + b[2]) / 2].map(|v| v as i16);
        if self.post[1] >= -0xFF {
            self.post[1] = -0x100;
        }
        // pivot: VectorNormal · SquareRoot0 with >> 12, i.e. the whole gap
        let d = [0, 1, 2].map(|k| self.post[k] as i32 - self.pos[k] as i32);
        let l = isqrt(d.iter().map(|&v| v as i64 * v as i64).sum::<i64>());
        if l > 0 {
            for k in 0..3 {
                let n = (d[k] << 12) / l;
                self.pos[k] = self.pos[k].wrapping_add(((n * l) >> 12) as i16);
            }
        }
        let (dx, dz) = (b[0] - a[0], b[2] - a[2]);
        let dist = isqrt(dx as i64 * dx as i64 + dz as i64 * dz as i64);
        let mut dev = dz; // the game's quirk, kept: the register still holds dz when the fighters are close
        if dist > 0x80 {
            let mut s = -t.atan2(dx, dz);
            dev = wrap12(self.ang[1] as i32 - s).abs();
            if dev > 0x400 {
                s += 0x800;
            }
            let base = s & 0xFFF;
            let q = ((self.ang[1] as i32 - base) >> 10) & 3;
            self.angt[1] = (base + t.cam.track[q as usize] as i32) as i16;
            let st = (wrap12(self.ang[1] as i32 - self.angt[1] as i32).abs() / 8).max(1);
            step_angle(&mut self.ang[1], self.angt[1], st);
            if dev > 0x400 {
                dev = 0x800 - dev;
            }
        }
        // pitch: more downward the closer and the more side-on
        let p = ((0x400 - (dist + 3 * dev)) * 45 / 256).min(0);
        self.angt[0] = -t.atan2(0x5A0, p) as i16;
        let st = (wrap12(self.ang[0] as i32 - self.angt[0] as i32).abs() / 8).max(1);
        step_angle(&mut self.ang[0], self.angt[0], st);
        self.angt[2] = 0;
        self.ang[2] = 0;
        // distance: out by 1/4, in by 1/16 of the gap
        self.zoom_fit2(t, [a[0], a[1].min(-0x80), a[2]], [b[0], b[1].min(-0x80), b[2]]);
        let d = self.zom[2] as i32 - self.zomt[2] as i32;
        let st = if self.zom[2] < self.zomt[2] { (d.abs() << 14) >> 16 } else { (d.abs() << 12) >> 16 };
        step_s16(&mut self.zom[2], self.zomt[2], st);
    }

    /// Mode 0x1E, the attract ranking: orbit (0, −0x100, 0) at 0x200, pitch 0; the ranking turns
    /// `angt.y` and the yaw follows by 1/32 of the gap.
    pub fn turntable(&mut self) {
        if self.timer == 0 {
            self.pos = [0, -0x100, 0];
            self.ang[0] = 0;
            self.ang[2] = 0;
            self.zom = [0, 0, 0x200];
            self.angt[1] = self.ang[1];
            self.timer = 1;
            return;
        }
        let st = (wrap12(self.ang[1] as i32 - self.angt[1] as i32).abs() >> 5).max(1);
        step_angle(&mut self.ang[1], self.angt[1], st);
    }

    /// Mode 0x20: the TV camera of the attract demo and WATCH — from preset PRESET[`preset`], looking
    /// at a point that follows the pelvis midpoint by 1/16, zoomed to fit both fighters.
    fn tv(&mut self, t: &Trig, s: [&Subject; 2]) {
        let (a, b) = (Self::yclamp(s[0].part4, s[0]), Self::yclamp(s[1].part4, s[1]));
        let mid = [0, 1, 2].map(|k| ((a[k] + b[k]) / 2) as i16);
        if self.timer == 0 {
            self.pos = t.cam.presets[(self.preset & 7) as usize];
            // aim at the midpoint and fit with the target angles
            self.org = mid;
            let (p, y) = self.lookat(t, mid.map(|v| v as i32));
            self.angt = [p, y, 0];
            self.ang = self.angt;
            self.zoom_fit2(t, a, b);
            self.look = self.org;
            self.zom = self.zomt;
            self.timer = 1;
        }
        if self.timer != 1 {
            return;
        }
        self.org = mid;
        let d = [0, 1, 2].map(|k| self.org[k] as i32 - self.look[k] as i32);
        let l = isqrt(d.iter().map(|&v| v as i64 * v as i64).sum::<i64>());
        if l > 0 {
            for k in 0..3 {
                let n = (d[k] << 12) / l;
                self.look[k] = self.look[k].wrapping_add(((n * l) >> 16) as i16);
            }
        }
        let (p, y) = self.lookat(t, self.look.map(|v| v as i32));
        self.angt = [p, y, 0];
        // angle vector step of 0x40, the raw target once within reach
        let dv = [0, 1, 2].map(|k| wrap12(self.angt[k] as i32 - self.ang[k] as i32));
        if dv.iter().map(|&v| v as i64 * v as i64).sum::<i64>() <= 0x40 * 0x40 {
            self.ang = self.angt;
        } else {
            Self::lerp_lin(&mut self.ang, self.angt, 0x40, true);
        }
        self.zoom_fit2(t, a, b);
        let d = self.zom[2] as i32 - self.zomt[2] as i32;
        let st = if self.zom[2] < self.zomt[2] { (d.abs() << 14) >> 16 } else { (d.abs() << 12) >> 16 };
        step_s16(&mut self.zom[2], self.zomt[2], st);
    }

    /// WIN_1P / WIN_2P: `r` = the game's byte-table random for the shot choice.
    /// LOSE_1P / LOSE_2P: straight down on the loser's pelvis (part 4), zooming in from
    /// 0xA00 to 0x300, the yaw following the fighter.
    fn lose(&mut self, s: &Subject) {
        self.pos = s.part4.map(|v| v as i16);
        if self.timer == 0 {
            self.ang[0] = 0x400;
            self.zom = [0, 0, 0xA00];
            self.ang[2] = 0;
            self.h = 360;
            self.timer = 1;
        }
        if self.timer == 1 && self.zom[2] > 0x300 {
            self.zom[2] -= 2;
        }
        self.ang[1] = (0x1000 - s.yaw as i32) as i16;
    }

    fn win(&mut self, s: &Subject, r: i32) {
        if self.timer == 0 {
            self.h = 360;
            self.ang[2] = 0;
            self.win_base = (0x1000 - (s.yaw as i32 + s.yaw_c6 as i32)) as i16;
            self.win_side = s.side_flag;
            self.win_frame = 0;
            self.timer = ((r & 3) + 1) as i16;
            self.pos[1] = s.part3[1] as i16;
        }
        self.pos[0] = s.part3[0] as i16;
        self.pos[2] = s.part3[2] as i16;
        let (base, side) = (self.win_base as i32, self.win_side);
        let first = self.win_frame == 0;
        let off = |v: i32| if side { v } else { -v };
        match self.timer {
            1 => {
                if first {
                    self.zom = [0, 0, 0x380];
                    self.zomt = [0, 0, 0x280];
                    self.ang[0] = 0x100;
                    self.ang[1] = (base + off(0x600)) as i16;
                    self.angt[1] = base as i16;
                }
                if step_angle(&mut self.ang[1], self.angt[1], 10) == 0 {
                    step_s16(&mut self.zom[2], self.zomt[2], 2);
                }
            }
            2 => {
                if first {
                    self.zom = [0, 0, 0x380];
                    self.zomt[2] = 0x280;
                    self.ang[0] = 0x180;
                    self.angt[0] = -0x80;
                    self.ang[1] = base as i16;
                }
                if step_angle(&mut self.ang[0], self.angt[0], 4) == 0 {
                    step_s16(&mut self.zom[2], self.zomt[2], 2);
                }
            }
            3 => {
                if first {
                    self.zom = [0, 0, 0x200];
                    self.zomt[2] = 0x300;
                    self.ang[0] = 0;
                    self.ang[1] = (base + off(0xC00)) as i16;
                    self.angt[1] = base as i16;
                }
                if step_angle(&mut self.ang[1], self.angt[1], 10) == 0 {
                    step_s16(&mut self.zom[2], self.zomt[2], 2);
                }
            }
            _ => {
                if first {
                    self.zom = [0, 0, 0x200];
                    self.zomt[2] = 0x380;
                    self.ang[0] = 0x80;
                    self.ang[1] = (base + off(0xE00)) as i16;
                    self.angt[1] = base as i16;
                }
                if step_s16(&mut self.zom[2], self.zomt[2], 2) == 0 {
                    step_angle(&mut self.ang[1], self.angt[1], 5);
                }
            }
        }
        self.win_frame += 1;
    }

    /// THROW: attacker / victim; `r` = random for the throw shot's angle table.
    fn throw(&mut self, t: &Trig, a: &Subject, v: &Subject, r: i32) {
        if self.timer == 0 {
            self.pos = a.part4.map(|x| x as i16);
            if self.pos[1] >= -255 {
                self.pos[1] = -256;
            }
            let s = t.atan2(v.part4[2] - a.part4[2], v.part4[0] - a.part4[0]);
            let (off, pitch) = t.cam.throw[(r & 7) as usize];
            let off = off as i32;
            self.ang[1] = if wrap12(s - self.ang[1] as i32) >= 0 { (s - off) as i16 } else { (s + off) as i16 };
            self.ang[0] = pitch;
            self.ang[2] = 0;
            self.zom = [-0x400; 3];
            self.zoom_fit2(t, a.part4, v.part4);
            self.zom = self.zomt;
            self.timer = 1;
        }
        let mut post = a.part4.map(|x| x as i16);
        if post[1] >= -127 {
            post[1] = -128;
        }
        self.post = post;
        self.pos = post;
        self.zoom_fit2(t, a.part4, v.part4);
        self.zomt[2] -= 0x100;
        let d = (self.zom[2] as i32 - self.zomt[2] as i32).abs();
        let st = if self.zom[2] < self.zomt[2] { (d << 13) >> 16 } else { (d << 11) >> 16 };
        step_s16(&mut self.zom[2], self.zomt[2], st);
    }

    /// Move-script op 2E: camera commands of intros, throws, win poses and transformations.
    /// `side` = F.2 of the fighter running the script (sub 5 names its own joint first).
    pub fn script(&mut self, t: &Trig, sub: i8, a: [i16; 3], side: usize) {
        match sub {
            0 => self.mode = a[0],
            1 => self.post = a,
            2 => {
                self.angt = a;
                self.rp_ofs = a[1] as i32; // OFS (+0x8A), the yaw offset of MODE2
            }
            3 => self.zomt = a,
            4 => self.org = a,
            5 => {
                // JT[self] = low byte, JT[opponent] = high byte (-1 = none); by player (`side & 1`, the game's two
                // entries; the team battle runs the camera ops on a scratch camera)
                self.jt[side & 1] = a[0] as u8 as i8;
                self.jt[(side & 1) ^ 1] = ((a[0] as u16) >> 8) as u8 as i8;
            }
            6 => self.pos_speed = a[0],
            7 => self.ang_speed = a[0],
            8 => self.zom_speed = a[0],
            9 => {
                self.preset = a[0];
                self.pos = t.cam.presets[(a[0] & 7) as usize];
            }
            _ => {}
        }
    }

    /// lerp_lin: move `cur` toward `tgt` by `speed` along the difference (snap when closer).
    fn lerp_lin(cur: &mut [i16; 3], tgt: [i16; 3], speed: i32, angle: bool) {
        let d = [0, 1, 2].map(|k| if angle { wrap12(tgt[k] as i32 - cur[k] as i32) } else { tgt[k] as i32 - cur[k] as i32 });
        let l = ((d[0] as f64).powi(2) + (d[1] as f64).powi(2) + (d[2] as f64).powi(2)).sqrt();
        if l <= speed as f64 {
            for k in 0..3 {
                cur[k] = if angle { (cur[k] as i32 + d[k]) as i16 } else { tgt[k] };
            }
            return;
        }
        for k in 0..3 {
            let n = (d[k] as f64 * 4096.0 / l) as i32;
            cur[k] = (cur[k] as i32 + ((n * speed) >> 12)) as i16;
        }
    }

    /// lerp_exp(…, 4): about 1/16 of the difference per frame, at least one unit.
    fn lerp_exp(cur: &mut [i16; 3], tgt: [i16; 3], angle: bool) {
        for k in 0..3 {
            let d = if angle { wrap12(tgt[k] as i32 - cur[k] as i32) } else { tgt[k] as i32 - cur[k] as i32 };
            let mut st = d >> 4;
            if st == 0 && d != 0 {
                st = d.signum();
            }
            cur[k] = (cur[k] as i32 + st) as i16;
        }
    }

    /// Zoom that fits one point (single margin).
    fn zoom_fit1(&mut self, t: &Trig, p: [i32; 3]) {
        let r = rot_xy(t, self.ang[0] as i32, -(self.ang[1] as i32));
        let ra = apply(&r, [0, 1, 2].map(|k| (p[k] as i16).wrapping_sub(self.pos[k]) as i32));
        let zx = ((ra[0].abs() + 0x180) * 360) / 320;
        let zy = ((ra[1].abs() + 0x180) * 360) / 240;
        self.zomt[2] = (zx.max(zy).max(0x4A4) - ra[2]) as i16;
    }

    /// The joint target of MODES 1/2: (look point, both bones if both joints are set).
    fn joint_target(&self, s: [&Subject; 2]) -> (Option<[i16; 3]>, Option<([i32; 3], [i32; 3])>) {
        let bone = |k: usize| s[k].bones[(self.jt[k] as usize).min(19)];
        match (self.jt[0] >= 0, self.jt[1] >= 0) {
            (true, true) => {
                let (a, b) = (bone(0), bone(1));
                (Some([0, 1, 2].map(|k| ((a[k] + b[k]) / 2) as i16)), Some((a, b)))
            }
            (true, false) => (Some(bone(0).map(|v| v as i16)), None),
            (false, true) => (Some(bone(1).map(|v| v as i16)), None),
            _ => (None, None),
        }
    }

    fn auto_fit(&mut self, t: &Trig, both: Option<([i32; 3], [i32; 3])>) {
        match both {
            Some((a, b)) => self.zoom_fit2(t, a, b),
            None => {
                let l = self.look.map(|v| v as i32);
                self.zoom_fit1(t, l)
            }
        }
    }

    /// MODE1: the joint camera — look at the joints' midpoint from POST.
    fn mode1(&mut self, t: &Trig, s: [&Subject; 2]) {
        let (pt, both) = self.joint_target(s);
        if let Some(p) = pt {
            self.org = p;
        }
        if self.mode & 0x80 != 0 {
            self.look = self.org;
            self.pos = self.post;
            let (p, y) = self.lookat(t, self.look.map(|v| v as i32));
            self.angt = [p, y, 0];
            self.ang = self.angt;
            if self.zom_speed == -1 {
                self.auto_fit(t, both);
            }
            self.zom = self.zomt;
            self.mode &= 0x7F;
            return;
        }
        Self::lerp_exp(&mut self.look, self.org, false);
        if self.pos_speed > 0 {
            Self::lerp_lin(&mut self.pos, self.post, self.pos_speed as i32, false);
        } else {
            Self::lerp_exp(&mut self.pos, self.post, false);
        }
        let (p, y) = self.lookat(t, self.look.map(|v| v as i32));
        self.angt = [p, y, 0];
        self.ease_ang_zoom(t, both);
    }

    /// MODE2: orbit the joints' midpoint; yaw = OFS (relative to the fighter's facing with one
    /// joint). Bit 0x80 snaps every frame (it is not cleared).
    fn mode2(&mut self, t: &Trig, s: [&Subject; 2]) {
        let (pt, both) = self.joint_target(s);
        if let Some(p) = pt {
            self.org = p;
        }
        match (self.jt[0] >= 0, self.jt[1] >= 0) {
            (true, true) => self.angt[1] = (self.rp_ofs & 0xFFF) as i16,
            (true, false) => self.angt[1] = ((self.rp_ofs + 0x1000 - s[0].yaw as i32) & 0xFFF) as i16,
            (false, true) => self.angt[1] = ((self.rp_ofs + 0x1000 - s[1].yaw as i32) & 0xFFF) as i16,
            _ => {}
        }
        self.look = self.org;
        self.post = self.org;
        if self.mode & 0x80 != 0 {
            self.pos = self.org;
            self.ang = self.angt;
            if self.zom_speed == -1 {
                self.auto_fit(t, both);
            }
            self.zom = self.zomt;
            return;
        }
        if self.pos_speed > 0 {
            Self::lerp_lin(&mut self.pos, self.post, self.pos_speed as i32, false);
        } else {
            Self::lerp_exp(&mut self.pos, self.post, false);
        }
        self.ease_ang_zoom(t, both);
    }

    fn ease_ang_zoom(&mut self, t: &Trig, both: Option<([i32; 3], [i32; 3])>) {
        if self.ang_speed > 0 {
            Self::lerp_lin(&mut self.ang, self.angt, self.ang_speed as i32, true);
        } else {
            Self::lerp_exp(&mut self.ang, self.angt, true);
        }
        if self.zom_speed > 0 {
            Self::lerp_lin(&mut self.zom, self.zomt, self.zom_speed as i32, false);
            return;
        }
        if self.zom_speed == -1 {
            self.auto_fit(t, both);
        }
        Self::lerp_exp(&mut self.zom, self.zomt, false);
    }

    /// MODE0: scripted free camera (targets set by move-script op 2E).
    fn mode0(&mut self) {
        if self.mode & 0x80 != 0 {
            self.pos = self.post;
            self.ang = self.angt;
            self.zom = self.zomt;
            self.mode &= 0x7F;
            return;
        }
        let ease = |c: &mut [i16; 3], tg: [i16; 3], sp: i16, ang: bool| {
            for k in 0..3 {
                let d = if ang { wrap12(tg[k] as i32 - c[k] as i32) } else { tg[k] as i32 - c[k] as i32 };
                let st = if sp > 0 {
                    d.signum() * d.abs().min(sp as i32)
                } else if d.abs() >= 16 {
                    d / 16
                } else {
                    d.signum()
                };
                c[k] = (c[k] as i32 + st) as i16;
            }
        };
        ease(&mut self.pos, self.post, self.pos_speed, false);
        ease(&mut self.ang, self.angt, self.ang_speed, true);
        ease(&mut self.zom, self.zomt, self.zom_speed, false);
    }

    /// One frame. `frozen` = the match freeze (+0x82): MOVE does not update then.
    pub fn update(&mut self, t: &Trig, s: [&Subject; 2], frozen: bool, r: i32) {
        match self.mode & 0x7F {
            0 => self.mode0(),
            1 => self.mode1(t, s),
            2 => self.mode2(t, s),
            3 => {
                if !frozen {
                    if self.timer == 0 {
                        self.timer = 1;
                    }
                    if self.snap >= 0 {
                        step_s16(&mut self.h, 360, 10);
                        self.move_follow(t, s[0], s[1]);
                    } else {
                        self.h = 360;
                        self.move_snap(t, s[0], s[1]);
                    }
                }
            }
            4 | 5 => self.win(s[(self.mode & 0x7F) as usize - 4], r),
            6 | 7 => self.lose(s[(self.mode & 0x7F) as usize - 6]),
            10 | 11 => {
                let w = ((self.mode & 0x7F) - 10) as usize;
                self.replay(t, s[w], s[1 - w], s, r);
            }
            12 | 13 => self.track(t, s[0], s[1]),
            14..=17 => self.special_ko(t, s),
            8 => self.throw(t, s[0], s[1], r),
            9 => self.throw(t, s[1], s[0], r),
            0x1E => self.turntable(),
            0x20 => self.tv(t, s),
            // 0x1C / 0x1D / 0x21..0x24 (dev-only shots) and the default (zoom on): nothing
            _ => {}
        }
        // shake decays every frame (alternating sign)
        for k in 0..2 {
            self.shake[k] = ((-3 * self.shake[k] as i32) / 4) as i16;
        }
    }

    /// Engine addition (the team battle): one frame of MOVE (snap / follow) over the living
    /// fighters `all`: the yaw from the pair furthest apart `s`, the pivot at the centre of the bounding box of all of
    /// them (y as `pivot_target`'s), the zoom fitting all of them, at most `zmax`; then the shake decays. No other mode
    /// runs (the scripted shots are not used in the team battle).
    pub(crate) fn update_team(&mut self, t: &Trig, s: [&Subject; 2], all: &[Subject], zmax: i16) {
        let pts: Vec<[i32; 3]> = all.iter().map(Self::sample).collect();
        let (a, b) = (Self::sample(s[0]), Self::sample(s[1]));
        let lo = [0, 1, 2].map(|k| pts.iter().map(|p| p[k]).min().unwrap_or(0));
        let hi = [0, 1, 2].map(|k| pts.iter().map(|p| p[k]).max().unwrap_or(0));
        let mut post = [0, 1, 2].map(|k| (lo[k] + hi[k]) / 2);
        if post[1] >= -255 && all.iter().all(|s| s.f5c & 0x8_0000 == 0) {
            post[1] = -256;
        }
        let post = post.map(|v| v as i16);
        let fit: Vec<[i32; 3]> = pts.iter().map(|p| [p[0], p[1].min(-128), p[2]]).collect();
        if self.timer == 0 {
            self.timer = 1;
        }
        if self.snap >= 0 {
            step_s16(&mut self.h, 360, 10);
            self.move_follow_on(t, a, b, post, &fit, zmax);
        } else {
            self.h = 360;
            self.move_snap_on(t, a, b, post, &fit, zmax);
        }
        for k in 0..2 {
            self.shake[k] = ((-3 * self.shake[k] as i32) / 4) as i16;
        }
    }

    /// Modes 0xE..0x11: the special-KO replays' fixed shots on a fighter's chest
    /// (P1 for 0xE / 0xF, P2 for 0x10 / 0x11), set every frame without easing. 0xF / 0x11 track instead
    /// while the other fighter is KO'd and blown away (F68 & 0x2000).
    fn special_ko(&mut self, t: &Trig, s: [&Subject; 2]) {
        let m = self.mode & 0x7F;
        if m & 1 != 0 {
            let o = if m == 15 { s[1] } else { s[0] };
            if o.ko && o.f68 & 0x2000 != 0 {
                self.track(t, s[0], s[1]);
                return;
            }
        }
        self.rp_ofs = 0;
        let who = if m < 16 { 0 } else { 1 };
        let (pitch, yaw, z) = if m % 2 == 0 { (-0x100, 0x400, 0x300) } else { (0, -0x200, 0x200) };
        let c = s[who].part3;
        let y = if c[1] >= -0xFF { -0x100 } else { c[1] };
        self.ang = [pitch, yaw, 0];
        self.angt = self.ang;
        self.pos = [c[0] as i16, y as i16, c[2] as i16];
        self.post = self.pos;
        self.zom = [0, 0, z];
        self.zomt = self.zom;
        self.h = 360;
    }

    /// Clip-space projection of view space: H over the logical 320×240 screen; a wide view shows more on the
    /// sides, a tall one more above and below (`Viewport::proj_scale`).
    pub fn projection(&self, vp: &crate::output::layout::Viewport) -> glam::Mat4 {
        let (n, f) = (16.0f32, 60000.0f32);
        let [kx, ky] = vp.proj_scale(self.h as f32);
        glam::Mat4::from_cols(
            glam::Vec4::new(kx, 0.0, 0.0, 0.0),
            glam::Vec4::new(0.0, -ky, 0.0, 0.0),
            glam::Vec4::new(0.0, 0.0, f / (f - n), 1.0),
            glam::Vec4::new(0.0, 0.0, -n * f / (f - n), 0.0),
        )
    }

    /// The camera transform: game-space point → view space (x right, y down, z depth), game units.
    pub fn view(&self, t: &Trig) -> glam::Mat4 {
        let sh_y = ((self.shake[1] as i32) << 8) / 10000;
        let sh_x = self.shake[0] as i32;
        let r = rot_xy(t, self.ang[0] as i32, -(self.ang[1] as i32));
        let m = |i: usize, j: usize| r[i][j] as f32 / 4096.0;
        let rot = glam::Mat3::from_cols(
            glam::Vec3::new(m(0, 0), m(1, 0), m(2, 0)),
            glam::Vec3::new(m(0, 1), m(1, 1), m(2, 1)),
            glam::Vec3::new(m(0, 2), m(1, 2), m(2, 2)),
        );
        let pos = glam::Vec3::new(self.pos[0] as f32, self.pos[1] as f32, self.pos[2] as f32);
        let ofs = glam::Vec3::new((self.zom[0] as i32 + sh_x) as f32, (self.zom[1] as i32 + sh_y) as f32, self.zom[2] as f32);
        glam::Mat4::from_translation(ofs) * glam::Mat4::from_mat3(rot) * glam::Mat4::from_translation(-pos)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `zoom_fit_n` over two points gives `zoom_fit2`'s distance (its two-point form, as `zoom_fit2` had it), whatever
    /// their order; a point repeated changes nothing.
    #[test]
    fn zoom_fit_n_is_zoom_fit2_for_two_points() {
        let Some(t) = crate::formats::tables::export_tables().map(|t| Trig::from_tables(&t)) else {
            eprintln!("skipped: no okb_export");
            return;
        };
        let fit2 = |c: &Camera, p1: [i32; 3], p2: [i32; 3]| {
            let r = rot_xy(&t, c.ang[0] as i32, -(c.ang[1] as i32));
            let rel = |p: [i32; 3]| [0, 1, 2].map(|k| (p[k] as i16).wrapping_sub(c.pos[k]) as i32);
            let (ra, rb) = (apply(&r, rel(p1)), apply(&r, rel(p2)));
            let ax = ra[0].abs().max(rb[0].abs());
            let ay = ra[1].abs().max(rb[1].abs());
            let zx = ((2 * ax + 0x180) * 360) / 320;
            let zy = ((2 * ay + 0x180) * 360) / 240;
            (zx.max(zy).max(0x4A4) - ra[2].min(rb[2])) as i16
        };
        let mut c = Camera::default();
        let pts = [[0, 0, 0], [-1536, -256, 700], [1400, -900, -1300], [3000, 0, 3000], [-40, -1800, 12], [70000, 5, -70000]];
        for (pitch, yaw) in [(0, 0), (91, 0), (91, 1024), (-200, -1800), (300, 2047)] {
            c.ang = [pitch, yaw, 0];
            c.pos = [yaw / 3, -268, -pitch];
            for &a in &pts {
                for &b in &pts {
                    let want = fit2(&c, a, b);
                    c.zoom_fit_n(&t, &[a, b]);
                    assert_eq!(c.zomt[2], want, "{a:?} {b:?}");
                    c.zoom_fit_n(&t, &[b, a, b]);
                    assert_eq!(c.zomt[2], want, "{b:?} {a:?} {b:?}");
                }
            }
        }
    }
}
