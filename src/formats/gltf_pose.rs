//! The logic's skeleton pose from simulation version 3, in `base::portable`'s f64 math.
//!
//! Engine addition.
//! State: none (its results feed the state: see `base::portable`)
//!
//! The logic's skeleton pose from simulation version 3 (`base::portable`'s f64 math, the same bits on every target):
//! a model's glTF nodes posed from a clip or from the pose records, then shaped; the f64 twins of
//! `gltf_model::pose_globals_scaled` / `pose_locals_globals`.

use crate::base::portable::{rot_x, slerp};
use crate::formats::gltf_model::{Clip, ModelData, Shape, Track};
use glam::{DMat4, DQuat, DVec3};

/// A node's local transform.
#[derive(Clone)]
struct Node {
    parent: Option<usize>,
    t: DVec3,
    r: DQuat,
    s: DVec3,
}

fn rest(model: &ModelData) -> Vec<Node> {
    model.nodes.iter().map(|n| Node { parent: n.parent, t: n.t.as_dvec3(), r: n.r.as_dquat(), s: n.s.as_dvec3() }).collect()
}

/// Global transforms of `nodes` (parents first or not).
fn globals(nodes: &[Node]) -> Vec<DMat4> {
    let locals: Vec<DMat4> = nodes.iter().map(|n| DMat4::from_scale_rotation_translation(n.s, n.r, n.t)).collect();
    let mut out: Vec<Option<DMat4>> = vec![None; nodes.len()];
    fn resolve(i: usize, nodes: &[Node], locals: &[DMat4], out: &mut [Option<DMat4>]) -> DMat4 {
        if let Some(m) = out[i] {
            return m;
        }
        let m = match nodes[i].parent {
            Some(p) => resolve(p, nodes, locals, out) * locals[i],
            None => locals[i],
        };
        out[i] = Some(m);
        m
    }
    (0..nodes.len()).map(|i| resolve(i, nodes, &locals, &mut out)).collect()
}

/// `gltf_model::shaped` (MODEL TYPE shaping, the jaw's X rotations, the root height), then the globals of the joints
/// (the identity for the other nodes, as `gltf_model::pose_globals_scaled`). The logic's shape has no spring aims
/// (`Fighter::shape`: only the drawing's `shape_render` adds them).
fn shaped(model: &ModelData, mut nodes: Vec<Node>, shape: &Shape) -> Vec<DMat4> {
    debug_assert!(shape.aims.is_empty(), "the logic's pose has no spring aims");
    for (part, s) in &shape.scale {
        if let Some(&j) = model.joints.get(*part) {
            nodes[j].s *= s.as_dvec3();
        }
    }
    for (part, a) in &shape.rot_x {
        if let Some(&j) = model.joints.get(*part) {
            nodes[j].r = rot_x(*a as f64) * nodes[j].r;
        }
    }
    if shape.root_y != 1.0
        && let Some(&j) = model.joints.get(1)
    {
        nodes[j].t.y *= shape.root_y as f64;
    }
    let g = globals(&nodes);
    let mut out = vec![DMat4::IDENTITY; nodes.len()];
    for &j in &model.joints {
        out[j] = g[j];
    }
    out
}

/// `gltf_model::pose_globals_scaled` in f64: `clip` sampled at `t` seconds, mirrored, shaped.
pub fn pose_globals(model: &ModelData, clip: Option<&Clip>, t: f32, mirror: bool, shape: &Shape) -> Vec<DMat4> {
    let mut nodes = rest(model);
    if let Some(clip) = clip {
        for ch in &clip.channels {
            let (a, b, f) = crate::formats::gltf_model::sample_index(&ch.times, t);
            let (n, f) = (&mut nodes[ch.node], f as f64);
            match &ch.track {
                Track::T(v) => n.t = v[a].as_dvec3().lerp(v[b].as_dvec3(), f),
                Track::R(v) => n.r = slerp(v[a].as_dquat(), v[b].as_dquat(), f),
                Track::S(v) => n.s = v[a].as_dvec3().lerp(v[b].as_dvec3(), f),
            }
        }
    }
    if mirror {
        let sampled = nodes.clone();
        for (i, n) in nodes.iter_mut().enumerate() {
            let src = crate::formats::gltf_model::counterpart(model, i).unwrap_or(i);
            let s = &sampled[src];
            n.r = DQuat::from_xyzw(s.r.x, -s.r.y, -s.r.z, s.r.w);
            n.t = if src == i { DVec3::new(-s.t.x, s.t.y, s.t.z) } else { model.nodes[i].t.as_dvec3() };
        }
    }
    shaped(model, nodes, shape)
}

/// `gltf_model::pose_locals_globals` in f64: the per-game-node local rotations (`pose::locals_portable`) and
/// translations on the rest pose, shaped.
pub fn pose_locals_globals(model: &ModelData, locals: &[(Option<DQuat>, Option<DVec3>)], shape: &Shape) -> Vec<DMat4> {
    let mut nodes = rest(model);
    for (n, (r, t)) in locals.iter().enumerate() {
        let Some(&j) = model.joints.get(n) else { continue };
        if let Some(r) = r {
            nodes[j].r = *r;
        }
        if let Some(t) = t {
            nodes[j].t = *t;
        }
    }
    shaped(model, nodes, shape)
}
