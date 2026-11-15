//! The extra skeleton nodes: spring chains (hair, tails, sleeves).
//!
//! State: serialised: part of `GameState` (field changes are `STATE_VERSION` changes)
//!
//! The extra skeleton nodes 19..: type 2 = spring chains (hair, ears, tails, braids, sleeves) driven by
//! the chain step, in float (the game's GTE fixed point is not reproduced). Per frame the child joint keeps a damped
//! share of its last motion (`p0 / 256`), falls by `41·p1 / 256` and is clamped to the floor (F.94). From simulation
//! version 27 (`v::SPRING_REST`) the link then turns as the game's step turns it: the child's offset is taken into
//! the parent frame, offset by the descriptor's direction `(p2, p3, p4)` and taken into the link's rest frame (its
//! animated local rotation turned so that +y lies along that direction); there its two swing angles (from +y towards
//! +x and towards +z) are limited (6 params: ±`p5`; 9 params: `[p5, p6]` × `[p7, p8]`; 4096 units a turn; fewer:
//! none) and rebuilt into a direction; the link's rotation is the rest frame turned onto it, the child sits at its
//! ANI bone offset under that rotation (in whole units, as the game's integer matrices place it, so a chain comes to
//! rest), and the next link of the chain hangs from that rotation. So a chain rests along its descriptor's direction
//! (Shina's hair standing back, the rabbit's ears up, Bakuryu's braid back from the neck, then down) and swings at
//! most `p5` from it. Before v27 the earlier model: the direction pushed by `(p2, p3, p4)` every frame, the swing
//! limited to twice `p5` around the animated direction, the child where the animated skeleton puts it (the chains'
//! end nodes are not placed by it: a last link could be several times its length) (`step_before_rest`). Type 3 (the
//! jaw) is `fighter::NodeSwing`; types 0 / 1 (rigid followers) are the rest pose.

use crate::formats::act_ani::Ani;
use serde::{Deserialize, Serialize};

/// One extra node's descriptor (ANI skeleton descriptor word 5): header type / chain flags and parameters.
#[derive(Clone, Debug)]
pub struct ExtraNode {
    pub node: usize,
    pub parent: i16,
    pub kind: u8,
    pub start: bool,
    pub end: bool,
    pub p: Vec<i16>,
    /// the node's bone offset from its parent (ANI skeleton descriptor word 2: 10-bit x, y, z; game units, the
    /// parent's frame): where a link's matrix puts this node
    pub offset: [f32; 3],
}

/// The extra-node descriptors of `form` (0 human, 1 beast).
pub fn extra_nodes(ani: &Ani, form: usize) -> Vec<ExtraNode> {
    let b = &ani.bytes;
    let rd32 = |o: usize| b.get(o..o + 4).map(|s| u32::from_le_bytes([s[0], s[1], s[2], s[3]])).unwrap_or(0) as usize;
    let rd16 = |o: usize| b.get(o..o + 2).map(|s| i16::from_le_bytes([s[0], s[1]])).unwrap_or(-1);
    let skel = rd32(24);
    let d = skel + 28 * form;
    let (parents, offsets, mut a) = (rd32(d), rd32(d + 8), rd32(d + 20));
    let s10 = |w: usize, shift: u32| (((w >> shift) as u32 & 0x3FF) as i32 ^ 0x200) - 0x200;
    let mut out = Vec::new();
    if a == 0 {
        return out;
    }
    for node in 19..32 {
        let (hdr, cnt) = (rd16(a) as u16, rd16(a + 2));
        if hdr == 0xFFFF || !(0..=32).contains(&cnt) {
            break;
        }
        let p = (0..cnt as usize).map(|k| rd16(a + 4 + 2 * k)).collect();
        let w = rd32(offsets + 4 * (node - 1));
        let offset = [s10(w, 0), s10(w, 10), s10(w, 20)].map(|v| v as f32);
        out.push(ExtraNode { node, parent: rd16(parents + 2 * node), kind: (hdr & 0xF) as u8, start: hdr & 0x8000 != 0, end: hdr & 0x4000 != 0, p, offset });
        a += 4 + 2 * cnt as usize;
    }
    out
}

/// Simulation state of one spring node: its child joint now and one frame ago (game units, y down).
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Spring {
    pub node: usize,
    pub child: [f32; 3],
    pub prev: [f32; 3],
}

type V3 = [f32; 3];

fn sub(a: V3, b: V3) -> V3 {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}
fn add(a: V3, b: V3) -> V3 {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}
fn len(a: V3) -> f32 {
    (a[0] * a[0] + a[1] * a[1] + a[2] * a[2]).sqrt()
}
fn mat(r: &[[i32; 3]; 3]) -> [[f32; 3]; 3] {
    r.map(|row| row.map(|v| v as f32 / 4096.0))
}
fn mul(m: &[[f32; 3]; 3], v: V3) -> V3 {
    [0, 1, 2].map(|i| m[i][0] * v[0] + m[i][1] * v[1] + m[i][2] * v[2])
}
fn mul_t(m: &[[f32; 3]; 3], v: V3) -> V3 {
    [0, 1, 2].map(|i| m[0][i] * v[0] + m[1][i] * v[1] + m[2][i] * v[2])
}

type M3 = [[f32; 3]; 3];

const IDENTITY: M3 = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];

fn mat_mul(a: &M3, b: &M3) -> M3 {
    [0, 1, 2].map(|i| [0, 1, 2].map(|j| a[i][0] * b[0][j] + a[i][1] * b[1][j] + a[i][2] * b[2][j]))
}
fn transpose(a: &M3) -> M3 {
    [0, 1, 2].map(|i| [0, 1, 2].map(|j| a[j][i]))
}

/// The rotation that turns +y onto `n` along the shortest arc (the game's aim of a link's frame at a direction): the
/// identity for a zero or +y `n`, a half turn about x for −y.
fn aim(n: V3) -> M3 {
    let l = len(n);
    if l < 1e-6 {
        return IDENTITY;
    }
    let [x, y, z] = n.map(|c| c / l);
    // the axis y × n = (z, 0, −x) / s with sin θ = s, cos θ = y (Rodrigues)
    let s = (x * x + z * z).sqrt();
    if s < 1e-6 {
        return if y > 0.0 { IDENTITY } else { [[1.0, 0.0, 0.0], [0.0, -1.0, 0.0], [0.0, 0.0, -1.0]] };
    }
    let (kx, kz, t) = (z / s, -x / s, 1.0 - y);
    [[y + t * kx * kx, -s * kz, t * kx * kz], [s * kz, y, -s * kx], [t * kx * kz, s * kx, y + t * kz * kz]]
}

/// One frame of every spring chain of a fighter. `nodes` / `rots` = the animated skeleton (world positions and
/// rotations, game units); returns the aim direction (world, game units) of each spring node, chain order.
/// `portable`: simulation version 3 on (`Loaded::portable`), the trig of `crate::base::portable`; `rest`: simulation
/// version 27 on (`v::SPRING_REST`), the game's rest frame and limits, else the earlier model.
#[allow(clippy::too_many_arguments)]
pub fn step(
    state: &mut Vec<Spring>,
    extras: &[ExtraNode],
    nodes: &[[i32; 3]],
    rots: &[[[i32; 3]; 3]],
    floor: i32,
    frozen: bool,
    portable: bool,
    rest: bool,
) -> Vec<(usize, V3)> {
    if rest { step_rest(state, extras, nodes, rots, floor, frozen, portable) } else { step_before_rest(state, extras, nodes, rots, floor, frozen, portable) }
}

/// A link's swing limits (x low, x high, z low, z high; 4096 units a turn): ±p5 with six parameters, p5..p6 and
/// p7..p8 with nine, none otherwise. A reversed pair (a damaged descriptor) is taken low to high, so the clamp
/// cannot panic (engine addition; the shipped descriptors are all in order).
fn limits(e: &ExtraNode) -> Option<(f32, f32, f32, f32)> {
    let pair = |a: f32, b: f32| (a.min(b), a.max(b));
    let p = |k: usize| e.p[k] as f32;
    let ((x0, x1), (z0, z1)) = match e.p.len() {
        6 => (pair(-p(5), p(5)), pair(-p(5), p(5))),
        9 => (pair(p(5), p(6)), pair(p(7), p(8))),
        _ => return None,
    };
    Some((x0, x1, z0, z1))
}

/// The chain step from simulation version 27 (the module's description).
fn step_rest(
    state: &mut Vec<Spring>,
    extras: &[ExtraNode],
    nodes: &[[i32; 3]],
    rots: &[[[i32; 3]; 3]],
    floor: i32,
    frozen: bool,
    portable: bool,
) -> Vec<(usize, V3)> {
    let pos = |n: usize| nodes.get(n).map(|p| p.map(|v| v as f32)).unwrap_or([0.0; 3]);
    let mut aims = Vec::new();
    // the links stepped so far: their child's position and their rotation (world), which the next link hangs from
    let mut moved: Vec<(usize, V3)> = Vec::new();
    let mut turned: Vec<(usize, M3)> = Vec::new();
    for e in extras.iter().filter(|e| e.kind == 2 && !e.end && e.p.len() >= 2) {
        let Some(child) = extras.iter().find(|c| c.parent as usize == e.node) else { continue };
        let (child, off) = (child.node, child.offset);
        let at = moved.iter().find(|(n, _)| *n == e.node).map(|m| m.1).unwrap_or_else(|| pos(e.node));
        let parent = e.parent.max(0) as usize;
        let pr = turned.iter().find(|(n, _)| *n == parent).map(|t| t.1).unwrap_or_else(|| rots.get(parent).map(mat).unwrap_or(IDENTITY));
        // the link's animated rotation, local to its parent; its child sits at its bone offset in the link's frame
        // (as the game's matrices place it: the animated skeleton does not place the chains' end nodes)
        let own = rots.get(e.node).map(mat).unwrap_or(pr);
        let base = mat_mul(&transpose(&rots.get(parent).map(mat).unwrap_or(IDENTITY)), &own);
        // the rest frame: the animated local rotation turned so that +y lies along the descriptor's direction
        let bias = [e.p.get(2), e.p.get(3), e.p.get(4)].map(|v| v.copied().unwrap_or(0) as f32);
        let frame = mat_mul(&base, &aim(bias));
        let s = match state.iter().position(|s| s.node == e.node) {
            Some(k) => k,
            None => {
                let c = add(at, mul(&mat_mul(&pr, &frame), off));
                state.push(Spring { node: e.node, child: c, prev: c });
                state.len() - 1
            }
        };
        let sp = &mut state[s];
        // the target: the child's last place with a damped share of its motion and the fall (frozen: where it is)
        let mut t = sp.child;
        if !frozen {
            let (p0, p1) = (e.p[0] as f32, e.p[1] as f32);
            let vel = sub(sp.child, sp.prev);
            t = [0, 1, 2].map(|k| sp.child[k] + (vel[k] * p0 / 256.0).trunc());
            t[1] += (41.0 * p1 / 256.0).trunc();
            t[1] = t[1].min(floor as f32);
        }
        // into the parent frame, offset by the direction, into the rest frame
        let u = mul_t(&frame, add(mul_t(&pr, sub(t, at)), bias));
        let unit = std::f32::consts::TAU / 4096.0;
        let lim = limits(e);
        let dir = match lim {
            Some((x0, x1, z0, z1)) => {
                let ang = |a: f32, b: f32| crate::base::portable::atan2(a, b, portable);
                let sx = ang(u[0], u[1]).clamp(x0 * unit, x1 * unit);
                let sz = ang(u[2], u[1]).clamp(z0 * unit, z1 * unit);
                let (sin, cos) = (|x| crate::base::portable::sin(x, portable), |x| crate::base::portable::cos(x, portable));
                [sin(sx), (cos(sx) + cos(sz)) / 2.0, sin(sz)]
            }
            None => u,
        };
        let rot = mat_mul(&pr, &mat_mul(&frame, &aim(dir)));
        // whole game units, as the game's integer matrices place the child (the product's `>> 12` floors): the
        // motion carried to the next frame then dies out once it is under a unit and a chain comes to rest. The
        // float product can land just under a whole number (24.99998 for 25), so a link may come out a unit short.
        let c = add(at, mul(&rot, off).map(f32::floor));
        if !frozen {
            sp.prev = sp.child;
            sp.child = c;
        }
        turned.push((e.node, rot));
        moved.push((child, sp.child));
        aims.push((e.node, sub(sp.child, at)));
    }
    aims
}

/// The chain step before simulation version 27: the direction pushed by `(p2, p3, p4)` every frame, the swing limited
/// to twice `p5` around the animated direction, each link from its animated parent rotation.
fn step_before_rest(
    state: &mut Vec<Spring>,
    extras: &[ExtraNode],
    nodes: &[[i32; 3]],
    rots: &[[[i32; 3]; 3]],
    floor: i32,
    frozen: bool,
    portable: bool,
) -> Vec<(usize, V3)> {
    let pos = |n: usize| nodes.get(n).map(|p| p.map(|v| v as f32)).unwrap_or([0.0; 3]);
    let mut aims = Vec::new();
    // corrected positions of the chain nodes simulated so far (a chain node's position follows its parent's aim)
    let mut moved: Vec<(usize, V3)> = Vec::new();
    for e in extras.iter().filter(|e| e.kind == 2 && !e.end && e.p.len() >= 2) {
        let Some(child) = extras.iter().find(|c| c.parent as usize == e.node).map(|c| c.node) else { continue };
        let at = moved.iter().find(|(n, _)| *n == e.node).map(|m| m.1).unwrap_or_else(|| pos(e.node));
        let rest = sub(pos(child), pos(e.node));
        let length = len(rest).max(1.0);
        let s = match state.iter().position(|s| s.node == e.node) {
            Some(k) => k,
            None => {
                let c = add(at, rest);
                state.push(Spring { node: e.node, child: c, prev: c });
                state.len() - 1
            }
        };
        let sp = &mut state[s];
        if !frozen {
            // Verlet: damped motion + gravity, then the floor
            let (p0, p1) = (e.p[0] as f32 / 256.0, e.p[1] as f32);
            let vel = sub(sp.child, sp.prev);
            let mut t = [sp.child[0] + vel[0] * p0, sp.child[1] + vel[1] * p0 + (41.0 * p1 / 256.0).trunc(), sp.child[2] + vel[2] * p0];
            if t[1] > floor as f32 {
                t[1] = floor as f32;
            }
            // direction in the parent frame, biased; swing limits relative to the animated direction
            let pr = rots.get(e.parent.max(0) as usize).map(mat).unwrap_or([[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]]);
            let bias = [e.p.get(2), e.p.get(3), e.p.get(4)].map(|v| v.copied().unwrap_or(0) as f32);
            let mut v = add(mul_t(&pr, sub(t, at)), bias);
            let r = mul_t(&pr, rest);
            let lim = limits(e);
            if let Some((x0, x1, z0, z1)) = lim {
                // the game limits the half-step angles of its two-stage aim: the total swing is twice that
                let unit = std::f32::consts::TAU / 4096.0;
                // the platform's trig before simulation version 3, then libm's (`portable`)
                let ang = |a: f32, b: f32| crate::base::portable::atan2(a, b, portable);
                let (rx, rz) = (ang(r[0], r[1]), ang(r[2], r[1]));
                let sx = (ang(v[0], v[1]) - rx).clamp(2.0 * x0 * unit, 2.0 * x1 * unit) + rx;
                let sz = (ang(v[2], v[1]) - rz).clamp(2.0 * z0 * unit, 2.0 * z1 * unit) + rz;
                let (sin, cos) = (|x| crate::base::portable::sin(x, portable), |x| crate::base::portable::cos(x, portable));
                v = [sin(sx), (cos(sx) + cos(sz)) / 2.0, sin(sz)];
            }
            let mut dir = mul(&pr, v);
            let l = len(dir);
            if l < 1e-3 {
                dir = rest;
            }
            let k = length / len(dir).max(1e-3);
            sp.prev = sp.child;
            sp.child = add(at, dir.map(|c| c * k));
        }
        moved.push((child, sp.child));
        aims.push((e.node, sub(sp.child, at)));
    }
    aims
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A chain node `node` under `parent`, `down` units below it (its bone offset, +y).
    fn link(node: usize, parent: i16, p: Vec<i16>, down: f32) -> ExtraNode {
        ExtraNode { node, parent, kind: 2, start: parent < 19, end: p.is_empty(), p, offset: [0.0, down, 0.0] }
    }
    fn angle(a: V3, b: V3) -> f32 {
        let d = (a[0] * b[0] + a[1] * b[1] + a[2] * b[2]) / (len(a) * len(b));
        d.clamp(-1.0, 1.0).acos().to_degrees()
    }
    /// A skeleton of a head (node 2) at the origin, unrotated, with a link 19 → 20 hanging 100 units down (+y).
    fn skeleton() -> (Vec<[i32; 3]>, Vec<[[i32; 3]; 3]>) {
        let mut nodes = vec![[0; 3]; 21];
        nodes[20] = [0, 100, 0];
        (nodes, vec![[[4096, 0, 0], [0, 4096, 0], [0, 0, 4096]]; 21])
    }

    #[test]
    fn aim_turns_y_onto_the_direction() {
        for n in [[0.0, 1.0, 0.0], [0.0, 0.0, 4.0], [-1.0, -1.0, 3.0], [3.0, 1.0, 2.0], [0.0, -2.0, 0.0], [0.5, -3.0, 0.1]] {
            let m = aim(n);
            assert!(angle(mul(&m, [0.0, 1.0, 0.0]), n) < 0.1, "{n:?}");
            // a rotation: its columns stay unit length and at right angles
            let c = |j: usize| [m[0][j], m[1][j], m[2][j]];
            assert!((len(c(0)) - 1.0).abs() < 1e-5 && (len(c(2)) - 1.0).abs() < 1e-5 && (angle(c(0), c(2)) - 90.0).abs() < 0.1);
        }
    }

    /// From v27 a link rests along its descriptor's direction (here +z) against the fall, with its length kept: the
    /// direction added before the angles are taken pulls it back, and the swing never passes ±p5 (256 = 22.5°). The
    /// earlier model kept it round the animated direction (+y) instead.
    #[test]
    fn a_link_rests_along_its_direction_within_its_limit() {
        let (nodes, rots) = skeleton();
        let extras = [link(19, 2, vec![64, 8, 0, 0, 4, 256], 0.0), link(20, 19, vec![], 100.0)];
        let (mut s, mut old) = (Vec::new(), Vec::new());
        let (mut aim_new, mut aim_old) = (Vec::new(), Vec::new());
        for _ in 0..120 {
            aim_new = step(&mut s, &extras, &nodes, &rots, 10_000, false, true, true);
            aim_old = step(&mut old, &extras, &nodes, &rots, 10_000, false, true, false);
        }
        let d = aim_new[0].1;
        assert!((len(d) - 100.0).abs() < 1.5, "length {}", len(d)); // whole units
        // a light fall (1 a frame against the direction's 4) leaves it on its direction, whole units dropping the rest
        let a = angle(d, [0.0, 0.0, 1.0]);
        assert!(d[1] >= 0.0 && a < 22.5, "{a} {d:?}");
        // a heavier fall stops at the limit
        let heavy = [link(19, 2, vec![64, 80, 0, 0, 4, 256], 0.0), link(20, 19, vec![], 100.0)];
        let mut h = Vec::new();
        let mut aims = Vec::new();
        for _ in 0..120 {
            aims = step(&mut h, &heavy, &nodes, &rots, 10_000, false, true, true);
        }
        let a = angle(aims[0].1, [0.0, 0.0, 1.0]);
        assert!((21.0..=22.6).contains(&a), "{a} {:?}", aims[0].1);
        // the earlier model: around the animated direction (+y), not the descriptor's
        assert!(angle(aim_old[0].1, [0.0, 1.0, 0.0]) < 50.0, "{:?}", aim_old[0].1);
    }

    /// A chain's second link hangs from the first's stepped rotation: with the second link's direction +y (along its
    /// parent), the whole chain lies along the first link's direction.
    #[test]
    fn a_link_follows_its_parent_links_rotation() {
        let mut nodes = vec![[0; 3]; 22];
        nodes[20] = [0, 50, 0];
        nodes[21] = [0, 100, 0];
        let rots = vec![[[4096, 0, 0], [0, 4096, 0], [0, 0, 4096]]; 22];
        let extras = [link(19, 2, vec![32, 0, 0, -4, 0, 256], 0.0), link(20, 19, vec![32, 0, 0, 3, 0, 256], 50.0), link(21, 20, vec![], 50.0)];
        let mut s = Vec::new();
        let mut aims = Vec::new();
        for _ in 0..30 {
            aims = step(&mut s, &extras, &nodes, &rots, 10_000, false, true, true);
        }
        // no fall: the first link points up (−y, its direction), the second goes on up from it
        assert!(angle(aims[0].1, [0.0, -1.0, 0.0]) < 0.5, "{:?}", aims[0].1);
        assert!(angle(aims[1].1, [0.0, -1.0, 0.0]) < 0.5, "{:?}", aims[1].1);
        assert!(s[1].child[1] < -99.0, "{:?}", s[1].child);
    }

    /// A frozen chain does not move.
    #[test]
    fn a_frozen_chain_keeps_its_place() {
        let (nodes, rots) = skeleton();
        let extras = [link(19, 2, vec![64, 8, 0, 0, 4, 256], 0.0), link(20, 19, vec![], 100.0)];
        let mut s = Vec::new();
        step(&mut s, &extras, &nodes, &rots, 10_000, false, true, true);
        let before = s[0].clone();
        for _ in 0..10 {
            step(&mut s, &extras, &nodes, &rots, 10_000, true, true, true);
        }
        assert_eq!((s[0].child, s[0].prev), (before.child, before.prev));
    }
}
