//! Animation clips built from ACT animations for any skeleton at load time.
//!
//! State: static data (not serialised; re-attached by `rebind`)
//!
//! Builds glTF-style clips from ACT animations for any skeleton at load time (the converter's pose decoder
//! `ok_br2_converter::model` again, plus the game's body translation). Every fighter clip comes from here:
//! the fighter's own ACT, CMN.ACT and the animations it plays from the opponent's ACT (op 05 sub 5, throw victims).
//!
//! Body (node 1) translation, as the pose sampler sets it: ANI bone offset + stretch record +
//! pose bytes 8..13 / 16 (positive y scaled by state +0xBE / 0x80). This also carries the
//! victim's distance in throw animations.
//!
//! From simulation version 3 (`Skel::portable`) the trig is `libm`'s and the quaternion steps f64
//! (`crate::base::portable`): the clips feed the logic's skeleton, so they must be the same bits on every target.

use crate::base::portable::sin_cos;
use crate::formats::act_ani::Ani;
use crate::formats::gltf_model::{Channel, Clip, ModelData, Track};
use glam::{Mat3, Quat, Vec3};
use std::collections::{BTreeMap, HashMap};

type M3 = [[f32; 3]; 3];
const I3: M3 = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];
const SCALE: f32 = 1.0 / 300.0;
const TICK: f32 = 1.0 / 60.0;

fn mm(a: &M3, b: &M3) -> M3 {
    let mut r = [[0.0; 3]; 3];
    for i in 0..3 {
        for j in 0..3 {
            r[i][j] = (0..3).map(|k| a[i][k] * b[k][j]).sum();
        }
    }
    r
}

fn s10(v: u32) -> f32 {
    let v = (v & 1023) as i32;
    (if v & 512 != 0 { v - 1024 } else { v }) as f32
}

fn ang(a: f32) -> f32 {
    a * std::f32::consts::PI / 2048.0
}

fn euler_zyx(ax: f32, ay: f32, az: f32, p: bool) -> M3 {
    let (sx, cx) = sin_cos(ang(ax), p);
    let (sy, cy) = sin_cos(ang(ay), p);
    let (sz, cz) = sin_cos(ang(az), p);
    let rx = [[1.0, 0.0, 0.0], [0.0, cx, -sx], [0.0, sx, cx]];
    let ry = [[cy, 0.0, sy], [0.0, 1.0, 0.0], [-sy, 0.0, cy]];
    let rz = [[cz, -sz, 0.0], [sz, cz, 0.0], [0.0, 0.0, 1.0]];
    mm(&mm(&rz, &ry), &rx)
}

fn swing_twist(x: f32, y: f32, z: f32, twist: f32, p: bool) -> M3 {
    let n = (x * x + y * y + z * z).sqrt();
    let n = if n == 0.0 { 1.0 } else { n };
    let (dx, dy, dz) = (x / n, y / n, z / n);
    let h = (dx * dx + dz * dz).sqrt();
    let (hx, hz) = if h > 1e-9 { (dx / h, dz / h) } else { (1.0, 0.0) };
    let k = 1.0 - dy;
    let swing = [[dy + k * hz * hz, dx, -k * hx * hz], [-dx, dy, -dz], [-k * hx * hz, dz, dy + k * hx * hx]];
    let (s, c) = sin_cos(ang(twist), p);
    mm(&swing, &[[c, 0.0, -s], [0.0, 1.0, 0.0], [s, 0.0, c]])
}

fn rot_y(a: f32, p: bool) -> M3 {
    let (s, c) = sin_cos(ang(a), p);
    [[c, 0.0, s], [0.0, 1.0, 0.0], [-s, 0.0, c]]
}

fn to_quat(m: &M3, p: bool) -> Quat {
    if p {
        return crate::base::portable::quat_of(m);
    }
    Quat::from_mat3(&Mat3::from_cols(Vec3::new(m[0][0], m[1][0], m[2][0]), Vec3::new(m[0][1], m[1][1], m[2][1]), Vec3::new(m[0][2], m[1][2], m[2][2])))
        .normalize()
}

fn from_quat(q: Quat) -> M3 {
    let m = Mat3::from_quat(q);
    [[m.x_axis.x, m.y_axis.x, m.z_axis.x], [m.x_axis.y, m.y_axis.y, m.z_axis.y], [m.x_axis.z, m.y_axis.z, m.z_axis.z]]
}

/// Half of a rotation (connector nodes turn half-way).
fn half_rot(m: &M3, p: bool) -> M3 {
    if p {
        use crate::base::portable::{axis_angle, mat_of, to_axis_angle};
        let mut q = to_quat(m, p).as_dquat();
        if q.w < 0.0 {
            q = -q;
        }
        let (axis, a) = to_axis_angle(q);
        let axis = if a > 1e-6 { axis } else { glam::DVec3::Y };
        return mat_of(axis_angle(axis, a / 2.0));
    }
    let mut q = to_quat(m, p);
    if q.w < 0.0 {
        q = -q;
    }
    let (axis, a) = q.to_axis_angle();
    let axis = if a > 1e-6 { axis } else { Vec3::Y };
    from_quat(Quat::from_axis_angle(axis, a / 2.0))
}

/// PS1 (y down, z forward) → glTF (y up, z toward the viewer), metres.
fn conv(p: [f32; 3]) -> Vec3 {
    Vec3::new(p[0] * SCALE, -p[1] * SCALE, -p[2] * SCALE)
}

/// Skeleton of one form, from the ANI's descriptor table (header word 6).
pub struct Skel {
    pub(crate) parent: Vec<i32>,
    /// ANI bone offsets (game units, parent frame)
    pub(crate) offset: Vec<[f32; 3]>,
    /// ANI base rotations BM (desc[4] Euler, PS1 axes)
    pub(crate) base_rot: Vec<[[f32; 3]; 3]>,
    /// type 1 connector → the node whose record it halves (in node order: `local` is the same on every run
    /// should a connector's source be a connector too)
    pub(crate) connector_of: BTreeMap<usize, usize>,
    /// simulation version 3 on (`Loaded::portable`): `libm` trig, f64 quaternion steps
    pub(crate) portable: bool,
}

fn u32at(b: &[u8], o: usize) -> u32 {
    b.get(o..o + 4).map(|s| u32::from_le_bytes([s[0], s[1], s[2], s[3]])).unwrap_or(0)
}

impl Skel {
    pub fn new(ani: &Ani, form: usize, nn: usize, portable: bool) -> Self {
        let b = &ani.bytes;
        let table = u32at(b, 24) as usize;
        let desc: Vec<usize> = (0..7).map(|k| u32at(b, table + 28 * form + 4 * k) as usize).collect();
        let mut parent: Vec<i32> = (0..nn).map(|n| ani.i16(desc[0] + 2 * n) as i32).collect();
        let mut offset = vec![[0.0; 3]];
        let mut base_rot = vec![I3];
        for n in 1..nn {
            let w = u32at(b, desc[2] + 4 * (n - 1));
            offset.push([s10(w), s10(w >> 10), s10(w >> 20)]);
            let r = u32at(b, desc[4] + 4 * (n - 1));
            base_rot.push(euler_zyx(s10(r) * 4.0, s10(r >> 10) * 4.0, s10(r >> 20) * 4.0, portable));
        }
        let mut connector_of = BTreeMap::new();
        let mut a = desc[5];
        for n in 19..nn {
            if a + 4 > b.len() {
                break;
            }
            let (hdr, cnt) = (ani.u16(a), ani.i16(a + 2));
            if !(0..=32).contains(&cnt) {
                break;
            }
            a += 4 + 2 * cnt as usize;
            if hdr & 0xFF == 1 && parent[n] > 0 {
                connector_of.insert(n, parent[n] as usize);
                parent[n] = parent[parent[n] as usize];
            }
        }
        Self { parent, offset, base_rot, connector_of, portable }
    }

    /// Local rotations (PS1 axes) of the pose at `p`.
    fn local(&self, act: &[u8], p: usize, short: bool) -> Vec<M3> {
        let nn = self.parent.len();
        let mut loc = vec![I3; nn];
        for n in 1..nn.min(17) {
            let o = p + 20 + 6 * (n - 1);
            let w = u32at(act, o);
            let tw = i16::from_le_bytes([act[o + 4], act[o + 5]]) as f32;
            loc[n] = mm(&self.base_rot[n], &swing_twist(s10(w), s10(w >> 10), s10(w >> 20), tw, self.portable));
        }
        if !short {
            for (n, o) in [(17usize, 116usize), (18, 118)] {
                if n < nn {
                    loc[n] = rot_y(-(i16::from_le_bytes([act[p + o], act[p + o + 1]]) as f32), self.portable);
                }
            }
        }
        for (&n, &src) in &self.connector_of {
            loc[n] = half_rot(&loc[src], self.portable);
        }
        loc
    }
}

/// ACT animation `id` of `act` as a clip for `model` (skeleton `skel`); `None` if the entry is empty.
/// `scale` = S.BE: 0x80 for the fighter's own ACT, S.BC (ANI +0x30) for CMN.ACT and the opponent's ACT.
pub fn build(model: &ModelData, skel: &Skel, act: &crate::formats::act_ani::Act, id: usize, short: bool, scale: i32) -> Option<Clip> {
    let base = act.offset(id)?;
    let b = &act.bytes;
    let pose_len = if short { 116 } else { 120 };
    let mut keys = Vec::new();
    let (mut a, mut t) = (base + 4, 0u32);
    while a + 4 <= b.len() && keys.len() < 256 {
        let off = u16::from_le_bytes([b[a], b[a + 1]]) as usize;
        let dur = u16::from_le_bytes([b[a + 2], b[a + 3]]) as u32;
        a += 4;
        keys.push((t, base + off));
        t += dur;
        if dur == 0 {
            break;
        }
    }
    if keys.is_empty() || keys.iter().any(|&(_, p)| p + pose_len > b.len()) {
        return None;
    }
    let nn = skel.parent.len();
    let c: M3 = [[1.0, 0.0, 0.0], [0.0, -1.0, 0.0], [0.0, 0.0, -1.0]];
    let mut times = Vec::new();
    let mut rots: Vec<Vec<Quat>> = Vec::new();
    let mut root_tr = Vec::new();
    let mut stretches: Vec<HashMap<usize, [f32; 3]>> = Vec::new();
    let mut stretch: HashMap<usize, [f32; 3]> = HashMap::new();
    for &(tk, p) in &keys {
        times.push(tk as f32 * TICK);
        let loc = skel.local(b, p, short);
        if short {
            stretch.clear();
        } else {
            // 4-byte records after the pose: slot, dx, dy, dz (0xFF ends)
            let mut q = p + 120;
            for _ in 0..32 {
                if q + 4 > b.len() || b[q] == 0xFF {
                    break;
                }
                // record slot k → node R[k] + 1 = node k (slot 0 → node 1), as the stretch records do
                let idx = b[q] as usize;
                if idx <= 19 {
                    stretch.insert(idx.max(1), [b[q + 1] as i8 as f32, b[q + 2] as i8 as f32, b[q + 3] as i8 as f32]);
                }
                q += 4;
            }
        }
        let s1 = stretch.get(&1).copied().unwrap_or([0.0; 3]);
        let o1 = skel.offset.get(1).copied().unwrap_or([0.0; 3]);
        // positive y × S.BE >> 7
        let v = [0, 1, 2].map(|k| {
            let r = i16::from_le_bytes([b[p + 8 + 2 * k], b[p + 9 + 2 * k]]) as i32;
            (if k == 1 && r > 0 { (r * scale) >> 7 } else { r }) as f32 / 16.0
        });
        root_tr.push(conv([o1[0] + s1[0] + v[0], o1[1] + s1[1] + v[1], o1[2] + s1[2] + v[2]]));
        let mut qs: Vec<Quat> = loc.iter().map(|m| to_quat(&mm(&mm(&c, m), &c), skel.portable)).collect();
        if let Some(prev) = rots.last() {
            for (n, q) in qs.iter_mut().enumerate() {
                let dot = if skel.portable { q.as_dquat().dot(prev[n].as_dquat()) } else { q.dot(prev[n]) as f64 };
                if dot < 0.0 {
                    *q = -*q;
                }
            }
        }
        rots.push(qs);
        stretches.push(stretch.clone());
    }
    if times.len() == 1 {
        times.push(TICK);
        rots.push(rots[0].clone());
        root_tr.push(root_tr[0]);
        stretches.push(stretches[0].clone());
    }
    let mut channels = Vec::new();
    for n in 0..nn.min(model.joints.len()) {
        let node = model.joints[n];
        channels.push(Channel { node, times: times.clone(), track: Track::R(rots.iter().map(|k| k[n]).collect()) });
        if n == 1 {
            channels.push(Channel { node, times: times.clone(), track: Track::T(root_tr.clone()) });
        } else if n > 1 && stretches.iter().any(|s| s.get(&n).is_some_and(|d| d.iter().any(|&x| x != 0.0))) {
            let o = skel.offset[n];
            let vals = stretches
                .iter()
                .map(|s| {
                    let d = s.get(&n).copied().unwrap_or([0.0; 3]);
                    conv([o[0] + d[0], o[1] + d[1], o[2] + d[2]])
                })
                .collect();
            channels.push(Channel { node, times: times.clone(), track: Track::T(vals) });
        }
    }
    Some(Clip { duration: *times.last().unwrap(), channels })
}

/// Animation ids a fighter's scripts play from the opponent's ACT (op 05, sub low nibble 5).
pub fn opponent_anim_ids(ani: &Ani) -> Vec<usize> {
    let b = &ani.bytes;
    let mut ids: Vec<usize> = (0..b.len().saturating_sub(4))
        .step_by(2)
        .filter(|&o| b[o] == 5 && b[o + 1] & 0x0F == 5 && b[o + 1] & 0xC0 == 0)
        .map(|o| u16::from_le_bytes([b[o + 2], b[o + 3]]) as usize)
        .filter(|&id| id < 1000)
        .collect();
    ids.sort_unstable();
    ids.dedup();
    ids
}
