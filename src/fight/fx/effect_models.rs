//! 3-D effect models: the 0xED1C4 records.
//!
//! State: serialised: part of `GameState` (field changes are `STATE_VERSION` changes)
//!
//! 3-D effect models: the 0xED1C4 records (checked word for word against a reference runtime
//! of the records). model_create, parts, tracks,
//! unlit drawers, CLUT set, UV scroll.

use serde::{Deserialize, Serialize};
use std::sync::Arc;

pub const MAGIC: u32 = 0xED1C4;

fn u32at(b: &[u8], o: usize) -> u32 {
    b.get(o..o + 4).map(|s| u32::from_le_bytes([s[0], s[1], s[2], s[3]])).unwrap_or(0)
}
fn u16at(b: &[u8], o: usize) -> u16 {
    b.get(o..o + 2).map(|s| u16::from_le_bytes([s[0], s[1]])).unwrap_or(0)
}
fn i16at(b: &[u8], o: usize) -> i16 {
    u16at(b, o) as i16
}

/// Face types: vertices per face, colour words per face, textured.
const TYPES: [(usize, usize, bool); 8] = [(3, 1, false), (4, 1, false), (3, 3, false), (4, 4, false), (3, 1, true), (4, 1, true), (3, 3, true), (4, 4, true)];

#[derive(Clone, Debug)]
pub struct Face {
    pub idx: [u16; 4],
    pub flags: u16,
    pub col: [[u8; 3]; 4],
    pub clut: u16,
    pub tpage: u16,
    pub uv: [[u8; 2]; 4],
}

#[derive(Clone, Debug)]
pub struct Part {
    pub ty: usize,
    pub faces: Vec<Face>,
}

#[derive(Clone, Debug)]
pub struct Seg {
    pub dur: i32,
    pub a: usize,
    pub b: usize,
    pub idx: Vec<i16>,
}

#[derive(Debug, Default)]
pub struct Record {
    pub b: Vec<u8>,
    pub vtx_anim: u32,
    pub col_anim: u32,
    pub abr: u32,
    pub nvtx_copy: u32,
    pub ncol: u32,
    pub total_frames: i32,
    pub counts: [u32; 8],
    pub anims: Vec<Vec<Vec<i16>>>,
    pub vsegs: Vec<Seg>,
    pub csegs: Vec<Seg>,
    pub parts: Vec<Part>,
    pub vtx: Vec<[i16; 3]>,
}

impl Record {
    pub fn parse(b: &[u8]) -> Option<Arc<Record>> {
        if b.len() < 0xB0 || u32at(b, 0) != MAGIC {
            return None;
        }
        let h = |i: usize| u32at(b, 4 * i) as usize;
        let (vtx_anim, col_anim, abr) = (h(3) as u32, h(5) as u32, h(7) as u32);
        let (nsegs, nanims, nvtx_copy, ncol, total) = (h(9), h(10), h(11) as u32, h(12) as u32, h(14) as i32);
        let counts: [u32; 8] = std::array::from_fn(|t| h(23 + t) as u32);
        let (p_anims, p_parts, p_vseg, p_cseg, p_vtx) = (h(31), h(32), h(33), h(35), h(36));
        let mut anims = Vec::new();
        for a in 0..nanims.min(64) {
            let (n, lp) = (u32at(b, p_anims + 8 * a) as usize, u32at(b, p_anims + 8 * a + 4) as usize);
            let mut seqs = Vec::new();
            for k in 0..n.min(64) {
                let so = u32at(b, lp + 4 * k) as usize;
                let mut seq = Vec::new();
                for j in 0..64 {
                    let c = i16at(b, so + 2 * j);
                    seq.push(c);
                    let hi = (c as u16) & 0xFF00;
                    if matches!(hi, 0x2000 | 0x4000 | 0x8000) || (hi == 0x100 && j > 0) {
                        break;
                    }
                }
                seqs.push(seq);
            }
            anims.push(seqs);
        }
        let segs = |base: usize| -> Vec<Seg> {
            (0..nsegs.min(4096))
                .map(|i| {
                    let o = base + 0x14 * i;
                    let n = u16at(b, o) as usize;
                    let pi = u32at(b, o + 16) as usize;
                    Seg {
                        dur: u32at(b, o + 4) as i32,
                        a: u32at(b, o + 8) as usize,
                        b: u32at(b, o + 12) as usize,
                        idx: (0..n).map(|k| i16at(b, pi + 2 * k)).collect(),
                    }
                })
                .collect()
        };
        let vsegs = if matches!(vtx_anim, 1 | 2) { segs(p_vseg) } else { Vec::new() };
        let csegs = if col_anim == 1 { segs(p_cseg) } else { Vec::new() };
        let mut parts = Vec::new();
        let mut po = p_parts;
        let mut maxv = 0usize;
        for t in 0..8 {
            if counts[t] == 0 {
                continue;
            }
            let w = |i: usize| u32at(b, po + 4 * i) as usize;
            let ty = w(0).min(7);
            let (nv, ncw, tex) = TYPES[ty];
            let nface = w(2).min(4096);
            let (p_idx, p_flags, p_col, p_clut, p_tpage, p_uv) = (w(6), w(8), w(9), w(10), w(11), w(12));
            let faces = (0..nface)
                .map(|f| {
                    let mut idx = [0u16; 4];
                    for k in 0..nv {
                        idx[k] = u16at(b, p_idx + 2 * (nv * f + k));
                        maxv = maxv.max(idx[k] as usize + 1);
                    }
                    let mut col = [[0u8; 3]; 4];
                    for k in 0..4 {
                        let c = u32at(b, p_col + 4 * (ncw * f + k.min(ncw - 1)));
                        col[k] = [c as u8, (c >> 8) as u8, (c >> 16) as u8];
                    }
                    let mut uv = [[0u8; 2]; 4];
                    let (mut clut, mut tpage) = (0, 0);
                    if tex {
                        clut = u16at(b, p_clut + 2 * f);
                        tpage = u16at(b, p_tpage + 2 * f);
                        for k in 0..nv {
                            let v = u16at(b, p_uv + 2 * (nv * f + k));
                            uv[k] = [v as u8, (v >> 8) as u8];
                        }
                    }
                    Face { idx, flags: u16at(b, p_flags + 2 * f), col, clut, tpage, uv }
                })
                .collect();
            parts.push(Part { ty, faces });
            po += 0x34;
        }
        let nvtx = maxv.max(nvtx_copy as usize).min(8192);
        let vtx = (0..nvtx).map(|i| [i16at(b, p_vtx + 8 * i), i16at(b, p_vtx + 8 * i + 2), i16at(b, p_vtx + 8 * i + 4)]).collect();
        Some(Arc::new(Record { b: b.to_vec(), vtx_anim, col_anim, abr, nvtx_copy, ncol, total_frames: total, counts, anims, vsegs, csegs, parts, vtx }))
    }
}

/// A track task: kind 0 = vertices, 2 = colours; `player` = sequence player.
#[derive(Clone, Debug, Serialize, Deserialize)]
struct Track {
    colour: bool,
    mode: u32,
    player: bool,
    status: i32,
    left: i16,
    seg: usize,
    seqs: Vec<Vec<i16>>,
    cur_seq: usize,
    pos: i32,
    dir: i32,
    t: i16,
    dt: i16,
    state: i16,
    loop_a: i16,
    loop_b: i16,
}

impl Track {
    fn new(colour: bool, mode: u32, player: bool, seqs: Vec<Vec<i16>>) -> Self {
        Self {
            colour,
            mode,
            player,
            status: if player { 1 } else { 0 },
            left: if player { -1 } else { 0 },
            seg: 0,
            seqs,
            cur_seq: 0,
            pos: -1,
            dir: 1,
            t: 0,
            dt: 0,
            state: 0,
            loop_a: 0,
            loop_b: 0,
        }
    }
}

/// Per-buffer state of a face (the two GPU packets the game keeps).
#[derive(Clone, Debug, Serialize, Deserialize)]
struct FaceBuf {
    col: [[u8; 3]; 4],
    uv: [[u8; 2]; 4],
    clut: u16,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct PartInst {
    visible: bool,
    depth: i32,
    bufs: [Vec<FaceBuf>; 2],
}

/// One drawn primitive: screen corners (TL TR BL BR order of the PS1 quad; triangles repeat the 3rd), OT slot.
#[derive(Clone, Debug)]
pub struct ModelPrim {
    pub xy: [[f32; 2]; 4],
    pub otz: i32,
    pub rgb: [[u8; 3]; 4],
    pub uv: [[u8; 2]; 4],
    pub tpage: u16,
    pub clut: u16,
    pub textured: bool,
    pub semi: bool,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct Instance {
    /// the record (static: `script_vm::Vm::rebind` puts it back)
    #[serde(skip)]
    pub r: Arc<Record>,
    pub alive: bool,
    /// p2 != 0: another drawer family that draws nothing
    pub hidden_family: bool,
    vtx: Vec<[i16; 3]>,
    parts: Vec<PartInst>,
    /// colour slots: (part, face, corner) in creation order
    colptr: Vec<(usize, usize, usize)>,
    /// (set: 0 = A vertices, 2 = B colours, anim) → track; `order` newest first
    tracks: Vec<((u8, usize), Track)>,
}

fn trunc_div(a: i32, b: i32) -> i32 {
    if b == 0 { 0 } else { a / b }
}

impl Instance {
    /// A decoded instance has the shape `Instance::new` gives it for `r` (its parts' faces, the colour slots).
    pub fn fits(&self, r: &Record) -> bool {
        self.parts.len() == r.parts.len()
            && self.parts.iter().zip(&r.parts).all(|(p, rp)| p.bufs.iter().all(|b| b.len() == rp.faces.len()))
            && self.colptr.iter().all(|&(pi, f, k)| pi < r.parts.len() && f < r.parts[pi].faces.len() && k < 4)
    }

    pub fn new(r: Arc<Record>, p2: i32) -> Self {
        let mut parts = Vec::new();
        let mut colptr = Vec::new();
        for (pi, p) in r.parts.iter().enumerate() {
            let (_, ncw, _) = TYPES[p.ty];
            let fb: Vec<FaceBuf> = p.faces.iter().map(|f| FaceBuf { col: f.col, uv: f.uv, clut: f.clut }).collect();
            if r.ncol != 0 {
                for f in 0..p.faces.len() {
                    for k in 0..ncw {
                        colptr.push((pi, f, k));
                    }
                }
            }
            parts.push(PartInst { visible: true, depth: 0, bufs: [fb.clone(), fb] });
        }
        Self { vtx: r.vtx.clone(), r, alive: true, hidden_family: p2 != 0, parts, colptr, tracks: Vec::new() }
    }

    fn track(&mut self, key: (u8, usize)) -> Option<&mut Track> {
        self.tracks.iter_mut().find(|(k, _)| *k == key).map(|(_, t)| t)
    }

    /// Op03.04: colour track `n`; returns COND.
    pub fn play_b(&mut self, n: i32) -> i32 {
        let r = self.r.clone();
        if r.col_anim == 0 || n < 0 || n as usize >= r.anims.len() || self.track((2, n as usize)).is_some() {
            return 0;
        }
        self.tracks.insert(0, ((2, n as usize), Track::new(true, 1, true, r.anims[n as usize].clone())));
        1
    }

    /// Op03.02: vertex track `n`.
    pub fn play_a(&mut self, n: i32) -> i32 {
        let r = self.r.clone();
        if !matches!(r.vtx_anim, 1 | 2) || n < 0 || n as usize >= r.anims.len() || self.track((0, n as usize)).is_some() {
            return 0;
        }
        self.tracks.insert(0, ((0, n as usize), Track::new(false, r.vtx_anim, true, r.anims[n as usize].clone())));
        1
    }

    /// Op03.06 / 07: the scrub track used with set_time; success even without colour animation.
    pub fn play_default_b(&mut self) -> i32 {
        if self.track((2, 0)).is_some() {
            return 0;
        }
        if self.r.col_anim != 1 {
            return 1;
        }
        self.tracks.insert(0, ((2, 0), Track::new(true, 1, false, Vec::new())));
        1
    }

    pub fn set_depth(&mut self, n: i32) {
        for p in &mut self.parts {
            p.depth = if n < 0 { 1 } else { n };
        }
    }

    pub fn clear_depth(&mut self) {
        for p in &mut self.parts {
            p.depth = 0;
        }
    }

    pub fn show(&mut self, v: bool) {
        for p in &mut self.parts {
            p.visible = v;
        }
    }

    /// The part walk of the CLUT set and the UV scroll: the cursor only advances on textured types 4..6, so in records
    /// with untextured parts first the writes land in the wrong part (the textured faces then keep their values).
    fn tex_walk(&mut self, mut f: impl FnMut(&mut FaceBuf)) {
        let mut pi = 0;
        for t in 0..8 {
            if self.r.counts[t] == 0 || !(4..=6).contains(&t) {
                continue;
            }
            let Some(p) = self.parts.get_mut(pi) else { return };
            pi += 1;
            if !TYPES[self.r.parts[pi - 1].ty].2 {
                continue; // the game corrupts an untextured part here (kept)
            }
            for b in 0..2 {
                for fb in p.bufs[b].iter_mut() {
                    f(fb);
                }
            }
        }
    }

    pub fn set_clut(&mut self, clut: u16) {
        self.tex_walk(|f| f.clut = clut);
    }

    pub fn uv_scroll(&mut self, du: u8, dv: u8) {
        self.tex_walk(|f| {
            for uv in f.uv.iter_mut() {
                uv[0] = uv[0].wrapping_add(du);
                uv[1] = uv[1].wrapping_add(dv);
            }
        });
    }

    /// pause / resume / loop range of set A (vertices) or B (colours).
    pub fn track_ctl(&mut self, colour: bool, n: i32, op: u8, a: i32, b: i32) {
        if n < 0 {
            return;
        }
        let key = (if colour { 2 } else { 0 }, n as usize);
        if let Some(t) = self.track(key) {
            match op {
                0 => t.state |= 0x100,
                1 => t.state &= 0xFF,
                _ => {
                    t.loop_a = a as i16;
                    t.loop_b = b as i16;
                    t.state |= 1;
                }
            }
        }
    }

    /// Op03.09: t = 0..0x1000 over +38; the lerp parameter is t itself (register reuse).
    pub fn set_time_b(&mut self, t: i32, buf: usize) {
        if self.track((2, 0)).is_none() || self.r.total_frames == 0 {
            return;
        }
        let t = t.clamp(0, 0x1000);
        let r = self.r.clone();
        let mut cum = 0;
        for (i, g) in r.csegs.iter().enumerate() {
            cum += g.dur;
            if trunc_div(cum << 12, r.total_frames as i16 as i32) >= t {
                if let Some(tr) = self.track((2, 0)) {
                    tr.seg = i;
                }
                self.apply_seg(true, 1, 1, i, t, buf);
                return;
            }
        }
    }

    fn apply_seg(&mut self, colour: bool, mode: u32, dir: i32, seg: usize, t: i32, buf: usize) {
        let r = self.r.clone();
        let b = &r.b;
        if !colour {
            let Some(g) = r.vsegs.get(seg) else { return };
            for (k, &vi) in g.idx.iter().enumerate() {
                let (ax, ay, az) = (i16at(b, g.a + 8 * k) as i32, i16at(b, g.a + 8 * k + 2) as i32, i16at(b, g.a + 8 * k + 4) as i32);
                let Some(v) = self.vtx.get_mut(vi as u16 as usize) else { continue };
                if mode == 2 {
                    let sg = if dir > 0 { 1 } else { -1 };
                    v[0] = (v[0] as i32 + sg * ax) as i16;
                    v[1] = (v[1] as i32 + sg * ay) as i16;
                    v[2] = (v[2] as i32 + sg * az) as i16;
                } else {
                    let (bx, by, bz) = (i16at(b, g.b + 8 * k) as i32, i16at(b, g.b + 8 * k + 2) as i32, i16at(b, g.b + 8 * k + 4) as i32);
                    *v = [(ax + ((bx * t) >> 12)) as i16, (ay + ((by * t) >> 12)) as i16, (az + ((bz * t) >> 12)) as i16];
                }
            }
        } else {
            let Some(g) = r.csegs.get(seg) else { return };
            for (k, &ci) in g.idx.iter().enumerate() {
                let (ca, cb) = (u32at(b, g.a + 4 * k), u32at(b, g.b + 4 * k));
                let Some(&(pi, f, corner)) = self.colptr.get(ci as u16 as usize) else { continue };
                let fb = &mut self.parts[pi].bufs[buf][f];
                for (j, sh) in [0, 8, 16].iter().enumerate() {
                    let (x, y) = (((ca >> sh) & 0xFF) as i32, ((cb >> sh) & 0xFF) as i32);
                    fb.col[corner][j] = (x + (((y - x) * t) >> 12)) as u8;
                }
            }
        }
    }

    /// One step of a sequence player.
    fn run_player(&mut self, ti: usize, buf: usize) {
        let r = self.r.clone();
        let (colour, mode) = {
            let tr = &self.tracks[ti].1;
            if tr.state < 0 || tr.state >= 2 {
                return;
            }
            (tr.colour, tr.mode)
        };
        let segs_len = if colour { r.csegs.len() } else { r.vsegs.len() };
        {
            let tr = &mut self.tracks[ti].1;
            if tr.left < 0 {
                let mut slo;
                let mut guard = 0;
                loop {
                    tr.pos += 1;
                    loop {
                        guard += 1;
                        if guard > 256 {
                            tr.status = -2;
                            tr.state = -1;
                            return;
                        }
                        let c = tr.seqs.get(tr.cur_seq).and_then(|s| s.get(tr.pos.max(0) as usize)).copied().unwrap_or(0x4000) as u16;
                        let (hi, lo) = (c & 0xFF00, c & 0xFF);
                        slo = lo as u8 as i8 as i32;
                        match hi {
                            0x2000 => {
                                tr.state |= 0x100;
                                return;
                            }
                            0x4000 => {
                                tr.status = -1;
                                tr.state = -1;
                                return;
                            }
                            0x8000 => {
                                tr.status = -2;
                                tr.state = -1;
                                return;
                            }
                            0x100 => {
                                tr.pos = slo;
                                continue;
                            }
                            0x200 => {
                                tr.cur_seq = slo.max(0) as usize;
                                tr.pos = 0;
                                continue;
                            }
                            _ => {}
                        }
                        break;
                    }
                    if tr.state == 1 && slo == tr.loop_a as i32 {
                        tr.cur_seq = tr.loop_b.max(0) as usize;
                        tr.pos = -1;
                        tr.state = 0;
                        continue;
                    }
                    break;
                }
                let mut k = slo;
                if k < 0 {
                    tr.dir = -1;
                    k = -k;
                } else {
                    tr.dir = 1;
                }
                if k < 1 || k as usize > segs_len {
                    tr.status = -2;
                    tr.state = -1;
                    return;
                }
                tr.seg = k as usize - 1;
                let dur = if colour { r.csegs[tr.seg].dur } else { r.vsegs[tr.seg].dur };
                tr.left = ((dur & 0xFFFF) - 1) as i16;
                if mode == 1 {
                    let d = trunc_div(0x1000, tr.left as i32 + 1);
                    if tr.dir > 0 {
                        tr.t = 0;
                        tr.dt = d as i16;
                    } else {
                        tr.t = 0x1000;
                        tr.dt = (-d) as i16;
                    }
                    tr.t = tr.t.wrapping_add(tr.dt);
                }
            }
        }
        let (seg, dir, t) = {
            let tr = &mut self.tracks[ti].1;
            if mode == 2 {
                (tr.seg, tr.dir, 0)
            } else {
                tr.t = tr.t.clamp(0, 0x1000);
                (tr.seg, tr.dir, tr.t as i32)
            }
        };
        self.apply_seg(colour, mode, dir, seg, t, buf);
        let tr = &mut self.tracks[ti].1;
        if mode != 2 {
            tr.t = tr.t.wrapping_add(tr.dt);
        }
        tr.left = tr.left.wrapping_sub(1);
    }

    /// One frame of lists 1 and 3 (instance check, then the tracks newest first). False = the model ended.
    pub fn update(&mut self, buf: usize) -> bool {
        if !self.alive {
            return false;
        }
        if self.tracks.iter().any(|(_, t)| t.status == -1) {
            self.alive = false;
            return false;
        }
        self.tracks.retain(|(_, t)| t.status != -2);
        for ti in 0..self.tracks.len() {
            if self.tracks[ti].1.player {
                self.run_player(ti, buf);
            }
        }
        true
    }

    /// The unlit drawers: vertices through the model matrix (3×3 4.12 + translation, world units), projected
    /// with `project` (world → screen x, y, otz = SZ / 4); OT depth = SZ of the third vertex, NCLIP ≥ 0 or
    /// double-sided, sorted 10 ≤ otz < 0x800, or the part's fixed slot.
    pub fn prims(&self, buf: usize, mr: &[[i32; 3]; 3], mt: [i32; 3], project: &dyn Fn([i32; 3]) -> Option<(f32, f32, i32)>, out: &mut Vec<ModelPrim>) {
        if !self.alive || self.hidden_family {
            return;
        }
        let world = |v: [i16; 3]| -> [i32; 3] {
            let v = v.map(|x| x as i32);
            [0, 1, 2].map(|i| ((mr[i][0] * v[0] + mr[i][1] * v[1] + mr[i][2] * v[2]) >> 12) + mt[i] as i16 as i32)
        };
        for (pi, p) in self.r.parts.iter().enumerate().rev() {
            let inst = &self.parts[pi];
            if !inst.visible || p.ty >= 6 {
                continue;
            }
            let (nv, _, tex) = TYPES[p.ty];
            let fixed = inst.depth;
            if fixed != 0 && !(1..=0x7FF).contains(&fixed) {
                continue;
            }
            for (fi, f) in p.faces.iter().enumerate() {
                let mut s = [(0f32, 0f32, 0i32); 4];
                let mut ok = true;
                for k in 0..nv {
                    match self.vtx.get(f.idx[k] as usize).and_then(|&v| project(world(v))) {
                        Some(q) => s[k] = q,
                        None => ok = false,
                    }
                }
                if !ok {
                    continue;
                }
                let otz = s[2].2;
                let nclip = (s[1].0 - s[0].0) * (s[2].1 - s[0].1) - (s[2].0 - s[0].0) * (s[1].1 - s[0].1);
                let no_cull = if fixed != 0 && p.ty == 5 { (f.flags as i16).wrapping_add(0x200) != 0 } else { f.flags & 0x200 != 0 };
                if !(nclip >= 0.0 || no_cull) {
                    continue;
                }
                if fixed == 0 && !(10..0x800).contains(&otz) {
                    continue;
                }
                if nv == 3 {
                    s[3] = s[2];
                }
                let fb = &inst.bufs[buf][fi];
                let semi = f.flags & 0x100 != 0;
                let gour = matches!(p.ty, 2 | 3);
                let rgb = if gour { [fb.col[0], fb.col[1], fb.col[2], if nv == 4 { fb.col[3] } else { fb.col[2] }] } else { [fb.col[0]; 4] };
                let mut uv = fb.uv;
                if nv == 3 {
                    uv[3] = uv[2];
                }
                out.push(ModelPrim {
                    xy: [0, 1, 2, 3].map(|k| [s[k].0, s[k].1]),
                    otz: if fixed != 0 { fixed } else { otz },
                    rgb,
                    uv,
                    tpage: if tex { f.tpage } else { (self.r.abr as u16 & 3) << 5 },
                    clut: fb.clut,
                    textured: tex,
                    semi,
                });
            }
        }
    }
}
