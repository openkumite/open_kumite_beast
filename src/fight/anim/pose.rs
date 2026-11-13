//! The bones' local transforms from the pose records.
//!
//! State: none (pure functions of the fighter's pose)
//!
//! The bones' local transforms from the pose records (`Fighter::pose`), as the game builds them every frame:
//! nodes 1..16 base · swing · twist in 4.12 fixed point on the GTE,
//! hips 17 / 18 a Y rotation, connectors half the parent's swing and twist on their own base,
//! translations ANI bone offset + stretch. The result feeds the glTF skeleton in place of clips.
//! Node 2 (the head) is then turned by the head turn's accumulators (`move_script::head_turn`).

use crate::fight::anim::act_clips::Skel;
use crate::fight::fighter::{F54, F58, Fighter};
use crate::fight::physics::{Trig, vector_normal_ss};
use glam::{DMat3, DQuat, DVec3, Mat3, Quat, Vec3};

pub(crate) type M = [[i32; 3]; 3];

/// MVMVA with sf = 1: m·v >> 12 (floor), IR saturated.
pub(crate) fn mv(m: &M, v: [i32; 3]) -> [i32; 3] {
    [0, 1, 2].map(|i| ((m[i][0] * v[0] + m[i][1] * v[1] + m[i][2] * v[2]) >> 12).clamp(-0x8000, 0x7FFF))
}

/// `BM · Swing(dir) · RotY(−tw)` (dir unnormalised, tw in 4096 units).
pub(crate) fn build(bm: &M, d: [i32; 3], tw: i32, t: &Trig, rsqrt: &dyn Fn(i32) -> i32) -> M {
    let [nx, ny, nz] = vector_normal_ss(d, rsqrt);
    let [hx, _, hz] = vector_normal_ss([nx, 0, nz], rsqrt);
    let k = 0x1000 - ny;
    let a = (k * ((hz * hz) >> 12)) >> 12;
    let b = (k * ((hx * hx) >> 12)) >> 12;
    let e = (k * ((-hz * hx) >> 12)) >> 12;
    let sw: M = [[a + ny, nx, e], [-nx, ny, -nz], [e, nz, b + ny]];
    let (c, s) = (t.cos(tw), t.sin(tw));
    // the twist's columns as the GTE gets them (VY of column 0 = the sign extension of the lh: a quirk)
    let c0 = mv(&sw, [c, if c < 0 { -1 } else { 0 }, s]);
    let c2 = mv(&sw, [-s, 0, c]);
    let cm: M = [[c0[0], sw[0][1], c2[0]], [c0[1], sw[1][1], c2[1]], [c0[2], sw[2][1], c2[2]]];
    // D = BM · C, column by column
    let mut out = [[0; 3]; 3];
    for j in 0..3 {
        let col = mv(bm, [cm[0][j], cm[1][j], cm[2][j]]);
        for i in 0..3 {
            out[i][j] = col[i] as i16 as i32;
        }
    }
    out
}

/// RotY(a), overwriting the matrix.
pub(crate) fn rot_y(a: i32, t: &Trig) -> M {
    let (c, s) = (t.cos(a), t.sin(a));
    [[c, 0, s], [0, 0x1000, 0], [-s, 0, c]]
}

/// RotMatrix(x, y, z), built from the sums and differences of the angles as the game does.
pub(crate) fn rot_matrix(x: i32, y: i32, z: i32, t: &Trig) -> M {
    let (cx, sx, cy, sy, cz, sz) = (t.cos(x), t.sin(x), t.cos(y), t.sin(y), t.cos(z), t.sin(z));
    let (cp, sp, cm, sm) = (t.cos(x + z), t.sin(x + z), t.cos(x - z), t.sin(x - z));
    let (a, b) = (sy - 0x1000, sy + 0x1000);
    let m = [
        [(cz * cy) >> 12, (sp * a + sm * b) >> 13, (cp * a + cm * b) >> 13],
        [(sz * cy) >> 12, (cm * b - cp * a) >> 13, (sp * a - sm * b) >> 13],
        [-sy, (sx * cy) >> 12, (cx * cy) >> 12],
    ];
    m.map(|r| r.map(|v| v as i16 as i32))
}

/// MulMatrix2(m0, m1): m0 · m1, column by column on the GTE.
pub(crate) fn mul_matrix2(m0: &M, m1: &M) -> M {
    let mut out = [[0; 3]; 3];
    for j in 0..3 {
        let col = mv(m0, [m1[0][j], m1[1][j], m1[2][j]]);
        for i in 0..3 {
            out[i][j] = col[i] as i16 as i32;
        }
    }
    out
}

/// PS1 axes (y down, z away) → glTF.
fn to_gltf(m: &M) -> Quat {
    let f = |i: usize, j: usize| m[i][j] as f32 / 4096.0 * if (i == 0) == (j == 0) { 1.0 } else { -1.0 };
    Quat::from_mat3(&Mat3::from_cols(Vec3::new(f(0, 0), f(1, 0), f(2, 0)), Vec3::new(f(0, 1), f(1, 1), f(2, 1)), Vec3::new(f(0, 2), f(1, 2), f(2, 2))))
        .normalize()
}

/// Local rotation and translation (glTF space, `None` = keep the rest value) of every game node of `skel`.
pub fn locals(f: &Fighter, skel: &Skel, t: &Trig, rsqrt: &dyn Fn(i32) -> i32) -> Vec<(Option<Quat>, Option<Vec3>)> {
    records(f, skel, t, rsqrt).into_iter().map(|(m, tr)| (m.as_ref().map(to_gltf), tr.map(|v| Vec3::from(v) / 300.0))).collect()
}

/// `locals` for the logic from simulation version 3 (`crate::base::portable`): f64, the same bits on every target.
pub fn locals_portable(f: &Fighter, skel: &Skel, t: &Trig, rsqrt: &dyn Fn(i32) -> i32) -> Vec<(Option<DQuat>, Option<DVec3>)> {
    let to_gltf = |m: &M| {
        let f = |i: usize, j: usize| m[i][j] as f64 / 4096.0 * if (i == 0) == (j == 0) { 1.0 } else { -1.0 };
        DQuat::from_mat3(&DMat3::from_cols(DVec3::new(f(0, 0), f(1, 0), f(2, 0)), DVec3::new(f(0, 1), f(1, 1), f(2, 1)), DVec3::new(f(0, 2), f(1, 2), f(2, 2))))
            .normalize()
    };
    records(f, skel, t, rsqrt).into_iter().map(|(m, tr)| (m.as_ref().map(to_gltf), tr.map(|v| Vec3::from(v).as_dvec3() / 300.0))).collect()
}

/// The local rotation (4.12, PS1 axes) and translation (glTF axes, game units) of every game node of `skel`.
fn records(f: &Fighter, skel: &Skel, t: &Trig, rsqrt: &dyn Fn(i32) -> i32) -> Vec<(Option<M>, Option<[f32; 3]>)> {
    let nn = skel.parent.len();
    let mut out = vec![(None, None); nn];
    let bm = |n: usize| -> M { skel.base_rot[n].map(|r| r.map(|v| (v * 4096.0).round() as i32)) };
    for n in 1..nn.min(19) {
        let r = &f.pose[n];
        let mut m = if n <= 16 { build(&bm(n), r.dir.map(|v| v as i32), (r.twist as i32) >> 1, t, rsqrt) } else { rot_y(-(r.twist as i32) >> 1, t) };
        if n == 2 && (f.head_yaw, f.head_pitch) != (0, 0) {
            // the head turn (simulation version 13; 0 / 0 is the identity, skipped)
            m = mul_matrix2(&rot_matrix(f.head_pitch as i32, -(f.head_yaw as i32), 0, t), &m);
        }
        let o = skel.offset[n];
        let tr = [0, 1, 2].map(|k| o[k] + ((r.stretch[k] as i32) >> 4) as f32);
        out[n] = (Some(m), Some([tr[0], -tr[1], -tr[2]]));
    }
    // type 1 connectors: the bisector of +Y and the parent's direction, half its twist (round toward zero)
    for (&n, &src) in &skel.connector_of {
        let Some(q) = f.pose.get(src) else { continue };
        let mut d = vector_normal_ss(q.dir.map(|v| v as i32), rsqrt);
        d[1] += 0x1000;
        let tw = ((q.twist as i32 / 2) as i16 as i32) >> 1;
        out[n].0 = Some(build(&bm(n), d, tw, t, rsqrt));
    }
    out
}

/// Node `n`'s local rotation (4.12, PS1 axes: model +0x04 + 0x50·n), for the traces.
pub fn local_matrix(f: &Fighter, skel: &Skel, t: &Trig, rsqrt: &dyn Fn(i32) -> i32, n: usize) -> Option<M> {
    records(f, skel, t, rsqrt).get(n).and_then(|r| r.0)
}

/// Node 2's world position (model +0xD4 / +0xD8 / +0xDC) as the game's integer world walk gives it, for the head
/// turn: the root (node 0's matrix RotMatrix(F+0x4C, −yaw, F+0x50), the position
/// model +0x18), then down to node 2 (t = W.m · t16 >> 12, IR saturated, + W.t;
/// m = W.m · local). The local translations are (bone offset + stretch >> 4), node 1's
/// height halved / × 7/8 by the model types (with the opponent `opp`). (The op 0x1D scales the game
/// folds into the base matrices are left out, as in `records`.)
pub fn head_world(f: &Fighter, opp: &Fighter, skel: &Skel, t: &Trig, rsqrt: &dyn Fn(i32) -> i32) -> [i32; 3] {
    let rec = records(f, skel, t, rsqrt);
    let mut path = vec![2usize];
    while let Some(&p) = skel.parent.get(*path.last().unwrap()) {
        if p <= 0 || path.len() > 19 {
            break;
        }
        path.push(p as usize);
    }
    let t16 = |n: usize| -> [i32; 3] {
        let (o, r) = (skel.offset.get(n).copied().unwrap_or_default(), f.pose.get(n).copied().unwrap_or_default());
        let mut v = [0, 1, 2].map(|k| (o[k] as i32 + ((r.stretch[k] as i32) >> 4)) as i16 as i32);
        if n == 1 {
            let (f54, o54, both58) = (f.flags[F54], opp.flags[F54], (f.flags[F58] | opp.flags[F58]) & 0x4010_0000 != 0);
            if f54 & 8 != 0 || o54 & 8 != 0 && both58 {
                v[1] = (v[1] / 2) as i16 as i32;
            } else if o54 & 8 == 0 && (f54 & 0x10 != 0 || o54 & 0x10 != 0 && both58) {
                v[1] = (v[1] * 7 / 8) as i16 as i32;
            }
        }
        v
    };
    let mut wm = rot_matrix(f.o_4c as i32, -(f.yaw as i32), f.o_50 as i32, t);
    let mut wt = f.m_pos.map(|v| v as i32);
    for &n in path.iter().rev() {
        let d = mv(&wm, t16(n));
        wt = [0, 1, 2].map(|k| wt[k].wrapping_add(d[k]));
        wm = mul_matrix2(&wm, &rec.get(n).and_then(|r| r.0).unwrap_or([[0x1000, 0, 0], [0, 0x1000, 0], [0, 0, 0x1000]]));
    }
    wt
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The game's RotMatrix against Rz · Ry · Rx in float (the sum / difference formulas lose at most a unit or two), and
    /// exactly the identity at 0.
    #[test]
    fn rot_matrix_is_rz_ry_rx() {
        let Some(t) = crate::formats::tables::export_tables().map(|t| Trig::from_tables(&t)) else {
            eprintln!("skipped: no okb_export");
            return;
        };
        assert_eq!(rot_matrix(0, 0, 0, &t), [[4096, 0, 0], [0, 4096, 0], [0, 0, 4096]]);
        let m = [[1, 2, 3], [-4, 5, 6], [7, -8, 9]];
        assert_eq!(mul_matrix2(&rot_matrix(0, 0, 0, &t), &m), m);
        for (x, y, z) in [(0x100, 0, 0), (0, -0x31C, 0), (-0x271, 0x2D, 0), (0x1C7, -0x400, 0x123), (0x7FF, 0x801, -0x555)] {
            let r = |a: i32| a as f64 * std::f64::consts::PI / 2048.0;
            let (rx, ry, rz) = (glam::DMat3::from_rotation_x(r(x)), glam::DMat3::from_rotation_y(r(y)), glam::DMat3::from_rotation_z(r(z)));
            let f = rz * ry * rx;
            let g = rot_matrix(x, y, z, &t);
            for i in 0..3 {
                for j in 0..3 {
                    let e = f.col(j)[i] * 4096.0;
                    assert!((g[i][j] as f64 - e).abs() <= 3.0, "({x}, {y}, {z}) [{i}][{j}] = {}, {e}", g[i][j]);
                }
            }
        }
    }
}
