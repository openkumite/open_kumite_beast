//! Movement: velocity, gravity, landing, position integration, yaw, ring-out and walls.
//!
//! State: none of its own: updates the serialised `Fighter`
//!
//! Movement: velocity sources, gravity, landing, position integration, yaw and opponent angle.
//! The tail of the fighter step (velocity), the vertical speed, landing, the pair's movement (generic path),
//! the yaw composition at the end of the animation step and the angle part of the pair step.
//! Angles are 4096 per turn; game axes: +X right, +Y down, forward of yaw `a` = (sin a, −cos a) in X/Z.

use crate::fight::fighter::*;

/// Sine and arctangent tables of the game, 0x401 entries each.
pub struct Trig {
    sin: Vec<i16>,
    atan: Vec<i16>,
    /// 256 random bytes of the game, read by the move scripts' random op.
    pub random: Vec<u8>,
    /// the CPU level tables.
    pub cpu: Vec<u8>,
    /// the CPU's recovery pads (`ai.recover_pads`)
    pub recover_pads: [u16; 8],
    /// the camera's tables
    pub cam: CamTables,
    /// the colour flash patterns (`fx.colour_flash`: a count, then that many back colours r, g, b with 4096 = 1.0, by
    /// its pointer table at the blob's end; none without it)
    pub flash: Vec<Vec<[i32; 3]>>,
}

/// The colour flash patterns: `fx.colour_flash`'s six, by the links at its end.
fn flash_patterns(t: &crate::formats::tables::Tables) -> Vec<Vec<[i32; 3]>> {
    const PATTERNS: usize = 6;
    let tab = t.table("fx.colour_flash");
    let links = tab.bytes().len().saturating_sub(4 * PATTERNS);
    if tab.bytes().is_empty() {
        return Vec::new();
    }
    (0..PATTERNS)
        .map(|k| {
            let at = tab.u32(links + 4 * k);
            let n = t.abs_i16(at).max(0) as u32;
            // (a link of bad data wraps rather than overflows: it reads as any other address, zeros outside the tables)
            (0..n).map(|j| [0, 1, 2].map(|c| t.abs_i16(at.wrapping_add(2 + 6 * j + 2 * c)) as i32)).collect()
        })
        .collect()
}

/// The camera's tables (`camera.*`; zeros without them): the TV camera's eye positions, the KO replay's random eye
/// positions and heights, the throw camera's angles (yaw offset, pitch), the tracking shot's yaw offsets.
#[derive(Clone, Debug, Default)]
pub struct CamTables {
    pub presets: [[i16; 3]; 8],
    pub random: [[i16; 3]; 8],
    pub heights: [i16; 4],
    pub throw: [(i16, i16); 8],
    pub track: [i16; 4],
}

impl CamTables {
    pub fn from_tables(t: &crate::formats::tables::Tables) -> Self {
        let vecs = |name: &str| {
            let t = t.table(name);
            std::array::from_fn(|k| [0, 1, 2].map(|j| t.i16(8 * k + 2 * j)))
        };
        let (h, th, tr) = (t.table("camera.heights"), t.table("camera.throw"), t.table("camera.track"));
        Self {
            presets: vecs("camera.presets"),
            random: vecs("camera.random"),
            heights: std::array::from_fn(|k| h.i16_at(k)),
            throw: std::array::from_fn(|k| (th.i16(4 * k), th.i16(4 * k + 2))),
            track: std::array::from_fn(|k| tr.i16_at(k)),
        }
    }
}

impl Trig {
    pub fn parse(b: &[u8]) -> Self {
        let h: Vec<i16> = b.as_chunks::<2>().0.iter().map(|c| i16::from_le_bytes([c[0], c[1]])).collect();
        Self {
            sin: h[..0x401].to_vec(),
            atan: h[0x401..0x802].to_vec(),
            random: b.get(0x1004..0x1104).map(|s| s.to_vec()).unwrap_or_else(|| (0..=255).collect()),
            cpu: b.get(0x1104..0x11A4).map(|s| s.to_vec()).unwrap_or_default(),
            recover_pads: [0; 8],
            cam: CamTables::default(),
            flash: Vec::new(),
        }
    }

    /// From the tables `math.sin` and `math.atan` (0x401 entries each), `math.random_bytes` and `ai.cpu`, as `parse` reads
    /// them (no 256 random bytes: 0..=255), `ai.recover_pads`, the camera's (`CamTables`) and the flash patterns.
    pub fn from_tables(t: &crate::formats::tables::Tables) -> Self {
        let i16s = |name: &str| {
            let t = t.table(name);
            (0..0x401).map(|k| t.i16_at(k)).collect()
        };
        let random = t.table("math.random_bytes").bytes();
        Self {
            sin: i16s("math.sin"),
            atan: i16s("math.atan"),
            random: if random.len() == 0x100 { random.to_vec() } else { (0..=255).collect() },
            cpu: t.table("ai.cpu").bytes().to_vec(),
            recover_pads: std::array::from_fn(|k| t.table("ai.recover_pads").u16(2 * k)),
            cam: CamTables::from_tables(t),
            flash: flash_patterns(t),
        }
    }

    pub fn sin(&self, a: i32) -> i32 {
        let mut i = (a & 0x3FF) as usize;
        if a & 0x400 != 0 {
            i = 0x400 - i;
        }
        let v = self.sin[i] as i32;
        if a & 0x800 != 0 { -v } else { v }
    }

    pub fn cos(&self, a: i32) -> i32 {
        self.sin(a + 0x400)
    }

    /// ratan2(a0, a1)
    pub fn atan2(&self, mut a0: i32, mut a1: i32) -> i32 {
        if a0 | a1 == 0 {
            return 0;
        }
        let mut t0 = 0;
        let mut v0 = 0x800;
        let mut t1 = false;
        if a1 <= 0 {
            a1 = -a1;
            t1 = !t1;
            v0 = -0x800;
        }
        if a0 < 0 {
            a0 = -a0;
            t1 = !t1;
            t0 = v0;
        }
        let num;
        if a0 < a1 {
            num = a0 << 12;
            a0 = a1;
            t1 = !t1;
            t0 = v0 >> 1;
        } else {
            num = a1 << 12;
        }
        let idx = ((num / a0) >> 2).clamp(0, 0x400) as usize;
        let mut r = self.atan[idx] as i32;
        if t1 {
            r = -r;
        }
        t0 + r
    }

    /// (speed·sin a, −speed·cos a), rounded.
    fn polar(&self, a: i32, speed: i32) -> (i32, i32) {
        let x = (speed * self.sin(a) + 0x800) >> 12;
        let z = (-speed * self.cos(a) + 0x800) >> 12;
        (x, z)
    }

    /// Polar step with 1/128 remainders kept in `frac` (fighter +0xA2 / +0xA6).
    fn polar_step(&self, a: i32, speed: i32, frac: &mut [i16; 2]) -> (i32, i32) {
        if speed == 0 {
            return (0, 0);
        }
        let (x, z) = self.polar(a, speed);
        let (x, z) = (x + frac[0] as i32, z + frac[1] as i32);
        let (xr, zr) = (x + 0x40, z + 0x40);
        frac[0] = (xr - (xr & -0x80) - 0x40) as i16;
        frac[1] = (zr - (zr & -0x80) - 0x40) as i16;
        (xr >> 7, zr >> 7)
    }

    /// Velocity X/Z = polar step.
    fn set_vel(&self, f: &mut Fighter, a: i32, speed: i32) {
        let (x, z) = self.polar_step(a, speed, &mut f.frac);
        f.vel[0] = x as i16;
        f.vel[2] = z as i16;
    }

    /// Velocity X/Z += polar step.
    fn add_vel(&self, f: &mut Fighter, a: i32, speed: i32) {
        if speed == 0 {
            return;
        }
        let (x, z) = self.polar_step(a, speed, &mut f.frac);
        f.vel[0] = f.vel[0].wrapping_add(x as i16);
        f.vel[2] = f.vel[2].wrapping_add(z as i16);
    }
}

pub fn wrap12(a: i32) -> i16 {
    ((a << 20) >> 20) as i16
}

/// Speed decay toward 0 by `step` (a pattern the game uses twice).
fn decay(speed: &mut i16, step: &mut i16) {
    if *speed > 0 {
        *speed = speed.wrapping_add(*step);
        if *speed < 0 {
            *step = 0;
            *speed = 0;
        }
    } else if *speed < 0 {
        *speed = speed.wrapping_add(*step);
        if *speed > 0 {
            *step = 0;
            *speed = 0;
        }
    }
}

fn scale_step(v: i32, step: i16) -> i32 {
    let p = v * step as i32;
    (if p < 0 { p + 0xF } else { p }) >> 4
}

/// Velocity of this frame, run after the move script.
pub fn velocity(f: &mut Fighter, t: &Trig) {
    if f.flags[F6C] & 0x800 != 0 && f.form & 0x10 == 0 && f.flags[F6C] & 0x8000 == 0 {
        f.o_d8 = f.o_d8.wrapping_add(((f.o_da >> 3) as i16).wrapping_mul(60));
    }
    let a = f.yaw as i32 - f.s_d2 as i32 + f.o_8a as i32;
    t.set_vel(f, a, f.o_8c as i32);
    decay(&mut f.o_8c, &mut f.o_8e);
    if f.o_b2 != 0 {
        t.add_vel(f, f.o_c4 as i32, f.o_b2 as i32);
        let p = f.o_b2 as i32 * f.o_b4 as i32;
        f.o_b2 = ((if p < 0 { p + 0x7F } else { p }) >> 7) as i16;
    }
    vertical(f, t);
    f.vel[0] = scale_step(f.vel[0] as i32, f.step) as i16;
    f.vel[1] = scale_step(f.vel[1] as i32, f.step) as i16;
    f.vel[2] = scale_step(f.vel[2] as i32, f.step) as i16;
}

/// Vertical speed, gravity and the second polar source (+0x96 / +0x98).
fn vertical(f: &mut Fighter, t: &Trig) {
    let mut vy = f.o_90 as i32;
    let mut g = f.o_92 as i32;
    let y = f.m_t[1];
    let ground = f.o_94 as i32;
    let mut s3 = 0;
    if (f.flags[F5C] as i32) < 0 && y < ground {
        g = 0xA0;
        f.o_92 = 0xA0;
    }
    if y == ground && vy | g == 0 {
        f.vel[1] = 0;
        f.o_a4 = 0;
        return;
    }
    if f.flags[F60] & 0x2000 == 0 && vy | g != 0 {
        let a = f.yaw_base as i32 + f.s_e4 as i32 + f.o_96 as i32;
        t.add_vel(f, a, f.o_98 as i32);
        decay(&mut f.o_98, &mut f.o_8e);
    }
    if f.flags[F60] & 0x800 == 0 && vy <= 0 {
        s3 = vy >> 7;
        vy += scale_step(g, f.step);
        f.o_90 = vy as i16;
    } else if f.flags[F60] & 0x1000 == 0 && vy > 0 {
        s3 = vy >> 7;
        vy += scale_step(g, f.step);
        if vy < -0x8000 {
            vy = -0x8000;
        }
        f.o_90 = vy as i16;
    }
    f.vel[1] = s3 as i16;
}

/// Airborne flag and landing.
pub fn landing(f: &mut Fighter) {
    f.flags[F5C] &= 0xFFFF_FE7F; // −0x181 = !0x180 (the guard bit stays)
    let (vy, ground, y) = (f.o_90 as i32, f.o_94 as i32, f.m_t[1]);
    if y == ground && vy | f.o_92 as i32 == 0 {
        return; // (standing on the ground)
    }
    f.flags[F5C] |= 0x80;
    if !(y < ground) && f.flags[F58] & 0x100_0000 == 0 {
        f.o_92 = 0;
        f.o_90 = 0;
        f.flags[F6C] |= 0x8_0000; // landed event, read by the move decision
        f.flags[F5C] &= !0x80;
        f.m_t[1] = ground;
        f.pos[1] = ground as i16;
    } else if f.o_90 > 0 {
        f.flags[F5C] |= 0x100; // FALL
    }
}

/// Move both fighters. A fighter with F6C 0x20000 / 0x40000 (set by a throw victim's script)
/// takes the opponent's base position after the opponent has moved (0x20000 is cleared: one frame).
pub fn integrate_pair(fs: &mut [Fighter; 2], t: &Trig) {
    integrate_pair_of(fs, [0, 1], t);
}

/// `integrate_pair` of the fighters `p` of `fs` (P1's place first; the team battle's throw pairs, engine addition).
pub fn integrate_pair_of(fs: &mut [Fighter], p: [usize; 2], t: &Trig) {
    let attached = |f: &Fighter| f.flags[F6C] & 0x6_0000 != 0;
    for k in 0..2 {
        let (v, o) = (p[k], p[1 - k]);
        if attached(&fs[v]) {
            fs[v].flags[F6C] &= 0xFFFD_FFFF;
            integrate(&mut fs[o], t);
            fs[v].pos = fs[o].pos;
            place(&mut fs[v], t, false);
            return;
        }
    }
    integrate(&mut fs[p[0]], t);
    integrate(&mut fs[p[1]], t);
}

/// Move the base position by the velocity and place the model at base + rotated root offset.
pub fn integrate(f: &mut Fighter, t: &Trig) {
    place(f, t, true);
}

fn place(f: &mut Fighter, t: &Trig, advance: bool) {
    let (yaw, ox, oy, oz) = (f.yaw_base as i32, f.s_dc as i32, f.s_de as i32, f.s_e0 as i32);
    f.m_pos = f.pos;
    if advance {
        for k in 0..3 {
            f.pos[k] = f.pos[k].wrapping_add(f.vel[k]);
        }
    }
    let (c, s) = (t.cos(yaw), t.sin(yaw));
    f.vel[1] = f.vel[1].wrapping_add(((oy << 16) >> 20) as i16);
    f.vel[0] = f.vel[0].wrapping_add(((ox * c - oz * s) >> 16) as i16);
    f.vel[2] = f.vel[2].wrapping_add(((ox * s + oz * c) >> 16) as i16);
    for k in 0..3 {
        f.m_pos[k] = f.m_pos[k].wrapping_add(f.vel[k]);
    }
    for k in 0..3 {
        f.vel[k] = (f.m_pos[k] as i32 - f.m_t[k]) as i16;
        f.m_t[k] = f.m_pos[k] as i32;
    }
}

/// End of the animation step: fold the turn request (+0xE4) into the facing, keeping the model in place.
pub fn compose_yaw(f: &mut Fighter, t: &Trig) {
    let s5 = -(f.s_e4 as i32);
    if s5 != 0 {
        let dx = f.pos[0] as i32 - f.m_t[0];
        let dz = f.pos[2] as i32 - f.m_t[2];
        let (sn, cs) = (t.sin(s5), t.cos(s5));
        f.pos[0] = (f.m_t[0] + ((dz * sn + dx * cs) >> 12)) as i16;
        f.pos[2] = (f.m_t[2] + ((dz * cs - dx * sn) >> 12)) as i16;
        f.yaw_base = wrap12(f.yaw_base as i32 + f.s_e4 as i32);
        f.s_e4 = 0;
    }
    f.yaw = wrap12(f.yaw_base as i32 + f.s_d2 as i32 + f.s_e4 as i32);
}

/// Root part: bake the animation's root offset and yaw into the base when a new animation starts.
pub fn bake_root(f: &mut Fighter, t: &Trig) {
    let (yaw, ox, oy, oz) = (f.yaw_base as i32, f.s_dc as i32, f.s_de as i32, f.s_e0 as i32);
    let (c, s) = (t.cos(yaw), t.sin(yaw));
    f.pos[1] = f.pos[1].wrapping_add(((oy << 16) >> 20) as i16);
    f.pos[0] = f.pos[0].wrapping_add(((ox * c - oz * s) >> 16) as i16);
    f.pos[2] = f.pos[2].wrapping_add(((ox * s + oz * c) >> 16) as i16);
    f.s_dc = 0;
    f.s_de = 0;
    f.s_e0 = 0;
    f.yaw_base = wrap12(yaw + f.s_d2 as i32 + f.s_e4 as i32);
    f.s_e4 = 0;
    f.s_d2 = 0;
}

/// θ of the pair step: the angle between the roots (before push-apart and walls).
pub fn root_angle(fs: &[Fighter; 2], t: &Trig) -> i32 {
    root_angle_of(&fs[0], &fs[1], t)
}

/// `root_angle` from fighter `a` to fighter `b`.
pub fn root_angle_of(a: &Fighter, b: &Fighter, t: &Trig) -> i32 {
    t.atan2(a.m_t[2] - b.m_t[2], b.m_t[0] - a.m_t[0])
}

/// End of the pair step: angle to the opponent (from θ of before the push), angle and distance to the stage centre.
pub fn opponent_angles(fs: &mut [Fighter; 2], t: &Trig, th: i32) {
    angles(&mut fs[0], t, th);
    angles(&mut fs[1], t, th + 0x800);
}

/// `opponent_angles` of one fighter: `th` the angle from it to its opponent.
pub fn angles(f: &mut Fighter, t: &Trig, th: i32) {
    f.o_74 = wrap12(th - f.yaw as i32);
    let (x, z) = (f.m_t[0], f.m_t[2]);
    f.o_70 = wrap12(t.atan2(z, -x) - f.yaw as i32);
    f.o_72 = hypot(z, x) as i16;
}

/// Integer hypot (Moler–Morrison, two iterations, Q15).
pub fn hypot(a0: i32, a1: i32) -> u32 {
    let sdiv = |a: u32, b: u32| -> u32 {
        let (a, b) = (a as i32, b as i32);
        if b == 0 { if a >= 0 { u32::MAX } else { 1 } } else { a.wrapping_div(b) as u32 }
    };
    let (mut p, mut q) = (a0.unsigned_abs(), a1.unsigned_abs());
    if p < q {
        std::mem::swap(&mut p, &mut q);
    }
    if q == 0 {
        return p;
    }
    let mut t3 = p << 15;
    q <<= 15;
    let mut v = p;
    for _ in 0..2 {
        let mut r = sdiv(q, v);
        r = r.wrapping_mul(r);
        r = sdiv(r, (r >> 15).wrapping_add(0x20000));
        t3 = t3.wrapping_add((((t3 << 1) as u64 * r as u64) >> 15) as u32);
        v = t3 >> 15;
        q = ((q as u64 * r as u64) >> 15) as u32;
    }
    v
}

/// VectorNormalSS: integer normalisation to length ≈ 4096 with the RSQRT table.
pub fn vector_normal_ss(v: [i32; 3], rsqrt: &dyn Fn(i32) -> i32) -> [i32; 3] {
    let l2 = v[0].wrapping_mul(v[0]).wrapping_add(v[1].wrapping_mul(v[1])).wrapping_add(v[2].wrapping_mul(v[2]));
    if l2 == 0 {
        return [0; 3];
    }
    let lz = if l2 >= 0 { l2.leading_zeros() } else { (!l2).leading_zeros() } as i32;
    let e = lz & !1;
    let t = if e >= 24 { l2 << (e - 24) } else { l2 >> (24 - e) };
    let f = rsqrt(t - 64);
    v.map(|c| ((c.wrapping_mul(f)) >> ((31 - e) >> 1)) as i16 as i32)
}

fn sat16(v: i32) -> i32 {
    v.clamp(-0x8000, 0x7FFF)
}

/// World positions (game units) of the skeleton nodes, from the posed glTF skeleton.
pub fn node_positions(globals: &[glam::Mat4], model_xf: glam::Mat4) -> Vec<[i32; 3]> {
    globals
        .iter()
        .map(|g| {
            let w = (model_xf * *g).w_axis;
            [(w.x * 300.0) as i32, (-w.y * 300.0) as i32, (-w.z * 300.0) as i32]
        })
        .collect()
}

/// Distance / height from the body objects (head + pelvis), then the push-apart of
/// overlapping body spheres along the previous roots' direction: `body_measure`, the distance and
/// height written to both, then `body_push`.
#[allow(clippy::too_many_arguments)]
pub fn body_pair(
    fs: &mut [Fighter; 2],
    th: i32,
    t: &Trig,
    bodies: [&[crate::formats::act_ani::Sphere]; 2],
    nodes: [&[[i32; 3]]; 2],
    rsqrt: &dyn Fn(i32) -> i32,
) {
    let m = body_measure(&fs[0], &fs[1], th, t);
    fs[0].o_76 = m.dist;
    fs[1].o_76 = m.dist;
    fs[0].o_78 = m.dy as i16;
    fs[1].o_78 = (-m.dy) as i16;
    fs[0].flags[F5C] &= !0x80_0000;
    fs[1].flags[F5C] &= !0x80_0000;
    body_push(fs, (0, 1), &m, th, t, bodies, nodes, rsqrt);
}

/// What `body_measure` finds between two fighters `a` and `b`.
#[derive(Clone, Copy, Debug)]
pub struct BodyGap {
    /// the distance along θ (+0x76 of both)
    pub dist: i16,
    /// the height of `b`'s pelvis over `a`'s (+0x78 of `a`; `b`'s is −dy)
    pub dy: i32,
    /// the signed distance along θ
    pub pz: i32,
}

/// The first part of the push-apart: the distance between the body objects (the mean of head and pelvis) of `a` and `b`
/// along θ, and the height of the pelvises (the caller writes them: +0x76, +0x78).
pub fn body_measure(a: &Fighter, b: &Fighter, th: i32, t: &Trig) -> BodyGap {
    let s16 = |v: i32| v as i16 as i32;
    let (sn, cs) = (t.sin(th), t.cos(th));
    let dx = s16(((b.head[0] + b.hips[0]) - (a.head[0] + a.hips[0])) / 2);
    let dy = s16(b.hips[1] - a.hips[1]);
    let dz = s16(((b.head[2] + b.hips[2]) - (a.head[2] + a.hips[2])) / 2);
    let pz = sat16((-sn * dx + cs * dz) >> 12);
    BodyGap { dist: s16(pz.abs()) as i16, dy, pz }
}

/// The rest of the push-apart: the push-apart of the fighters `ab` = (a, b) of `fs` (their body spheres `bodies` at
/// `nodes`, in that order) by the deepest overlap along θ, once `body_measure` gave `gap`; sets F5C 0x800000 (the
/// bodies touch) on both when they overlap (the caller clears it first).
#[allow(clippy::too_many_arguments)]
pub fn body_push(
    fs: &mut [Fighter],
    (a, b): (usize, usize),
    gap: &BodyGap,
    th: i32,
    t: &Trig,
    bodies: [&[crate::formats::act_ani::Sphere]; 2],
    nodes: [&[[i32; 3]]; 2],
    rsqrt: &dyn Fn(i32) -> i32,
) {
    let s16 = |v: i32| v as i16 as i32;
    let (sn, cs) = (t.sin(th), t.cos(th));
    let zrow = |x: i32, z: i32| sat16((-sn * x + cs * z) >> 12);
    let (dist, dy) = (gap.dist, gap.dy);
    let (fa, fb) = (&fs[a], &fs[b]);
    if gap.pz.abs() >= 0x181 || dy.abs() >= 0x141 || (fa.flags[F60] | fb.flags[F60]) & 0x20_0000 != 0 {
        return;
    }
    // deepest overlap, measured along the root line; sphere 0 only when someone lies down, else skipped
    let lying = |f: &Fighter| f.flags[F58] & 0x3_0000 != 0 && f.flags[F58] & 0x800_0000 == 0;
    let down = lying(fa) || lying(fb);
    let range = |n: usize| if down { 0..n.min(1) } else { 1.min(n)..n };
    let mut pen = 0i32;
    for sa in &bodies[0][range(bodies[0].len())] {
        for sb in &bodies[1][range(bodies[1].len())] {
            let (Some(pa), Some(pb)) = (nodes[0].get(sa.node as usize), nodes[1].get(sb.node as usize)) else { continue };
            let rs = sa.radius as i32 + sb.radius as i32;
            let d = [pb[0] - pa[0], pb[1] - pa[1], pb[2] - pa[2]];
            if !(d[0].abs() < rs && d[1].abs() < rs && d[2].abs() < rs) {
                continue;
            }
            let sq = d[0].wrapping_mul(d[0]).wrapping_add(d[1].wrapping_mul(d[1])).wrapping_add(d[2].wrapping_mul(d[2]));
            if sq >= rs.wrapping_mul(rs) {
                continue;
            }
            let za = zrow(s16(pa[0]), s16(pa[2]));
            let zb = zrow(s16(pb[0]), s16(pb[2]));
            pen = pen.max((zb + sb.radius as i32) - (za - sa.radius as i32));
        }
    }
    if crate::base::trace::flag("PUSHDBG") && !crate::base::trace::is_muted() {
        eprintln!("push: dist {dist} dy {dy} pen {pen}");
    }
    if pen == 0 {
        return;
    }
    fs[a].flags[F5C] |= 0x80_0000;
    fs[b].flags[F5C] |= 0x80_0000;
    let pen = pen.min(0xC8) * 3 / 4;
    let (l0, l1) = (fs[a].flags[F5C] & 0x8000 != 0, fs[b].flags[F5C] & 0x8000 != 0);
    let (wa, wb) = if l0 || l1 { (l0 as i32, l1 as i32) } else { (fs[a].o_24 as i32, fs[b].o_24 as i32) };
    let sum = wa + wb;
    if sum == 0 {
        return;
    }
    let (pa, pb) = (fs[a].prev_root, fs[b].prev_root);
    let n = vector_normal_ss([s16(pb[0] as i32 - pa[0] as i32), 0, s16(pb[2] as i32 - pa[2] as i32)], rsqrt);
    for (k, w, sgn) in [(a, wb, -1), (b, wa, 1)] {
        let m = (pen << 7) * w / sum;
        let (mx, mz) = (sgn * ((n[0] * m) >> 19), sgn * ((n[2] * m) >> 19));
        let f = &mut fs[k];
        f.pos[0] = f.pos[0].wrapping_add(mx as i16);
        f.pos[2] = f.pos[2].wrapping_add(mz as i16);
        f.m_t[0] += mx;
        f.m_t[2] += mz;
        f.m_pos[0] = f.m_t[0] as i16;
        f.m_pos[2] = f.m_t[2] as i16;
        f.hips[0] += mx;
        f.hips[2] += mz;
        f.head[0] += mx;
        f.head[2] += mz;
        f.chest[0] += mx;
        f.chest[2] += mz;
    }
}

/// Facing flags and the automatic turn toward the opponent (armed by op 11 / F60 0x2000000).
pub fn auto_turn(f: &mut Fighter, opp_f60: u32) {
    let mut a2 = f.o_74 as i32;
    if a2.abs() >= 0x401 {
        f.flags[F5C] |= 0x40_0000
    } else {
        f.flags[F5C] &= !0x40_0000
    }
    a2 = wrap12(a2 + f.s_d2 as i32) as i32;
    if a2.abs() >= 0x2AB {
        f.flags[F5C] |= 0x400
    } else {
        f.flags[F5C] &= !0x400
    }
    if a2.abs() >= 0x556 {
        f.flags[F5C] |= 0x10_0000
    } else {
        f.flags[F5C] &= !0x10_0000
    }
    let mut a3 = 0;
    if opp_f60 & 0x400_0000 == 0 {
        let f5c = f.flags[F5C];
        if f5c & 0x2_0000 != 0 {
            let mut turn = false;
            if f5c & 0x800_0000 != 0 {
                if (f.s_10c as i32) < a2.abs() || f.s_10a & 0x80 != 0 {
                    a2 = wrap12(a2 - 0x800) as i32;
                    turn = true;
                }
            } else if a2.abs() < f.s_10c as i32 || f.s_10a & 0x80 != 0 {
                turn = true;
            }
            if turn {
                a3 = if a2.abs() < 0xB7 {
                    a2
                } else if a2 >= 0 {
                    0xB6
                } else {
                    -0xB6
                };
            }
        } else if f.flags[F60] & 0x200_0000 != 0 {
            let v1 = a2.abs();
            let mut go = v1 < 0x156;
            if !go && v1 >= 0x6AA {
                a2 = wrap12(a2 - 0x800) as i32;
                go = true;
            }
            if go {
                a3 = if a2.abs() < 0x23 {
                    a2
                } else if a2 >= 0 {
                    0x22
                } else {
                    -0x22
                };
            }
        }
    }
    if f.s_10a & 0x7F != 0 {
        f.s_10a -= 1;
        if f.s_10a & 0x7F == 0 {
            f.flags[F5C] &= !0x2_0000;
        }
    }
    if a3 != 0 {
        f.o_74 = wrap12(f.o_74 as i32 - a3);
        f.yaw = wrap12(f.yaw as i32 + a3);
        f.s_e4 = wrap12(f.s_e4 as i32 + a3);
    }
}

/// Ring walls: keep the body spheres (except the first) inside the stage box, set the contact bits
/// Ground under the hips and the ring-out test (from the fighter step): `+0x94` = 0 inside the ring,
/// the stage's drop outside; edge parts (nodes 2, 7, 10) over the edge raise the RINGOUT event or, on the
/// ground (or 0x200 below the ring), the ring-out KO (status 0x81). Then the OUT / AIR bits of F5C.
pub fn ground(f: &mut Fighter, stage: &crate::fight::stage::Stage) {
    let part = |f: &Fighter, n: usize| f.parts.get(n).copied().unwrap_or(f.m_t);
    let r = &stage.ring;
    let mut past = false;
    if f.flags[F60] & 0x1000_0000 == 0 && f.flags[F6C] & 0x10_0000 == 0 {
        let h = part(f, 4);
        f.o_94 = r.floor(h[0], h[2]) as i16;
        if f.o_94 != 0 {
            let (p2, p10, p7) = (part(f, 2), part(f, 10), part(f, 7));
            let (a, b, c) = (r.floor(p2[0], p2[2]), r.floor(p10[0], p10[2]), r.floor(p7[0], p7[2]));
            if a | b | c != 0 {
                if f.m_t[1] >= 0 && f.flags[F6C] & 0x10_0000 == 0 {
                    if a & b & c == 0 && f.flags[F58] & 0x100_0000 == 0 {
                        if f.flags[F5C] & 0x80 != 0 {
                            f.flags[F6C] |= 0x8_0000;
                        }
                        f.flags[F6C] |= 0x10_0000; // RINGOUT event (decide)
                    }
                    if f.flags[F5C] & 0x80 == 0 || f.m_t[1] >= 0x200 {
                        if f.o_00 == 1 {
                            f.o_c0 = -1;
                            f.o_00 = 0x81; // ring-out KO
                        }
                        f.o_ac = -1;
                        f.o_ae = -1;
                        f.flags[F5C] |= 0x8_0000;
                    }
                }
                f.flags[F60] |= 0x2000_0000; // past the edge
                past = true;
            }
        }
        if !past {
            f.o_94 = 0;
            f.flags[F60] &= !0x2000_0000;
        }
    }
    if f.o_94 != 0 {
        f.flags[F5C] |= 0x8000_0000;
    } else {
        f.flags[F5C] &= 0x7FFF_FFFF;
    }
    f.flags[F5C] &= 0xEFFB_A5FC;
    let g = f.o_94 as i32 - 0x66;
    if part(f, 7)[1] < g && part(f, 10)[1] < g && part(f, 4)[1] < g {
        f.flags[F5C] |= 0x4_0000; // AIR
    }
}

/// Ring walls: keep the body spheres inside the ring box. Returns (contact mask, push x, push z).
/// Missing walls still block in normal play; only fighters in a hit reaction (or a juggle on a ring-out
/// opponent) pass through open sides; nothing blocks once past the edge or rung out.
pub fn ring_walls(f: &mut Fighter, opp_status: u8, bodies: &[crate::formats::act_ani::Sphere], stage: &mut crate::fight::stage::Stage) -> (u32, i32, i32) {
    let (hx, hz) = (stage.ring.hx as i32, stage.ring.hz as i32);
    let w = &mut stage.walls;
    let f58 = f.flags[F58];
    let mut mask = 0xFu32;
    if f58 & 0x10_0000 != 0 {
    } else if f58 & 0x8000 != 0 || ((f58 as i32) < 0 && f.flags[F64] & 0x1000 != 0 && opp_status >= 0x81) {
        mask = (0..4).filter(|&i| w.present(i)).fold(0, |m, i| m | 1 << i);
    } else if f.flags[F60] & 0x2000_0000 != 0 || f.flags[F5C] & 0x8_0000 != 0 {
        mask = 0;
    }
    let (mut t0, mut t1, mut px, mut pz) = (0u32, 0u32, 0i32, 0i32);
    for s in bodies.iter().skip(1) {
        let Some(n) = f.parts.get(s.node as usize).copied() else { continue };
        let r = s.radius as i32;
        if n[0] >= 0 {
            let e = hx - (n[0] + r);
            if e < 0x40 {
                t1 |= 1;
            }
            if e < 0 && mask & 1 != 0 {
                t0 |= 1;
                px = px.min(e);
            }
        } else {
            let e = -hx - (n[0] - r);
            if e > -0x40 {
                t1 |= 2;
            }
            if e > 0 && mask & 2 != 0 {
                t0 |= 2;
                px = px.max(e);
            }
        }
        if n[2] >= 0 {
            let e = hz - (n[2] + r);
            if e < 0x40 {
                t1 |= 4;
            }
            if e < 0 && mask & 4 != 0 {
                t0 |= 4;
                pz = pz.min(e);
            }
        } else {
            let e = -hz - (n[2] - r);
            if e > -0x40 {
                t1 |= 8;
            }
            if e > 0 && mask & 8 != 0 {
                t0 |= 8;
                pz = pz.max(e);
            }
        }
    }
    f.flags[F6C] &= !0xFF;
    f.flags[F5C] &= !0x100_0000;
    if t0 != 0 {
        let (vx, vz) = (f.vel[0] as i32, f.vel[2] as i32);
        let bit = if vx.abs() >= vz.abs() {
            if vx > 0 {
                1
            } else if vx < 0 {
                2
            } else {
                0
            }
        } else if vz > 0 {
            4
        } else if vz < 0 {
            8
        } else {
            0
        };
        if t0 & bit != 0 {
            f.flags[F5C] |= 0x100_0000; // M_KABE: moving into the wall
        }
    }
    if f.flags[F60] & 0x2000_0000 == 0 {
        f.flags[F6C] |= t0 | (t1 << 4);
    }
    if t0 == 0 {
        return (0, 0, 0);
    }
    let (ax, az) = (px.abs().min(0x80), pz.abs().min(0x80));
    for (i, a) in [(0, ax), (1, ax), (2, az), (3, az)] {
        if t0 & (1 << i) != 0 && w.st[i] == 0 {
            w.pow[i] = (f.o_24 as i32 * a / 1280) as i16;
        }
    }
    (t0, px, pz)
}

/// Engine addition (simulation v14): an extra stage's solid parts (`stage::extra::Extra::solids`: Q3DM17's teleporter)
/// as walls seen from outside, the ring walls' test turned round: each body sphere but the first that
/// overlaps a part is pushed out of it, a box's along the axis it went in least, a disc's along the line from its
/// centre (`hypot`); per direction the deepest push of all spheres, as the ring walls take it. Who is
/// blocked is who an indestructible wall (hp −1) blocks: everyone but a fighter past the ring's edge or rung out (a
/// thrown victim and a fighter in a hit reaction too). The parts set no wall contact bits (F6C, M_KABE, the walls'
/// impact power): those name the ring's four sides, and the main game has no wall inside its ring. Returns (pushed, x, z).
pub fn solids(
    f: &Fighter,
    opp_status: u8,
    bodies: &[crate::formats::act_ani::Sphere],
    stage: &crate::fight::stage::Stage,
    g: crate::fight::version::Gates,
) -> (bool, i32, i32) {
    let Some(e) = crate::fight::stage::extra::by_stage(stage.id) else { return (false, 0, 0) };
    let f58 = f.flags[F58];
    let reacting = f58 & 0x8000 != 0 || ((f58 as i32) < 0 && f.flags[F64] & 0x1000 != 0 && opp_status >= 0x81);
    if f58 & 0x10_0000 == 0 && !reacting && (f.flags[F60] & 0x2000_0000 != 0 || f.flags[F5C] & 0x8_0000 != 0) {
        return (false, 0, 0);
    }
    // the deepest push per direction: −x, +x, −z, +z
    let mut push = [0i32; 4];
    let mut hit = false;
    for p in e.solids(g) {
        for b in bodies.iter().skip(1) {
            let Some(n) = f.parts.get(b.node as usize).copied() else { continue };
            let r = b.radius as i32;
            let (dx, dz) = (n[0] - p.centre[0], n[2] - p.centre[1]);
            let (mx, mz) = if p.disc {
                let reach = p.half[0] + r;
                if dx.abs() >= reach || dz.abs() >= reach {
                    continue;
                }
                let d = hypot(dx, dz) as i32;
                if d >= reach {
                    continue;
                }
                if d == 0 { (0, -reach) } else { (dx * (reach - d) / d, dz * (reach - d) / d) }
            } else {
                let (ex, ez) = (p.half[0] + r - dx.abs(), p.half[1] + r - dz.abs());
                if ex <= 0 || ez <= 0 {
                    continue;
                }
                if ex <= ez { (if dx >= 0 { ex } else { -ex }, 0) } else { (0, if dz >= 0 { ez } else { -ez }) }
            };
            hit = true;
            push = [push[0].min(mx), push[1].max(mx), push[2].min(mz), push[3].max(mz)];
        }
    }
    if crate::base::trace::flag("SOLIDDBG") && hit && !crate::base::trace::is_muted() {
        eprintln!("solids: push {push:?} at {:?}", f.m_t);
    }
    (hit, push[0] + push[1], push[2] + push[3])
}

/// Engine addition (the team battle T7, simulation v25): an extra stage's chamfered corners (`Extra::chamfers`: Q3DM6's
/// courtyard) as walls, the ring walls' test on a 45° face: each body sphere but the first that reaches
/// over a face (its centre's distance to the face's line less than its radius; √2 as 181 / 128) is pushed back
/// along the face's normal, per face by the deepest sphere, all of it in the frame as the ring walls push (the ring walls cap
/// only the walls' impact power at 0x80). Who is blocked is who `solids` blocks; no wall contact bits (they name the ring's four sides). Returns
/// (pushed, x, z).
pub fn chamfers(
    f: &Fighter,
    opp_status: u8,
    bodies: &[crate::formats::act_ani::Sphere],
    stage: &crate::fight::stage::Stage,
    g: crate::fight::version::Gates,
) -> (bool, i32, i32) {
    let Some(e) = crate::fight::stage::extra::by_stage(stage.id) else { return (false, 0, 0) };
    let f58 = f.flags[F58];
    let reacting = f58 & 0x8000 != 0 || ((f58 as i32) < 0 && f.flags[F64] & 0x1000 != 0 && opp_status >= 0x81);
    if f58 & 0x10_0000 == 0 && !reacting && (f.flags[F60] & 0x2000_0000 != 0 || f.flags[F5C] & 0x8_0000 != 0) {
        return (false, 0, 0);
    }
    chamfer_push(f, bodies, e, g, (0, 0))
}

/// The push of `f`'s body spheres but the first (at `f.parts`, moved by `off` (x, z)) out of extra stage `e`'s
/// chamfers (by `g`): per face its deepest sphere, half of √2 times its depth per axis along the face's normal.
/// Returns (any face touched, x, z).
fn chamfer_push(
    f: &Fighter,
    bodies: &[crate::formats::act_ani::Sphere],
    e: &crate::fight::stage::extra::Extra,
    g: crate::fight::version::Gates,
    off: (i32, i32),
) -> (bool, i32, i32) {
    let (mut px, mut pz, mut hit) = (0i32, 0i32, false);
    for ch in e.chamfers(g) {
        // √2 times the depth along the unit normal (n has length √2): n · p − c + r·√2; per axis half of it
        let deepest = bodies
            .iter()
            .skip(1)
            .filter_map(|b| f.parts.get(b.node as usize).map(|n| ch.n[0] * (n[0] + off.0) + ch.n[1] * (n[2] + off.1) - ch.c + ((b.radius as i32 * 181) >> 7)))
            .max()
            .unwrap_or(0);
        if deepest > 0 {
            hit = true;
            px -= ch.n[0] * ((deepest + 1) / 2);
            pz -= ch.n[1] * ((deepest + 1) / 2);
        }
    }
    (hit, px, pz)
}

/// Engine addition (the team battle T7, simulation v25): the push that puts every body sphere but the first of `f` (at
/// `f.parts`) its radius inside the ring box's standing walls (`Walls::present`) and an extra stage's chamfers, all in
/// one frame and for everyone (no exemption): for a body posed from another engine's proxy (a throw across the
/// engines), whose own spheres the proxy's walls never saw. Returns (x, z).
pub fn inside(f: &Fighter, bodies: &[crate::formats::act_ani::Sphere], stage: &crate::fight::stage::Stage, g: crate::fight::version::Gates) -> (i32, i32) {
    let (hx, hz) = (stage.ring.hx as i32, stage.ring.hz as i32);
    let w = &stage.walls;
    // the deepest push per direction: −x, +x, −z, +z
    let mut push = [0i32; 4];
    if stage.ring.kind == 0 {
        for b in bodies.iter().skip(1) {
            let Some(n) = f.parts.get(b.node as usize) else { continue };
            let r = b.radius as i32;
            if w.present(0) {
                push[0] = push[0].min(hx - (n[0] + r));
            }
            if w.present(1) {
                push[1] = push[1].max(-hx - (n[0] - r));
            }
            if w.present(2) {
                push[2] = push[2].min(hz - (n[2] + r));
            }
            if w.present(3) {
                push[3] = push[3].max(-hz - (n[2] - r));
            }
        }
    }
    let (px, pz) = (push[0] + push[1], push[2] + push[3]);
    // the chamfers, on the body moved by the box's push
    let Some(e) = crate::fight::stage::extra::by_stage(stage.id) else { return (px, pz) };
    let (_, cx, cz) = chamfer_push(f, bodies, e, g, (px, pz));
    (px + cx, pz + cz)
}

/// For the wall push: move the model and the base position.
pub fn shift(f: &mut Fighter, dx: i32, dz: i32) {
    f.pos[0] = f.pos[0].wrapping_add(dx as i16);
    f.pos[2] = f.pos[2].wrapping_add(dz as i16);
    f.m_t[0] += dx;
    f.m_t[2] += dz;
    f.m_pos[0] = f.m_t[0] as i16;
    f.m_pos[2] = f.m_t[2] as i16;
    for p in f.parts.iter_mut() {
        p[0] += dx;
        p[2] += dz;
    }
    // (below simulation version 13 head_w is never computed: it only drifts with these shifts, read by nothing)
    f.head_w[0] += dx;
    f.head_w[2] += dz;
}

#[cfg(test)]
mod tests {
    /// (T7) A body sphere over Q3DM6's north-east chamfer is pushed back along its normal, half the depth (times √2
    /// for the radius) per axis, all in the frame; inside, or the first sphere, no push; Q3DM17 has no chamfers.
    #[test]
    fn a_sphere_over_a_chamfer_is_pushed_back_along_its_normal() {
        use crate::formats::act_ani::Sphere;
        let mut stage = crate::fight::stage::Stage::default();
        stage.id = 14;
        let mut f = crate::fight::fighter::Fighter::new(0, 0);
        let bodies = [Sphere { group: 0, node: 0, radius: 500 }, Sphere { group: 0, node: 1, radius: 64 }];
        // the face x + z = 3024: a centre at x + z = 3000 with radius 64: (3000 − 3024 + 64·181 / 128) / 2 = 33 per axis
        f.parts = vec![[1500, 0, 1500], [1500, 0, 1500]];
        assert_eq!(super::chamfers(&f, 1, &bodies, &stage, crate::fight::version::Gates::of(25)), (true, -33, -33));
        f.parts[1] = [2000, 0, 1500];
        assert_eq!(super::chamfers(&f, 1, &bodies, &stage, crate::fight::version::Gates::of(25)), (true, -283, -283));
        f.parts[1] = [1000, 0, 1000];
        assert_eq!(super::chamfers(&f, 1, &bodies, &stage, crate::fight::version::Gates::of(25)), (false, 0, 0));
        stage.id = 13;
        f.parts[1] = [2000, 0, 1500];
        assert_eq!(super::chamfers(&f, 1, &bodies, &stage, crate::fight::version::Gates::of(25)), (false, 0, 0));
        // `inside`: Q3DM6's box (2240 × 1680, walls on all sides) and chamfers; a sphere 100 past the north wall comes
        // back by 164 (100 + its radius), then the chamfer takes what is left over its face
        stage.id = 14;
        stage.ring.hx = 2240;
        stage.ring.hz = 1680;
        stage.walls.hp = [-1; 4];
        f.parts[1] = [0, 0, 1780];
        assert_eq!(super::inside(&f, &bodies, &stage, crate::fight::version::Gates::of(25)), (0, -164));
        f.parts[1] = [1500, 0, 1780];
        // after the box: (1500, 1616): 3116 − 3024 + 90 = 182, 91 per axis
        assert_eq!(super::inside(&f, &bodies, &stage, crate::fight::version::Gates::of(25)), (-91, -164 - 91));
        stage.walls.hp = [0; 4];
        stage.id = 13;
        assert_eq!(super::inside(&f, &bodies, &stage, crate::fight::version::Gates::of(25)), (0, 0));
    }

    #[test]
    fn hypot_matches_the_game() {
        // the game's integer hypot (verified against the game's code under emulation)
        for (a, b, r) in [(3, 4, 5), (600, 800, 999), (-2944, 2944, 4163), (12000, -9000, 14998), (1000, 0, 1000), (1, 1, 1)] {
            assert_eq!(super::hypot(a, b), r, "hypot({a}, {b})");
        }
    }

    #[test]
    fn vector_normal_matches_the_game() {
        // the game's normalisation, with the reciprocal square root table (skipped without the export)
        let Some(tables) = crate::formats::tables::export_tables() else { return };
        let rsqrt = tables.rsqrt_fn();
        for (v, r) in [
            ([40, 0, 30], [3278, 0, 2459]),
            ([90, 0, 120], [2470, 0, 3293]),
            ([1000, 0, 0], [4095, 0, 0]),
            ([3, 0, 4], [2457, 0, 3276]),
            ([-200, 0, 7], [-4099, 0, 143]),
            ([10000, 0, -20000], [1832, 0, -3666]),
        ] {
            assert_eq!(super::vector_normal_ss(v, &rsqrt), r, "{v:?}");
        }
    }
}
