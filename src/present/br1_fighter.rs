//! An imported fighter on screen: its glTF posed from the core's world matrices, its objects shown and hidden,
//! its spring chains (hair, ears, tails).
//!
//! State: presentation (reads the state, never writes it; the chains' motion is the view's own)
//!
//! ok_br1_converter's models (`br1/models/XX_{human,beast}.glb`) hold the mesh in the bind pose, one joint `bone_NN`
//! per node of the form's skeleton and a joint `vis_objNN` (a child of the object's node) for each object op 0x1E
//! toggles (11, 14..18); their inverse binds are the bind pose's world. The core keeps every node's world matrix
//! and translation of its last world walk (`Br1Fighter::forms[cur].md`, after the push-apart and the walls), which
//! is what the game draws an object with (its world = the node's · the object's offset), so a joint's matrix is
//! that world in glTF space (game units, y down → the renderer's (x, −y, −z) / 300; the model's 1/256 units) times its
//! inverse bind: the pose is the game's integer one, mirroring included.
//!
//! The chain nodes 19.. the game moves itself (after the walk, also while frozen); as the user's
//! decision puts them in the presentation they run here, once per new fight frame, a
//! ok_br1_converter's integer chain simulation (`chainsim.rs`, checked bit for bit against recorded runs by its
//! author except Uriko's human node 25, mode 5, approximated; mode 3, Greg's, held at rest). A rewind or a saved
//! state starts them again from rest.

use crate::fight::br1::object::Br1Fighter;
use crate::fight::br1::res::Br1Res;
use crate::formats::gltf_model::ModelData;
use glam::{Mat4, Vec4};

type Mi = [[i64; 3]; 3];
const IDI: Mi = [[4096, 0, 0], [0, 4096, 0], [0, 0, 4096]];

/// The objects op 0x1E toggles that have a joint of their own in the model (ok_br1_converter `SWAPPABLE`).
const OWN_JOINT: u32 = 1 << 11 | 1 << 14 | 0xF << 15;
/// All 26 objects (op 0x1E sub 0x80).
const ALL: u32 = (1 << 26) - 1;

/// One model's joints: the game node each follows, and the object a visibility joint stands for.
pub struct Rig {
    node: Vec<usize>,
    obj: Vec<Option<u8>>,
}

impl Rig {
    pub fn new(m: &ModelData) -> Self {
        let mut node = Vec::new();
        let mut obj = Vec::new();
        for &j in &m.joints {
            let n = &m.nodes[j];
            let own = |s: &str| n.name.strip_prefix(s).and_then(|k| k.parse::<usize>().ok());
            match (own("bone_"), own("vis_obj")) {
                (Some(b), _) => {
                    node.push(b);
                    obj.push(None);
                }
                (None, Some(o)) => {
                    // its parent's bone
                    let b = n.parent.and_then(|p| m.nodes[p].name.strip_prefix("bone_").and_then(|k| k.parse().ok())).unwrap_or(0);
                    node.push(b);
                    obj.push(Some(o as u8));
                }
                _ => {
                    node.push(0);
                    obj.push(None);
                }
            }
        }
        Self { node, obj }
    }
}

/// A node's world (4.12 matrix, game units) → glTF space of the renderer, the model in 1/256 units.
fn to_gltf(wm: &[[i32; 3]; 3], wt: [i32; 3]) -> Mat4 {
    let k = 256.0 / 300.0 / 4096.0;
    let s = [1.0, -1.0, -1.0];
    let col = |j: usize| Vec4::new(wm[0][j] as f32 * s[0] * s[j] * k, wm[1][j] as f32 * s[1] * s[j] * k, wm[2][j] as f32 * s[2] * s[j] * k, 0.0);
    Mat4::from_cols(col(0), col(1), col(2), Vec4::new(wt[0] as f32 / 300.0, -wt[1] as f32 / 300.0, -wt[2] as f32 / 300.0, 1.0))
}

/// The joint matrices of `m` (rig `rig`) for core `f`'s current form, the chain nodes' worlds from `chains`, and the
/// renderer's hidden-joint mask (the objects hidden by op 0x1E; all of them: every joint).
pub fn joints(m: &ModelData, rig: &Rig, f: &Br1Fighter, chains: &Chains) -> (Vec<Mat4>, u32) {
    let form = f.cur as usize & 1;
    let md = &f.forms[form].md;
    let world = |n: usize| -> Mat4 {
        if n >= crate::fight::br1::pose::BASE_NODES
            && let Some((wm, wt)) = chains.world(n)
        {
            return to_gltf(&wm.map(|r| r.map(|v| v as i32)), wt.map(|v| v as i32));
        }
        md.get(n).map_or(Mat4::IDENTITY, |d| to_gltf(&d.wm, d.wt))
    };
    let joints = rig.node.iter().zip(&m.inverse_bind).map(|(&n, ib)| world(n) * *ib).collect();
    let hidden = f.parts.hidden[form];
    let mut mask = 0u32;
    if hidden & ALL & !OWN_JOINT == ALL & !OWN_JOINT {
        mask = u32::MAX;
    } else {
        for (j, o) in rig.obj.iter().enumerate() {
            if let Some(o) = o
                && j < 32
                && hidden & (1 << o) != 0
            {
                mask |= 1 << j;
            }
        }
    }
    (joints, mask)
}

/// Node 0's world translation in glTF space (the shadow's position).
pub fn root(f: &Br1Fighter) -> glam::Vec3 {
    let t = f.forms[f.cur as usize & 1].md.first().map_or([0; 3], |d| d.wt);
    glam::Vec3::new(t[0] as f32, -t[1] as f32, -t[2] as f32) / 300.0
}

// ---- the spring chains (ok_br1_converter chainsim.rs: the init, a frame) ----

fn s32(v: i64) -> i64 {
    v as i32 as i64
}
/// Arithmetic shift right rounding toward zero.
fn tdiv(a: i64, sh: u32) -> i64 {
    if a < 0 { -((-a) >> sh) } else { a >> sh }
}
fn sat(v: i64) -> i64 {
    v.clamp(-0x8000, 0x7FFF)
}

/// The chains' tables: the sine table (`math.sin`) and ratan2's arctangents (`math.atan_b`).
struct Tables {
    sin: Vec<i64>,
    atn: Vec<i64>,
}

impl Tables {
    fn of(res: &Br1Res) -> Self {
        let read = |name: &str| (0..1025).map(|k| res.tables.table(name).i16_at(k) as i64).collect();
        Self { sin: read("math.sin"), atn: read("math.atan_b") }
    }
    fn isin(&self, a: i64) -> i64 {
        let i = (a & 0x3FF) as usize;
        let v = if a & 0x400 != 0 { self.sin[0x400 - i] } else { self.sin[i] };
        if a & 0x800 != 0 { -v } else { v }
    }
    fn icos(&self, a: i64) -> i64 {
        self.isin(a + 0x400)
    }
    fn ratan2(&self, y: i64, x: i64) -> i64 {
        let (nx, ny) = (x < 0, y < 0);
        let (x, y) = (x.abs(), y.abs());
        if x == 0 && y == 0 {
            return 0;
        }
        let quot = |a: i64, b: i64| -> usize {
            let q = if a & 0x7FE0_0000 != 0 { s32(a / (b >> 10).max(1)) } else { s32((a << 10) & 0xFFFF_FFFF) / b };
            q.clamp(0, 1024) as usize
        };
        let mut r = if y < x { self.atn[quot(y, x)] } else { 0x400 - self.atn[quot(x, y)] };
        if nx {
            r = 0x800 - r;
        }
        if ny { -r } else { r }
    }
    fn rx(&self, a: i64) -> Mi {
        let (c, n) = (self.icos(a), self.isin(a));
        [[4096, 0, 0], [0, c, -n], [0, n, c]]
    }
    /// Rx(a)·Rz(b)
    fn rx_rz(&self, a: i64, b: i64) -> Mi {
        let (sa, ca, sb, cb) = (self.isin(a), self.icos(a), self.isin(b), self.icos(b));
        [[cb, -sb, 0], [(ca * sb) >> 12, (ca * cb) >> 12, -sa], [(sa * sb) >> 12, (sa * cb) >> 12, ca]]
    }
    fn ry(&self, a: i64) -> Mi {
        let (c, n) = (self.icos(a), self.isin(a));
        [[c, 0, n], [0, 4096, 0], [-n, 0, c]]
    }
}

/// GTE MulMatrix.
fn mmul(a: &Mi, b: &Mi) -> Mi {
    let mut r = [[0; 3]; 3];
    for i in 0..3 {
        for j in 0..3 {
            r[i][j] = sat((0..3).map(|k| a[i][k] * b[k][j]).sum::<i64>() >> 12);
        }
    }
    r
}
/// GTE MVMVA (sf = 1) on a short vector.
fn mvec(a: &Mi, v: [i64; 3]) -> [i64; 3] {
    [0, 1, 2].map(|i| (a[i][0] * v[0] + a[i][1] * v[1] + a[i][2] * v[2]) >> 12)
}
/// ApplyMatrixLV (a long vector).
fn apply_long(a: &Mi, v: [i64; 3]) -> [i64; 3] {
    let hi = v.map(|x| tdiv(x, 15));
    let lo = [0, 1, 2].map(|k| v[k] - (hi[k] << 15));
    [0, 1, 2].map(|i| {
        let ph = (0..3).map(|k| a[i][k] * hi[k]).sum::<i64>() * 8;
        let pl = (0..3).map(|k| a[i][k] * lo[k]).sum::<i64>() >> 12;
        ph + pl
    })
}
fn transpose(a: &Mi) -> Mi {
    [0, 1, 2].map(|i| [a[0][i], a[1][i], a[2][i]])
}

/// A chain node's data the game keeps past +8 of its pose record: the angles, the last world position, the phase.
#[derive(Clone, Copy, Default)]
struct Bone {
    a: [i64; 4],
    prev: [i64; 3],
    phase: i64,
}

/// One fighter form's chains: the records (the static tables' chain records by form and character, 20 bytes per chain node 19..: flags /
/// mode, inertia, rest pull, rest angles X / Z, X limits, Z limits, wag speed) and their motion.
pub struct Chains {
    t: Tables,
    recs: Vec<[i64; 10]>,
    parents: Vec<i16>,
    offs: Vec<[i64; 3]>,
    bd: Vec<Bone>,
    m: Vec<Mi>,
    wm: Vec<Mi>,
    wt: Vec<[i64; 3]>,
    first: bool,
    /// the fight frame last stepped and the form (None: never)
    pub last: Option<(u64, u8)>,
}

impl Chains {
    /// Form `form` of fighter `res` (its character `chr` in the game's order), at rest.
    pub fn new(res: &Br1Res, form: usize, chr: u8) -> Self {
        // (`fighter.parts`: a pointer per form and character, each to a list of record pointers)
        let tables = &res.tables;
        let mut recs = Vec::new();
        let list = tables.abs_u32(tables.table("fighter.parts").origin() + 4 * (form as u32 * 9 + chr as u32));
        if list != 0 {
            while recs.len() <= 16 {
                let r = tables.abs_u32(list + 4 * recs.len() as u32);
                if r == 0 {
                    break;
                }
                recs.push(std::array::from_fn(|i| tables.abs_i16(r + 2 * i as u32) as i64));
            }
        }
        let skel = &res.skels[form & 1];
        let n = skel.nodes();
        Self {
            t: Tables::of(res),
            recs,
            parents: skel.parents.clone(),
            offs: skel.t16.iter().map(|o| o.map(|v| v as i64)).collect(),
            bd: vec![Bone { a: [0, 0x7FFF, 0, 0], ..Default::default() }; n + 1],
            m: vec![IDI; n],
            wm: vec![IDI; n],
            wt: vec![[0; 3]; n],
            first: true,
            last: None,
        }
    }

    /// Chain node `n`'s world after the last step (None before the first or past the skeleton).
    pub fn world(&self, n: usize) -> Option<(Mi, [i64; 3])> {
        (!self.first && n < self.wm.len()).then(|| (self.wm[n], self.wt[n]))
    }

    /// One game frame on core `f`'s worlds of nodes 0..18 and its chain nodes' mode bits (record +6,
    /// the twist word: the chains' setup and op 0x25 set them).
    pub fn step(&mut self, f: &Br1Fighter) {
        let form = f.cur as usize & 1;
        let md = &f.forms[form].md;
        let rec = &f.forms[form].rec;
        let n = self.parents.len();
        let base = crate::fight::br1::pose::BASE_NODES;
        for b in 0..base.min(n) {
            if let Some(d) = md.get(b) {
                self.wm[b] = d.wm.map(|r| r.map(|v| v as i64));
                self.wt[b] = d.wt.map(|v| v as i64);
            }
        }
        for b in base..n {
            self.bd[b].a[3] = rec.get(b).map_or(0, |r| r.twist as u16 as i64);
        }
        let t = &self.t;
        let (wm, wt) = (&mut self.wm, &mut self.wt);
        if self.first {
            for b in base..n {
                let p = self.parents[b].max(0) as usize;
                wm[b] = wm[p];
                let v = mvec(&wm[p], self.offs[b]);
                wt[b] = [0, 1, 2].map(|k| v[k] + wt[p][k]);
                self.bd[b].prev = wt[b];
            }
            self.first = false;
        }
        let floor = 0;
        for b in base..n {
            let r = self.recs.get(b - base).copied().unwrap_or([0x20, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
            let (fl, mode) = (r[0], r[0] & 0xF);
            let p = self.parents[b].max(0) as usize;
            let pbd = self.bd[b - 1];
            let v = mvec(&wm[p], self.offs[b]);
            let newt = [0, 1, 2].map(|k| v[k] + wt[p][k]);
            if mode == 0 {
                self.bd[b].prev = wt[b].map(sat);
                wt[b] = newt;
                if fl & 0x20 != 0 {
                    continue; // the tip: its rotation never updated
                }
                // the child's world position of the last frame (not yet moved this frame)
                let ct = if b + 1 < n { wt[b + 1] } else { newt };
                let cprev = self.bd[b + 1].prev;
                let mut d = [0, 1, 2].map(|k| tdiv((ct[k] - cprev[k]) * r[1], 8) + ct[k] - newt[k]);
                d[1] += 3;
                let over = newt[1] + d[1] - floor;
                if over > 0 {
                    d[1] -= over;
                }
                let fr = if r[3] != 0 || r[4] != 0 { mmul(&wm[p], &t.rx_rz(r[3], r[4])) } else { wm[p] };
                let mut dl = apply_long(&transpose(&fr), d.map(|x| s32(x << 16)));
                let bd = &mut self.bd[b];
                if fl & 0x10 == 0 {
                    bd.a[3] = pbd.a[3];
                }
                let (mut vx, mut vz) = (0, 0);
                if fl & 0x80 != 0 {
                    if fl & 0x10 != 0 {
                        if bd.a[3] & 1 != 0 {
                            bd.phase = (bd.phase + r[9]) & 0xFFFF;
                        }
                    } else {
                        bd.phase = (pbd.phase + 0x400) & 0xFFFF;
                    }
                    bd.a[0] = tdiv(t.isin(bd.phase), 4);
                    vx = bd.a[0];
                } else if fl & 0x100 != 0 {
                    if fl & 0x10 != 0 {
                        if bd.a[3] & 1 != 0 {
                            bd.phase = (bd.phase + r[9]) & 0xFFFF;
                            bd.a[2] = tdiv(t.isin(bd.phase), 4);
                        }
                    } else {
                        bd.a[2] = pbd.a[2];
                    }
                    vz = bd.a[2];
                }
                let w = mvec(&t.rx_rz(vx, vz), [0, r[2], 0]);
                dl = [0, 1, 2].map(|k| s32(dl[k] + (w[k] << 16)));
                let s0 = t.ratan2(dl[2], dl[1]).clamp(r[5].min(r[6]), r[6].max(r[5]));
                let dp = apply_long(&t.rx(-s0), dl);
                let a1 = t.ratan2(-dp[0], dp[1]).clamp(r[7].min(r[8]), r[8].max(r[7]));
                self.m[b] = t.rx_rz(s0, a1);
                wm[b] = mmul(&fr, &self.m[b]);
            } else {
                wt[b] = newt;
                let bd = &mut self.bd[b];
                if mode == 1 {
                    let bits = bd.a[3];
                    let mid = tdiv(r[5] + r[6], 1);
                    // toward the target by at most 0x40 (12-bit wraparound)
                    let appr = |cur: i64, tgt: i64| -> i64 {
                        let (c, g) = ((cur << 20) & 0xFFFF_FFFF, (tgt << 20) & 0xFFFF_FFFF);
                        let dd = s32(g - c);
                        let st = 0x40i64 << 20;
                        if dd == 0 {
                            cur
                        } else if dd > st {
                            s32(c + st) >> 20
                        } else if -dd > st {
                            s32(c - st) >> 20
                        } else {
                            s32(g) >> 20
                        }
                    };
                    if bits & 1 != 0 {
                        bd.a[1] = 0x271;
                        bd.a[0] = appr(bd.a[0], bd.a[1]);
                        self.m[b] = t.rx(bd.a[0]);
                    } else if bits & 2 != 0 {
                        bd.phase = (bd.phase + r[9]) & 0xFFFF;
                        bd.a[1] = tdiv(t.isin(bd.phase) * (r[6] - r[5]), 12) + mid;
                        bd.a[0] = appr(bd.a[0], bd.a[1]);
                        self.m[b] = t.rx(bd.a[0]);
                    } else if bits & 0xC != 0 {
                        bd.phase = (bd.phase + r[9]) & 0xFFFF;
                        let sn = t.isin(bd.phase);
                        let sn = if bits & 4 != 0 { sn } else { -sn };
                        bd.a[0] = tdiv(sn * (r[6] - r[5]), 12) + mid;
                        bd.a[1] = appr(bd.a[1], bd.a[0]);
                        self.m[b] = t.ry(bd.a[1]);
                    }
                } else if mode == 5 {
                    self.m[b] = t.rx(-0x71); // ok_br1_converter's approximation (Uriko's human node 25)
                } else {
                    self.m[b] = IDI; // mode 3 (Greg's) at rest
                }
                wm[b] = mmul(&wm[p], &self.m[b]);
            }
        }
    }
}
