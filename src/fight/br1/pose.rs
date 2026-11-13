//! An imported fighter's clip playback and skeleton: timing, the pose sampler, node matrices, the world walk.
//!
//! State: serialised: `Anim`, `Rec` and `Parts` (they become part of the imported fighter's state in stage S4)
//!
//! The game's script state S and fighter block F as far as the pose uses them; the fields name their offsets. The
//! matrix routines are the main game's (`fight/anim/pose.rs`: the same instructions, byte-identical tables); the
//! sampler is simpler than the main game's: fixed 120-byte keys, no stretch records, node 1 / node 4 translations kept in the
//! state, `left = next_dur` at each key (no carry-over), `frac` unclamped, a 1-frame key lasting one step (F.80).
//! Every multiplication is the MIPS `mult` / `mflo` (the low 32 bits, `wrapping_mul`), every store an s16.

use crate::fight::anim::pose::{build, mul_matrix2, mv, rot_matrix, rot_y};
use crate::fight::move_script::square_root0;
use crate::fight::physics::{Trig, wrap12};
use crate::formats::br1_anm::{Anm, Clips, s10x3};
use serde::{Deserialize, Serialize};

/// A 4.12 rotation matrix, rows first, as the GTE's (PS1 axes: y down).
pub type M = [[i32; 3]; 3];

pub const IDENTITY: M = [[0x1000, 0, 0], [0, 0x1000, 0], [0, 0, 0x1000]];

/// Nodes 0..18 of every form: the base skeleton the sampler and the matrices run on (19.. are chain nodes).
pub const BASE_NODES: usize = 19;

/// A pose record (`*(F+0x38)` + 0x18·n): the direction (raw 10-bit × 32, unnormalised) and the twist (raw × 2).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Rec {
    pub dir: [i16; 3],
    pub twist: i16,
}

/// Which clip block the running clip is in.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Src {
    /// the fighter's own ANM (S.B4)
    #[default]
    Own,
    /// CMN.ACT (S.B0)
    Cmn,
    /// the opponent's ANM (a throw victim playing the thrower's clip ≥ 0x100)
    Opp,
    /// the program image's clips (by file offset): CMN.ACT's table read past its end points there (clip 143,
    /// `script::ops`); a state file of an older build names it "Exe"
    #[serde(alias = "Exe")]
    Image,
}

/// The script-state fields of the animation.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Anim {
    /// S.A4: toggle the mirror at the next clip start (a byte XORed into S.128; op 0x06 could store any value, the
    /// scripts store 0 / 1)
    pub flip_next: bool,
    /// S.AA: clip index (op 0x05)
    pub id: i16,
    pub src: Src,
    /// S.B8: the clip (file offset in its block's file); None = S.B8 == 0, no clip
    pub clip: Option<usize>,
    /// S.C0: the target key (file offset)
    pub target: usize,
    /// S.C4: node 1 y scale of CMN / opponent clips (ANM +0x3C)
    pub root_scale: i16,
    /// S.C6: node 1 positive-y scale of the running clip (0x80 own, S.C4 CMN / opponent)
    pub scale: i16,
    /// S.C8: non-zero = no root bake at the next clip start
    pub keep_root: i16,
    /// S.CA: total length (frames × 16)
    pub total: i16,
    /// S.CC: elapsed (+= F.80 while < total)
    pub elapsed: i16,
    /// S.CE: key index
    pub key: i16,
    /// S.D0: duration of the key being blended into
    pub d0: i16,
    /// S.D2: time left to reach the target key
    pub left: i16,
    /// S.D4: duration of the following key, 0 = last
    pub next_dur: i16,
    /// S.D6: 0 = finished, else F.80 at key setup
    pub speed: i16,
    /// S.D8: facing
    pub yaw_base: i16,
    /// S.DA: animation yaw
    pub yaw: i16,
    /// S.DC / DE / E0: base position
    pub pos: [i16; 3],
    /// S.E4 / E6 / E8: root offset (1/16 units, facing-relative)
    pub root: [i16; 3],
    /// S.EC: node 1's bind offset (form setup)
    pub bind1: [i16; 3],
    /// S.F4: node 1's translation (1/16 units)
    pub t1: [i16; 3],
    /// S.FC: node 4's "bind" (never written: 0)
    pub bind4: [i16; 3],
    /// S.104: node 4's translation (1/16 units)
    pub t4: [i16; 3],
    /// S.10C: turn request
    pub turn: i16,
    /// S.128: mirror
    pub mirror: bool,
    /// S.12E / S.130: head turn yaw / pitch
    pub head_yaw: i16,
    pub head_pitch: i16,
}

impl Anim {
    /// The script condition 12 "own clip ended" (`S.D4 == 0 && S.D2 <= 0`).
    pub fn ended(&self) -> bool {
        self.next_dur == 0 && self.left <= 0
    }
}

/// A clip block and the bytes of its file.
#[derive(Clone, Copy)]
pub struct Bank<'a> {
    pub bytes: &'a [u8],
    pub clips: &'a Clips,
}

impl<'a> Bank<'a> {
    pub fn of(anm: &'a Anm) -> Self {
        Self { bytes: &anm.bytes, clips: &anm.clips }
    }
}

/// The blocks a fighter's clips come from: its own ANM, CMN.ACT and the opponent's ANM (None: a main-game
/// opponent has none).
#[derive(Clone, Copy)]
pub struct Banks<'a> {
    pub own: Bank<'a>,
    pub cmn: Bank<'a>,
    pub opp: Option<Bank<'a>>,
    /// the program image's clips (`Src::Image`)
    pub image: &'a [u8],
    /// the fighter's bone maps (its own ANM's header word 9; S+0x120 / +0x124 = the current form's): R[n] = the key
    /// slot node n reads
    pub maps: &'a [[u8; 20]; 4],
}

impl<'a> Banks<'a> {
    pub fn bank(&self, src: Src) -> Option<Bank<'a>> {
        match src {
            Src::Own => Some(self.own),
            Src::Cmn => Some(self.cmn),
            Src::Opp => self.opp,
            Src::Image => Some(Bank { bytes: self.image, clips: &NO_CLIPS }),
        }
    }
}

/// The static tables' image has no clip table.
static NO_CLIPS: Clips = Clips { base: 0, entries: Vec::new(), clips: Vec::new() };

pub(crate) fn rd16(b: &[u8], o: usize) -> i16 {
    #[cfg(test)]
    image_reads::note(b, o, 2);
    o.checked_add(2).and_then(|e| b.get(o..e)).map_or(0, |s| i16::from_le_bytes([s[0], s[1]]))
}
fn rd32(b: &[u8], o: usize) -> u32 {
    #[cfg(test)]
    image_reads::note(b, o, 4);
    o.checked_add(4).and_then(|e| b.get(o..e)).map_or(0, |s| u32::from_le_bytes([s[0], s[1], s[2], s[3]]))
}

/// (Tests) The reads of the clip bytes the program image's clips come from (`Br1Res::image_clips`, `Src::Image`): `watch`
/// names the buffers on this thread, `take` returns the (offset, length) of every read of them since.
#[cfg(test)]
pub(crate) mod image_reads {
    use std::cell::{Cell, RefCell};

    thread_local! {
        static WATCHING: Cell<bool> = const { Cell::new(false) };
        static WATCH: RefCell<(Vec<(usize, usize)>, Vec<(usize, usize)>)> = const { RefCell::new((Vec::new(), Vec::new())) };
    }

    pub(crate) fn watch(bufs: &[&[u8]]) {
        WATCHING.with(|w| w.set(!bufs.is_empty()));
        WATCH.with(|w| *w.borrow_mut() = (bufs.iter().map(|b| (b.as_ptr() as usize, b.len())).collect(), Vec::new()));
    }

    pub(crate) fn take() -> Vec<(usize, usize)> {
        WATCH.with(|w| std::mem::take(&mut w.borrow_mut().1))
    }

    /// Asserts that every read noted since `watch` lies whole inside one of the clips' ranges `ranges`
    /// (`Tables::clip_ranges`: none in the zeros between them; `what` names the run); the number of reads.
    pub(crate) fn check(ranges: &[(usize, usize)], what: &str) -> usize {
        let reads = take();
        for &(o, n) in &reads {
            assert!(ranges.iter().any(|&(a, b)| a <= o && o + n <= b), "{what}: the read of {o:#x}..+{n} is outside {ranges:x?}");
        }
        reads.len()
    }

    pub(super) fn note(b: &[u8], o: usize, n: usize) {
        if !WATCHING.with(Cell::get) {
            return;
        }
        WATCH.with(|w| {
            let mut w = w.borrow_mut();
            if w.0.contains(&(b.as_ptr() as usize, b.len())) {
                w.1.push((o, n));
            }
        });
    }
}

/// `cur + ((tgt − cur)·frac) / 4096` rounded toward zero, the product's low 32 bits, stored s16 (the sampler's
/// lerps).
fn lerp(cur: i16, tgt: i32, frac: i32) -> i16 {
    let d = tgt.wrapping_sub(cur as i32).wrapping_mul(frac);
    cur.wrapping_add(((if d < 0 { d.wrapping_add(0xFFF) } else { d }) >> 12) as i16)
}

/// Op 0x05 sub 0's table walk (steps 3..5): the own entry, else below 0x100 CMN.ACT's (scale
/// S.C4), else the opponent's entry (scale S.C4). None: no clip (the op then starts state 1; an index past the own
/// table's count does nothing; both are the interpreter's).
pub fn locate(id: i32, banks: &Banks, root_scale: i16) -> Option<(Src, usize, i16)> {
    if id < 0 || id as usize >= banks.own.clips.count() {
        return None;
    }
    let id = id as usize;
    if let Some(c) = banks.own.clips.get(id) {
        return Some((Src::Own, c.offset, 0x80));
    }
    if id < 0x100 {
        return banks.cmn.clips.get(id).map(|c| (Src::Cmn, c.offset, root_scale));
    }
    banks.opp.and_then(|b| b.clips.get(id)).map(|c| (Src::Opp, c.offset, root_scale))
}

/// Op 0x05 sub 0: clip `id` found by `locate` set (S.AA, S.B8, S.C6) and started; false (nothing changed) when none.
pub fn start_clip(a: &mut Anim, rec: &mut [Rec], banks: &Banks, t: &Trig, step: i16, id: i32) -> bool {
    let Some((src, clip, scale)) = locate(id, banks, a.root_scale) else { return false };
    a.src = src;
    a.clip = Some(clip);
    a.scale = scale;
    a.id = id as i16;
    start(a, rec, banks, t, step);
    true
}

/// Start the clip S.B8 (set by op 0x05). The held pose blends into key 1 over key 0's duration.
pub fn start(a: &mut Anim, rec: &mut [Rec], banks: &Banks, t: &Trig, step: i16) {
    if a.keep_root == 0 {
        root_bake(a, rec, t);
    }
    a.mirror ^= a.flip_next;
    a.flip_next = false;
    a.keep_root = 0;
    let (Some(clip), Some(bank)) = (a.clip, banks.bank(a.src)) else { return };
    a.elapsed = 0;
    a.key = 0;
    a.total = rd16(bank.bytes, clip).wrapping_shl(4);
    let d0 = rd16(bank.bytes, clip + 6);
    a.next_dur = if d0 == 1 { step } else { d0.wrapping_shl(4) };
    a.key = 1;
    key_setup(a, banks, step);
}

/// Twists of nodes 1..18 wrapped into ±half a turn (even); the root offset committed into the base
/// position along the facing, the animation yaw and the turn request into the facing; all three cleared.
pub fn root_bake(a: &mut Anim, rec: &mut [Rec], t: &Trig) {
    for r in rec.iter_mut().take(BASE_NODES).skip(1) {
        r.twist = (((((r.twist as u16 as u32) << 19) as i32) >> 20) << 1) as i16;
    }
    let yaw = a.yaw_base as i32;
    let [ex, ey, ez] = a.root.map(|v| v as i32);
    let (c, s) = (t.cos(yaw), t.sin(yaw));
    a.pos[1] = a.pos[1].wrapping_add((ey >> 4) as i16);
    a.pos[0] = a.pos[0].wrapping_add((ex.wrapping_mul(c).wrapping_sub(ez.wrapping_mul(s)) >> 16) as i16);
    a.yaw_base = wrap12(yaw + a.yaw as i32 + a.turn as i32);
    a.pos[2] = a.pos[2].wrapping_add((ex.wrapping_mul(s).wrapping_add(ez.wrapping_mul(c)) >> 16) as i16);
    a.root = [0; 3];
    a.turn = 0;
    a.yaw = 0;
}

/// Aim at key S.CE. The time left is the new key's whole duration (main-game adds it to what is left).
pub fn key_setup(a: &mut Anim, banks: &Banks, step: i16) {
    let (Some(clip), Some(bank)) = (a.clip, banks.bank(a.src)) else { return };
    let e = clip + 4 + 4 * a.key as u16 as usize;
    let (off, dur) = (rd16(bank.bytes, e), rd16(bank.bytes, e + 2));
    a.d0 = a.next_dur;
    if a.next_dur != 0 {
        a.left = a.next_dur;
        a.speed = step;
        a.target = clip.wrapping_add_signed(off as isize);
        a.next_dur = if dur == 1 { step } else { dur.wrapping_shl(4) };
    } else {
        a.speed = 0;
        a.left = 0;
    }
}

/// The timing and sampling part of the animation step (before the local matrices).
pub fn step(a: &mut Anim, rec: &mut [Rec], banks: &Banks, form: usize, step: i16) {
    if a.clip.is_none() {
        return;
    }
    if a.speed != 0 {
        if a.left <= 0 {
            a.key = a.key.wrapping_add(1);
            key_setup(a, banks, step);
        }
        if a.speed != 0 {
            sample(a, rec, banks, form);
            a.left = a.left.wrapping_sub(a.speed);
        } else if a.keep_root == 0 {
            sample(a, rec, banks, form);
        }
    } else {
        sample(a, rec, banks, form);
    }
    if a.elapsed < a.total {
        a.elapsed = a.elapsed.wrapping_add(step);
    }
}

/// Move every record toward the target key by speed / time left. The bone map R (S+0x120 + 4·mirror)
/// is `form`'s.
pub fn sample(a: &mut Anim, rec: &mut [Rec], banks: &Banks, form: usize) {
    if a.left == 0 {
        return;
    }
    let Some(bank) = banks.bank(a.src) else { return };
    let frac = ((a.speed as i32) << 12) / a.left as i32;
    let (m, b, p) = (a.mirror, bank.bytes, a.target);
    let r = banks.maps[(form & 1) * 2 + m as usize];
    let ps = |o: usize| rd16(b, p.wrapping_add(o)) as i32;
    // the animation yaw
    a.yaw = lerp(a.yaw, if m { -ps(0) } else { ps(0) }, frac);
    // (±x, y > 0 ? y·scale >> 7 : y, z)
    let root = |cur: &mut [i16; 3], src: usize, mm: bool, sc: i32| {
        let x = if mm { -ps(src) } else { ps(src) };
        let mut y = ps(src + 2);
        if y > 0 {
            y = y.wrapping_mul(sc) >> 7;
        }
        for (c, v) in cur.iter_mut().zip([x, y, ps(src + 4)]) {
            *c = lerp(*c, v, frac);
        }
    };
    root(&mut a.root, 2, m, 0x80);
    root(&mut a.t1, 8, m, a.scale as i32);
    // node 4: never mirrored, never scaled
    root(&mut a.t4, 14, false, 0x80);
    // nodes 1..16, direction × 32 and twist × 2 (node 1 is sampled unmirrored)
    for n in 1..=16 {
        let o = p.wrapping_add(20 + 6 * r[n] as usize);
        let w = rd32(b, o);
        let [mut x, y, z] = s10x3(w).map(|v| v << 5);
        let mut tw = rd16(b, o + 4) as i32;
        if m && n != 1 {
            x = -x;
            tw = -tw;
        }
        let Some(q) = rec.get_mut(n) else { return };
        q.dir = [lerp(q.dir[0], x, frac), lerp(q.dir[1], y, frac), lerp(q.dir[2], z, frac)];
        q.twist = lerp(q.twist, tw << 1, frac);
    }
    // hips 17 / 18, twist only
    for n in 17..=18 {
        let mut tw = (rd16(b, p.wrapping_add(0x54 + 2 * r[n] as usize)) as i32) << 1;
        if m {
            tw = -tw;
        }
        let Some(q) = rec.get_mut(n) else { return };
        q.twist = lerp(q.twist, tw, frac);
    }
}

/// The animation step's turn request (S.10C ≠ 0): the base position pivots about node 0's world position `w0` (x, z of
/// MD+0x34 / +0x3C, the last world walk), the facing takes the turn.
pub fn turn(a: &mut Anim, w0: [i32; 3], t: &Trig) {
    if a.turn == 0 {
        return;
    }
    let ang = -(a.turn as i32);
    let dx = (a.pos[0] as i32).wrapping_sub(w0[0]);
    let dz = (a.pos[2] as i32).wrapping_sub(w0[2]);
    let (s, c) = (t.sin(ang), t.cos(ang));
    a.pos[0] = w0[0].wrapping_add(dz.wrapping_mul(s).wrapping_add(dx.wrapping_mul(c)) >> 12) as i16;
    a.pos[2] = w0[2].wrapping_add(dz.wrapping_mul(c).wrapping_sub(dx.wrapping_mul(s)) >> 12) as i16;
    a.yaw_base = wrap12(a.yaw_base as i32 + a.turn as i32);
    a.turn = 0;
}

/// F.46 as the animation step writes it: facing + animation yaw (+ the turn request, 0 by then).
pub fn facing(a: &Anim) -> i16 {
    wrap12(a.yaw_base as i32 + a.yaw as i32 + a.turn as i32)
}

/// Node 1's height halved (the animation step's tail): `F.4C & 8`, or the opponent's `F.4C & 8` with
/// `(F.50 | opp.F.50) & 0x40100000`.
pub fn node1_halved(f4c: u32, f50: u32, o4c: u32, o50: u32) -> bool {
    f4c & 8 != 0 || o4c & 8 != 0 && (f50 | o50) & 0x4010_0000 != 0
}

/// One form's skeleton as the form setup builds it.
#[derive(Clone, Debug)]
pub struct Skeleton {
    /// parent per node (−1 root, −2 node 0)
    pub parents: Vec<i16>,
    /// static local translation per node (bind offset; node 0's is the position, set by the mover)
    pub t16: Vec<[i32; 3]>,
    /// base matrix per node (F+0x3C / +0x40: node 0 identity, node n RotMatrix of 4 × its base angles)
    pub bm: Vec<M>,
}

impl Skeleton {
    /// The form setup for form `form` of `anm`.
    pub fn new(anm: &Anm, form: usize, t: &Trig) -> Self {
        let f = &anm.forms[form & 1];
        let n = f.nodes();
        let mut t16 = vec![[0; 3]; n];
        let mut bm = vec![IDENTITY; n];
        for k in 1..n {
            t16[k] = f.bind[k - 1];
            let [x, y, z] = f.base_angles[k - 1];
            bm[k] = rot_matrix(4 * x, 4 * y, 4 * z, t);
        }
        Self { parents: f.parents.clone(), t16, bm }
    }

    pub fn nodes(&self) -> usize {
        self.parents.len()
    }

    /// S.EC: node 1's bind offset (copied by the form setup).
    pub fn bind1(&self) -> [i16; 3] {
        self.t16[1].map(|v| v as i16)
    }
}

/// The local matrices of nodes 1..18 from the records (`BM · Swing(dir) · RotY(−twist)` for 1..16, the
/// hips RotY(−twist)); the other entries stay identity.
pub fn local_matrices(rec: &[Rec], skel: &Skeleton, t: &Trig, rsqrt: &dyn Fn(i32) -> i32) -> Vec<M> {
    let mut out = vec![IDENTITY; skel.nodes().max(BASE_NODES)];
    for n in 1..BASE_NODES {
        let r = rec.get(n).copied().unwrap_or_default();
        out[n] = if n <= 16 {
            let bm = skel.bm.get(n).copied().unwrap_or(IDENTITY);
            build(&bm, r.dir.map(|v| v as i32), (r.twist as i32) >> 1, t, rsqrt)
        } else {
            rot_y(-(r.twist as i32) >> 1, t)
        };
    }
    out
}

/// Node 0's local matrix: RotMatrix(F.44, −F.46, F.48).
pub fn root_matrix(angles: [i16; 3], t: &Trig) -> M {
    rot_matrix(angles[0] as i32, -(angles[1] as i32), angles[2] as i32, t)
}

/// The local translations: the skeleton's, with node 0 at `pos` (MD0's, the mover's), node 1 = S.EC + S.F4 >> 4 (y
/// halved toward zero when `half`), node 4 = S.FC + S.104 >> 4, each an s16.
pub fn local_translations(a: &Anim, skel: &Skeleton, pos: [i32; 3], half: bool) -> Vec<[i32; 3]> {
    let mut t = skel.t16.clone();
    t[0] = pos;
    let add = |b: [i16; 3], d: [i16; 3]| -> [i32; 3] { std::array::from_fn(|k| b[k].wrapping_add(d[k] >> 4) as i32) };
    t[1] = add(a.bind1, a.t1);
    if half {
        t[1][1] = (t[1][1] / 2) as i16 as i32;
    }
    if t.len() > 4 {
        t[4] = add(a.bind4, a.t4);
    }
    t
}

/// The head turn's matrix: node 2 ← RotMatrix(S.130, −S.12E, 0) · node 2 (MulMatrix2).
pub fn turn_head(m2: &M, a: &Anim, t: &Trig) -> M {
    mul_matrix2(&rot_matrix(a.head_pitch as i32, -(a.head_yaw as i32), 0, t), m2)
}

/// A node's world matrix and translation.
pub type World = (M, [i32; 3]);

/// The world matrices and translations of every node from the local ones; node 0's world is its local
/// (the mover's). `world.t = W.m · t16 >> 12 (IR saturated) + W.t`, `world.m = W.m · local`;
/// a node's world depends only on its parent's, so the recursion's order does not matter.
/// Nodes past `m` / `t16` (or the skeleton) are left out (none when either is empty), a parent past them as missing.
pub fn world(skel: &Skeleton, m: &[M], t16: &[[i32; 3]]) -> Vec<World> {
    let n = skel.nodes().min(m.len()).min(t16.len());
    if n == 0 {
        return Vec::new();
    }
    let mut out: Vec<Option<World>> = vec![None; n];
    out[0] = Some((m[0], t16[0]));
    // parents may come after their children (MIT's 2 → 3 → 4): repeat until every node is placed (a tree, checked
    // by the parser, so at most n passes)
    for _ in 0..n {
        let mut done = true;
        for k in 1..n {
            if out[k].is_some() {
                continue;
            }
            let p = if skel.parents[k] == -2 { 0 } else { skel.parents[k] };
            let Some(parent) = usize::try_from(p).ok().and_then(|p| out.get(p).copied().flatten()) else {
                done = false;
                continue;
            };
            out[k] = Some(world_step(&parent, &m[k], t16[k]));
        }
        if done {
            break;
        }
    }
    out.into_iter().map(|w| w.unwrap_or((IDENTITY, [0; 3]))).collect()
}

/// A CRC-32 of a pose for the comparisons with recorded runs (`cliptest br1`, the tests): the local matrices of
/// nodes 0..18, the local translations of nodes 1.., the world matrices of nodes 1..18 and their translations
/// relative to the parent's (the push-apart and the walls shift whole subtrees after the walk).
pub fn digest(parents: &[i16], m: &[M], t16: &[[i32; 3]], world: &[World]) -> u32 {
    let mut b = Vec::new();
    let mut put = |v: i32| b.extend_from_slice(&v.to_le_bytes());
    for mn in m.iter().take(BASE_NODES) {
        mn.iter().flatten().for_each(|&v| put(v));
    }
    for t in t16.iter().skip(1) {
        t.iter().for_each(|&v| put(v));
    }
    for n in 1..BASE_NODES.min(world.len()) {
        let p = parents.get(n).map_or(0, |&p| p.max(0) as usize);
        world[n].0.iter().flatten().for_each(|&v| put(v));
        (0..3).for_each(|i| put(world[n].1[i].wrapping_sub(world[p].1[i])));
    }
    crate::base::hash::crc32(&b)
}

/// One node of the walk: its world from its parent's (`W.m · t16 >> 12`, IR saturated, `+ W.t`; `W.m · local`).
pub fn world_step(parent: &World, m: &M, t16: [i32; 3]) -> World {
    let (wm, wt) = parent;
    let d = mv(wm, t16);
    (mul_matrix2(wm, m), std::array::from_fn(|i| wt[i].wrapping_add(d[i])))
}

/// What the head turn reads besides the fighter's own state: node 0's and node 2's world translations of both
/// fighters from the last world walk (MD+0x34.. / +0xD4..).
#[derive(Clone, Copy, Debug)]
pub struct HeadInputs {
    pub own0: [i32; 3],
    pub own2: [i32; 3],
    pub opp0: [i32; 3],
    pub opp2: [i32; 3],
}

/// The tail of the script step, after the animation step: with `on` (F.54 &
/// 0x10000 and F.0 < 0x80) the yaw (±0x31C, ±0x2D a frame) and pitch (−0x271..0x1C7, ±0x22 a frame) accumulators
/// step toward the opponent's head; otherwise both are 0. The same code as the main game's
/// (`move_script::head_turn`), on the imported game's fields. `f46` = F.46 as the step left it.
pub fn head_turn(a: &mut Anim, rec: &[Rec], on: bool, f46: i16, w: &HeadInputs, t: &Trig, sqrt: &dyn Fn(i32) -> i32) {
    if !on {
        a.head_yaw = 0;
        a.head_pitch = 0;
        return;
    }
    let r2 = rec.get(2).copied().unwrap_or_default();
    let tw = (r2.twist as i32) >> 1;
    let mut y = wrap12(t.atan2(w.own0[2].wrapping_sub(w.opp0[2]), w.opp0[0].wrapping_sub(w.own0[0])) - f46 as i32 + tw) as i32;
    if y.abs() >= 0x31D {
        y = if y < 0 { -0x31C } else { 0x31C };
    }
    // the pitch's bias for an opponent behind: never taken after the clamp above (kept as the game has it)
    let bias = if y.abs() >= 0x401 { (0x800 - y.abs()) / 8 } else { 0 };
    let mut d = y - wrap12(tw + a.head_yaw as i32) as i32;
    if d.abs() >= 0x2E {
        d = if d < 0 { -0x2D } else { 0x2D };
    }
    a.head_yaw = a.head_yaw.wrapping_add(d as i16);
    // Square0: the GTE's IR1 / IR3 take the low 16 bits
    let (dx, dz) = (w.opp2[0].wrapping_sub(w.own2[0]) as i16 as i32, w.opp2[2].wrapping_sub(w.own2[2]) as i16 as i32);
    // SquareRoot0: the main game's routine on the imported game's table (`math.sqrt`, `tables.okt`)
    let r = square_root0(sqrt, (dx * dx).wrapping_add(dz * dz));
    // the height difference kept within atan2's range (an engine guard, as for the main game)
    let p = t.atan2(r, w.opp2[1].wrapping_sub(w.own2[1]).clamp(-(1 << 19), 1 << 19));
    let q = t.atan2(r2.dir[1] as i32, r2.dir[2] as i32);
    let target = (wrap12(p + q + bias) as i32).clamp(-0x271, 0x1C7);
    let mut d = target - wrap12(q + a.head_pitch as i32) as i32;
    if d.abs() >= 0x23 {
        d = if d < 0 { -0x22 } else { 0x22 };
    }
    a.head_pitch = a.head_pitch.wrapping_add(d as i16);
}

/// Object visibility of both forms' object lists (`F.24` +0x0C / +0x28 lists, records of 0x7C bytes, +0x0C / +0x0E
/// = 1 hidden): bit k of `hidden[form]` = object k (0..25).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Parts {
    pub hidden: [u32; 2],
}

/// Objects 15..18 (the alternate hands, extras): hidden by the ANM loader.
const LOADER_HIDDEN: u32 = 0xF << 15;

impl Default for Parts {
    fn default() -> Self {
        Self { hidden: [LOADER_HIDDEN; 2] }
    }
}

impl Parts {
    /// The state start: hands 11 / 14 shown, 15 / 16 hidden in the current form's list.
    pub fn reset_hands(&mut self, form: usize) {
        let h = &mut self.hidden[form & 1];
        *h = (*h & !(1 << 11 | 1 << 14)) | 1 << 15 | 1 << 16;
    }

    /// Op 0x1E: object `k` hidden (`flag` ≠ 0) or shown in the current form's list; mirrored, the hands
    /// 11 ↔ 14 and 15 ↔ 16 swap; 0x80 = all objects 0..25.
    pub fn set(&mut self, form: usize, k: u8, flag: u16, mirror: bool) {
        let h = &mut self.hidden[form & 1];
        let bits = match k {
            0x80 => (1 << 26) - 1,
            k if k < 26 => {
                let k = if mirror {
                    match k {
                        11 => 14,
                        14 => 11,
                        15 => 16,
                        16 => 15,
                        k => k,
                    }
                } else {
                    k
                };
                1u32 << k
            }
            _ => 0,
        };
        if flag != 0 { *h |= bits } else { *h &= !bits }
    }

    pub fn is_hidden(&self, form: usize, k: usize) -> bool {
        k < 26 && self.hidden[form & 1] & (1 << k) != 0
    }
}

/// The whole pose of a frame as the game holds it after the animation step and the head turn: local matrices and
/// translations (MD+0x04 / +0x18) of every node.
#[allow(clippy::too_many_arguments)]
pub fn locals(
    a: &Anim,
    rec: &[Rec],
    skel: &Skeleton,
    angles: [i16; 3],
    pos: [i32; 3],
    half: bool,
    head: bool,
    t: &Trig,
    rsqrt: &dyn Fn(i32) -> i32,
) -> (Vec<M>, Vec<[i32; 3]>) {
    let mut m = local_matrices(rec, skel, t, rsqrt);
    m[0] = root_matrix(angles, t);
    if head {
        m[2] = turn_head(&m[2], a, t);
    }
    m.truncate(skel.nodes());
    (m, local_translations(a, skel, pos, half))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fight::br1::res::Br1Res;
    use crate::formats::br1_anm::tests::{br1_file, synthetic};

    fn trig() -> Trig {
        // a sine table good enough for the synthetic tests (the real one comes from the static tables)
        let mut b = Vec::new();
        for k in 0..0x802 {
            let v = if k < 0x401 { ((k as f64 * std::f64::consts::PI / 2048.0).sin() * 4096.0).round() as i16 } else { 0 };
            b.extend_from_slice(&v.to_le_bytes());
        }
        Trig::parse(&b)
    }

    /// A self-describing state file of an older build names the image's clips "Exe": it reads as `Src::Image`, and a
    /// new one names them "Image".
    #[test]
    fn src_reads_the_old_name() {
        assert_eq!(serde_json::from_str::<Src>("\"Exe\"").unwrap(), Src::Image);
        assert_eq!(serde_json::from_str::<Src>("\"Image\"").unwrap(), Src::Image);
        assert_eq!(serde_json::to_string(&Src::Image).unwrap(), "\"Image\"");
    }

    #[test]
    fn lerp_rounds_toward_zero_and_wraps() {
        assert_eq!(lerp(0, 100, 0x800), 50);
        assert_eq!(lerp(0, -3, 0x800), -1);
        assert_eq!(lerp(0, 3, 0x800), 1);
        assert_eq!(lerp(10, 20, 0x1000), 20);
        // frac is not clamped: past the key
        assert_eq!(lerp(0, 100, 0x2000), 200);
        // the product's low 32 bits (mult / mflo)
        assert_eq!(lerp(-0x8000, 0x7FFF, 1 << 20), (-0x8000i32 + ((0xFFFFi32.wrapping_mul(1 << 20)) >> 12)) as i16);
    }

    /// The synthetic clip (key 0 of 4 frames, key 1 with yaw 0x40): the held pose reaches key 1 in 4 steps of 16, a
    /// 1-frame key lasts one step, the clip then ends.
    #[test]
    fn synthetic_clip_plays() {
        let t = trig();
        let anm = Anm::parse(synthetic()).unwrap();
        let cmn = Clips::default();
        let banks = Banks { own: Bank::of(&anm), cmn: Bank { bytes: &[], clips: &cmn }, opp: None, image: &[], maps: &anm.bone_maps };
        let mut a = Anim { root_scale: 112, ..Default::default() };
        let mut rec = vec![Rec::default(); BASE_NODES];
        assert!(!start_clip(&mut a, &mut rec, &banks, &t, 16, 0));
        assert!(!start_clip(&mut a, &mut rec, &banks, &t, 16, 3));
        assert!(start_clip(&mut a, &mut rec, &banks, &t, 16, 1));
        assert_eq!((a.id, a.src, a.scale, a.total, a.key, a.left, a.speed, a.next_dur), (1, Src::Own, 0x80, 5 * 16, 1, 64, 16, 0));
        let mut yaws = Vec::new();
        for _ in 0..6 {
            step(&mut a, &mut rec, &banks, 0, 16);
            yaws.push(a.yaw);
        }
        assert_eq!(yaws, [16, 31, 47, 64, 64, 64]);
        assert!(a.ended());
        assert_eq!(a.speed, 0);
        // the root bake at the next start commits the yaw into the facing
        a.clip = Some(anm.clips.get(1).unwrap().offset);
        start(&mut a, &mut rec, &banks, &t, 16);
        assert_eq!((a.yaw_base, a.yaw), (64, 0));
        // a 1-frame key at a step of 8 lasts one step
        let mut b = Anim { clip: a.clip, next_dur: 16, key: 0, ..Default::default() };
        key_setup(&mut b, &banks, 8);
        assert_eq!((b.left, b.speed, b.next_dur), (16, 8, 64));
    }

    #[test]
    fn root_bake_wraps_twists() {
        let t = trig();
        let mut a = Anim { yaw_base: 0, yaw: 0x100, root: [16 << 4, 32, 0], ..Default::default() };
        let mut rec = vec![Rec { dir: [0; 3], twist: 0x1001 }; BASE_NODES];
        rec[0].twist = 0x1001;
        root_bake(&mut a, &mut rec, &t);
        // node 0 untouched; 1..18 into ±0x1000, even
        assert_eq!((rec[0].twist, rec[1].twist, rec[18].twist), (0x1001, -0x1000, -0x1000));
        assert_eq!((a.pos, a.yaw_base, a.yaw, a.root), ([16, 2, 0], 0x100, 0, [0; 3]));
    }

    #[test]
    fn parts_follow_the_mirror() {
        let mut p = Parts::default();
        assert!(p.is_hidden(0, 15) && p.is_hidden(1, 18) && !p.is_hidden(0, 11));
        p.set(0, 11, 1, true);
        assert!(p.is_hidden(0, 14) && !p.is_hidden(0, 11));
        p.reset_hands(0);
        assert!(!p.is_hidden(0, 14) && p.is_hidden(0, 16) && p.is_hidden(0, 17));
        p.set(1, 0x80, 0, false);
        assert_eq!(p.hidden[1], 0);
        p.set(1, 0x80, 1, false);
        assert_eq!(p.hidden[1], (1 << 26) - 1);
        p.set(1, 26, 0, false);
        assert_eq!(p.hidden[1], (1 << 26) - 1);
    }

    /// The pose of every fighter in the recorded RAM dumps `pose1` (MITSUKO vs FOX: walking,
    /// punches, kicks, the transformation, the beast form, a throw), as `cliptest br1 … fixture` extracted it: the
    /// engine's local matrices, translations and world walk against a CRC-32 of the game's, and for consecutive dumps
    /// the animation step, the facing and the head turn of the next frame.
    #[test]
    fn recorded_dumps() {
        let (Some(cmn), Some(tables)) = (br1_file("engine/CMN/CMN.ACT"), crate::formats::tables::br1_export_tables()) else {
            eprintln!("skipped: no okb_export/br1");
            return;
        };
        let tables = crate::fight::br1::res::Br1Tables::new(tables).unwrap();
        let mut res: std::collections::HashMap<String, Br1Res> = Default::default();
        let cases: Vec<serde_json::Value> =
            include_str!("fixtures/pose1.jsonl").lines().filter(|l| !l.starts_with('#')).map(|l| serde_json::from_str(l).unwrap()).collect();
        for c in &cases {
            let code = c["code"].as_str().unwrap();
            if !res.contains_key(code) {
                let anm = br1_file(&format!("engine/{code}/{code}31.DAT")).unwrap();
                res.insert(code.into(), Br1Res::new(code, anm, cmn.clone(), &tables).unwrap());
            }
        }
        image_reads::watch(&res.values().map(|r| &r.image_clips[..]).collect::<Vec<_>>());
        let num = |v: &serde_json::Value| v.as_i64().unwrap() as i32;
        let v3 = |v: &serde_json::Value| -> [i32; 3] { std::array::from_fn(|i| num(&v[i])) };
        let anim = |c: &serde_json::Value| -> Anim { serde_json::from_value(c["anim"].clone()).unwrap() };
        let recs = |c: &serde_json::Value| -> Vec<Rec> {
            c["rec"].as_array().unwrap().iter().map(|r| Rec { dir: std::array::from_fn(|i| num(&r[i]) as i16), twist: num(&r[3]) as i16 }).collect()
        };
        let (mut poses, mut steps) = (0, 0);
        for (i, c) in cases.iter().enumerate() {
            let r = &res[c["code"].as_str().unwrap()];
            let rs = r.tables.rsqrt_fn();
            let form = num(&c["form"]) as usize;
            let skel = &r.skels[form];
            let a = anim(c);
            let angles = v3(&c["angles"]).map(|v| v as i16);
            let (mut m, mut t16) = locals(&a, &recs(c), skel, angles, v3(&c["pos"]), c["half"].as_bool().unwrap(), c["head"].as_bool().unwrap(), &r.trig, &rs);
            t16[0] = v3(&c["w0"]);
            m.truncate(skel.nodes());
            let w = world(skel, &m, &t16);
            assert_eq!(digest(&skel.parents, &m, &t16, &w), c["digest"].as_u64().unwrap() as u32, "frame {} side {}", c["frame"], c["side"]);
            poses += 1;
            // the next frame of the same fighter
            let Some(n) = cases[i + 1..].iter().find(|n| n["side"] == c["side"] && num(&n["frame"]) == num(&c["frame"]) + 1) else { continue };
            let opp = cases.iter().find(|o| o["frame"] == c["frame"] && o["side"] != c["side"]).unwrap();
            let (want, mut got, mut rec) = (anim(n), a.clone(), recs(c));
            if got.turn == 0 {
                got.yaw_base = want.yaw_base; // the auto-turn before the script
            }
            let banks = r.banks(Some(&res[opp["code"].as_str().unwrap()].anm));
            step(&mut got, &mut rec, &banks, form, num(&n["step"]) as i16);
            turn(&mut got, v3(&c["w0"]), &r.trig);
            assert_eq!(rec[1..], recs(n)[1..], "records, frame {}", n["frame"]);
            let f46 = facing(&got);
            assert_eq!(f46, num(&n["angles"][1]) as i16, "F.46, frame {}", n["frame"]);
            let w = HeadInputs { own0: v3(&c["w0"]), own2: v3(&c["w2"]), opp0: v3(&opp["w0"]), opp2: v3(&opp["w2"]) };
            let sq = r.tables.sqrt_fn();
            head_turn(&mut got, &rec, n["head"].as_bool().unwrap(), f46, &w, &r.trig, &sq);
            got.pos = want.pos; // the physics moves it later in the frame
            assert_eq!(got, want, "state, frame {}", n["frame"]);
            steps += 1;
        }
        assert_eq!((poses, steps), (30, 18));
        if let Some(r) = res.values().next() {
            image_reads::check(&r.tables.clip_ranges(), "the dumps");
        }
    }
}
