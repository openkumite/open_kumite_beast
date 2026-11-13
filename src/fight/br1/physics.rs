//! The imported game's movement: the ring and ground, the velocity sources, gravity and landing, the automatic turn, the
//! position integration with the world walk, the push-apart and the ring walls.
//!
//! State: none of its own: updates the serialised `Br1Fighter`s and `Walls`
//!
//! The main game's routines with the imported game's offsets and its differences: two ring records per stage (the hips'
//! and the edge parts'), no wall-shake offsets, a push share of 1/2, the push along the angle between the roots.
//! Angles are 4096 per turn; +Y is down; forward of yaw `a` is (sin a, −cos a) in X / Z.

use super::object::Br1Fighter;
use super::pose::{self, IDENTITY, World};
use super::res::Br1Res;
use crate::fight::physics::{Trig, hypot, wrap12};
use crate::formats::tables::Tables;
use serde::{Deserialize, Serialize};

/// The hips' ring records (inner), 8 bytes per stage (`G.94`): {u16 kind, s16 drop, u16 hx, u16 hz}.
pub const RING_HIPS: &str = "stage.ring_hips";
/// The edge parts' ring records (outer, 64 units larger).
pub const RING_EDGE: &str = "stage.ring_edge";

/// A ring record: kind 0 a box ±hx / ±hz, else a circle (the second word = r²); `drop` = the ground height
/// outside.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Ring {
    pub w0: u32,
    pub w1: u32,
}

impl Ring {
    /// Stage `stage`'s record of the table `table` (a stage before 0 or past the table: the bytes there, of whichever
    /// table holds them, `Tables::abs_u32`).
    pub fn of(tables: &Tables, table: &str, stage: i8) -> Self {
        let a = tables.table(table).origin().wrapping_add((stage as i32 * 8) as u32);
        Self { w0: tables.abs_u32(a), w1: tables.abs_u32(a.wrapping_add(4)) }
    }
    /// The half sizes hx, hz of a box.
    pub fn half(&self) -> (i32, i32) {
        ((self.w1 & 0xFFFF) as i32, (self.w1 >> 16) as i32)
    }
}

/// 0 inside the ring, the record's drop outside (and for `a3` ≥ 10).
pub fn ground(x: i32, z: i32, r: Ring, a3: i32) -> i32 {
    let drop = (r.w0 as i32) >> 16;
    if a3 >= 10 {
        return drop;
    }
    if r.w0 & 0xFFFF != 0 {
        let d = (r.w1 as i32).wrapping_sub(x.wrapping_mul(x)).wrapping_sub(z.wrapping_mul(z));
        return if d < 0 { drop } else { 0 };
    }
    let (hx, hz) = r.half();
    if hz < z.wrapping_abs() || hx < x.wrapping_abs() { drop } else { 0 }
}

/// The ring walls' state (the stage block + 0x44 .., stage S8's; the decision and the walls write it): +0x44, the
/// four walls' hp (+0x5A, < 0 = no wall), the hit strength (+0x6A) and its lock (+0x72).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Walls {
    pub w44: i16,
    pub hp: [i16; 4],
    pub hit: [i16; 4],
    pub lock: [i16; 4],
}

/// (speed·sin a, −speed·cos a) rounded, plus the 1/128 remainders kept in `frac` (F.9A / F.9E), in
/// whole units.
fn polar_step(t: &Trig, a: i32, speed: i32, frac: (&mut i16, &mut i16)) -> (i32, i32) {
    if speed == 0 {
        return (0, 0);
    }
    let x = (speed.wrapping_mul(t.sin(a)).wrapping_add(0x800) >> 12).wrapping_add(*frac.0 as i32).wrapping_add(0x40);
    let z = ((-speed).wrapping_mul(t.cos(a)).wrapping_add(0x800) >> 12).wrapping_add(*frac.1 as i32).wrapping_add(0x40);
    *frac.0 = (x - (x & -0x80) - 0x40) as i16;
    *frac.1 = (z - (z & -0x80) - 0x40) as i16;
    (x >> 7, z >> 7)
}

/// F.92 / F.96 = the polar step (0 for speed 0).
pub fn polar_set(f: &mut Br1Fighter, t: &Trig, a: i32, speed: i32) {
    let [_, fx, _, fz, _] = &mut f.f.f98;
    let (x, z) = polar_step(t, a, speed, (fx, fz));
    f.f.vel[0] = x as i16;
    f.f.vel[2] = z as i16;
}

/// F.92 / F.96 += the polar step.
pub fn polar_add(f: &mut Br1Fighter, t: &Trig, a: i32, speed: i32) {
    if speed == 0 {
        return;
    }
    let [_, fx, _, fz, _] = &mut f.f.f98;
    let (x, z) = polar_step(t, a, speed, (fx, fz));
    f.f.vel[0] = f.f.vel[0].wrapping_add(x as i16);
    f.f.vel[2] = f.f.vel[2].wrapping_add(z as i16);
}

/// A speed toward 0 by its decay: passing 0 stops both.
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

/// `v · step >> 4`, rounded toward 0 (the frame step is 1/16 frames).
fn scale(v: i32, step: i16) -> i32 {
    let p = v.wrapping_mul(step as i32);
    (if p < 0 { p + 15 } else { p }) >> 4
}

/// This frame's velocity: source 1 (F.82 / F.84 along the facing), the push (F.BA /
/// F.AA), the vertical, all scaled by the frame step F.80.
pub fn velocity(f: &mut Br1Fighter, t: &Trig) {
    let a = (f.f.yaw as i32).wrapping_sub(f.anim.yaw as i32).wrapping_add(f.f.v_angle as i32);
    polar_set(f, t, a, f.f.v_speed as i32);
    let (mut sp, mut dc) = (f.f.v_speed, f.f.v_decay);
    decay(&mut sp, &mut dc);
    (f.f.v_speed, f.f.v_decay) = (sp, dc);
    if f.f.push_speed != 0 {
        polar_add(f, t, f.f.knock_dir as i32, f.f.push_speed as i32);
        let p = (f.f.push_speed as i32).wrapping_mul(f.f.push_decay as i32);
        f.f.push_speed = ((if p < 0 { p + 0x7F } else { p }) >> 7) as i16;
    }
    vertical(f, t);
    let step = f.f.step;
    for v in f.f.vel.iter_mut() {
        *v = scale(*v as i32, step) as i16;
    }
}

/// Gravity on F.88 (y down), the launch source (F.8E / F.90 along the facing, while airborne) and this
/// frame's F.94.
pub fn vertical(f: &mut Br1Fighter, t: &Trig) {
    let mut vy = f.f.vy as i32;
    let mut g = f.f.gravity as i32;
    let y = f.world(0)[1];
    let floor = f.f.floor as i32;
    let mut out = 0;
    if (f.f.f54 as i32) < 0 && y < floor {
        g = 0xC0;
        f.f.gravity = 0xC0;
    }
    if y == floor && vy | g == 0 {
        f.f.vel[1] = 0;
        f.f.f98[2] = 0;
        return;
    }
    if f.f.f58 & 0x2000 == 0 && vy | g != 0 {
        let a = (f.anim.yaw_base as i32).wrapping_add(f.anim.turn as i32).wrapping_add(f.f.launch_angle as i32);
        polar_add(f, t, a, f.f.launch_speed as i32);
        let (mut sp, mut dc) = (f.f.launch_speed, f.f.v_decay);
        decay(&mut sp, &mut dc);
        (f.f.launch_speed, f.f.v_decay) = (sp, dc);
    }
    if f.f.f58 & 0x800 == 0 && vy <= 0 {
        out = vy >> 7;
        vy = vy.wrapping_add(scale(g, f.f.step));
        f.f.vy = vy as i16;
    } else if f.f.f58 & 0x1000 == 0 && vy > 0 {
        out = vy >> 7;
        vy = vy.wrapping_add(scale(g, f.f.step));
        if vy < -0x8000 {
            vy = -0x8000;
        }
        f.f.vy = vy as i16;
    }
    f.f.vel[1] = out as i16;
}

/// The airborne flag (F.54 0x80) and the landing: node 0 at or below the floor ends the fall.
pub fn landing(f: &mut Br1Fighter) {
    f.f.f54 &= 0xFFFF_FE7F;
    let y = f.world(0)[1];
    let floor = f.f.floor as i32;
    if y == floor && f.f.vy as i32 | f.f.gravity as i32 == 0 {
        return;
    }
    f.f.f54 |= 0x80;
    if y >= floor && f.f.f50 & 0x100_0000 == 0 {
        f.f.gravity = 0;
        f.f.vy = 0;
        f.f.f64 |= 0x8_0000;
        f.f.f54 &= 0xF7FF_FF7F;
        let cur = f.cur as usize & 1;
        if let Some(n) = f.forms[cur].md.first_mut() {
            n.wt[1] = floor;
        }
        f.anim.pos[1] = floor as i16;
    } else if f.f.vy > 0 {
        f.f.f54 |= 0x100;
    }
}

/// The facing flags (F.54 0x400000 opponent behind, 0x400 / 0x100000 turned away) and the automatic turn
/// toward the opponent (armed by op 0x11, F.54 0x20000; away with F.58 8, op 0x28), or the slow one of F.58 0x2000000.
pub fn auto_turn(f: &mut Br1Fighter, opp_f58: u32) {
    let o = f.f.opp_angle as i32;
    if o.abs() >= 0x400 {
        f.f.f54 |= 0x40_0000;
    } else {
        f.f.f54 &= !0x40_0000;
    }
    let mut a2 = wrap12(o + f.anim.yaw as i32) as i32;
    if a2.abs() >= 0x4E4 {
        f.f.f54 |= 0x400;
    } else {
        f.f.f54 &= !0x400;
    }
    if a2.abs() >= 0x555 {
        f.f.f54 |= 0x10_0000;
    } else {
        f.f.f54 &= !0x10_0000;
    }
    let mut a3 = 0;
    if opp_f58 & 0x400_0000 == 0 {
        if f.f.f54 & 0x2_0000 != 0 {
            let turn = if f.f.f58 & 8 != 0 {
                if a2.abs() < 0x2AB {
                    false
                } else {
                    a2 = wrap12(a2 - 0x800) as i32;
                    true
                }
            } else {
                a2.abs() < 0x555 || f.s.auto_turn & 0x80 != 0
            };
            if turn {
                a3 = if a2.abs() < 0x5C {
                    a2
                } else if a2 >= 0 {
                    0x5B
                } else {
                    -0x5B
                };
            }
        } else if f.f.f58 & 0x200_0000 != 0 {
            let mut v1 = a2.abs();
            let go = if v1 < 0x2AB {
                true
            } else if v1 < 0x555 {
                false
            } else {
                a2 = wrap12(a2 - 0x800) as i32;
                v1 = a2.abs();
                true
            };
            if go {
                if v1 < 0x23 {
                    if v1 >= 0x17 {
                        a3 = a2;
                    }
                } else {
                    a3 = if a2 >= 0 { 0x22 } else { -0x22 };
                }
            }
        }
    }
    // the request's timer
    let v = f.s.auto_turn;
    if v & 0x7F != 0 {
        f.s.auto_turn = v.wrapping_sub(1);
        if f.s.auto_turn & 0x7F == 0 {
            f.f.f54 &= !0x2_0000;
        }
    }
    if a3 != 0 {
        f.f.opp_angle = wrap12(f.f.opp_angle as i32 - a3);
        f.f.yaw = wrap12(f.f.yaw as i32 + a3);
        f.anim.turn = wrap12(f.anim.turn as i32 + a3);
    }
}

/// The world of every linked node of the current form (`Br1Res::linked`) from the local ones (node 0's
/// world is its local, set by the mover), and the objects 0 / 2 at their node: `node.wt + node.wm · offset >> 12`
/// (IR saturated).
pub fn walk(f: &mut Br1Fighter, res: &Br1Res) {
    let cur = f.cur as usize & 1;
    let skel = &res.skels[cur];
    let form = &mut f.forms[cur];
    // (the link mask covers nodes 0..31: the imported game's skeletons have ≤ 26)
    let n = form.md.len().min(skel.nodes()).min(32);
    let (mut m, mut t16) = ([IDENTITY; 32], [[0; 3]; 32]);
    for (k, d) in form.md[..n].iter().enumerate() {
        m[k] = d.lm;
        t16[k] = d.lt;
    }
    let w = pose::world(skel, &m[..n], &t16[..n]);
    let mask = res.linked[cur];
    for (n, (d, (wm, wt))) in form.md.iter_mut().zip(w).enumerate() {
        if n > 0 && mask & (1 << n) != 0 {
            d.wm = wm;
            d.wt = wt;
        }
    }
    for (slot, k) in [0usize, 2].into_iter().enumerate() {
        let node = res.anm.object_node(cur, k);
        if let Some(d) = usize::try_from(node).ok().and_then(|i| form.md.get(i)) {
            let w: World = (d.wm, d.wt);
            f.objs[cur][slot] = pose::world_step(&w, &IDENTITY, res.anm.object_offset(cur, k)).1;
        }
    }
}

/// Node 0's world = its local (the matrix words copied, the translation sign-extended).
fn root_world(f: &mut Br1Fighter) {
    let cur = f.cur as usize & 1;
    if let Some(d) = f.forms[cur].md.first_mut() {
        d.wm = d.lm;
        d.wt = d.lt.map(|v| v as i16 as i32);
    }
}

/// The model shift: every linked node's world translation (`Br1Res::linked`; and the objects') moved by
/// (dx, −dy, dz).
pub fn shift(f: &mut Br1Fighter, res: &Br1Res, dx: i32, dy: i32, dz: i32) {
    let cur = f.cur as usize & 1;
    let mask = res.linked[cur];
    for (n, d) in f.forms[cur].md.iter_mut().enumerate() {
        if n < 32 && (n == 0 || mask & (1 << n) != 0) {
            d.wt = [d.wt[0].wrapping_add(dx), d.wt[1].wrapping_sub(dy), d.wt[2].wrapping_add(dz)];
        }
    }
    for o in f.objs[cur].iter_mut() {
        *o = [o[0].wrapping_add(dx), o[1].wrapping_sub(dy), o[2].wrapping_add(dz)];
    }
}

/// Node 0's local translation = its world's low halfwords.
fn root_from_world(f: &mut Br1Fighter) {
    let cur = f.cur as usize & 1;
    if let Some(d) = f.forms[cur].md.first_mut() {
        d.lt = d.wt.map(|v| v as i16 as i32);
    }
}

/// One fighter's move in the pair's integration: node 0 at the base position S.DC (or the leader's, copied in: a throw victim),
/// the base moved by the velocity (`advance`), the animation's root offset (S.E4..) turned by the facing S.D8 into
/// the velocity, node 0 = base + velocity, the world walk.
fn place(f: &mut Br1Fighter, res: &Br1Res, advance: bool, leader: Option<[i16; 3]>) {
    let t = &res.trig;
    let [rx, ry, rz] = f.anim.root.map(|v| v as i32);
    let yaw = f.anim.yaw_base as i32;
    if let Some(p) = leader {
        f.anim.pos = p;
    }
    let cur = f.cur as usize & 1;
    let pos = f.anim.pos;
    if let Some(d) = f.forms[cur].md.first_mut() {
        d.lt = pos.map(|v| v as i32);
    }
    if advance {
        for k in 0..3 {
            f.anim.pos[k] = f.anim.pos[k].wrapping_add(f.f.vel[k]);
        }
    }
    let (c, s) = (t.cos(yaw), t.sin(yaw));
    f.f.vel[1] = f.f.vel[1].wrapping_add((ry >> 4) as i16);
    f.f.vel[0] = f.f.vel[0].wrapping_add((rx.wrapping_mul(c).wrapping_sub(rz.wrapping_mul(s)) >> 16) as i16);
    f.f.vel[2] = f.f.vel[2].wrapping_add((rx.wrapping_mul(s).wrapping_add(rz.wrapping_mul(c)) >> 16) as i16);
    let vel = f.f.vel;
    if let Some(d) = f.forms[cur].md.first_mut() {
        for k in 0..3 {
            d.lt[k] = (d.lt[k] as i16).wrapping_add(vel[k]) as i32;
        }
    }
    root_world(f);
    walk(f, res);
}

/// The pair integration's move of one fighter (engine addition, the import S7: a fighter of a mixed pair, whose opponent is
/// the main game's).
pub fn integrate_one(f: &mut Br1Fighter, res: &Br1Res) {
    place(f, res, true, None);
}

/// A push of (`dx`, `dz`) from outside the core (engine addition, the import S7: the push-apart and the walls of
/// a mixed pair): the model shifted, the base position moved, the root re-derived (as `after_hits` moves a fighter).
pub fn push_out(f: &mut Br1Fighter, res: &Br1Res, dx: i32, dz: i32) {
    shift(f, res, dx, 0, dz);
    f.anim.pos[0] = f.anim.pos[0].wrapping_add(dx as i16);
    f.anim.pos[2] = f.anim.pos[2].wrapping_add(dz as i16);
    root_from_world(f);
}

/// Engine addition (the import S7, the cross-engine throws): core `f` posed by a proxy of another engine: nodes
/// 1..18's direction / twist records `recs`, nodes 1 / 4's translations `t1` / `t4` (1/16 units, S.F4 / S.104), the
/// base position `pos` (S.DC), the model's translation `model` (node 0), the yaw `yaw` (F.46) and facing `yaw_base`
/// (S.D8); the local matrices as the animation step builds them (node 0) and the world walk.
#[allow(clippy::too_many_arguments)]
pub fn puppet(f: &mut Br1Fighter, res: &Br1Res, recs: &[pose::Rec], t1: [i16; 3], t4: [i16; 3], pos: [i16; 3], model: [i32; 3], yaw: i16, yaw_base: i16) {
    let cur = f.cur as usize & 1;
    let skel = &res.skels[cur];
    for (r, s) in f.forms[cur].rec.iter_mut().zip(recs).take(pose::BASE_NODES) {
        *r = *s;
    }
    (f.anim.t1, f.anim.t4, f.anim.pos, f.anim.yaw_base, f.f.yaw) = (t1, t4, pos, yaw_base, yaw);
    let rs = res.tables.rsqrt_fn();
    let lm = pose::local_matrices(&f.forms[cur].rec, skel, &res.trig, &rs);
    let lt = pose::local_translations(&f.anim, skel, model, false);
    let root = pose::root_matrix([f.f.pitch, yaw, f.f.roll], &res.trig);
    let form = &mut f.forms[cur];
    for (n, d) in form.md.iter_mut().enumerate().take(pose::BASE_NODES) {
        if n == 0 {
            (d.lm, d.lt) = (root, model);
        } else {
            d.lm = lm[n];
            if n == 1 || n == 4 {
                d.lt = lt[n];
            }
        }
    }
    root_world(f);
    walk(f, res);
}

/// Both fighters moved; a throw victim (F.64 0x60000) follows its thrower's base position.
pub fn integrate(fs: &mut [Br1Fighter; 2], res: [&Br1Res; 2]) {
    for k in 0..2 {
        if fs[k].f.f64 & 0x6_0000 != 0 {
            fs[k].f.f64 &= 0xFFFD_FFFF;
            let o = 1 - k;
            place(&mut fs[o], res[o], true, None);
            let p = fs[o].anim.pos;
            place(&mut fs[k], res[k], false, Some(p));
            return;
        }
    }
    place(&mut fs[0], res[0], true, None);
    place(&mut fs[1], res[1], true, None);
}

/// MVMVA with a rotation about Y by `th` (sf = 1, IR saturated) on an s16 vector.
fn rot_y(t: &Trig, th: i32, v: [i32; 3]) -> [i32; 3] {
    let (c, s) = (t.cos(th) as i64, t.sin(th) as i64);
    let v = v.map(|x| x as i16 as i64);
    let sat = |x: i64| (x >> 12).clamp(-0x8000, 0x7FFF) as i32;
    [sat(c * v[0] + s * v[2]), sat(0x1000 * v[1]), sat(-s * v[0] + c * v[2])]
}

/// Two spheres overlap (each axis first, then the squared distance).
fn overlap(r: i32, a: [i32; 3], b: [i32; 3]) -> bool {
    let d = [b[0].wrapping_sub(a[0]), b[1].wrapping_sub(a[1]), b[2].wrapping_sub(a[2])];
    if d.iter().any(|v| v.wrapping_abs() >= r) {
        return false;
    }
    let rr = r.wrapping_mul(r);
    let mut s = d[0].wrapping_mul(d[0]);
    if s >= rr {
        return false;
    }
    s = s.wrapping_add(d[1].wrapping_mul(d[1]));
    if s >= rr {
        return false;
    }
    s.wrapping_add(d[2].wrapping_mul(d[2])) < rr
}

/// The distance and height to the opponent from the body objects 0 / 2 (F.6E, F.70), then
/// the push-apart of overlapping body spheres along θ (the angle between the roots).
pub fn push_apart(fs: &mut [Br1Fighter; 2], res: [&Br1Res; 2], th: i32) {
    let t = &res[0].trig;
    let s16 = |v: i32| v as i16 as i32;
    let obj = |f: &Br1Fighter, k: usize| f.objs[f.cur as usize & 1][k];
    let (a, b) = (&fs[0], &fs[1]);
    let d = [
        s16(obj(b, 0)[0].wrapping_add(obj(b, 1)[0]).wrapping_sub(obj(a, 0)[0].wrapping_add(obj(a, 1)[0])) / 2),
        s16(obj(b, 1)[1].wrapping_sub(obj(a, 1)[1])),
        s16(obj(b, 0)[2].wrapping_add(obj(b, 1)[2]).wrapping_sub(obj(a, 0)[2].wrapping_add(obj(a, 1)[2])) / 2),
    ];
    let r = rot_y(t, th, d);
    let dist = (r[2].wrapping_abs()) as i16;
    fs[0].f.opp_dist = dist;
    fs[1].f.opp_dist = dist;
    fs[0].f.opp_dy = r[1] as i16;
    fs[1].f.opp_dy = (r[1] as i16).wrapping_neg();
    fs[0].f.f54 &= 0xFF7F_FFFF;
    fs[1].f.f54 &= 0xFF7F_FFFF;
    // (node 0's link word MD+0x40, also tested here, is 0 in every traced frame: nothing in the fight sets it)
    if r[2].abs() >= 0x181 || r[1].abs() >= 0x141 || (fs[0].f.f58 | fs[1].f.f58) & 0x20_0000 != 0 {
        return;
    }
    let lying = |f: &Br1Fighter| f.f.f50 & 0x3_0000 != 0 && f.f.f50 & 0x800_0000 == 0;
    let down = lying(&fs[0]) || lying(&fs[1]);
    let (lo, na, nb) = if down { (0, 1, 1) } else { (1, fs[0].f.body_spheres as i8 as i32, fs[1].f.body_spheres as i8 as i32) };
    let sph = |k: usize| &res[k].anm.spheres[fs[k].cur as usize & 1];
    let node = |f: &Br1Fighter, n: i32| f.form().md.get(n as usize).map_or([0; 3], |d| d.wt);
    let mut pen = 0i32;
    for i in lo..na {
        let Some(sa) = sph(0).get(i as usize) else { continue };
        let pa = node(&fs[0], sa.node as i8 as i32);
        for j in lo..nb {
            let Some(sb) = sph(1).get(j as usize) else { continue };
            let pb = node(&fs[1], sb.node as i8 as i32);
            if !overlap(sa.radius as i32 + sb.radius as i32, pa, pb) {
                continue;
            }
            let (za, zb) = (rot_y(t, th, pa)[2], rot_y(t, th, pb)[2]);
            let v = zb.wrapping_add(sb.radius as i32).wrapping_sub(za.wrapping_sub(sa.radius as i32));
            if pen < v {
                pen = v;
            }
        }
    }
    if pen == 0 {
        return;
    }
    fs[0].f.f54 |= 0x80_0000;
    fs[1].f.f54 |= 0x80_0000;
    let pen = pen.min(200) / 2;
    let (l0, l1) = (fs[0].f.f54 & 0x8000 != 0, fs[1].f.f54 & 0x8000 != 0);
    let (wa, wb, sum) = if l0 || l1 {
        (l0 as i32, l1 as i32, l0 as i32 + l1 as i32)
    } else {
        let (wa, wb) = (fs[0].f.weight as i32, fs[1].f.weight as i32);
        (wa, wb, wa + wb)
    };
    let pen = pen << 7;
    for (k, w, sgn) in [(0usize, wb, -1i32), (1, wa, 1)] {
        let speed = super::script::div(pen.wrapping_mul(w), sum);
        let (f, r) = (&mut fs[k], res[k]);
        polar_set_out(f, t, th, speed, |f, x, z| {
            let (x, z) = (sgn.wrapping_mul(x), sgn.wrapping_mul(z));
            shift(f, r, x, 0, z);
            f.anim.pos[0] = f.anim.pos[0].wrapping_add(x as i16);
            f.anim.pos[2] = f.anim.pos[2].wrapping_add(z as i16);
            root_from_world(f);
        });
    }
}

/// The polar step into a local vector (the push-apart's): the step with `f`'s remainders, then `then(f, x, z)`.
fn polar_set_out(f: &mut Br1Fighter, t: &Trig, a: i32, speed: i32, then: impl FnOnce(&mut Br1Fighter, i32, i32)) {
    let [_, fx, _, fz, _] = &mut f.f.f98;
    let (x, z) = polar_step(t, a, speed, (fx, fz));
    then(f, x as i16 as i32, z as i16 as i32);
}

/// The body spheres (from the second) against the hips' ring box (`hips`, the stage's `RING_HIPS` record): the contact bits (F.64 low byte: the
/// walls touched | the walls near << 4, F.54 0x1000000 = moving into one) and the correction (x, z) back inside;
/// the walls' hit strength. Returns the walls touched.
pub fn walls(f: &mut Br1Fighter, res: &Br1Res, opp_status: u8, hips: Ring, w: &mut Walls) -> (u32, [i16; 2]) {
    walls_by(f, res, opp_status, hips, w, false)
}

/// `walls` with `radius` (engine addition, the team battle T7, simulation v25): each sphere's radius as its margin, as
/// the main game's ring walls have it, where the imported game's take the node number; an imported fighter on a stage whose
/// walls are the main game's records stops at them as a main-game fighter does, its body outside the stone.
pub fn walls_by(f: &mut Br1Fighter, res: &Br1Res, opp_status: u8, hips: Ring, w: &mut Walls, radius: bool) -> (u32, [i16; 2]) {
    let (hx, hz) = hips.half();
    let (hx, hz) = (hx as i16 as i32, hz as i16 as i32);
    let mut t0: u32 = 0xF;
    let f50 = f.f.f50;
    if f50 & 0x10_0000 == 0 {
        let held = f50 & 0x8000 != 0 || (f50 as i32) < 0 && f.f.f5c & 0x1000 != 0 && opp_status >= 0x81;
        if held {
            // in hit-stun (or attacking a KO'd opponent): only the walls present stop
            for k in 0..4 {
                if w.hp[k] < 0 {
                    t0 &= !(1 << k);
                }
            }
        } else if f.f.f58 & 0x2000_0000 != 0 || f.f.f54 & 0x8_0000 != 0 {
            t0 = 0;
        }
    }
    let (mut t1, mut t2) = (0u32, 0u32);
    let (mut t5, mut t6) = (0i32, 0i32);
    let n = f.f.body_spheres as i8 as i32;
    let sph = &res.anm.spheres[f.cur as usize & 1];
    for k in 1..n {
        let Some(s) = sph.get(k as usize) else { continue };
        // the margin is the sphere's node number (the radius was meant; `radius`: the radius)
        let n = s.node as i8 as i32;
        let p = f.form().md.get(n as usize).map_or([0; 3], |d| d.wt);
        let r = if radius { s.radius as i32 } else { n };
        let a2 = hx.wrapping_sub(p[0].wrapping_add(r));
        if a2 < 0x40 {
            t2 |= 1;
        }
        if a2 < 0 {
            if t0 & 1 != 0 {
                t1 |= 1;
                if a2 < t5 {
                    t5 = a2;
                }
            }
        } else {
            let a2 = hx.wrapping_neg().wrapping_sub(p[0].wrapping_sub(r));
            if a2 >= -0x3F {
                t2 |= 2;
            }
            if a2 > 0 && t0 & 2 != 0 {
                t1 |= 2;
                if t5 < a2 {
                    t5 = a2;
                }
            }
        }
        let a3 = hz.wrapping_sub(p[2].wrapping_add(r));
        if a3 < 0x40 {
            t2 |= 4;
        }
        if a3 < 0 {
            if t0 & 4 != 0 {
                t1 |= 4;
                if a3 < t6 {
                    t6 = a3;
                }
            }
        } else {
            let a3 = hz.wrapping_neg().wrapping_sub(p[2].wrapping_sub(r));
            if a3 >= -0x3F {
                t2 |= 8;
            }
            if a3 > 0 && t0 & 8 != 0 {
                t1 |= 8;
                if t6 < a3 {
                    t6 = a3;
                }
            }
        }
    }
    f.f.f64 &= 0xFFFF_FF00;
    f.f.f54 &= 0xFEFF_FFFF;
    if t1 != 0 {
        let (vx, vz) = (f.f.vel[0] as i32, f.f.vel[2] as i32);
        let into = if vx.abs() < vz.abs() {
            if vz > 0 {
                t1 & 4
            } else if vz < 0 {
                t1 & 8
            } else {
                0
            }
        } else if vx > 0 {
            t1 & 1
        } else if vx < 0 {
            t1 & 2
        } else {
            0
        };
        if into != 0 {
            f.f.f54 |= 0x100_0000;
        }
    }
    if f.f.f58 & 0x2000_0000 == 0 {
        f.f.f64 |= t1 | t2 << 4;
    }
    if t1 == 0 {
        return (0, [0, 0]);
    }
    let out = [t5 as i16, t6 as i16];
    let (t5, t6) = (t5.wrapping_abs().min(0x80), t6.wrapping_abs().min(0x80));
    let wt = f.f.weight as i32;
    for (k, m) in [(0usize, t5), (1, t5), (2, t6), (3, t6)] {
        if t1 & (1 << k) != 0 && w.lock[k] == 0 {
            // ·0x66666667 >> 41: / 1280, toward 0
            w.hit[k] = (wt.wrapping_mul(m) / 1280) as i16;
        }
    }
    (t1, out)
}

/// The angle between the roots (from node 0's world, before the push and the walls).
pub fn root_angle(fs: &[Br1Fighter; 2], t: &Trig) -> i32 {
    let (a, b) = (fs[0].world(0), fs[1].world(0));
    t.atan2(a[2].wrapping_sub(b[2]), b[0].wrapping_sub(a[0]))
}

/// The angle to the opponent from θ, the angle and distance to the stage centre.
pub fn angles(fs: &mut [Br1Fighter; 2], t: &Trig, th: i32) {
    fs[0].f.opp_angle = wrap12(th - fs[0].f.yaw as i32);
    fs[1].f.opp_angle = wrap12(((th + 0x800) & 0xFFF) - fs[1].f.yaw as i32);
    for f in fs.iter_mut() {
        let w = f.world(0);
        let a = t.atan2(w[2], w[0].wrapping_neg());
        f.f.centre_dist = hypot(w[2], w[0]) as i16;
        f.f.centre_angle = wrap12(a - f.f.yaw as i32);
    }
}

/// The pair's step after the hits: the hit flags, the combo counters, θ, the push-apart, the walls and their corrections
/// (a held pair moves together), the angles.
pub fn after_hits(fs: &mut [Br1Fighter; 2], res: [&Br1Res; 2], hips: Ring, w: &mut Walls) {
    after_hits_by(fs, res, hips, w, false);
}

/// `after_hits` with the walls' margin `radius` (`walls_by`; the team battle T7).
pub fn after_hits_by(fs: &mut [Br1Fighter; 2], res: [&Br1Res; 2], hips: Ring, w: &mut Walls, radius: bool) {
    // the hit flags F.54 0x20000000 / 0x40000000 end with their causes
    for f in fs.iter_mut() {
        if f.f.f50 & 0x4_8000 == 0 && f.f.f54 & 0x400_0000 == 0 {
            f.f.f54 &= 0xDFFF_FFFF;
        }
        if f.f.f50 & 0x1000_0000 == 0 && f.f.f54 & 0x400_0000 == 0 {
            f.f.f54 &= 0xBFFF_FFFF;
        }
    }
    if fs[0].f.f54 & 0x6000_0000 == 0 {
        fs[1].f.combo = 0;
    }
    if fs[1].f.f54 & 0x6000_0000 == 0 {
        fs[0].f.combo = 0;
    }
    let t = &res[0].trig;
    let th = root_angle(fs, t);
    push_apart(fs, res, th);
    let (st0, st1) = (fs[0].f.status, fs[1].f.status);
    let (s0, o0) = walls_by(&mut fs[0], res[0], st1, hips, w, radius);
    let (s1, o1) = walls_by(&mut fs[1], res[1], st0, hips, w, radius);
    let (a50, b50) = (fs[0].f.f50, fs[1].f.f50);
    let held = a50 & 0x4000_0000 != 0 && b50 & 0x10_0000 != 0 || b50 & 0x4000_0000 != 0 && a50 & 0x10_0000 != 0;
    let mv = |f: &mut Br1Fighter, r: &Br1Res, o: [i16; 2]| {
        shift(f, r, o[0] as i32, 0, o[1] as i32);
        f.anim.pos[0] = f.anim.pos[0].wrapping_add(o[0]);
        f.anim.pos[2] = f.anim.pos[2].wrapping_add(o[1]);
        root_from_world(f);
    };
    if held {
        if s0 | s1 != 0 {
            let o = [o0[0].wrapping_add(o1[0]), o0[1].wrapping_add(o1[1])];
            mv(&mut fs[0], res[0], o);
            mv(&mut fs[1], res[1], o);
        }
    } else {
        if s0 != 0 {
            mv(&mut fs[0], res[0], o0);
        }
        if s1 != 0 {
            mv(&mut fs[1], res[1], o1);
        }
    }
    angles(fs, t, th);
}
