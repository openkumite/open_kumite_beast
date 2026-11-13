//! Animation playback as the game does it: start, key setup, the per-frame step, the pose sampler.
//!
//! State: serialised: part of `GameState` (field changes are `STATE_VERSION` changes)
//!
//! Animation playback as the game does it: start, key setup,
//! per-frame step and the pose sampler: yaw, root offset and the per-node pose records
//! (`Fighter::pose`), which `fight/anim/pose.rs` turns into the bones' local matrices.
//!
//! An ACT animation is `u16 head, (u16 pose_offset, u16 duration)…` with duration 0 ending it.
//! Playback starts at key 1: the current pose blends into key 1 over key 0's duration, and so on.
//! Pose bytes: 0..1 yaw change, 2..7 root offset, 8..13 node 1 translation, 14..19 node 4 translation,
//! 20 + 6·k data slot k (u32 packed 10-bit direction, s16 twist), 116 / 118 hip twists, 120.. stretch records.

use crate::fight::fighter::Fighter;
use crate::fight::physics::{self, Trig};
use crate::formats::act_ani::{Act, Ani};
use serde::{Deserialize, Serialize};

/// Which ACT an animation comes from (state +0xAC = the fighter's own, +0xA8 = CMN.ACT).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum Src {
    #[default]
    Own,
    Common,
    /// the opponent's ACT (op 05 sub 5: throw victims play the thrower's animation)
    Opp,
}

/// The three ACTs an animation can come from.
#[derive(Clone, Copy)]
pub struct Acts<'a> {
    pub own: &'a Act,
    pub cmn: &'a Act,
    pub opp: &'a Act,
    /// S.BC = u16 at ANI+0x30: root-height scale of CMN / opponent animations (0x70 ALI, BAK, ULI, else 0x80)
    pub root_scale: i32,
    /// the fighter's ANI (node remap tables, header word 7)
    pub ani: &'a Ani,
}

/// The node → data slot remap table R (S+0xF8 / +0xFC, from ANI header word 7: a (straight, mirrored) pair of
/// 20-byte tables per form). R[n] = n − 1 straight; the mirrored one swaps the left and right limbs.
pub fn remap(ani: &Ani, form: usize, mirror: bool) -> [u8; 20] {
    let mut r = [0u8; 20];
    r.copy_from_slice(ani.bone_map(form, mirror));
    r
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Anim {
    pub id: i32, // state +0xA2
    pub src: Src,
    pub base: usize,     // state +0xB0 (offset of the animation in its ACT)
    pub target: usize,   // state +0xB8 (offset of the target pose)
    pub c0: i16,         // state +0xC0 (non-zero: keep the root offset when the next anim starts)
    pub c2: i16,         // +0xC2 total length (1/16)
    pub c4: i16,         // +0xC4 elapsed (1/16)
    pub key: i16,        // +0xC6 key number
    pub c8: i16,         // +0xC8 duration of the key being blended into
    pub left: i16,       // +0xCA time left to reach the target key (1/16)
    pub next_dur: i16,   // +0xCC duration of the following key (1/16), 0 = last
    pub speed: i16,      // +0xCE (16 = normal, 0 = finished)
    pub mirror: bool,    // state +0x100
    pub flip_next: bool, // state +0x9C (toggles `mirror` at the next start)
    pub scale: i32,      // state +0xBE: positive root-y scale (0x80 own ACT, S.BC for CMN / opponent ACT)
}

impl Anim {
    pub fn ended(&self) -> bool {
        self.next_dur == 0 && self.left <= 0
    }
}

fn u16at(b: &[u8], o: usize) -> u16 {
    b.get(o..o + 2).map(|s| u16::from_le_bytes([s[0], s[1]])).unwrap_or(0)
}

pub fn act_of<'a>(src: Src, acts: &Acts<'a>) -> &'a Act {
    match src {
        Src::Own => acts.own,
        Src::Common => acts.cmn,
        Src::Opp => acts.opp,
    }
}

/// Offset of animation `id` in the fighter's own ACT, else in CMN.ACT (op 05 sub 0).
pub fn locate(id: i32, own: &Act, cmn: &Act) -> Option<(Src, usize)> {
    if id < 0 {
        return None;
    }
    own.offset(id as usize).map(|o| (Src::Own, o)).or_else(|| cmn.offset(id as usize).map(|o| (Src::Common, o)))
}

pub fn start(f: &mut Fighter, t: &Trig, acts: &Acts, id: i32) {
    start_from(f, t, acts, locate(id, acts.own, acts.cmn), id)
}

/// Animation `id` of the opponent's ACT.
pub fn start_opp(f: &mut Fighter, t: &Trig, acts: &Acts, id: i32) {
    let loc = if id >= 0 { acts.opp.offset(id as usize).map(|o| (Src::Opp, o)) } else { None };
    start_from(f, t, acts, loc, id)
}

fn start_from(f: &mut Fighter, t: &Trig, acts: &Acts, loc: Option<(Src, usize)>, id: i32) {
    let Some((src, base)) = loc else {
        f.anim.id = id;
        f.anim.speed = 0;
        f.anim.left = 0;
        f.anim.next_dur = 0;
        return;
    };
    if f.anim.c0 == 0 {
        // twists wrap into ±half a turn, stretches clear (not nodes 1 and 4); directions and twists
        // stay, so the new animation blends from the current pose
        for n in 1..=18 {
            let r = &mut f.pose[n];
            r.twist = ((((r.twist as i32) << 19) >> 20) << 1) as i16;
            if n != 1 && n != 4 {
                r.stretch = [0; 3];
            }
        }
        physics::bake_root(f, t);
    }
    // the hand meshes follow the side flip (rotations are mirrored by the pose sampler)
    if f.anim.flip_next {
        f.swap_hands();
    }
    let a = &mut f.anim;
    a.scale = if src == Src::Own { 0x80 } else { acts.root_scale };
    a.mirror ^= a.flip_next;
    a.flip_next = false;
    a.c0 = 0;
    a.id = id;
    a.src = src;
    a.base = base;
    let b = &act_of(src, acts).bytes;
    a.c2 = (u16at(b, base) as i16).wrapping_mul(16);
    a.c4 = 0;
    a.key = 0;
    a.left = 0;
    a.next_dur = (u16at(b, base + 4 + 2) as i16).wrapping_mul(16);
    a.key = 1;
    key_setup(f, acts);
}

/// Aim at the pose of key `+0xC6`.
pub fn key_setup(f: &mut Fighter, acts: &Acts) {
    let step = f.step;
    let a = &mut f.anim;
    let b = &act_of(a.src, acts).bytes;
    let e = a.base + 4 + 4 * a.key as usize;
    let (off, dur) = (u16at(b, e), u16at(b, e + 2));
    a.c8 = a.next_dur;
    if a.next_dur != 0 {
        a.left = a.left.wrapping_add(a.next_dur);
        a.speed = step;
        a.target = a.base + off as usize;
        a.next_dur = (dur as i16).wrapping_mul(16);
    } else {
        a.speed = 0;
        a.left = 0;
    }
}

/// The per-frame step (timing + root/yaw sampling); afterwards the caller composes the yaw.
pub fn step(f: &mut Fighter, acts: &Acts) {
    if f.anim.id < 0 || (f.anim.src != Src::Opp && locate(f.anim.id, acts.own, acts.cmn).is_none()) {
        return;
    }
    if f.anim.speed != 0 {
        if f.anim.left <= 0 {
            f.anim.key += 1;
            key_setup(f, acts);
        }
        if f.anim.speed != 0 {
            sample(f, acts);
            f.anim.left = f.anim.left.wrapping_sub(f.anim.speed);
        } else if f.anim.c0 == 0 {
            sample(f, acts);
        }
    } else {
        sample(f, acts);
    }
    if f.anim.c4 < f.anim.c2 {
        f.anim.c4 = f.anim.c4.wrapping_add(f.step);
    }
}

/// Root offset and yaw part of the pose sampler: move toward the target pose by speed / time-left.
fn sample(f: &mut Fighter, acts: &Acts) {
    let a = &f.anim;
    let frac = if (a.speed as i32) < a.left as i32 { ((a.speed as i32) << 12) / a.left as i32 } else { 0x1000 };
    // the records start out zeroed: the first pose is taken as it is
    let rfrac = if f.pose_set { frac } else { 0x1000 };
    let b = &act_of(a.src, acts).bytes;
    let p = a.target;
    let rd = |o: usize| u16at(b, p + o) as i16 as i32;
    let m = a.mirror;
    let lerp = |cur: i16, tgt: i32| -> i16 {
        let d = (tgt - cur as i32) * frac;
        cur.wrapping_add(((if d < 0 { d + 0xFFF } else { d }) >> 12) as i16)
    };
    // yaw change
    let yaw = if m { -rd(0) } else { rd(0) };
    f.s_d2 = lerp(f.s_d2, yaw);
    // the root offset is not scaled (S.BE scales node 1, see actclip)
    let x = if m { -rd(2) } else { rd(2) };
    let y = rd(4);
    let z = rd(6);
    f.s_dc = lerp(f.s_dc, x);
    f.s_de = lerp(f.s_de, y);
    f.s_e0 = lerp(f.s_e0, z);
    let lerp = |cur: i16, tgt: i32| -> i16 {
        let d = (tgt - cur as i32) * rfrac;
        cur.wrapping_add(((if d < 0 { d + 0xFFF } else { d }) >> 12) as i16)
    };
    let lerp3 = |cur: &mut [i16; 3], t: [i32; 3]| {
        for k in 0..3 {
            cur[k] = lerp(cur[k], t[k]);
        }
    };
    // node 1's translation, positive y scaled
    let sc = if a.scale != 0 { a.scale } else { 0x80 };
    let y = rd(10);
    lerp3(&mut f.pose[1].stretch, [if m { -rd(8) } else { rd(8) }, if y > 0 { (y * sc) >> 7 } else { y }, rd(12)]);
    // node 4's, mirror negates y
    lerp3(&mut f.pose[4].stretch, [rd(14), if m { -rd(16) } else { rd(16) }, rd(18)]);
    let r = remap(acts.ani, f.beast() as usize, m);
    let u32at = |o: usize| b.get(o..o + 4).map(|s| u32::from_le_bytes([s[0], s[1], s[2], s[3]])).unwrap_or(0);
    let s10 = |v: u32| (((v & 0x3FF) << 22) as i32) >> 22;
    // nodes 1..16, direction ×32 and twist ×2 (node 1 is sampled unmirrored)
    for n in 1..=16 {
        let o = p + 20 + 6 * r[n] as usize;
        let w = u32at(o);
        let neg = m && n != 1;
        let (mut x, y, z) = (s10(w) << 5, s10(w >> 10) << 5, s10(w >> 20) << 5);
        let mut tw = u16at(b, o + 4) as i16 as i32;
        if neg {
            x = -x;
            tw = -tw;
        }
        let rec = &mut f.pose[n];
        lerp3(&mut rec.dir, [x, y, z]);
        rec.twist = lerp(rec.twist, (tw << 1) as i16 as i32);
    }
    // hips 17 / 18, twist only
    for n in 17..=18 {
        let mut tw = ((u16at(b, p + 0x54 + 2 * r[n] as usize) as i32) << 1) as i16 as i32;
        if m {
            tw = -tw;
        }
        f.pose[n].twist = lerp(f.pose[n].twist, tw);
    }
    // stretch records (slot, dx, dy, dz) until 0xFF; slot k goes to node R[k] + 1
    let mut q = p + 120;
    for _ in 0..32 {
        let Some(&slot) = b.get(q) else { break };
        if slot == 0xFF || q + 4 > b.len() {
            break;
        }
        let k = r.get(slot as usize).map_or(slot as usize, |&v| v as usize + 1);
        let v = |i: usize| ((b[q + i] as i8 as i32) << 4) as i16 as i32;
        if let Some(rec) = f.pose.get_mut(k) {
            lerp3(&mut rec.stretch, [if m { -v(1) } else { v(1) }, v(2), v(3)]);
        }
        q += 4;
    }
    f.pose_set = true;
}

/// Time in seconds at which the glTF clip shows the current pose (the clip has key k at sum(dur[0..k]) ticks).
pub fn glb_time(f: &Fighter, acts: &Acts) -> f32 {
    let a = &f.anim;
    let b = &act_of(a.src, acts).bytes;
    let mut ticks = 0i32;
    for k in 0..a.key.max(0) as usize {
        ticks += u16at(b, a.base + 4 + 4 * k + 2) as i32;
    }
    let t = ticks as f32 - (a.left.max(0) as f32) / 16.0;
    t.max(0.0) / 60.0
}
