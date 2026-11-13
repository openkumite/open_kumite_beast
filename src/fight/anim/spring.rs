//! The extra skeleton nodes: spring chains (hair, tails, sleeves).
//!
//! State: serialised: part of `GameState` (field changes are `STATE_VERSION` changes)
//!
//! The extra skeleton nodes 19..: type 2 = spring chains (hair, tails, braids, sleeves) driven by
//! the chain step. A behavioural model (the game's GTE fixed point is not reproduced): per frame the child joint
//! keeps a damped share of its last motion (`p0 / 256`), falls by `41·p1 / 256`, is clamped to the floor (F.94),
//! biased by `(p2, p3, p4)` in the parent frame, limited in its two swing angles (6 params: ±`p5`; 9 params:
//! `[p5, p6]` × `[p7, p8]`) and the node is re-aimed at it with the chain length kept. Type 3 (the jaw) is
//! `fighter::NodeSwing`; types 0 / 1 (rigid followers) are the rest pose.

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
}

/// The extra-node descriptors of `form` (0 human, 1 beast).
pub fn extra_nodes(ani: &Ani, form: usize) -> Vec<ExtraNode> {
    let b = &ani.bytes;
    let rd32 = |o: usize| b.get(o..o + 4).map(|s| u32::from_le_bytes([s[0], s[1], s[2], s[3]])).unwrap_or(0) as usize;
    let rd16 = |o: usize| b.get(o..o + 2).map(|s| i16::from_le_bytes([s[0], s[1]])).unwrap_or(-1);
    let skel = rd32(24);
    let d = skel + 28 * form;
    let (parents, mut a) = (rd32(d), rd32(d + 20));
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
        out.push(ExtraNode { node, parent: rd16(parents + 2 * node), kind: (hdr & 0xF) as u8, start: hdr & 0x8000 != 0, end: hdr & 0x4000 != 0, p });
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

/// One frame of every spring chain of a fighter. `nodes` / `rots` = the animated skeleton (world positions and
/// rotations, game units); returns the aim direction (world, game units) of each spring node, chain order.
/// `portable`: simulation version 3 on (`Loaded::portable`), the trig of `crate::base::portable`.
pub fn step(
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
            let lim = match e.p.len() {
                6 => Some((-(e.p[5] as f32), e.p[5] as f32, -(e.p[5] as f32), e.p[5] as f32)),
                9 => Some((e.p[5] as f32, e.p[6] as f32, e.p[7] as f32, e.p[8] as f32)),
                _ => None,
            };
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
