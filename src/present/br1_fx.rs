//! The imported game's effects in a match: its particles and its background flash (engine addition, the import S8).
//!
//! State: presentation (reads the state, never writes it; the effects' motion is the view's own)
//!
//! The imported game's fight code asks for its effects through calls the core logs as requests (`Request::Fx` / `Call`,
//! `Br1Match::fx_log`); as the user's decision puts them outside the simulation (their
//! random numbers from a presentation stream, never read back) they run here, once per new fight frame:
//! - a hit leaves its sparks (an emitter of type-6 particles, kind 1) and its glow: one
//!   particle of type 0 at the contact point (CLUT 4, mode 0, no gravity) of size `0x64 + 4 · damage`
//!   (`0xF0` from damage 0x1F); a guarded one its sparks, a crystal and two
//!   crescents (the effect tasks 0x13 and 0x0D: flat added quads at OT 10). The sparks' mode byte is written over
//!   after the spawn by type (as the recorded runs' pool dumps show);
//! - the particles live in the imported game's pool (40 records, a free stack, the live list; a record keeps what its last
//!   particle left in the fields a spawn does not write, which the random modes read);
//! - the background's flash (frames, colour) fills the background with the colour for those frames
//!   (the background strip's draw shows it in the strip's place; on a main-game stage its backdrop);
//! - a hit's light flash (by kind and damage, stepped each frame, shown by the fighters' lighting): light #3's colour at
//!   `min(damage · 128, 0xFFF)` in the kind's channels for 10 frames, then down by a fifteenth a frame, over after 40
//!   (`light3`, the fighters' lighting);
//! - the particle code itself is ok_br1_converter's (`vfx.rs`: the update / draw for the modes and types it
//!   models, checked against recorded runs by its author), projected by the main game's camera (`output::gte::Gte`),
//!   its quads drawn from the imported game's effect pages (CMN/EFFECT.DAT, moved in the effects' VRAM image: `BR1_PAGES`,
//!   `clut_word`).
//!
//! - the effect tasks (7 slots, their colour slots stepped before them): a strong hit's burst
//!   (0x09), the attack trails (0x0F), the beast claws (0x10), the hand ribbons (0x12), the floor shockwave (0x1F, from
//!   the floor-effect spawner), cut by a move cancel (the flags cleared each frame after the tasks); op 0x18's emitters on
//!   the fighters' nodes (stepped after the particles); the camera's shake words
//!   (`Request::Cam`, stepped with the camera: `shake`);
//! - the effect scripts ((command, frame) lists run after the tasks; a transformation's, op 0x16's): back
//!   colour scripts (the fighter's back colour, `back`), effect tasks on the fighter's
//!   nodes (0x01 beams, 0x02 a ring, 0x05 a dark aura subtracted from the screen, 0x08 a seven-spike star, 0x0A a band,
//!   0x0B a tornado, 0x0E a whirl, 0x17 a globe, 0x1B a flame, 0x20 a radial flash) and emitter presets (kinds 2, 13,
//!   15); a special hit's stars (0x0C, with an emitter of kind 8).
//!
//! - the beast form's lightning along the body (the effect scripts' command 3).
//! - Fox's web at the transformation (the effect scripts' command 5).
//! - the rave's circling sparks (the effect scripts' preset 2: emitter kind 19) and task 0x16 (six turning
//!   wedges); neither runs for the engine's four fighters (the rave is off under the
//!   main game's rules; task 0x16 is GAD's transformation's and op 0x16 sub 3's, which no script of theirs has).
//! - the spark presets (op 0x17 sub 3: emitter kind 3, ok_br1_converter's `vfx::spark_frames`) and op 0x17 sub 2's
//!   cases 0, 1 (kind 0), 3, 5, 6, 7 (tasks 0x01 / 0x05 / 0x0A / 0x1F), 8 (emitter kind 6: sparks from a node's tip),
//!   9 (task 0x21: a ring round a node), 10 (two trails, type 0x11), 11 (emitter kind 16: bands and sparks on the floor
//!   under a moving node), 12, 13 (Uriko's drop in her beast form: particle mode 12, checked against the recorded
//!   records), 14 and 15 (op 0x17 sub 2) (the S11 additions).
//!
//! Not implemented: the floor-effect spawner's kinds but 0 (its sub-emitters, emitter kind 9),
//! op 0x17 sub 2's cases 2 and 4 and 5's kinds past 13 (no script of the four fighters has them), particle mode 10, the
//! hit glow's children.

#[cfg(test)]
use crate::fight::br1::bridge::Br1Match;
use crate::fight::br1::script::{Call, CamWord, Request};
use crate::fight::scene::Br1Scene;
use crate::formats::vram::{BR1_CLUT_FROM, BR1_CLUT_TO, BR1_PAGES};
use crate::output::gte::Gte;
use crate::render::PsxQuad;

/// An imported game CLUT position as the engine's image holds it (`BR1_CLUT_FROM` → `BR1_CLUT_TO`), as a CLUT word.
fn clut_word(x: usize, y: usize) -> u16 {
    let (x, y) = if (BR1_CLUT_FROM.0..BR1_CLUT_FROM.0 + 32).contains(&x) && (BR1_CLUT_FROM.1..BR1_CLUT_FROM.1 + 16).contains(&y) {
        (x - BR1_CLUT_FROM.0 + BR1_CLUT_TO.0, y - BR1_CLUT_FROM.1 + BR1_CLUT_TO.1)
    } else {
        (x, y)
    };
    ((y << 6) | (x >> 4)) as u16
}

/// An imported texture page word with its page moved (`BR1_PAGES`).
fn tpage_word(tp: u16) -> u16 {
    let (x, y) = ((tp & 0xF) as usize * 64, ((tp >> 4) & 1) as usize * 256);
    let (nx, ny) = BR1_PAGES.iter().find(|(f, _)| *f == (x, y)).map_or((x, y), |(_, t)| *t);
    (tp & !0x1F) | (nx / 64) as u16 | ((ny / 256) as u16) << 4
}

fn s16(v: i64) -> i64 {
    v as i16 as i64
}
fn s32(v: i64) -> i64 {
    v as i32 as i64
}
fn cmod(a: i64, b: i64) -> i64 {
    if b == 0 { 0 } else { a % b }
}

/// One particle type (20 bytes: the frames' UV quads, their hold ticks, the base size, the primitive
/// code, the page).
struct Type {
    frames: Vec<[(u8, u8); 4]>,
    durations: Vec<u8>,
    dur_addr: u32,
    size: i64,
    tpage: u16,
    semi: bool,
    raw: bool,
}

/// The particle tables of the imported game's static tables.
pub struct Tables {
    tables: crate::formats::tables::Tables,
    /// the origins of the effects' linked data `fx.data_a` and `fx.data_b` (`a`, `b`)
    origin: [u32; 2],
    types: Vec<Type>,
    cluts: Vec<(usize, usize)>,
}

impl Tables {
    /// The particle tables of `tables` (the imported game's, `Br1Res::tables`; in `fx.data_b`: +0x1498 the types, +0x39C / +0x3B4 the
    /// pages, +0x3C0 the CLUT positions).
    pub fn of(tables: &crate::formats::tables::Tables) -> Self {
        let origin = ["fx.data_a", "fx.data_b"].map(|n| tables.table(n).origin());
        let b = |off: u32| origin[1] + off;
        let tpv: Vec<u16> = (0..5).map(|i| tables.abs_u16(b(0x3B4) + 2 * i)).collect();
        let types = (0..23u32)
            .map(|t| {
                let a = b(0x1498) + 20 * t;
                let (uvp, dp, n) = (tables.abs_u32(a), tables.abs_u32(a + 4), tables.abs_u8(a + 8) as u32);
                let tp = tpv.get(tables.abs_u8(b(0x39C) + t) as usize).copied().unwrap_or(0) + ((tables.abs_u8(a + 0xC) as u16) << 5);
                let mut code = tables.abs_u8(a + 0xB);
                if matches!(t, 4 | 14 | 16 | 21) {
                    code &= 0xFE; // colour-modulated
                }
                Type {
                    frames: (0..n)
                        .map(|k| std::array::from_fn(|j| (tables.abs_u8(uvp + 8 * k + 2 * j as u32), tables.abs_u8(uvp + 8 * k + 2 * j as u32 + 1))))
                        .collect(),
                    durations: (0..n).map(|k| tables.abs_u8(dp + k)).collect(),
                    dur_addr: dp,
                    size: tables.abs_u8(a + 0xA) as i64,
                    tpage: tp,
                    semi: code & 2 != 0,
                    raw: code & 1 != 0,
                }
            })
            .collect();
        let cluts = (0..17u32).map(|i| (tables.abs_u16(b(0x3C0) + 4 * i) as usize, tables.abs_u16(b(0x3C2) + 4 * i) as usize)).collect();
        Self { tables: tables.clone(), origin, types, cluts }
    }

    /// The address `off` bytes into `fx.data_a`.
    fn a(&self, off: u32) -> u32 {
        self.origin[0] + off
    }

    /// The address `off` bytes into `fx.data_b`.
    fn b(&self, off: u32) -> u32 {
        self.origin[1] + off
    }

    fn rsin(&self, a: i64) -> i64 {
        let mut v = (a & 0x3FF) << 1;
        if a & 0x400 != 0 {
            v = 0x800 - v;
        }
        let r = self.tables.table("math.sin").i16(v as usize) as i64;
        if a & 0x800 != 0 { -r } else { r }
    }
    fn rcos(&self, a: i64) -> i64 {
        self.rsin(a + 0x400)
    }
}

/// libc rand and the byte table's: the presentation stream.
struct Rand {
    seed: u32,
    tidx: u32,
}

impl Rand {
    fn rand(&mut self) -> i64 {
        self.seed = self.seed.wrapping_mul(0x41C6_4E6D).wrapping_add(0x3039);
        ((self.seed >> 16) & 0x7FFF) as i64
    }
    fn trand(&mut self, t: &Tables) -> i64 {
        let v = t.tables.table("math.random_bytes").u8(self.tidx as usize) as i64;
        self.tidx = (self.tidx + 1) & 0xFF;
        v
    }
    fn sym(&mut self, n: i64) -> i64 {
        n - cmod(self.rand(), 2 * n)
    }
}

/// The particle records.
const POOL: usize = 40;

/// A particle (the particle spawn's record).
#[derive(Clone)]
struct Particle {
    ty: usize,
    mode: u8,
    p: [i64; 3],
    pos: [i64; 3],
    vel: [i64; 3],
    frame: usize,
    nleft: i64,
    dcount: i64,
    size: i64,
    b3a: i64,
    b3c: i64,
    b3e: i64,
    b40: i64,
    damp: i64,
    grav: i64,
    clut: usize,
    rgb: [i64; 3],
    dead: bool,
    /// +0x30: the fighter (side) a mode-12 drop hangs from (op 0x17 sub 2's case 13; the kind-6 emitter's sparks set it
    /// too, which no mode of theirs reads)
    fighter: Option<usize>,
}

impl Particle {
    /// A record never used (the BSS's zeros).
    fn blank() -> Self {
        Self {
            ty: 0,
            mode: 0,
            p: [0; 3],
            pos: [0; 3],
            vel: [0; 3],
            frame: 0,
            nleft: 0,
            dcount: 0,
            size: 0,
            b3a: 0,
            b3c: 0,
            b3e: 0,
            b40: 0,
            damp: 0,
            grav: 0,
            clut: 0,
            rgb: [0; 3],
            dead: false,
            fighter: None,
        }
    }

    /// The particle spawn (pos, vel, type, CLUT index, mode).
    fn new(t: &Tables, ty: usize, mode: u8, pos: [i64; 3], vel: [i64; 3], clut: usize) -> Self {
        let tt = &t.types[ty.min(t.types.len() - 1)];
        Self {
            ty,
            mode,
            p: pos,
            pos: pos.map(|v| v << 12),
            vel: vel.map(|v| v << 8),
            frame: 0,
            nleft: tt.frames.len() as i64,
            dcount: tt.durations.first().copied().unwrap_or(1) as i64,
            size: tt.size,
            b3a: 0,
            b3c: if ty == 16 { 0x2D } else { 0 },
            b3e: 0,
            b40: 0,
            damp: if !(12..14).contains(&ty) { 0xE66 } else { 0xEE1 },
            grav: match ty {
                8 | 9 => 0,
                10 | 12 => 0x1980,
                _ => 0x880,
            },
            clut,
            rgb: [0xFF; 3],
            dead: false,
            fighter: None,
        }
    }
}

/// A particle's quad this frame, in the GTE's display pixels: corners TL, TR, BL, BR, its OT slot.
#[derive(Clone)]
struct Draw {
    uv: [(u8, u8); 4],
    quad: [(f32, f32); 4],
    otz: i32,
    ty: usize,
    clut: usize,
    rgb: [i64; 3],
}

fn sq(cx: f32, cy: f32, w: f32, h: f32) -> [(f32, f32); 4] {
    let (x0, y0) = (cx - w / 2.0, cy - h / 2.0);
    [(x0, y0), (x0 + w, y0), (x0, y0 + h), (x0 + w, y0 + h)]
}

/// The point (0, y, z + extra) of the tip table (per
/// character 8 bytes, per form 4: y, z) in node `node`'s frame, in the world (ApplyMatrixSV plus the translation, s16).
fn tip(t: &Tables, f: &Fighter, node: i16, extra: i32) -> [i16; 3] {
    let e = t.b(0x434) + 8 * f.chr as u32 + 4 * (f.form & 1) as u32;
    let v = [0, t.tables.abs_u16(e) as i16 as i64, (t.tables.abs_u16(e + 2) as i32).wrapping_add(extra) as i16 as i64];
    let Some(d) = f.md.get(node as usize) else { return [0; 3] };
    std::array::from_fn(|i| {
        let r = ((d.wm[i][0] as i64 * v[0] + d.wm[i][1] as i64 * v[1] + d.wm[i][2] as i64 * v[2]) >> 12).clamp(-0x8000, 0x7FFF);
        (r as i32).wrapping_add(d.wt[i]) as i16
    })
}

/// A frame of a mode-12 drop (before mode 11's band draw: from the record's point +0x00 to
/// the one in its +0x14 halfwords): +0x3A counts the frames, +0x3C grows by +0x3E; the far end (+0x16) down by
/// `+0x3C >> 8`, the near one (+0x02, while above the floor) by `+0x3C >> 9`; gone once the far end reaches the floor.
/// For its first 20 frames (+0x3A) +0x3E goes down by 10 (to 0) and the drop hangs from its fighter's tip of node 19
/// (`tip`, +0x40 more in z): the far end at it, the near one under it, let go at once (+0x3A = 20) when the tip moved
/// 31 or more from the far end; at 20 +0x3E is 6. False when it died.
fn drop_step(t: &Tables, pt: &mut Particle, fs: &[Fighter]) -> bool {
    pt.b3a = s16(pt.b3a + 1);
    pt.b3c = s16(pt.b3c + pt.b3e);
    let far_y = s16(s16(pt.vel[0] >> 16) + (pt.b3c >> 8));
    pt.vel[0] = (pt.vel[0] & 0xFFFF) | (far_y << 16);
    if pt.p[1] < 0 {
        pt.p[1] = s16(pt.p[1] + (pt.b3c >> 9));
    }
    if far_y >= 0 {
        pt.dead = true;
        return false;
    }
    if pt.b3a < 0x14 {
        pt.b3e = s16(pt.b3e - 10).max(0);
        if let Some(f) = pt.fighter.and_then(|s| fs.get(s)) {
            let tip = tip(t, f, 19, pt.b40 as i32).map(|v| v as i64);
            if (tip[1] - far_y).abs() >= 0x1F {
                pt.b3a = 0x14;
            }
            pt.vel[0] = (tip[0] & 0xFFFF) | (tip[1] << 16);
            pt.vel[1] = (pt.vel[1] & !0xFFFF) | (tip[2] & 0xFFFF);
            (pt.p[0], pt.p[2]) = (tip[0], tip[2]);
        }
    }
    if pt.b3a == 0x14 {
        pt.b3e = 6;
    }
    true
}

/// One frame of one particle (ok_br1_converter's `vfx::step` with the GTE of the main game's camera); `fs` the
/// fighters (a mode-12 drop's, +0x30).
fn step(t: &Tables, pt: &mut Particle, gte: &Gte, pitch: i64, rnd: &mut Rand, fs: &[Fighter]) -> Option<Draw> {
    let tt = &t.types[pt.ty.min(t.types.len() - 1)];
    let n = tt.frames.len();
    let Some(&uv) = tt.frames.get(pt.frame.min(n.saturating_sub(1))) else {
        // a type without frames (a table that has none): gone at once
        pt.dead = true;
        return None;
    };
    pt.dcount = (pt.dcount - 1) & 0xFF;
    if pt.dcount == 0 {
        pt.nleft -= 1;
        pt.frame += 1;
        pt.dcount = tt.durations.get(pt.frame).map_or_else(|| t.tables.abs_u8(tt.dur_addr + pt.frame as u32) as i64, |&d| d as i64);
    }
    match pt.mode {
        1 | 2 | 3 | 6 | 7 | 9 => {
            pt.vel[0] = s32(pt.vel[0] * pt.damp) >> 12;
            pt.vel[2] = s32(pt.vel[2] * pt.damp) >> 12;
            pt.vel[1] = (s32(pt.vel[1] * pt.damp) >> 12) + pt.grav;
            for k in 0..3 {
                pt.pos[k] = s32(pt.pos[k] + pt.vel[k]);
            }
        }
        4 | 5 => {
            pt.b3c = s16(pt.b3c - 3).max(0);
            pt.rgb = [pt.b3c & 0xFF; 3];
        }
        8 => {
            pt.pos[0] = s32(pt.vel[0] + pt.p[1] * t.rcos(pt.p[0]));
            pt.pos[2] = s32(pt.vel[2] + pt.p[1] * t.rsin(pt.p[0]));
            pt.p[1] = s16(pt.p[1] + 6);
            pt.p[0] = s16(pt.p[0] + pt.p[2]);
            pt.p[2] = s16((pt.p[2] * pt.damp) >> 12);
        }
        _ => {}
    }
    match pt.ty {
        5 => pt.size = s16(pt.size + 2),
        12 => {
            pt.size = s16(pt.size - 1).max(0);
            if pt.pos[1] > 0 {
                pt.vel[1] = -pt.vel[1];
            }
        }
        13 => {
            if pt.pos[1] > 0 {
                pt.vel[1] = -pt.vel[1];
            }
        }
        14 => {
            let n = pt.nleft;
            pt.size = (if 14 - n >= 0 { n + 2 } else { 30 - n }) * 8 + 0x40;
        }
        15 | 17 => {
            if pt.ty == 17 {
                pt.size = s16(pt.size - 1);
            }
            if pt.pos[1] > 0 {
                pt.vel[1] = -pt.vel[1];
            }
            pt.clut = 2 + 2 * (rnd.trand(t) & 1) as usize;
        }
        16 => {
            pt.b3c = s16(pt.b3c + 2);
            pt.size = s16(pt.size + 60);
        }
        21 => {
            pt.vel[0] = (pt.vel[0] - 2).max(0);
            pt.rgb = [pt.vel[0] & 0xFF; 3];
        }
        _ => {}
    }
    let pos = pt.pos.map(|v| s16(v >> 12));
    if pt.nleft <= 0 {
        pt.dead = true;
        return None;
    }
    if pt.mode == 12 && !drop_step(t, pt, fs) {
        return None;
    }
    let size = pt.size as f32;
    let rtp = |p: [i64; 3]| gte.rtps([p[0] as i32, p[1] as i32, p[2] as i32]);
    let project = |zoff: i32, p: [i64; 3]| {
        let q = rtp(p)?;
        let s = if q.otz != 0 { size * gte.h / (q.otz * 4) as f32 } else { 0.0 };
        Some((q.sx, q.sy, s, q.otz - zoff))
    };
    let visible = |z: i32| (0x33..0x800).contains(&z);
    let (quad, otz);
    match pt.mode {
        0 | 1 | 7 | 8 | 9 => {
            if pt.mode == 7 && pos[1] > 0 {
                pt.dead = true;
                return None;
            }
            let (sx, sy, s, mut z) = project(30, pos)?;
            if !visible(z) {
                return None;
            }
            let mut w = s;
            if pt.mode == 9 {
                pt.b3c = s16(pt.b3c + 1);
                if pt.b3c & 1 != 0 {
                    w = -s;
                }
            }
            if pt.mode != 9 && pt.ty == 0 {
                z = 10;
            }
            quad = sq(sx, sy, w, s);
            otz = z;
        }
        2 => {
            // SquareRoot0(sum << 12) >> 12 of the 32-bit sum of the squares
            let sum = (0..3).fold(0i32, |a, k| a.wrapping_add(((pt.p[k] - pos[k]) as i32).wrapping_mul((pt.p[k] - pos[k]) as i32)));
            let d = ((sum.wrapping_shl(12) as u32 as f64).sqrt() as i64) >> 12;
            if pt.b3a < d || pt.nleft == 0 {
                pt.dead = true;
                return None;
            }
            pt.b3a = d;
            let (sx, sy, s, z) = project(50, pos)?;
            if !visible(z) {
                return None;
            }
            quad = sq(sx, sy, s, s);
            otz = z;
        }
        3 | 6 if pos[1] > 0 => {
            pt.dead = true;
            return None;
        }
        3 => {
            let (sx, sy, s, z) = project(0, pos)?;
            if !visible(z) {
                return None;
            }
            quad = sq(sx, sy, s, s);
            otz = z;
        }
        4 => {
            let (sx, sy, mut s, mut z) = project(0, pos)?;
            if !visible(z) {
                return None;
            }
            if pt.ty == 16 {
                z = 0x7FC;
                s = pt.size as f32 * gte.h / (z * 4) as f32;
            }
            let h = s * t.rsin(pitch + 0xA0) as f32 / 4096.0;
            quad = sq(sx, sy, s, h);
            otz = z;
        }
        6 => {
            let q = rtp(pos)?;
            let z = q.otz - 5;
            if !visible(z) {
                return None;
            }
            pt.grav += 0xC0;
            pt.size = s16(pt.size + 1);
            let s = pt.size as f32 * gte.h / (z * 4) as f32;
            quad = sq(q.sx, q.sy, s, s);
            otz = z;
        }
        5 => {
            // from the position to the record's s16 point (+0x00)
            let q = rtp(pos)?;
            let z = q.otz;
            if !visible(z) {
                return None;
            }
            let q2 = rtp(pt.p)?;
            let (w, h) = (q2.sx - q.sx, q2.sy - q.sy);
            if w <= 0.0 {
                return None;
            }
            quad = sq(q.sx, q.sy, w, h);
            otz = z;
        }
        11 | 12 => {
            // a band from the record's s16 point (+0x00) to the one its +0x14 halfwords hold, as wide as its size:
            // the corners each end ± the size along the effects' cheap angle of the screen delta (taken here as the
            // delta's normal)
            let q1 = rtp(pt.p)?;
            let v = [pt.vel[0] as i16 as i64, (pt.vel[0] >> 16) as i16 as i64, pt.vel[1] as i16 as i64];
            let q2 = rtp(v)?;
            let z = (q1.otz - 0x14).clamp(1, 0x7EF);
            let s = pt.size as f32 * gte.h / (z * 4) as f32;
            let a = (q2.sy - q1.sy).atan2(q2.sx - q1.sx) + std::f32::consts::FRAC_PI_2;
            let (nx, ny) = (s * a.cos(), s * a.sin());
            quad = [(q1.sx + nx, q1.sy + ny), (q1.sx - nx, q1.sy - ny), (q2.sx + nx, q2.sy + ny), (q2.sx - nx, q2.sy - ny)];
            otz = z;
        }
        13 => {
            let (sx, sy, s, z) = project(30, pos)?;
            if !visible(z) {
                return None;
            }
            pt.b3c = s16(pt.b3c + pt.b3e);
            pt.b3e = s16((pt.b3e * 475) >> 9);
            pt.b40 = s16(pt.b40 + 0x80);
            pt.size = s16(pt.b40) >> 8;
            pt.pos[1] = s32(pt.pos[1] - 0x1800);
            let q = sq(sx, sy, s, s);
            let (a, b) = (s * t.rcos(pt.b3c) as f32 / 16384.0, s * t.rsin(pt.b3c) as f32 / 16384.0);
            quad = [(q[0].0 - a, q[0].1 + b), (q[1].0 + a, q[1].1 + b), (q[2].0 - a, q[2].1 - b), (q[3].0 + a, q[3].1 - b)];
            otz = z;
        }
        14 => {
            pt.b3a = s16(pt.b3a - pt.b3c);
            pt.b3c = s16((pt.b3c * 1825) >> 11);
            if pt.b3a < 0 {
                pt.dead = true;
                return None;
            }
            let (ax, ay, az) = (s16(pt.vel[0]), s16(pt.vel[0] >> 16), s16(pt.vel[1]));
            let p2 = [pt.p[0] + ((pt.b3a * t.rcos(ax)) >> 12), pt.p[1] + ((pt.b3a * t.rsin(ay)) >> 12), pt.p[2] + ((pt.b3a * t.rcos(az)) >> 12)];
            let (sx, sy, s, z) = project(50, p2)?;
            if !visible(z) {
                return None;
            }
            quad = sq(sx, sy, s, s);
            otz = z;
        }
        // 10 draws from a pointer of the record (+0x30) that only its own spawners set (not implemented): not drawn; 15..
        // are not drawn
        _ => return None,
    }
    Some(Draw { uv, quad, otz, ty: pt.ty, clut: pt.clut, rgb: pt.rgb })
}

/// The children of a dying particle (its in-ring branches).
fn kill_children(t: &Tables, pt: &Particle, rnd: &mut Rand) -> Vec<(Particle, bool)> {
    let (x, z) = (s16(pt.pos[0] >> 12), s16(pt.pos[2] >> 12));
    let mut out = Vec::new();
    if pt.ty == 5 && rnd.rand() & 1 == 0 {
        let mut c = Particle::new(t, 4, 4, [x, 0, z], [0; 3], 16);
        c.grav = 0;
        c.size = s16(pt.b3c + rnd.sym(0x20));
        c.b3c = 0xFF;
        out.push((c, true));
    } else if pt.ty == 19 || (pt.ty == 10 && pt.mode != 6 && rnd.rand() & 1 == 0) {
        let mut c = Particle::new(t, 11, 4, [x, 0, z], [0; 3], if pt.ty == 19 { 6 } else { 5 });
        c.grav = 0;
        c.size = 0x50;
        out.push((c, false));
    }
    out
}

/// An emitter slot (4 slots of 0x30 bytes).
#[derive(Clone, Copy, Default)]
struct Emitter {
    /// +0x28.. the position (a request's: a world point)
    pos: [i64; 3],
    /// +0x20 the particles' type (the request's low byte), +0x18 their CLUT index
    ty: usize,
    clut: usize,
    /// +0x1A / +0x1C the angles, +0x1E the speed
    yaw: i64,
    pitch: i64,
    speed: i64,
    /// +0x23 the frames it runs, +0x24 the particles a frame, +0x25 the frames run, +0x26 the kind
    frames: u8,
    per: u8,
    n: u8,
    kind: u8,
    /// +0x10: kind 3's spark preset (its record in the preset table)
    preset: u32,
    /// +0x14: kind 6's fighter (side); kind 6's +0x28 (its flag, over the position's x) and +0x2A (its node, over y);
    /// kind 16's +0x0C (a pointer to the node `node`'s translation of fighter `side`)
    side: usize,
    flag: i16,
    node: i16,
}

impl Emitter {
    /// `total / frames` particles a frame.
    #[allow(clippy::too_many_arguments)]
    fn new(pos: [i16; 3], ty: u16, yaw: i32, pitch: i32, speed: i32, total: i32, frames: i32, clut: i32, kind: i32) -> Self {
        Self {
            pos: pos.map(|v| v as i64),
            ty: (ty & 0xFF) as usize,
            clut: clut.max(0) as usize,
            yaw: yaw as i16 as i64,
            pitch: pitch as i16 as i64,
            speed: speed as i16 as i64,
            frames: frames as u8,
            per: if frames != 0 { (total / frames) as u8 } else { 0 },
            n: 0,
            kind: kind as u8,
            preset: 0,
            side: 0,
            flag: 0,
            node: 0,
        }
    }
}

/// The fighters a scene can have (a match's two, the team battle's four): the per-fighter back colour scripts and
/// move-cancel flags (by player in the imported game, two).
pub const SIDES: usize = crate::fight::team::N;

/// A quad of the tasks (POLY_F4 / POLY_G4, semi-transparent): its screen points, their colours, its OT entry, its
/// texture page word (its blending: `ADD`, or `SUB` for a DR_MODE of 0xD5).
type Flat = ([(f32, f32); 4], [[u8; 3]; 4], i32, u16);
const ADD: u16 = 0x20;
const SUB: u16 = 0x40;

/// An effect task (7 slots of 0xDD0 bytes, its type's init and update from the task table).
enum Task {
    Crescent(Crescent),
    Crystal(Box<Crystal>),
    Burst(Box<Burst>),
    Trail(Box<Trail>),
    Claw(Box<Claw>),
    Ribbon(Box<Ribbon>),
    Ring(Box<Ring>),
    Star(Box<Star>),
    Ring2(Box<Ring2>),
    Rays(Box<Rays>),
    Aura(Box<Aura>),
    Whirl(Box<Whirl>),
    Globe(Box<Globe>),
    Star7(Box<Star7>),
    Beams(Box<Beams>),
    Band(Box<Band>),
    Tornado(Box<Tornado>),
    Flame(Box<Flame>),
    Pie(Box<Pie>),
    Pulse(Box<Pulse>),
}

impl Task {
    /// A frame of the task; `fs` the fighters, `cancel` the fighters' move-cancel flags.
    fn step(&mut self, t: &Tables, gte: &Gte, fs: &[Fighter], cancel: [u8; SIDES], rnd: &mut Rand, yaw: i32) -> Option<Vec<Flat>> {
        match self {
            Task::Pulse(k) => {
                let f = &fs[k.side];
                k.step(t, gte, f)
            }
            Task::Pie(k) => {
                let f = &fs[k.side];
                k.step(t, gte, f)
            }
            Task::Flame(k) => {
                let f = &fs[k.side];
                k.step(gte, f)
            }
            Task::Tornado(k) => {
                let f = &fs[k.side];
                k.step(t, gte, f)
            }
            Task::Band(k) => {
                let f = &fs[k.side];
                k.step(gte, f)
            }
            Task::Beams(k) => {
                let f = &fs[k.side];
                k.step(t, gte, f)
            }
            Task::Star7(k) => {
                let f = &fs[k.side];
                k.step(t, gte, f)
            }
            Task::Globe(k) => {
                let f = &fs[k.side];
                k.step(t, gte, f)
            }
            Task::Whirl(k) => {
                let f = &fs[k.side];
                k.step(t, gte, f, yaw, rnd)
            }
            Task::Trail(k) => {
                let f = &fs[k.side];
                k.step(gte, f, cancel[k.side] != 0)
            }
            Task::Ribbon(k) => {
                let f = &fs[k.side];
                k.step(t, gte, f, cancel[k.side] != 0, rnd)
            }
            Task::Aura(k) => {
                let f = &fs[k.side];
                k.step(t, gte, f, rnd)
            }
            Task::Rays(k) => {
                let f = &fs[k.side];
                k.step(t, gte, f)
            }
            Task::Ring2(k) => {
                let f = &fs[k.side];
                k.step(t, gte, f)
            }
            Task::Claw(k) => {
                let f = &fs[k.side];
                k.step(t, gte, f, cancel[k.side] != 0)
            }
            Task::Crescent(c) => c.step(t, gte).map(|q| q.into_iter().map(|q| (q, [[0, 0, 0xA4]; 4], 10, ADD)).collect()),
            Task::Crystal(c) => c.step(t, gte),
            Task::Burst(b) => b.step(t, gte),
            Task::Ring(r) => r.step(t, gte),
            Task::Star(s) => s.step(t, gte),
        }
    }
}

/// The colour slots' tables: the RGB palette, the index lists of the flat and of the
/// gouraud quads (4 indices an entry), the records (12 bytes: the steps' frames up to an 0xFF, from +6 the steps'
/// entries).
/// (offsets in `fx.data_b`)
const PALETTE: u32 = 0x25C;
const INDICES: u32 = 0x314;
const INDICES4: u32 = 0x354;
const RECORDS: u32 = 0x160;

/// A colour slot (7 of 0x28 bytes), stepped before the tasks: the record
/// (+0x1C), the step in it (+0x20), the step's frames and those left (+0x21 / +0x22), the fade (+0x0C, down by +0x0E),
/// gouraud (+0x23 = 4) or flat (1), the colour groups (+0x24: quad k takes group k mod groups), the frames before it
/// steps (+0x25).
struct Colours {
    rec: u32,
    gouraud: bool,
    groups: u32,
    delay: u8,
    idx: u32,
    dur: i32,
    left: i32,
    fade: i16,
    fade_step: i16,
}

impl Colours {
    fn new(t: &Tables, model: u32, gouraud: bool, fade: i16, fade_step: i16, groups: u32, delay: u8) -> Self {
        let rec = t.b(RECORDS) + 12 * model;
        let dur = (t.tables.abs_u8(rec) & 0x3F) as i32;
        Self { rec, gouraud, groups, delay, idx: 0, dur, left: dur, fade, fade_step }
    }

    /// The colours of group `g` of the step's pair `k` (0 from, 1 to) at a quad's 4 points: the palette's entries of
    /// the index list's (gouraud: 4 an entry).
    fn pair(&self, t: &Tables, k: u32, g: u32) -> [[i32; 3]; 4] {
        let e = t.tables.abs_u8(self.rec + 6 + self.idx + k) as u32;
        std::array::from_fn(|j| {
            let i = if self.gouraud { t.tables.abs_u8(t.b(INDICES4) + 4 * (e + g) + j as u32) } else { t.tables.abs_u8(t.b(INDICES) + e + g) } as u32;
            std::array::from_fn(|c| t.tables.abs_u8(t.b(PALETTE) + 3 * i + c as u32) as i32)
        })
    }

    /// A frame: each group's colours `to + (from − to) · left / frames` (`>> 12`), faded
    /// (· fade >> 12); then (once the delay ran out) the step's frames down, at 0 the next step (past an 0xFF the first),
    /// and the fade down; None once the fade went below 0 (the slot freed, its quads keeping their last colours).
    fn step(&mut self, t: &Tables) -> Option<Vec<[[u8; 3]; 4]>> {
        if self.fade < 0 {
            return None;
        }
        let tt = (self.left << 12) / self.dur.max(1);
        let rgb = (0..self.groups)
            .map(|g| {
                let (from, to) = (self.pair(t, 0, g), self.pair(t, 1, g));
                std::array::from_fn(|j| {
                    std::array::from_fn(|c| {
                        let d = (from[j][c] - to[j][c]) as i16 as i32;
                        let v = (to[j][c] + ((d * tt) >> 12)) as i16 as i32;
                        ((v * self.fade as i32) >> 12) as u8
                    })
                })
            })
            .collect();
        if self.delay != 0 {
            self.delay -= 1;
            if self.delay != 0 {
                return Some(rgb);
            }
        }
        self.left -= 1;
        if self.left & 0xFF == 0 {
            self.idx += 1;
            if t.tables.abs_u8(self.rec + self.idx) == 0xFF {
                self.idx = 0;
            }
            self.dur = (t.tables.abs_u8(self.rec + self.idx) & 0x3F) as i32;
            self.left = self.dur;
        }
        self.fade = self.fade.wrapping_sub(self.fade_step);
        Some(rgb)
    }
}

/// Type 0x0D (a guard's two crescents): `d[k]` = its halfwords +0xD80 + 2k.
struct Crescent {
    pos: [i16; 3],
    d: [i16; 11],
}

impl Crescent {
    /// The init (a1 = the radius' speed, a2 = the arc step's growth, a3 = the life's step divisor).
    fn new(pos: [i16; 3], a1: i32, a2: i32, a3: i32, rnd: &mut Rand) -> Self {
        let r1 = rnd.rand();
        let r2 = rnd.rand();
        let a1 = a1 as i16;
        let d8c = (if a1 < 0 { a1 as i32 + 3 } else { a1 as i32 }) >> 2;
        let d94 = if a3 != 0 { 60 / a3 } else { 0 };
        Self { pos, d: [0, a1, r1 as i16, (0x200 - (r2 & 0x3FF)) as i16, 0x10, a2 as i16, d8c as i16, 0xE66, 0x14, 0x3C, d94 as i16] }
    }

    /// A frame of the crescents (the motion while the game runs), Some(quads) to draw, None once over:
    /// the radius d80 and the angle d84 grow by their speeds, the speeds damped by d8e (itself down by d90), the arc step
    /// d88 grows by d8a up to 0x100, the life d92 down by d94 (over below d94); the inner edge's offset d8c =
    /// d80 · d92 / 100. Each of the two arcs: 9 points at the angle d84 + k · ±d88 at the projected radius of d80
    /// outside and from that of d80 − d8c widening to it inside (the depth otz − 0x32, at most 0x7EF),
    /// 8 flat quads between them.
    fn step(&mut self, t: &Tables, gte: &Gte) -> Option<Vec<[(f32, f32); 4]>> {
        let d = &mut self.d;
        let w = |a: i16, b: i16| a.wrapping_add(b);
        d[0] = w(d[0], d[1]);
        d[2] = w(d[2], d[3]);
        d[4] = w(d[4], d[5]);
        d[7] = d[7].wrapping_sub(d[8]);
        d[1] = ((d[1] as i32 * d[7] as i32) >> 12) as i16;
        d[3] = ((d[3] as i32 * d[7] as i32) >> 12) as i16;
        d[9] = d[9].wrapping_sub(d[10]);
        if d[9] < d[10] {
            return None;
        }
        d[6] = (d[0] as i32 * d[9] as i32 / 100) as i16;
        d[4] = d[4].min(0x100);
        let q = gte.rtps([self.pos[0] as i32, self.pos[1] as i32, self.pos[2] as i32])?;
        let z = if (0..0x7EF).contains(&(q.otz - 0x33)) { q.otz - 0x32 } else { 0x7EF } as f32;
        let proj = |s: i32| s as f32 * gte.h / (z * 4.0);
        let outer = proj(d[0] as i32);
        let inner = proj(d[0] as i32 - d[6] as i32);
        let step = (outer - inner) / 9.0;
        let mut quads = Vec::new();
        for side in 0..2 {
            let da = if side == 0 { d[4] } else { -d[4] } as i64;
            let mut a = d[2] as i64;
            let mut r_in = inner + step;
            let mut pts = Vec::new();
            for _ in 0..9 {
                let (c, s) = (t.rcos(a) as f32 / 4096.0, t.rsin(a) as f32 / 4096.0);
                pts.push(((q.sx + outer * c, q.sy + outer * s), (q.sx + r_in * c, q.sy + r_in * s)));
                a += da;
                r_in += step;
            }
            for k in 0..8 {
                quads.push([pts[k].0, pts[k].1, pts[k + 1].0, pts[k + 1].1]);
            }
        }
        Some(quads)
    }
}

/// Type 0x13 (a guard's crystal): a spinning shell of 11 points in a 3-D object
/// (6 of them), its 8 flat quads coloured by a colour slot.
struct Crystal {
    pos: [i16; 3],
    /// the points (object +0x3C)
    v: [[i16; 3]; 11],
    /// d84 (the spin), d86 (its speed), d88 (the tilt), d8a (the speed's step down)
    spin: i16,
    speed: i16,
    tilt: i16,
    slow: i16,
    col: Colours,
    rgb: [[u8; 3]; 4],
}

impl Crystal {
    /// The points (the top (0, size, 0), a ring of 3 at radius and height size / √2
    /// (`size · 1000 · 0x2E591331 >> 40`), one of 3 at radius size on the middle, their mirrors; the rings' points
    /// 0x200 apart from the random spin), the spin's speed 0x168, a random tilt; the colour slot (its setup (task, 0,
    /// 8 quads, fade 0x0DAC by 0xAF, record `model`)).
    fn new(t: &Tables, pos: [i16; 3], model: u32, size: i32, slow: i32, rnd: &mut Rand) -> Self {
        let r0 = ((size as i64 * 1000 * 0x2E59_1331) >> 40) as i16;
        let r = [r0, size as i16];
        let spin = rnd.rand() as i16;
        let tilt = rnd.rand() as i16;
        let mut v = [[0i16; 3]; 11];
        v[0] = [0, size as i16, 0];
        for ring in 0..2 {
            let y = if ring == 0 { r0 } else { 0 };
            let mut a = spin as i64;
            for k in 0..3 {
                let rr = r[ring] as i64;
                v[1 + 3 * ring + k] = [((rr * t.rcos(a)) >> 12) as i16, y, ((rr * t.rsin(a)) >> 12) as i16];
                a += 0x200;
            }
        }
        v[10] = [v[0][0], v[0][1].wrapping_neg(), v[0][2]];
        for k in 0..3 {
            v[7 + k] = [v[1 + k][0], v[1 + k][1].wrapping_neg(), v[1 + k][2]];
        }
        let col = Colours::new(t, model, false, 0x0DAC, 0xAF, 1, 0);
        Self { pos, v, spin, speed: 0x168, tilt, slow: slow as i16, col, rgb: [[0; 3]; 4] }
    }

    /// A frame: the colour slot's, then the update: the points turned by Rx(tilt) · Ry(spin)
    /// to the position, projected, 8 quads between the rings (the ends a point each; the quad drawer at OT 10); the spin on
    /// by its speed, the speed down by `slow`, over (after this frame's draw) below 0x78.
    fn step(&mut self, t: &Tables, gte: &Gte) -> Option<Vec<Flat>> {
        if self.speed < 0x78 {
            return None;
        }
        if let Some(c) = self.col.step(t) {
            self.rgb = c[0];
        }
        // Rx(tilt) · Ry(spin)
        let (sa, ca, sb, cb) = (t.rsin(self.tilt as i64), t.rcos(self.tilt as i64), t.rsin(self.spin as i64), t.rcos(self.spin as i64));
        let m = [[cb, 0, sb], [(sb * sa) >> 12, ca, -((cb * sa) >> 12)], [-((sb * ca) >> 12), sa, (cb * ca) >> 12]].map(|r| r.map(|e| e as i16 as i64));
        let pts: Vec<Option<(f32, f32)>> = self
            .v
            .iter()
            .map(|p| {
                let w: [i32; 3] = std::array::from_fn(|i| {
                    let r = (m[i][0] * p[0] as i64 + m[i][1] * p[1] as i64 + m[i][2] * p[2] as i64) >> 12;
                    self.pos[i].wrapping_add(r as i16) as i32
                });
                gte.rtps(w).map(|q| (q.sx, q.sy))
            })
            .collect();
        let row = |r: usize, k: usize| match r {
            0 => pts[0],
            4 => pts[10],
            _ => pts[1 + 3 * (r - 1) + k],
        };
        let mut quads = Vec::new();
        for r in 0..4 {
            for k in 0..2 {
                if let (Some(a), Some(b), Some(c), Some(d)) = (row(r, k), row(r, k + 1), row(r + 1, k), row(r + 1, k + 1)) {
                    quads.push(([a, b, c, d], [self.rgb[0]; 4], 10, ADD));
                }
            }
        }
        self.spin = self.spin.wrapping_add(self.speed);
        self.speed = self.speed.wrapping_sub(self.slow);
        Some(quads)
    }
}

/// Type 0x09 (a strong hit's burst): spikes from the contact point, each a
/// gouraud triangle from the centre to two points flying out and slowing, coloured by a colour slot.
struct Burst {
    pos: [i16; 3],
    /// per spike its two points and their velocities (object +0x44, 0x20 bytes a spike)
    spikes: Vec<[[i16; 3]; 4]>,
    /// d84: the frames left
    life: i16,
    col: Colours,
    rgb: [[u8; 3]; 4],
}

impl Burst {
    /// `((damage − 10) >> 2) + 3` spikes (at most 8) spread round the turn (0x1000 /
    /// their count apart from 0x200, about the direction a quarter turn off the hit's; the pitch mirrored when the yaw
    /// is outside 0x400..=0xC00 and the pitch below 0x800), each two velocities at `rand % (damage · 8) + 0x40`
    /// (the second halved), their angles 0x14 + rand & 0xF on from each other; the colour slot (record 0, gouraud,
    /// fade 0x1000 by 0x100), 14 frames.
    fn new(t: &Tables, pos: [i16; 3], yaw: i32, pitch: i32, dmg: i32, rnd: &mut Rand) -> Self {
        let n = (((dmg - 10) >> 2) + 3).min(8);
        let step = 0x1000 / n as i64;
        let col = Colours::new(t, 0, true, 0x1000, 0x100, 1, 0);
        let yaw = (yaw & 0xFFF) as i64;
        let mut pitch = (pitch & 0xFFF) as i64;
        if !(0x400..=0xC00).contains(&yaw) && pitch < 0x800 {
            pitch = 0x1000 - pitch;
        }
        let (a, b) = ((yaw + 0x400) & 0xFFF, (pitch + 0x400) & 0xFFF);
        let (mut s7, mut s6) = (0x200i64, 0i64);
        let off = |base: i64, s: i64| if 0x800 - s >= 0 { base - 0x800 + s } else { base + 0x800 - s } & 0xFFF;
        let mut spikes = Vec::new();
        for _ in 0..n.max(0) {
            s7 = (s7 + step) & 0xFFF;
            s6 = (s6 + step) & 0xFFF;
            let (mut s1, mut s5) = (off(a, s7), off(b, s6));
            let mut sp = [pos, pos, [0; 3], [0; 3]];
            for (k, sh) in [12, 13].into_iter().enumerate() {
                let s4 = rnd.rand() % (dmg as i64 * 8) + 0x40;
                sp[2 + k] = [((s4 * t.rcos(s1)) >> sh) as i16, ((s4 * t.rcos(s5)) >> sh) as i16, ((s4 * t.rsin(s1)) >> sh) as i16];
                s1 += 0x14 + (rnd.rand() & 0xF);
                s5 += 0x14 + (rnd.rand() & 0xF);
            }
            spikes.push(sp);
        }
        Self { pos, spikes, life: 14, col, rgb: [[0; 3]; 4] }
    }

    /// A frame: the colour slot's, then the update: the centre projected (its OT entry the otz, at most
    /// 0x7EF), each spike's points on by their velocities, the velocities · 0xD99 >> 12; a triangle per spike (centre,
    /// centre, point, point: gouraud); over after 14 frames.
    fn step(&mut self, t: &Tables, gte: &Gte) -> Option<Vec<Flat>> {
        if self.life <= 0 {
            return None;
        }
        if let Some(c) = self.col.step(t) {
            self.rgb = c[0];
        }
        let c = gte.rtps(self.pos.map(|v| v as i32));
        for s in &mut self.spikes {
            for k in 0..2 {
                for i in 0..3 {
                    s[k][i] = s[k][i].wrapping_add(s[k + 2][i]);
                }
            }
            for k in 2..4 {
                s[k] = s[k].map(|v| ((0xD99 * v as i32) >> 12) as i16);
            }
        }
        let mut quads = Vec::new();
        if let Some(c) = c {
            let otz = if (1..0x7F0).contains(&c.otz) { c.otz } else { 0x7EF };
            for s in &self.spikes {
                if let (Some(p), Some(q)) = (gte.rtps(s[0].map(|v| v as i32)), gte.rtps(s[1].map(|v| v as i32))) {
                    quads.push(([(c.sx, c.sy), (c.sx, c.sy), (p.sx, p.sy), (q.sx, q.sy)], self.rgb, otz, ADD));
                }
            }
        }
        self.life -= 1;
        Some(quads)
    }
}

/// The trails' colour ramps (`fx.data_b` +0x4E4: per character 0x87 bytes, per form 0x36, two ramps of 9 RGB a trail).
const RAMPS: u32 = 0x4E4;

/// The fighter data a trail reads: its nodes' world positions (`F.1C`'s MD, +0x34), character and form, player.
struct Fighter<'a> {
    md: &'a [crate::fight::br1::object::Node],
    chr: u8,
    form: u8,
    player: u8,
}

impl Fighter<'_> {
    fn of(f: &crate::fight::br1::object::Br1Fighter) -> Fighter<'_> {
        Fighter { md: &f.forms[f.cur as usize & 1].md, chr: f.f.chr, form: f.f.form, player: f.f.player }
    }
    fn node(&self, n: i16) -> [i16; 3] {
        self.md.get(n as usize).map_or([0; 3], |d| d.wt.map(|v| v as i16))
    }
}

/// Type 0x0F (an attack's trail between two nodes): 10 rows of the two nodes'
/// positions (object +0x3C, 8 bytes a point), the newest first, a frame's two rows apart with a Catmull-Rom midpoint
/// between; gouraud quads between the rows in the fighter's two colour ramps.
struct Trail {
    side: usize,
    /// d8c / d8e: the nodes
    nodes: [i16; 2],
    /// d80: the rows drawn (from −1, 2 more a frame up to 8)
    rows: i16,
    /// d90: the fade (0x15, 2 down a frame), d96: the frames before it, d98: the frames run
    fade: i16,
    life: i16,
    age: i16,
    /// da6: the frames before the trail starts, da4: the frames it records (0 for ever: 0x7FFF)
    delay: i16,
    rec: i16,
    e: [[i16; 3]; 20],
    /// the two ramps (object +0 / +0x1E) and the quads' colours as last written
    ramp: [[u8; 27]; 2],
    rgb: [[[u8; 3]; 4]; 9],
}

impl Trail {
    /// The init (timing = delay | frames << 16, life, node a, node b) for fighter `f` (the trail types:
    /// a life below 0 is 0xE10; none when the delay equals the life).
    fn new(t: &Tables, side: usize, f: &Fighter, tm: i32, life: i32, na: i32, nb: i32) -> Option<Self> {
        let life = if life < 0 { 0xE10 } else { life };
        if tm & 0xFFFF == life {
            return None;
        }
        let rec = (tm >> 16) as i16;
        let base = t.b(RAMPS) + f.chr as u32 * 0x87 + (f.form & 1) as u32 * 0x36;
        let ramp = [0, 1].map(|k| std::array::from_fn(|i| t.tables.abs_u8(base + k * 0x1B + i as u32)));
        let mut tr = Self {
            side,
            nodes: [na as i16, nb as i16],
            rows: -1,
            fade: 0,
            life: life as i16,
            age: 0,
            delay: tm as i16,
            rec: if rec == 0 { 0x7FFF } else { rec },
            e: [[0; 3]; 20],
            ramp,
            rgb: [[[0; 3]; 4]; 9],
        };
        tr.colour();
        Some(tr)
    }

    /// Quad k's points coloured by the ramps' entries k and k + 1 (node a's from the second ramp).
    fn colour(&mut self) {
        let c = |r: &[u8; 27], k: usize| [r[3 * k], r[3 * k + 1], r[3 * k + 2]];
        for k in 0..8 {
            self.rgb[k] = [c(&self.ramp[1], k), c(&self.ramp[0], k), c(&self.ramp[1], k + 1), c(&self.ramp[0], k + 1)];
        }
    }

    /// A frame: over at once when the player's moves were cancelled; waits out the delay;
    /// then fills four rows with the nodes; then each frame (while it records) moves the rows two down, the newest
    /// two the nodes now (node b a quarter further from node a) and the midpoint between rows 1 and 3
    /// (`(−256 · r0 + 2304 · r1 + 2304 · r3) >> 12 − r4 / 16`, MulMatrix by the trail's blend matrix), fading after `life` frames
    /// (the ramps 0x18 down a frame); the quads between the rows at the OT entries of their groups of two rows.
    fn step(&mut self, gte: &Gte, f: &Fighter, cancelled: bool) -> Option<Vec<Flat>> {
        if cancelled {
            return None;
        }
        let (a, b) = (f.node(self.nodes[0]), f.node(self.nodes[1]));
        if self.delay > 0 {
            self.delay -= 1;
            self.age += 1;
            if self.rec != 0 {
                self.rec -= 1;
            }
            return Some(Vec::new());
        }
        if self.delay == 0 {
            for r in 0..4 {
                (self.e[2 * r], self.e[2 * r + 1]) = (a, b);
            }
            self.rec = self.rec.wrapping_sub(1);
            self.delay -= 1;
            return Some(Vec::new());
        }
        self.age += 1;
        if self.fade == 0 && self.age >= self.life {
            self.fade = 0x15;
        }
        if self.fade != 0 {
            self.fade -= 2;
            if self.fade < 0 {
                return None;
            }
        }
        let e = &mut self.e;
        if self.rec != 0 {
            self.rec -= 1;
            self.rows = (self.rows + 2).min(8);
            for k in (2..16).rev() {
                e[k + 4] = e[k];
            }
            (e[2], e[3]) = (e[0], e[1]);
            (e[0], e[1]) = (a, b);
            for i in 0..3 {
                e[1][i] = e[1][i].wrapping_add(((e[1][i] as i32 - e[0][i] as i32) / 4) as i16);
            }
        }
        for s in 0..2 {
            for i in 0..3 {
                let m = (-256 * e[s][i] as i32 + 2304 * e[2 + s][i] as i32 + 2304 * e[6 + s][i] as i32) >> 12;
                e[4 + s][i] = (m as i16).wrapping_add(((-((e[8 + s][i] as i32) << 8)) >> 12) as i16);
            }
        }
        if self.fade != 0 {
            self.colour();
            for v in self.ramp.iter_mut().flatten() {
                *v = v.saturating_sub(0x18);
            }
        }
        if self.rows <= 0 {
            return Some(Vec::new());
        }
        // the points by fours (RotTransPers4: their mean otz, 1..=0x7EF); the rows' OT entries those of
        // groups 0..4, the last four rows group 4's
        let p: Vec<Option<crate::output::gte::Proj>> = self.e.iter().map(|v| gte.rtps(v.map(|c| c as i32))).collect();
        let otz: Vec<i32> = (0..5)
            .map(|g| {
                let z = (0..4).map(|k| p[4 * g + k].as_ref().map_or(0, |q| q.otz)).sum::<i32>() / 4;
                if (1..0x7F0).contains(&z) { z } else { 0x7EF }
            })
            .collect();
        let mut quads = Vec::new();
        for r in 0..self.rows as usize {
            if let (Some(a), Some(b), Some(c), Some(d)) = (&p[2 * r], &p[2 * r + 1], &p[2 * r + 2], &p[2 * r + 3]) {
                quads.push(([(a.sx, a.sy), (b.sx, b.sy), (c.sx, c.sy), (d.sx, d.sy)], self.rgb[r], otz[r.min(4)], ADD));
            }
        }
        Some(quads)
    }
}

/// The claws' sizes (`fx.data_b` +0xB04: 17 bytes a character, +0x0D).
const CLAW_SIZES: u32 = 0xB04;

/// Type 0x10 (a beast's claw slash beside type 0x09's): three claws (Mitsuko
/// two), each a triangle (tip and two sides) on the attack's node, turned by its world matrix, whose last three
/// positions make three rows of gouraud quads; coloured by a colour slot of three groups.
struct Claw {
    side: usize,
    node: i16,
    /// d80 / d82 / d84: the rows' counters (a row drawn from 1)
    rows: [i16; 3],
    /// d96: the frames before the fade, d98: the frames run, d9a: the fade (0x15, 2 down a frame), da6: the delay,
    /// da2 / da4: the first claw drawn and the claws' step (2 for Mitsuko; Fox's and Uriko's hand ribbons the middle one)
    life: i16,
    age: i16,
    fade: i16,
    delay: i16,
    start: usize,
    step: usize,
    /// the claws' triangles (object +0x12C) and their points (object +0x3C: 3 rows of 3 and the third row's middle again)
    tri: [[[i16; 3]; 3]; 3],
    p: [[[i16; 3]; 10]; 3],
    col: Colours,
    rgb: Vec<[[u8; 3]; 4]>,
}

impl Claw {
    /// The init (delay, life, node, _) for fighter `f`: the claws' size from the character (half of it, Mitsuko's
    /// whole with the claws' spread an eighth), across x on nodes 7 and 10 and z on the others, the outer two moved by
    /// ±(size + spread); the colour slot (record 3, Uriko's 0x14, gouraud, 3 groups, fade 0x1000 by 0x199 after `life`
    /// frames).
    fn new(t: &Tables, side: usize, f: &Fighter, tm: i32, life: i32, node: i32) -> Self {
        let size = t.tables.abs_u8(t.b(CLAW_SIZES) + f.chr as u32 * 17 + 0xD) as i32;
        let (a2, t1, step) = if f.chr == 4 { (size, size >> 3, 2) } else { (size >> 1, size >> 2, 1) };
        let (hi, lo) = ((t1 as i16) >> 1, (-(t1 as i16)) >> 1);
        let across = node == 10 || node == 7;
        let a2s = a2 as i16;
        let mut tri = [[[0i16; 3]; 3]; 3];
        for t in tri.iter_mut() {
            t[0] = [0, 2 * a2s, 0];
            (t[1], t[2]) = if across { ([hi, a2s, 0], [lo, a2s, 0]) } else { ([0, a2s, hi], [0, a2s, lo]) };
        }
        let mut o = (a2 + t1) as i16;
        if step == 2 {
            o >>= 1;
        }
        for c in [0, 2] {
            for v in tri[c].iter_mut() {
                let i = if across { 0 } else { 2 };
                v[i] = v[i].wrapping_add(o);
            }
            o = o.wrapping_neg();
        }
        let col = Colours::new(t, if f.chr == 8 { 0x14 } else { 3 }, true, 0x1000, 0x199, 3, life as u8);
        Self {
            side,
            node: node as i16,
            rows: [-1, -2, -3],
            life: life as i16,
            age: 0,
            fade: 0,
            delay: tm as i16,
            start: 0,
            step,
            tri,
            p: [[[0; 3]; 10]; 3],
            col,
            rgb: Vec::new(),
        }
    }

    /// A frame: over when the player's moves were cancelled; the delay; then while not fading: after
    /// `life` frames the fade starts (each claw's newest row closed on its middle), else the rows move down one (the
    /// third row's middle kept as the last point), the newest the triangles on the node now, the rows' counters on;
    /// fading: 2 down a frame, over below 0. Drawn: per claw (by the step) two fans of the rows drawn, quad k coloured
    /// by group k mod 3, at the OT entry of the last claw's row of that claw's index (the OT grouping's table).
    fn step(&mut self, t: &Tables, gte: &Gte, f: &Fighter, cancelled: bool) -> Option<Vec<Flat>> {
        if let Some(c) = self.col.step(t) {
            self.rgb = c;
        }
        if cancelled {
            return None;
        }
        if self.delay > 0 {
            self.delay -= 1;
            self.age += 1;
            return Some(Vec::new());
        }
        if self.fade != 0 {
            self.fade -= 2;
            if self.fade < 0 {
                return None;
            }
        } else {
            self.age += 1;
            if self.age >= self.life {
                self.fade = 0x15;
                for c in &mut self.p {
                    (c[0], c[2]) = (c[1], c[1]);
                }
            } else {
                for c in &mut self.p {
                    c[9] = c[7];
                    for k in (0..6).rev() {
                        c[k + 3] = c[k];
                    }
                }
                let d = f.md.get(self.node as usize);
                let (wm, wt) = d.map_or(([[0; 3]; 3], [0; 3]), |d| (d.wm, d.wt));
                for (c, tri) in self.p.iter_mut().zip(&self.tri) {
                    for (k, v) in tri.iter().enumerate() {
                        // ApplyMatrix(world, v) + the node's position
                        c[k] = std::array::from_fn(|i| {
                            let r = (wm[i][0] as i64 * v[0] as i64 + wm[i][1] as i64 * v[1] as i64 + wm[i][2] as i64 * v[2] as i64) >> 12;
                            (r as i16).wrapping_add(wt[i] as i16)
                        });
                    }
                }
                for r in &mut self.rows {
                    *r += 1;
                }
            }
        }
        // the OT grouping per claw (RotTransPers3 by rows: their mean otz into the table's entries 0..2) and the last point
        let proj: Vec<Vec<Option<crate::output::gte::Proj>>> = self.p.iter().map(|c| c.iter().map(|v| gte.rtps(v.map(|x| x as i32))).collect()).collect();
        let otz: Vec<i32> = (0..3)
            .map(|r| {
                let z = (0..3).map(|k| proj[2][3 * r + k].as_ref().map_or(0, |q| q.otz)).sum::<i32>() / 3;
                if (1..0x7F0).contains(&z) { z } else { 0x7EF }
            })
            .collect();
        let pt = |c: usize, r: usize, k: usize| if r == 3 { &proj[c][9] } else { &proj[c][3 * r + k] };
        let mut quads = Vec::new();
        let mut k = 0;
        let mut c = self.start;
        while c < 3 {
            for s in 0..2 {
                for r in 0..3 {
                    if self.rows[r] <= 0 {
                        continue;
                    }
                    let rgb = self.rgb.get(k % 3).copied().unwrap_or([[0; 3]; 4]);
                    k += 1;
                    if let (Some(a), Some(b), Some(d), Some(e)) = (pt(c, r, 0), pt(c, r, s + 1), pt(c, r + 1, 0), pt(c, r + 1, s + 1)) {
                        quads.push(([(a.sx, a.sy), (b.sx, b.sy), (d.sx, d.sy), (e.sx, e.sy)], rgb, otz[c], ADD));
                    }
                }
            }
            c += self.step;
        }
        Some(quads)
    }
}

/// The effects' cheap angle of a screen vector (a turn of 0x1000, linear in the ratio per octant).
fn angle(x: i32, y: i32) -> i32 {
    let (x, y) = (if x == 0 { 1 } else { x }, if y == 0 { 1 } else { y });
    let (ax, ay) = (x.abs(), y.abs());
    if ay < ax {
        let v = ((y << 9) + if y > 0 { ax << 3 } else { -(ax << 3) }) / ax;
        if x > 0 { v + 0x400 } else { 0xC00 - v }
    } else {
        let v = ((x << 9) + if x > 0 { ay << 3 } else { -(ay << 3) }) / ay;
        if y > 0 {
            0x800 - v
        } else if x > 0 {
            v
        } else {
            v + 0xFFF
        }
    }
}

/// The hand ribbons' extra width in beast form (`fx.data_b` +0xBA0, a byte a character) and colour ramps (+0x550: 9 RGB
/// a character, 0x87 apart).
const RIBBON_BEAST: u32 = 0xBA0;
const RIBBON_RAMPS: u32 = 0x550;

/// Type 0x12 (a human's hand ribbon): the node's last positions (object +0x3C, the
/// newest first, a frame two rows apart with a Catmull-Rom midpoint between), projected, widened on screen across
/// their direction by a radius tapering from the hand, each row two gouraud quads from the line (the ramp's colour) to
/// its edges (black); a fan of six around the hand at 1.5 times its radius.
struct Ribbon {
    side: usize,
    node: i16,
    /// d80: the rows drawn (from −1, 2 more a frame up to 9); d90: the fade; d96: the frames before it; d98: the
    /// frames run; da6: the delay
    rows: i16,
    fade: i16,
    life: i16,
    age: i16,
    delay: i16,
    p: [[i16; 3]; 10],
    /// the radii along the ribbon (object +0x154) and the ramp (object +0)
    r: [i16; 10],
    ramp: [u8; 27],
}

impl Ribbon {
    /// The init (delay, life, node, _) for fighter `f` (Fox's and Uriko's are a claw instead: `Claw` with only its
    /// middle claw): the radius from the claws' size table (plus the beast form's extra), tapering by 6% a point.
    fn new(t: &Tables, side: usize, f: &Fighter, tm: i32, life: i32, node: i32) -> Self {
        let mut size = t.tables.abs_u8(t.b(CLAW_SIZES) + f.chr as u32 * 17 + 0xD) as i32;
        if f.form & 1 != 0 {
            size += t.tables.abs_u8(t.b(RIBBON_BEAST) + f.chr as u32) as i32;
        }
        let r = std::array::from_fn(|k| ((size as i16 as i32 * (((100 - 6 * k as i32) << 12) / 100)) >> 12) as i16);
        let base = t.b(RIBBON_RAMPS) + f.chr as u32 * 0x87;
        Self {
            side,
            node: node as i16,
            rows: -1,
            fade: 0,
            life: life as i16,
            age: 0,
            delay: tm as i16,
            p: [[0; 3]; 10],
            r,
            ramp: std::array::from_fn(|i| t.tables.abs_u8(base + i as u32)),
        }
    }

    /// A frame: over when the player's moves were cancelled; the delay; the four first rows filled; then
    /// the fade after `life` frames (the ramp 0x17 down a frame, over below 0), the rows moved down two (the newest
    /// the node now, the midpoint as the trail's); drawn: the points projected (OT entry otz − 10), the radii
    /// `r · H / (4 · z)`, each point's direction that to the next averaged along the ribbon, the edges at
    /// ±radius across it; the fan's points 0x2AA apart from a random angle (libc rand).
    fn step(&mut self, t: &Tables, gte: &Gte, f: &Fighter, cancelled: bool, rnd: &mut Rand) -> Option<Vec<Flat>> {
        if cancelled {
            return None;
        }
        let a = f.node(self.node);
        if self.delay > 0 {
            self.delay -= 1;
            self.age += 1;
            return Some(Vec::new());
        }
        if self.delay == 0 {
            for k in 0..4 {
                self.p[k] = a;
            }
            self.delay -= 1;
            return Some(Vec::new());
        }
        self.age += 1;
        if self.fade == 0 && self.age >= self.life {
            self.fade = 0x15;
        }
        if self.fade != 0 {
            self.fade -= 2;
            if self.fade < 0 {
                return None;
            }
        }
        self.rows = (self.rows + 2).min(9);
        let p = &mut self.p;
        for k in (1..8).rev() {
            p[k + 2] = p[k];
        }
        p[1] = p[0];
        p[0] = a;
        for i in 0..3 {
            let m = (-256 * p[0][i] as i32 + 2304 * p[1][i] as i32 + 2304 * p[3][i] as i32) >> 12;
            p[2][i] = (m as i16).wrapping_add(((-((p[5][i] as i32) << 8)) >> 12) as i16);
        }
        if self.fade != 0 {
            for v in &mut self.ramp {
                *v = v.saturating_sub(0x17);
            }
        }
        let c = |k: usize| [self.ramp[3 * k], self.ramp[3 * k + 1], self.ramp[3 * k + 2]];
        let proj: Vec<Option<crate::output::gte::Proj>> = self.p.iter().map(|v| gte.rtps(v.map(|x| x as i32))).collect();
        if proj.iter().any(|q| q.is_none()) {
            return Some(Vec::new());
        }
        let pt: Vec<(f32, f32)> = proj.iter().flatten().map(|q| (q.sx, q.sy)).collect();
        let z: Vec<i32> = proj.iter().flatten().map(|q| if (0..0x7EF).contains(&(q.otz - 0xB)) { q.otz - 10 } else { 0x7EF }).collect();
        let mut rad = [0f32; 10];
        let mut ang = [0i32; 10];
        for k in 0..9 {
            rad[k] = self.r[k] as f32 * gte.h / (z[k] * 4) as f32;
            ang[k] = angle((pt[k + 1].0 - pt[k].0).round() as i32, (pt[k + 1].1 - pt[k].1).round() as i32);
        }
        (rad[9], ang[9]) = (rad[8], ang[8]);
        for k in 0..9 {
            let (a, b) = (ang[k], ang[k + 1]);
            let m = (a + b) / 2;
            ang[k + 1] = if (a - b).abs() < 0x800 { m } else { (m + 0x800) & 0xFFF };
        }
        let off = |k: usize| (rad[k] * t.rcos(ang[k] as i64) as f32 / 4096.0, rad[k] * t.rsin(ang[k] as i64) as f32 / 4096.0);
        let edge: Vec<[(f32, f32); 2]> = (0..10).map(|k| [(pt[k].0 + off(k).0, pt[k].1 + off(k).1), (pt[k].0 - off(k).0, pt[k].1 - off(k).1)]).collect();
        let mut quads = Vec::new();
        for s in 0..self.rows.max(0) as usize {
            let e = s.saturating_sub(1);
            for side in 0..2 {
                quads.push(([pt[s], pt[s + 1], edge[s][side], edge[s + 1][side]], [c(e), c(e + 1), [0; 3], [0; 3]], z[s], ADD));
            }
        }
        // the fan round the hand
        let r0 = rad[0].floor() + (rad[0].floor() / 2.0).floor();
        let mut s0 = rnd.rand();
        let fan: Vec<(f32, f32)> = (0..6)
            .map(|_| {
                let q = (pt[0].0 + r0 * t.rcos(s0) as f32 / 4096.0, pt[0].1 + r0 * t.rsin(s0) as f32 / 4096.0);
                s0 += 0x2AA;
                q
            })
            .collect();
        for k in 0..6 {
            quads.push(([pt[0], pt[0], fan[k], fan[(k + 1) % 6]], [c(0), c(0), [0; 3], [0; 3]], z[0], ADD));
        }
        Some(quads)
    }
}

/// Type 0x1F (a shockwave on the floor): two rings of 12 points (the outer at
/// the radius, the inner the width inside it) widening and slowing, 12 flat quads between them coloured by a colour
/// slot (record 15, fade 0x400 by 0x35).
struct Ring {
    pos: [i16; 3],
    /// d94: the radius, d96: its speed, d98: the width, da6: their damping
    radius: i16,
    speed: i16,
    width: i16,
    damp: i16,
    col: Colours,
    rgb: [[u8; 3]; 4],
}

impl Ring {
    fn new(t: &Tables, pos: [i16; 3]) -> Self {
        let col = Colours::new(t, 15, false, 0x400, 0x35, 1, 0);
        Self { pos, radius: 0x96, speed: 0x28, width: 0xB4, damp: 0xEB8, col, rgb: [[0; 3]; 4] }
    }

    /// A frame: over once the speed is below 6; the radius on by the speed, speed and width damped; the
    /// rings' points 0x156 apart from angle 0, projected by fours (groups of four points' OT entries,
    /// each ring closed on its first), the quads between the rings in three rows of four (drawn at the
    /// groups' entries).
    fn step(&mut self, t: &Tables, gte: &Gte) -> Option<Vec<Flat>> {
        if let Some(c) = self.col.step(t) {
            self.rgb = c[0];
        }
        if self.speed < 6 {
            return None;
        }
        self.radius = self.radius.wrapping_add(self.speed);
        self.speed = ((self.speed as i32 * self.damp as i32) >> 12) as i16;
        self.width = ((self.width as i32 * self.damp as i32) >> 12) as i16;
        let mut p = Vec::with_capacity(24);
        let mut r = self.radius as i64;
        for _ in 0..2 {
            let mut a = 0i64;
            for _ in 0..12 {
                let x = self.pos[0].wrapping_add(((r * t.rcos(a)) >> 12) as i16);
                let z = self.pos[2].wrapping_add(((r * t.rsin(a)) >> 12) as i16);
                p.push(gte.rtps([x as i32, self.pos[1] as i32, z as i32]));
                a += 0x156;
            }
            r -= self.width as i64;
        }
        let otz = |g: usize| {
            let z = (0..4).map(|k| p[4 * g + k].as_ref().map_or(0, |q| q.otz)).sum::<i32>() / 4;
            if (1..0x7F0).contains(&z) { z } else { 0x7EF }
        };
        let mut quads = Vec::new();
        for row in 0..3 {
            for k in 0..4 {
                let i = 4 * row + k;
                let (a, b) = (&p[i], &p[(i + 1) % 12]);
                let (c, d) = (&p[12 + i], &p[12 + (i + 1) % 12]);
                if let (Some(a), Some(b), Some(c), Some(d)) = (a, b, c, d) {
                    quads.push(([(a.sx, a.sy), (b.sx, b.sy), (c.sx, c.sy), (d.sx, d.sy)], [self.rgb[0]; 4], otz(row), ADD));
                }
            }
        }
        Some(quads)
    }
}

/// Type 0x0C (a special hit's stars): four stars of four thin spikes round the
/// contact point, 0xE4 apart, each from its radius to twice it on screen, growing by their speeds; coloured by a colour
/// slot (record 9, flat, fade 0xC00 by 0x18).
struct Star {
    pos: [i16; 3],
    /// d80..d8e: per star its radius and speed (10, 15, 20, 25 each)
    r: [[i16; 2]; 4],
    col: Colours,
    rgb: [[u8; 3]; 4],
}

impl Star {
    fn new(t: &Tables, pos: [i16; 3]) -> Self {
        let col = Colours::new(t, 9, false, 0xC00, 0x18, 1, 0);
        Self { pos, r: [[10; 2], [15; 2], [20; 2], [25; 2]], col, rgb: [[0; 3]; 4] }
    }

    /// A frame: over once the third star's radius passed 0x78, else the radii on by their speeds; the position
    /// projected (its OT entry otz − 0x32, at most 0x7EF), each star's spikes at the screen
    /// radius `r · H / (4 · z)`: (0, ρ), (ρ >> 4, ρ + ρ / 4), (−(ρ >> 4), ρ + ρ / 4), (0, 2ρ) turned by its angle plus
    /// k · 0x400.
    fn step(&mut self, t: &Tables, gte: &Gte) -> Option<Vec<Flat>> {
        if let Some(c) = self.col.step(t) {
            self.rgb = c[0];
        }
        if self.r[2][0] > 0x78 {
            return None;
        }
        for s in &mut self.r {
            s[0] = s[0].wrapping_add(s[1]);
        }
        let q = gte.rtps(self.pos.map(|v| v as i32))?;
        let z = if (0..0x7EF).contains(&(q.otz - 0x33)) { q.otz - 0x32 } else { 0x7EF };
        let mut quads = Vec::new();
        let mut a0 = 0i64;
        for s in &self.r {
            let rr = (s[0] as f32 * gte.h / (4 * z) as f32).trunc();
            let w = (rr / 16.0).floor();
            let shape = [(0.0, rr), (w, rr + (rr / 4.0).trunc()), (-w, rr + (rr / 4.0).trunc()), (0.0, 2.0 * rr)];
            let mut a = a0;
            for _ in 0..4 {
                let (c, sn) = (t.rcos(a) as f32 / 4096.0, t.rsin(a) as f32 / 4096.0);
                let p = shape.map(|(x, y)| (q.sx + x * c - y * sn, q.sy + x * sn + y * c));
                quads.push((p, [self.rgb[0]; 4], z, ADD));
                a += 0x400;
            }
            a0 += 0xE4;
        }
        Some(quads)
    }
}

/// Type 0x02 (a ring on the screen round a node): an annulus of 12 flat quads
/// from the projected radius out by the projected width, growing (mode 0: from 50 by 75 a frame, the
/// speed · 875 >> 10 past 500) or closing in (mode 1: from 500 by 140, the speed · 1625 >> 11), the width 90 down by
/// 4 (mode 1: 7) a frame, over at 0; coloured by a colour slot (record `model`, fade 0x1000 by 0x111).
struct Ring2 {
    side: usize,
    node: i16,
    /// d80 / d82: the radius and its speed; d86 / d88: the width and its step down; da6: the mode
    radius: i16,
    speed: i16,
    width: i16,
    step: i16,
    closing: bool,
    done: bool,
    col: Colours,
    rgb: [[u8; 3]; 4],
}

impl Ring2 {
    fn new(t: &Tables, side: usize, node: i16, model: i32, mode: i32) -> Self {
        let (radius, speed, step) = if mode != 0 { (500, 0x8C, 7) } else { (0x32, 0x4B, 4) };
        let col = Colours::new(t, model as u32, false, 0x1000, 0x111, 1, 0);
        Self { side, node, radius, speed, width: 0x5A, step, closing: mode != 0, done: false, col, rgb: [[0; 3]; 4] }
    }

    fn step(&mut self, t: &Tables, gte: &Gte, f: &Fighter) -> Option<Vec<Flat>> {
        if self.done {
            return None;
        }
        if let Some(c) = self.col.step(t) {
            self.rgb = c[0];
        }
        // the node projected, the depth otz − 0x32 (at most 0x7EF), the radii r · H / (4 · z)
        let q = gte.rtps(f.node(self.node).map(|v| v as i32));
        let mut quads = Vec::new();
        if let Some(q) = &q {
            let z = if (0..0x7EF).contains(&(q.otz - 0x33)) { q.otz - 0x32 } else { 0x7EF };
            let proj = |r: i16| (r as f32 * gte.h / (4 * z) as f32).trunc();
            let inner = proj(self.radius);
            let outer = proj(self.width) + inner;
            let ring = |r: f32| -> Vec<(f32, f32)> {
                (0..13)
                    .map(|k| (k % 12) as i64 * 0x156)
                    .map(|a| (q.sx + (r * t.rcos(a) as f32 / 4096.0).floor(), q.sy + (r * t.rsin(a) as f32 / 4096.0).floor()))
                    .collect()
            };
            let (a, b) = (ring(inner), ring(outer));
            for k in 0..12 {
                quads.push(([a[k], a[k + 1], b[k], b[k + 1]], [self.rgb[0]; 4], z, ADD));
            }
        }
        self.width = self.width.wrapping_sub(self.step);
        if self.closing {
            self.radius = self.radius.wrapping_sub(self.speed);
            self.speed = ((self.speed as i32 * 1625) >> 11) as i16;
        } else {
            self.radius = self.radius.wrapping_add(self.speed);
            if self.radius > 500 {
                self.speed = ((self.speed as i32 * 875) >> 10) as i16;
            }
        }
        // (over at width 0: drawn this frame, freed the next)
        self.done = self.width <= 0;
        Some(quads)
    }
}

/// Type 0x21 (type 0x04's init and update; op 0x17 sub 2's case 9): a ring round a node on the
/// screen (as type 0x02's) whose radius grows by 0x35 a frame from 0x32 while its width, from 0x28, thins by
/// 6 (its table), over once the width is below 0 (not drawn that frame); coloured by a colour slot (record
/// 17, flat, fade 0x1000 by 0x333).
struct Pulse {
    side: usize,
    node: i16,
    /// d80 / d84: the radius and its step, d82: the width
    radius: i16,
    step: i16,
    width: i16,
    col: Colours,
    rgb: [[u8; 3]; 4],
}

impl Pulse {
    fn new(t: &Tables, side: usize, node: i16) -> Self {
        let w = |k: u32| t.tables.abs_u16(t.a(0xD8) + 2 * k) as i16;
        let col = Colours::new(t, 17, false, 0x1000, 0x333, 1, 0);
        Self { side, node, radius: w(0), width: w(1), step: w(2), col, rgb: [[0; 3]; 4] }
    }

    fn step(&mut self, t: &Tables, gte: &Gte, f: &Fighter) -> Option<Vec<Flat>> {
        if let Some(c) = self.col.step(t) {
            self.rgb = c[0];
        }
        self.radius = self.radius.wrapping_add(self.step);
        self.width = self.width.wrapping_sub(6);
        if self.width < 0 {
            return None;
        }
        // the node (the task's pointer to its translation) projected, the depth otz − 0x32 (at most
        // 0x7EF), the radii r · H / (4 · z); the inner one d8a, the outer d8c = d8a + the width's
        let mut quads = Vec::new();
        if let Some(q) = gte.rtps(f.node(self.node).map(|v| v as i32)) {
            let z = if (0..0x7EF).contains(&(q.otz - 0x33)) { q.otz - 0x32 } else { 0x7EF };
            let proj = |r: i16| (r as f32 * gte.h / (4 * z) as f32).trunc();
            let inner = proj(self.radius);
            let outer = inner + proj(self.width);
            let ring = |r: f32| -> Vec<(f32, f32)> {
                (0..13)
                    .map(|k| (k % 12) as i64 * 0x156)
                    .map(|a| (q.sx + (r * t.rcos(a) as f32 / 4096.0).floor(), q.sy + (r * t.rsin(a) as f32 / 4096.0).floor()))
                    .collect()
            };
            let (a, b) = (ring(inner), ring(outer));
            for k in 0..12 {
                quads.push(([a[k], a[k + 1], b[k], b[k + 1]], [self.rgb[0]; 4], z, ADD));
            }
        }
        Some(quads)
    }
}

/// An effect script (4 slots of 0x10 bytes, run after the tasks): its (command, frame) list,
/// the commands' parameter stream, its fighter, its frame counter.
#[derive(Clone)]
struct Script {
    list: u32,
    data: u32,
    side: usize,
    counter: u8,
}

/// A fighter's back colour script (2 slots of 0x10 bytes, run after the effect scripts):
/// its entries (6 bytes each, from the back colour table), the one now and their count, the frames left.
#[derive(Clone)]
struct BackScript {
    start: u32,
    at: u32,
    count: i16,
    n: i16,
    left: i16,
}

/// Type 0x20 (a radial flash round a node): `n` rays 0x1000 / n apart from angle
/// 0x100, each a thin triangle from the inner radius out to the outer one and back to their middle 0x18 further on, at
/// screen radii (inner 300 down by 75 a frame to 0, outer 1000 · 0xB33 >> 12 a frame, over below 150); flat, half of
/// colour `c` of the flash colour table.
struct Rays {
    side: usize,
    node: i16,
    /// d80 / d84: the inner radius and its step, d82: the outer, d92 / d94: the rays and their angle step
    inner: i16,
    in_step: i16,
    outer: i16,
    n: i32,
    step: i32,
    rgb: [u8; 3],
    done: bool,
}

impl Rays {
    fn new(t: &Tables, side: usize, node: i16, n: i32, c: i32) -> Self {
        let p = t.tables.abs_u32(t.b(0x9B4) + 4 * (c as u32 & 3));
        let rgb = [0, 1, 2].map(|k| t.tables.abs_u8(p + k) >> 1);
        Self { side, node, inner: 300, in_step: 300 / 4, outer: 1000, n, step: if n != 0 { 0x1000 / n } else { 0 }, rgb, done: false }
    }

    fn step(&mut self, t: &Tables, gte: &Gte, f: &Fighter) -> Option<Vec<Flat>> {
        if self.done {
            return None;
        }
        self.inner = (self.inner - self.in_step).max(0);
        self.outer = ((self.outer as i32 * 0xB33) >> 12) as i16;
        self.done = self.outer < 0x96;
        let q = gte.rtps(f.node(self.node).map(|v| v as i32));
        let mut quads = Vec::new();
        if let Some(q) = q {
            let z = if (0..0x7EF).contains(&(q.otz - 0x33)) { q.otz - 0x32 } else { 0x7EF };
            // (raw screen units, ×2 on the 640×480 display)
            let at = |r: i32, a: i64| (q.sx + 2.0 * ((r as i64 * t.rcos(a)) >> 12) as f32, q.sy + 2.0 * ((r as i64 * t.rsin(a)) >> 12) as f32);
            let mut a = 0x100i64;
            for _ in 0..self.n.max(0) {
                let p0 = at(self.inner as i32, a);
                let p2 = at(self.outer as i32, a);
                let p3 = at((self.outer as i32 + self.inner as i32) >> 1, a + 0x18);
                quads.push(([p0, p0, p2, p3], [self.rgb; 4], z, ADD));
                a += self.step as i64;
            }
        }
        Some(quads)
    }
}

/// Type 0x16 (a flash of six turning wedges round a node): six arms from angle
/// k · 0x2AA + rand & 0x7F, their widths damped by 0xFAE and their speeds by 0xF33 a frame,
/// each drawn as a wedge from the node's point to two points 0x1000 >> 3 out (raw screen units) at its angle and
/// angle + width; flat, coloured by a colour slot (record 10, fade 0x1000 by 0x66); 40 frames.
struct Pie {
    side: usize,
    node: i16,
    /// d80: the frames run; the arms (object +0x3C: width, angle, speed)
    age: i16,
    arms: [[i16; 3]; 6],
    col: Colours,
    rgb: [u8; 3],
}

impl Pie {
    fn new(t: &Tables, side: usize, node: i16, rnd: &mut Rand) -> Self {
        let arms = std::array::from_fn(|k| {
            let k32 = k as u32;
            [t.tables.abs_i16(t.a(0x84) + 2 * k32), (k as i64 * 0x2AA + (rnd.rand() & 0x7F)) as i16, t.tables.abs_i16(t.a(0x78) + 2 * k32)]
        });
        Self { side, node, age: 0, arms, col: Colours::new(t, 10, false, 0x1000, 0x66, 1, 0), rgb: [0; 3] }
    }

    fn step(&mut self, t: &Tables, gte: &Gte, f: &Fighter) -> Option<Vec<Flat>> {
        if let Some(c) = self.col.step(t) {
            self.rgb = c[0][0];
        }
        self.age += 1;
        if self.age >= 0x28 {
            return None;
        }
        for a in &mut self.arms {
            a[0] = ((a[0] as i32 * 0xFAE) >> 12) as i16;
            a[2] = ((a[2] as i32 * 0xF33) >> 12) as i16;
            a[1] = a[1].wrapping_add(a[2]);
        }
        let mut quads = Vec::new();
        if let Some(q) = gte.rtps(f.node(self.node).map(|v| v as i32)) {
            let z = if (0..0x7EF).contains(&(q.otz - 0x33)) { q.otz - 0x32 } else { 0x7EF };
            // (raw screen units, ×2 on the 640×480 display)
            let at = |a: i64| (q.sx + 2.0 * (t.rcos(a) >> 3) as f32, q.sy + 2.0 * (t.rsin(a) >> 3) as f32);
            for a in &self.arms {
                let (p0, p1) = (at(a[1] as i64), at(a[1] as i64 + a[0] as i64));
                quads.push(([(q.sx, q.sy), (q.sx, q.sy), p0, p1], [self.rgb; 4], z, ADD));
            }
        }
        Some(quads)
    }
}

/// Type 0x05 (a beast's dark aura): a hexagon round the node (the inner ring of
/// radius 35 + j, the outer 55 + j, j = byte table & 0x3F a frame, projected; from a random angle, 0x2AC apart)
/// subtracted from the screen (DR_MODE 0xD5): (0xF4, 0xF4, 0x80) inside, (0x20, 0x20, 0) between the rings, in three
/// frames of four (the byte table & 3), at a depth jittered by ±0x32; and three spikes ((0, ρ), (ρ / 8,
/// 1.5ρ), (−ρ / 8, 1.5ρ), (0, 2ρ), 0x514 apart) whose radius runs from 400 down by 40 a frame, then again at a new
/// random angle; coloured by a colour slot (record 14, fade 0x1000 by 0x66); 40 frames.
struct Aura {
    side: usize,
    node: i16,
    /// d8a / d8c: the spikes' radius and angle; d9e: the frames left
    spike: i16,
    angle: i64,
    left: i16,
    col: Colours,
    rgb: [[u8; 3]; 4],
}

impl Aura {
    fn new(t: &Tables, side: usize, node: i16) -> Self {
        let col = Colours::new(t, 14, false, 0x1000, 0x66, 1, 0);
        Self { side, node, spike: 400, angle: 0, left: 40, col, rgb: [[0; 3]; 4] }
    }

    fn step(&mut self, t: &Tables, gte: &Gte, f: &Fighter, rnd: &mut Rand) -> Option<Vec<Flat>> {
        if let Some(c) = self.col.step(t) {
            self.rgb = c[0];
        }
        self.spike -= 0x28;
        if self.spike < 0 {
            self.spike = 400;
            self.angle = rnd.rand();
        }
        let j = (rnd.trand(t) & 0x3F) as i16;
        let r = [35 + j, 55 + j];
        self.left -= 1;
        match self.left {
            1 => return Some(Vec::new()),
            0 => return None,
            _ => {}
        }
        let Some(q) = gte.rtps(f.node(self.node).map(|v| v as i32)) else { return Some(Vec::new()) };
        let z = (if (0..0x7EF).contains(&(q.otz - 0x33)) { q.otz - 0x32 } else { 0x7EF }) + rnd.sym(0x32) as i32;
        if !(1..0x800).contains(&z) {
            return Some(Vec::new());
        }
        let proj = |v: i16| (v as f32 * gte.h / (4 * z) as f32).trunc();
        let a0 = rnd.rand();
        let ring: Vec<Vec<(f32, f32)>> = r
            .iter()
            .map(|&rv| {
                let rr = proj(rv);
                (0..7)
                    .map(|k| a0 + (k % 6) as i64 * 0x2AC)
                    .map(|a| (q.sx + (rr * t.rcos(a) as f32 / 4096.0).floor(), q.sy + (rr * t.rsin(a) as f32 / 4096.0).floor()))
                    .collect()
            })
            .collect();
        let mut quads = Vec::new();
        if rnd.trand(t) & 3 != 0 {
            let c = (q.sx, q.sy);
            for k in 0..6 {
                quads.push(([c, c, ring[0][k], ring[0][k + 1]], [[0xF4, 0xF4, 0x80]; 4], z, SUB));
            }
            for k in 0..6 {
                quads.push(([ring[1][k], ring[1][k + 1], ring[0][k], ring[0][k + 1]], [[0x20, 0x20, 0]; 4], z, SUB));
            }
        }
        let rho = proj(self.spike);
        let w = (rho / 8.0).floor();
        let shape = [(0.0, rho), (w, rho + (rho / 2.0).floor()), (-w, rho + (rho / 2.0).floor()), (0.0, 2.0 * rho)];
        let mut a = self.angle;
        for _ in 0..3 {
            let (cs, sn) = (t.rcos(a) as f32 / 4096.0, t.rsin(a) as f32 / 4096.0);
            quads.push((shape.map(|(x, y)| (q.sx + x * cs - y * sn, q.sy + x * sn + y * cs)), [self.rgb[0]; 4], z, ADD));
            a += 0x514;
        }
        Some(quads)
    }
}

/// A quad drawn opaque (its semi-transparency bit cleared) among the tasks' quads.
const OPAQUE: u16 = u16::MAX;

/// The rings of the whirl (`fx.data_a` +0x48: scale, height, angle, its speed).
const WHIRL_RINGS: u32 = 0x48;

/// A 3×3 GTE matrix product (each element's sum of products >> 12, as MulMatrix / the GTE).
fn mat_mul(a: &[[i64; 3]; 3], b: &[[i64; 3]; 3]) -> [[i64; 3]; 3] {
    std::array::from_fn(|i| std::array::from_fn(|j| s16((a[i][0] * b[0][j] + a[i][1] * b[1][j] + a[i][2] * b[2][j]) >> 12)))
}

/// Type 0x0E (a beast's whirl): a blade (a lens of six points in a profile turned
/// by the step angle, the middle two lifted ± the height) swept round the node by six rings (two draws of three: each
/// its scale, height and spinning angle), all turned by Ry(yaw + camera's + 0x142) · Rx(tilt); a thin opaque line from
/// the centre; coloured by a colour slot of three groups (record 1, after 15 frames fading 0xDAC over the rest).
struct Whirl {
    side: usize,
    node: i16,
    /// d80.. (then the parameters): radius, height, step angle, tilt, its speed, yaw, its speed; d9a the
    /// radius' speed, d9e the frames left, da0 / da2 the line's angle and spread
    d: [i16; 7],
    speed: i16,
    left: i16,
    line: (i64, i16),
    rings: [[i16; 4]; 6],
    col: Colours,
    rgb: Vec<[[u8; 3]; 4]>,
}

impl Whirl {
    /// The init (tilt >> 8, speed, frames, tilt speed).
    fn new(t: &Tables, side: usize, node: i16, a0: i32, a1: i32, a2: i32, a3: i32, rnd: &mut Rand) -> Self {
        let mut d: [i16; 7] = std::array::from_fn(|k| t.tables.abs_i16(t.a(0x38) + 2 * k as u32));
        d[3] = (a0 << 8) as i16;
        d[4] = a3 as i16;
        d[6] = (3 * a3) as i16;
        let rings = std::array::from_fn(|r| std::array::from_fn(|k| t.tables.abs_i16(t.a(WHIRL_RINGS) + 8 * r as u32 + 2 * k as u32)));
        let line = (rnd.rand() as i16 as i64, (rnd.trand(t) & 0x3F) as i16);
        let step = 0xDAC / (a2 - 15).max(1);
        let col = Colours::new(t, 1, false, 0xDAC, step as i16, 3, 15);
        Self { side, node, d, speed: a1 as i16, left: a2 as i16, line, rings, col, rgb: Vec::new() }
    }

    fn step(&mut self, t: &Tables, gte: &Gte, f: &Fighter, yaw: i32, rnd: &mut Rand) -> Option<Vec<Flat>> {
        if let Some(c) = self.col.step(t) {
            self.rgb = c;
        }
        self.left -= 1;
        if self.left == 0 {
            return None;
        }
        let d = &mut self.d;
        d[0] = d[0].wrapping_add(self.speed);
        if self.left < 10 {
            d[1] -= 2;
        }
        d[2] = d[2].wrapping_add(10);
        d[3] = d[3].wrapping_add(d[4]);
        d[5] = d[5].wrapping_add(d[6]);
        self.line.1 -= 6;
        if self.line.1 <= 0 {
            self.line = (rnd.rand() as i16 as i64, 0x20);
        }
        self.speed = ((self.speed as i32 * 0xD99) >> 12) as i16;
        for r in &mut self.rings {
            r[2] = r[2].wrapping_add(r[3]);
            r[3] = ((r[3] as i32 * 0xF0A) >> 12) as i16;
        }
        // the profile: the radius at 0, 1, 2, 3 steps, the middle two at ± the height
        let (rad, h) = (d[0] as i64, d[1]);
        let mut prof = Vec::with_capacity(6);
        for k in 0..4i64 {
            let a = k * d[2] as i64;
            let p = [s16((rad * t.rcos(a)) >> 12) as i16, 0, s16((rad * t.rsin(a)) >> 12) as i16];
            if k == 1 || k == 2 {
                prof.push([p[0], h, p[2]]);
                prof.push([p[0], h.wrapping_neg(), p[2]]);
            } else {
                prof.push(p);
            }
        }
        // Ry(b) · Rx(a)
        let (a, b) = (d[3] as i64, (d[5] as i64 + yaw as i64 + 0x142) & 0xFFFF);
        let (sa, ca, sb, cb) = (t.rsin(a), t.rcos(a), t.rsin(b), t.rcos(b));
        let m = [[cb, (sb * sa) >> 12, (sb * ca) >> 12], [0, ca, -sa], [-sb, (cb * sa) >> 12, (cb * ca) >> 12]].map(|r| r.map(s16));
        let pos = f.node(self.node);
        let mut quads = Vec::new();
        for set in 0..2 {
            let mut rows = Vec::new();
            for ring in &self.rings[3 * set..3 * set + 3] {
                let sc = ring[0] as i64;
                let (ry_s, ry_c) = (t.rsin(ring[2] as i64), t.rcos(ring[2] as i64));
                let n = mat_mul(&[[sc, 0, 0], [0, sc, 0], [0, 0, sc]], &[[ry_c, 0, ry_s], [0, 4096, 0], [-ry_s, 0, ry_c]]);
                let r = mat_mul(&m, &n);
                let pts: Vec<Option<crate::output::gte::Proj>> = prof
                    .iter()
                    .map(|p| {
                        let v = [p[0] as i64, p[1].wrapping_add(ring[1]) as i64, p[2] as i64];
                        let w: [i32; 3] =
                            std::array::from_fn(|i| pos[i].wrapping_add(s16((r[i][0] * v[0] + r[i][1] * v[1] + r[i][2] * v[2]) >> 12) as i16) as i32);
                        gte.rtps(w)
                    })
                    .collect();
                rows.push(pts);
            }
            // the OT grouping by threes (their mean otz); the rows' OT entries those of the first three groups
            let groups: Vec<i32> = (0..3)
                .map(|g| {
                    let z = (0..3).map(|k| rows[g / 2][3 * (g % 2) + k].as_ref().map_or(0, |q| q.otz)).sum::<i32>() / 3;
                    if (1..0x7F0).contains(&z) { z } else { 0x7EF }
                })
                .collect();
            for (ri, row) in rows.iter().enumerate() {
                let s: Vec<&Option<crate::output::gte::Proj>> = [0, 0, 1, 2, 3, 4, 5, 5].iter().map(|&k| &row[k]).collect();
                for k in 0..3 {
                    if let (Some(p0), Some(p1), Some(p2), Some(p3)) = (s[2 * k], s[2 * k + 1], s[2 * k + 2], s[2 * k + 3]) {
                        let rgb = self.rgb.get(k).copied().unwrap_or([[0; 3]; 4]);
                        quads.push(([(p0.sx, p0.sy), (p1.sx, p1.sy), (p2.sx, p2.sy), (p3.sx, p3.sy)], rgb, groups[ri], ADD));
                    }
                }
            }
        }
        // the line from the centre (raw screen units, ×2 on the 640×480 display), opaque at OT entry 10
        if let Some(c) = gte.rtps(pos.map(|v| v as i32)) {
            let (a, sp) = self.line;
            let end = |ang: i64| (c.sx + 2.0 * (t.rcos(ang) >> 3) as f32, c.sy + 2.0 * (t.rsin(ang) >> 3) as f32);
            let rgb = self.rgb.first().copied().unwrap_or([[0; 3]; 4]);
            quads.push(([(c.sx, c.sy), (c.sx, c.sy), end(a + sp as i64), end(a - sp as i64)], rgb, 10, OPAQUE));
        }
        Some(quads)
    }
}

/// Type 0x17 (a globe round a node): four latitude rings of six points (the
/// first and last 0x155 turned) at `y + r · sin a` and radius `r · cos a`, `a` from the spin on by 200 a ring; the spin
/// on by a speed growing by 1 a frame (by 0x23 from the 11th); three bands of gouraud quads (a fan, a band, a fan)
/// coloured by a colour slot of three groups (record 19); 18 frames.
struct Globe {
    side: usize,
    node: i16,
    /// d80..: the radius, the spin, its speed, the rings' step, the frames; d9c: the frames run
    d: [i16; 5],
    age: i16,
    col: Colours,
    rgb: Vec<[[u8; 3]; 4]>,
}

impl Globe {
    fn new(t: &Tables, side: usize, node: i16) -> Self {
        let d = std::array::from_fn(|k| t.tables.abs_i16(t.a(0x90) + 2 * k as u32));
        Self { side, node, d, age: 0, col: Colours::new(t, 19, true, 0x1000, 1, 3, 0), rgb: Vec::new() }
    }

    fn step(&mut self, t: &Tables, gte: &Gte, f: &Fighter) -> Option<Vec<Flat>> {
        if let Some(c) = self.col.step(t) {
            self.rgb = c;
        }
        self.age += 1;
        if self.age >= self.d[4] {
            return None;
        }
        self.d[2] = self.d[2].wrapping_add(if self.age >= 11 { 0x23 } else { 1 });
        self.d[1] = self.d[1].wrapping_add(self.d[2]);
        let pos = f.node(self.node);
        let mut a = self.d[1] as i64;
        let mut rows = Vec::new();
        for ring in 0..4 {
            let r = self.d[0] as i64;
            let y = pos[1].wrapping_add(((r * t.rsin(a)) >> 12) as i16);
            let h = (r * t.rcos(a)) >> 12;
            let mut b = if ring == 0 || ring == 3 { 0x155 } else { 0 };
            let mut row = Vec::new();
            for _ in 0..6 {
                let x = pos[0].wrapping_add(((h * t.rcos(b)) >> 12) as i16);
                let z = pos[2].wrapping_add(((h * t.rsin(b)) >> 12) as i16);
                row.push(gte.rtps([x as i32, y as i32, z as i32]).map(|q| (q.sx, q.sy, q.otz)));
                b += 0x2AA;
            }
            row.push(row[0]);
            rows.push(row);
            a += self.d[3] as i64;
        }
        // (the OT entry its first group's, the first ring's first three points' mean otz)
        let z = (0..3).map(|k| rows[0][k].map_or(0, |q| q.2)).sum::<i32>() / 3;
        let z = if (1..0x7F0).contains(&z) { z } else { 0x7EF };
        let mut quads = Vec::new();
        for (band, (a, b, next)) in [(0, 1, false), (1, 2, true), (3, 2, false)].into_iter().enumerate() {
            let rgb = self.rgb.get(band).copied().unwrap_or([[0; 3]; 4]);
            for k in 0..6 {
                if let (Some(p0), Some(p1), Some(p2), Some(p3)) = (rows[a][k], rows[a][k + next as usize], rows[b][k], rows[b][k + 1]) {
                    quads.push(([(p0.0, p0.1), (p1.0, p1.1), (p2.0, p2.1), (p3.0, p3.1)], rgb, z, ADD));
                }
            }
        }
        Some(quads)
    }
}

/// Type 0x08 (a seven-spike star): spikes 0x249 apart, each the diamond (0, r0),
/// (±w, (r0 + r1) / 2), (0, r1) projected (`v · H / (4 · z)`), turning by a speed; r0, r1 and the speed grow by
/// 0x11EB, 0x130A and 0x1599 / 0x1000 a frame; coloured by a colour slot (record 6, fade 0x1000 by 0x198); 17 frames.
struct Star7 {
    side: usize,
    node: i16,
    /// d80 / d82 / d84: r0, r1, the speed; d86: w; d8a: the angle; d9e: the frames left
    r: [i16; 3],
    w: i16,
    angle: i16,
    left: i16,
    col: Colours,
    rgb: [[u8; 3]; 4],
}

impl Star7 {
    fn new(t: &Tables, side: usize, node: i16) -> Self {
        Self { side, node, r: [0x1E, 0x12C, 0xF], w: 0x23, angle: 0, left: 0x11, col: Colours::new(t, 6, false, 0x1000, 0x198, 1, 0), rgb: [[0; 3]; 4] }
    }

    fn step(&mut self, t: &Tables, gte: &Gte, f: &Fighter) -> Option<Vec<Flat>> {
        if let Some(c) = self.col.step(t) {
            self.rgb = c[0];
        }
        self.left -= 1;
        if self.left <= 0 {
            return None;
        }
        for (v, g) in self.r.iter_mut().zip([0x11EB, 0x130A, 0x1599]) {
            *v = ((*v as i32 * g) >> 12) as i16;
        }
        self.angle = self.angle.wrapping_add(self.r[2]);
        let q = gte.rtps(f.node(self.node).map(|v| v as i32))?;
        let z = if (0..0x7EF).contains(&(q.otz - 0x33)) { q.otz - 0x32 } else { 0x7EF };
        let proj = |v: i32| (v as f32 * gte.h / (4 * z) as f32).trunc();
        let mid = (self.r[0] as i32 + self.r[1] as i32) / 2;
        let shape = [(0.0, proj(self.r[0] as i32)), (proj(self.w as i32), proj(mid)), (proj(-(self.w as i32)), proj(mid)), (0.0, proj(self.r[1] as i32))];
        let mut a = self.angle as i64;
        let mut quads = Vec::new();
        for _ in 0..7 {
            let (c, s) = (t.rcos(a) as f32 / 4096.0, t.rsin(a) as f32 / 4096.0);
            quads.push((shape.map(|(x, y)| (q.sx + x * c - y * s, q.sy + x * s + y * c)), [self.rgb[0]; 4], z, ADD));
            a += 0x249;
        }
        Some(quads)
    }
}

/// Type 0x01 (a fan of beams): ten thin triangles from the node out to a radius
/// of 0x1000 >> 3 screen units (×2 on the display), each from its angle across its width (0x20 + the byte
/// table & 0x7F), the angles turning by their speeds, the widths 2 down a frame; coloured by a colour
/// slot of two groups (record 2, fade 0x1000 by 0xB2); 23 frames.
struct Beams {
    side: usize,
    node: i16,
    angle: [i16; 10],
    width: [u8; 10],
    left: i16,
    col: Colours,
    rgb: Vec<[[u8; 3]; 4]>,
}

impl Beams {
    fn new(t: &Tables, side: usize, node: i16, rnd: &mut Rand) -> Self {
        let angle = std::array::from_fn(|k| t.tables.abs_i16(t.b(0x1690) + 2 * k as u32));
        let width = std::array::from_fn(|_| ((rnd.trand(t) & 0x7F) + 0x20) as u8);
        Self { side, node, angle, width, left: 0x17, col: Colours::new(t, 2, false, 0x1000, 0xB2, 2, 0), rgb: Vec::new() }
    }

    fn step(&mut self, t: &Tables, gte: &Gte, f: &Fighter) -> Option<Vec<Flat>> {
        if let Some(c) = self.col.step(t) {
            self.rgb = c;
        }
        self.left -= 1;
        if self.left <= 0 {
            return None;
        }
        for k in 0..10 {
            self.angle[k] = self.angle[k].wrapping_add(t.tables.abs_u8(t.b(0x1684) + k as u32) as i8 as i16);
            self.width[k] = self.width[k].saturating_sub(2);
        }
        let q = gte.rtps(f.node(self.node).map(|v| v as i32))?;
        let z = if (0..0x7EF).contains(&(q.otz - 0x33)) { q.otz - 0x32 } else { 0x7EF };
        let at = |a: i64| (q.sx + 2.0 * (t.rcos(a) >> 3) as f32, q.sy + 2.0 * (t.rsin(a) >> 3) as f32);
        let c = (q.sx, q.sy);
        Some(
            (0..10)
                .map(|k| {
                    let a = self.angle[k] as i64;
                    let rgb = self.rgb.get(k % 2).copied().unwrap_or([[0; 3]; 4]);
                    ([c, c, at(a), at(a + self.width[k] as i64)], rgb, z, ADD)
                })
                .collect(),
        )
    }
}

/// Type 0x0A (a widening band): a circle of 12 points (radius 300: `cos · 75 >>
/// 10`) scaled by two growing factors (0x830 and 0x254, by 0x5DC a frame) at the node's height plus `dy` and 50 more,
/// 12 flat quads between them in a grey of 0xC0 down by 9 a frame; 20 frames.
struct Band {
    side: usize,
    node: i16,
    circle: [[i16; 2]; 12],
    scale: [i16; 2],
    dy: i16,
    grey: i16,
    left: i16,
}

impl Band {
    fn new(t: &Tables, side: usize, node: i16, dy: i32) -> Self {
        let circle = std::array::from_fn(|k| {
            let a = k as i64 * 0x156;
            [(((t.rcos(a) * 75) as u32) >> 10) as i16, (((t.rsin(a) * 75) as u32) >> 10) as i16]
        });
        Self { side, node, circle, scale: [0x830, 0x254], dy: dy as i16, grey: 0xC0, left: 20 }
    }

    fn step(&mut self, gte: &Gte, f: &Fighter) -> Option<Vec<Flat>> {
        self.left -= 1;
        if self.left == 0 {
            return None;
        }
        for s in &mut self.scale {
            *s = s.wrapping_add(0x5DC);
        }
        self.grey = (self.grey - 9).max(0);
        let pos = f.node(self.node);
        let mut p = Vec::with_capacity(24);
        let mut y = pos[1].wrapping_add(self.dy);
        for &sc in &self.scale {
            for c in &self.circle {
                let x = pos[0].wrapping_add(((c[0] as i64 * sc as i64) >> 12) as i16);
                let z = pos[2].wrapping_add(((c[1] as i64 * sc as i64) >> 12) as i16);
                p.push(gte.rtps([x as i32, y as i32, z as i32]).map(|q| (q.sx, q.sy, q.otz)));
            }
            y = y.wrapping_add(0x32);
        }
        let otz = |g: usize| {
            let z = (0..4).map(|k| p[4 * g + k].map_or(0, |q| q.2)).sum::<i32>() / 4;
            if (1..0x7F0).contains(&z) { z } else { 0x7EF }
        };
        let g = self.grey as u8;
        let mut quads = Vec::new();
        for row in 0..3 {
            for k in 0..4 {
                let i = 4 * row + k;
                if let (Some(a), Some(b), Some(c), Some(d)) = (p[i], p[(i + 1) % 12], p[12 + i], p[12 + (i + 1) % 12]) {
                    quads.push(([(a.0, a.1), (b.0, b.1), (c.0, c.1), (d.0, d.1)], [[g; 3]; 4], otz(row), ADD));
                }
            }
        }
        Some(quads)
    }
}

/// Type 0x0B (a tornado): four layers (height, angle, the radius'
/// speed, radius) of a blade of 12 points (seven at `cos / 8`, `sin / 8` 0x14D apart from `a1 << 8`, the middle five
/// lifted ± 50), each scaled across by its radius plus `R · sin(3 · (h − |y + h|))` (h = the climb / 2) and up by
/// `Ry`, turned by its angle; the layers' angles on by a speed, heights down by a fall (back to 0 below the climb), the
/// speeds damped by 1925 / 2048 in the last frames; 6 flat quads a layer in three colour groups (record 12, fade 0x1000
/// by 0x111 after 20 frames, variant 1 16); 40 frames. Variant v (12 bytes each): R, Ry, the spin, the fall, the
/// climb and its speed.
struct Tornado {
    side: usize,
    node: i16,
    d: [i16; 6],
    layers: [[i16; 4]; 4],
    prof: [[i16; 3]; 12],
    left: i16,
    col: Colours,
    rgb: Vec<[[u8; 3]; 4]>,
}

impl Tornado {
    fn new(t: &Tables, side: usize, node: i16, v: i32, a1: i32) -> Self {
        let d = std::array::from_fn(|k| t.tables.abs_i16(t.a(0x20) + 12 * (v as u32 & 1) + 2 * k as u32));
        let layers = std::array::from_fn(|l| std::array::from_fn(|k| t.tables.abs_i16(t.a(0) + 8 * l as u32 + 2 * k as u32)));
        let mut prof = Vec::with_capacity(12);
        let mut a = (a1 << 8) as i64;
        for k in 0..7 {
            let p = [((t.rcos(a) as u32) >> 3) as i16, 0, ((t.rsin(a) as u32) >> 3) as i16];
            if (1..=5).contains(&k) {
                prof.push([p[0], 0x32, p[2]]);
                prof.push([p[0], -0x32, p[2]]);
            } else {
                prof.push(p);
            }
            a += 0x14D;
        }
        let col = Colours::new(t, 12, false, 0x1000, 0x111, 3, (0x14 - 4 * (v & 1)) as u8);
        Self { side, node, d, layers, prof: std::array::from_fn(|k| prof[k]), left: 0x28, col, rgb: Vec::new() }
    }

    fn step(&mut self, t: &Tables, gte: &Gte, f: &Fighter) -> Option<Vec<Flat>> {
        if let Some(c) = self.col.step(t) {
            self.rgb = c;
        }
        self.left -= 1;
        if self.left == 0 {
            return None;
        }
        let d = &mut self.d;
        for l in &mut self.layers {
            l[1] = l[1].wrapping_add(d[2]);
            l[0] = l[0].wrapping_sub(d[3]);
            if l[0] < d[4].wrapping_neg() {
                l[0] = 0;
            }
            l[3] = l[3].wrapping_add(l[2]);
        }
        d[4] = d[4].wrapping_add(d[5]);
        if self.left < 0x19 {
            d[2] = ((d[2] as i32 * 1925) >> 11) as i16;
        }
        if self.left < 0x14 {
            d[3] = ((d[3] as i32 * 1925) >> 11) as i16;
        }
        let pos = f.node(self.node);
        let mut quads = Vec::new();
        for (li, l) in self.layers.iter().enumerate() {
            let h = (d[4] >> 1) as i64;
            let a = h - (l[0] as i64 + h).abs();
            let s = s16((t.rsin(3 * a) * d[0] as i64) >> 12);
            let sc = s16(l[3] as i64 + s);
            let m = [[sc, 0, 0], [0, d[1] as i64, 0], [0, 0, sc]];
            let (ys, yc) = (t.rsin(l[1] as i64), t.rcos(l[1] as i64));
            let r = mat_mul(&[[yc, 0, ys], [0, 4096, 0], [-ys, 0, yc]], &m);
            let p: Vec<Option<(f32, f32, i32)>> = self
                .prof
                .iter()
                .map(|v| {
                    let w: [i64; 3] = std::array::from_fn(|i| s16((r[i][0] * v[0] as i64 + r[i][1] * v[1] as i64 + r[i][2] * v[2] as i64) >> 12));
                    let x = pos[0].wrapping_add(w[0] as i16);
                    let y = pos[1].wrapping_add(w[1] as i16).wrapping_add(l[0]).wrapping_add(0xFF);
                    let z = pos[2].wrapping_add(w[2] as i16);
                    gte.rtps([x as i32, y as i32, z as i32]).map(|q| (q.sx, q.sy, q.otz))
                })
                .collect();
            // the OT grouping by fours (rows' OT entries: groups 0..2)
            let otz = |g: usize| {
                let z = (0..4).map(|k| p[4 * g + k].map_or(0, |q| q.2)).sum::<i32>() / 4;
                if (1..0x7F0).contains(&z) { z } else { 0x7EF }
            };
            let slot = |k: usize| p[(k.max(1) - 1).min(11)];
            for r in 0..3 {
                for k in 0..2 {
                    let b = 4 * r + 2 * k;
                    if let (Some(p0), Some(p1), Some(p2), Some(p3)) = (slot(b), slot(b + 1), slot(b + 2), slot(b + 3)) {
                        let rgb = self.rgb.get((6 * li + 2 * r + k) % 3).copied().unwrap_or([[0; 3]; 4]);
                        quads.push(([(p0.0, p0.1), (p1.0, p1.1), (p2.0, p2.1), (p3.0, p3.1)], rgb, otz(r), ADD));
                    }
                }
            }
        }
        Some(quads)
    }
}

/// Type 0x1B (a flame): a fan of six gouraud triangles from the node to the
/// projected outline (0, r / 2), (w1, r / 3), (w2, −r / 3), (0, −r), (−w2, −r / 3), (−w1, r / 3), yellow from
/// (c0, c0, 0) at the centre to (c1, c1, 0) outside, at OT entry 10; the radius growing slowly for 15
/// frames, then by 8 · t, the widths by their speeds (down 4 a frame, 14 after the 15th; the first from below 60),
/// at least 15; the colours down from 0xE0 by 12 and from 0x20 by 1 after the 15th; over at radius 0x5DD.
struct Flame {
    side: usize,
    node: i16,
    /// d80 / d82 / d84: r, w1, w2; d94: the frame (from −15); d96 / d98: the widths' speeds; d9e / da0: c0, c1
    r: i16,
    w: [i16; 2],
    t: i16,
    speed: [i16; 2],
    c: [i16; 2],
    done: bool,
}

impl Flame {
    fn new(side: usize, node: i16) -> Self {
        Self { side, node, r: 0, w: [0; 2], t: -0xF, speed: [0x50, 0x3C], c: [0xE0, 0x20], done: false }
    }

    fn step(&mut self, gte: &Gte, f: &Fighter) -> Option<Vec<Flat>> {
        if self.done {
            return None;
        }
        let q = gte.rtps(f.node(self.node).map(|v| v as i32));
        self.t += 1;
        for s in &mut self.speed {
            *s -= 4;
        }
        if self.t > 0 {
            self.r = self.r.wrapping_add(self.t.abs() << 3);
            for s in &mut self.speed {
                *s -= 10;
            }
            self.c[0] = (self.c[0] - 12).max(0);
            self.c[1] = (self.c[1] - 1).max(0);
        } else {
            self.r = self.r.wrapping_add(self.t.abs() >> 2);
        }
        if self.speed[0] < 0x3C {
            self.w[0] = (self.w[0] + self.speed[0]).max(0xF);
        }
        self.w[1] = (self.w[1] + self.speed[1]).max(0xF);
        self.done = self.r >= 0x5DD;
        let q = q?;
        let z = if (0..0x7EF).contains(&(q.otz - 0x33)) { q.otz - 0x32 } else { 0x7EF };
        let proj = |v: i16| (v as i32 * gte.h as i32) / (4 * z);
        let (a, c, e) = (proj(self.r), proj(self.w[0]), proj(self.w[1]));
        let pts = [(0, a / 2), (c, a / 3), (e, -a / 3), (0, -a), (-e, -a / 3), (-c, a / 3), (0, a / 2)];
        let at = |(x, y): (i32, i32)| (q.sx + x as f32, q.sy + y as f32);
        let (c0, c1) = (self.c[0] as u8, self.c[1] as u8);
        let rgb = [[c0, c0, 0], [c0, c0, 0], [c1, c1, 0], [c1, c1, 0]];
        Some((0..6).map(|k| ([(q.sx, q.sy), (q.sx, q.sy), at(pts[k]), at(pts[k + 1])], rgb, 10, ADD)).collect())
    }
}

/// A textured quad of the beast form's lightning (POLY_FT4 on EFFECT.DAT's page (384, 0), tpage 0x26, CLUT 0x7808):
/// its OT entry, screen points, texel corners, whether semi-transparent.
type Sprite = (i32, [(f32, f32); 4], [[u8; 2]; 4], bool);

/// The beast form's lightning (the effect scripts' command 3: 4 slots of 0x3F4 bytes, its
/// path in 4 slots of 0x25C; a frame after the effect scripts): a path of points
/// along a chain of the fighter's nodes (the chain table, 16 bytes a chain: its length, its nodes), two a node, each the node's
/// position less its world matrix times a zigzag offset taken when the path was made (radius from the radius table by chr and
/// node, less the human form's amount by chr; 0x200 turns apart from a random angle; up by half the way to the next
/// node); every 3 frames a bolt starts (4 slots), a sprite between two of five points from the path's head (the head
/// moving on once four run), its 12 frames of texels (from the bolt texel table, 64 bytes a colour set), the bolts of one
/// parity drawn a frame (opaque the frame one starts); 12 frames after the head passes the path's end it is over.
struct Crackle {
    side: usize,
    chain: u8,
    form: u8,
    /// the path: the nodes of its points and their offsets
    nodes: Vec<i16>,
    off: Vec<[i16; 3]>,
    /// +0x3D8: the sprites' size; +0x3EE: the path's length · 2; +0x3EF: its head; +0x3D4: the bolts started; +0x3D6:
    /// the frame in three; +0x3F0: the newest bolt; +0x3EC: the end's countdown
    size: i16,
    len: u8,
    head: u8,
    started: i16,
    tick: i16,
    newest: u8,
    end: u8,
    /// the bolts: running (+0x3DC), their point (+0x3E0), their frame (+0x3E8)
    on: [bool; 4],
    at: [u8; 4],
    age: [u8; 4],
    /// the texels of the 12 frames (the colour set's)
    uv: [[[u8; 2]; 4]; 12],
}

/// The chains' records (`fx.data_b` +0xBAC, 16 bytes a chain).
const CHAINS: u32 = 0xBAC;

impl Crackle {
    /// The path's nodes and offsets on the fighter's pose now.
    fn path(t: &Tables, f: &Fighter, chain: u8, thinner: i32, rnd: &mut Rand) -> (Vec<i16>, Vec<[i16; 3]>, u8) {
        let c = t.b(CHAINS) + 16 * chain as u32;
        let n = t.tables.abs_u8(c) as usize;
        let node = |k: usize| t.tables.abs_u8(c + 1 + k as u32) as i16;
        let mut ang = rnd.rand();
        let (mut nodes, mut off) = (Vec::new(), Vec::new());
        for s in 0..n + 3 {
            let (a, b) = (node(s), node(s + 1));
            let (pa, pb) = (f.node(a), f.node(b));
            let d2: i64 = (0..3).map(|i| (pa[i] as i64 - pb[i] as i64).pow(2)).sum();
            let dist = (d2 as f64).sqrt() as i64;
            let r = t.tables.abs_u8(t.b(CLAW_SIZES) + f.chr as u32 * 17 + a as u32) as i64 - thinner as i64;
            let wm = f.md.get(a as usize).map_or([[0; 3]; 3], |d| d.wm);
            let mut y = 0i64;
            for _ in 0..2 {
                let v = [(r * t.rcos(ang)) >> 12, y >> 1, (r * t.rsin(ang)) >> 12].map(|x| x as i16 as i64);
                y += dist;
                ang += 0x200;
                let w: [i16; 3] = std::array::from_fn(|i| (-((wm[i][0] as i64 * v[0] + wm[i][1] as i64 * v[1] + wm[i][2] as i64 * v[2]) >> 12)) as i16);
                nodes.push(a);
                off.push(w);
            }
        }
        (nodes, off, (2 * n) as u8)
    }

    /// The lightning's start (MD, chr, chain, colour set + 0x80 · beast) from command 3's two bytes.
    fn new(t: &Tables, side: usize, f: &Fighter, chain: u8, set: u8, rnd: &mut Rand) -> Self {
        let beast = f.form & 1 != 0;
        let thinner = if beast { 0 } else { t.tables.abs_u8(t.b(RIBBON_BEAST) + f.chr as u32) as i32 };
        let (nodes, off, len) = Self::path(t, f, chain, thinner, rnd);
        let size = (t.tables.abs_u8(t.b(CLAW_SIZES) + f.chr as u32 * 17 + 0x10) as i32 + 0x14 - thinner) as i16;
        let uv = std::array::from_fn(|k| {
            let e = t.b(0x10CC) + 8 * t.tables.abs_u8(t.b(0x428) + k as u32) as u32 + 64 * (set as u32 & 0x7F);
            std::array::from_fn(|j| [t.tables.abs_u8(e + 2 * j as u32), t.tables.abs_u8(e + 2 * j as u32 + 1)])
        });
        Self {
            side,
            chain,
            form: f.form & 1,
            nodes,
            off,
            size,
            len,
            head: 0,
            started: 0,
            tick: 3,
            newest: 3,
            end: 0,
            on: [false; 4],
            at: [0; 4],
            age: [0; 4],
            uv,
        }
    }

    /// A frame; false once over.
    fn step(&mut self, t: &Tables, gte: &Gte, f: &Fighter, rnd: &mut Rand, out: &mut Vec<Sprite>) -> bool {
        if f.form & 1 != self.form {
            // the form changed: the path again on the new model (no thinning)
            self.form = f.form & 1;
            (self.nodes, self.off, self.len) = Self::path(t, f, self.chain, 0, rnd);
        }
        for k in 0..4 {
            if self.on[k] {
                self.age[k] += 1;
                if self.age[k] >= 12 {
                    self.on[k] = false;
                }
            }
        }
        let mut semi = true;
        if self.end == 0 {
            self.tick += 1;
            if self.tick >= 3 {
                self.tick = 0;
                self.newest = (self.newest + 1) % 4;
                semi = false;
                let c = self.newest as usize;
                if self.started >= 4 {
                    for a in &mut self.at {
                        *a = a.wrapping_sub(1);
                    }
                    self.at[c] = 3;
                    self.head += 1;
                } else {
                    self.at[c] = self.started as u8;
                }
                self.started += 1;
                (self.on[c], self.age[c]) = (true, 0);
            }
        }
        // five points from the head, projected (their otz 1..=0x7EF), their sizes and the angles between them
        let pts: Vec<Option<(f32, f32, i32)>> = (0..5)
            .map(|i| {
                let k = self.head as usize + i;
                let (Some(&n), Some(o)) = (self.nodes.get(k), self.off.get(k)) else { return None };
                let p = f.node(n);
                gte.rtps(std::array::from_fn(|j| p[j].wrapping_add(o[j]) as i32)).map(|q| (q.sx, q.sy, if (1..0x7F0).contains(&q.otz) { q.otz } else { 0x7EF }))
            })
            .collect();
        let size: Vec<f32> = pts.iter().map(|p| p.map_or(0.0, |p| (self.size as i32 * gte.h as i32 / (4 * p.2)) as f32)).collect();
        let mut k = (self.tick & 1) as usize;
        while k < 4 {
            let i = self.at[k] as usize;
            if self.on[k]
                && i < 4
                && let (Some(p0), Some(p1)) = (pts[i], pts[i + 1])
            {
                let a = angle((p1.0 - p0.0).round() as i32, (p1.1 - p0.1).round() as i32) as i64;
                let (c, s) = (t.rcos(a) as f32 / 4096.0, t.rsin(a) as f32 / 4096.0);
                let (v, w) = ((size[i] * c, size[i] * s), (size[i + 1] * c, size[i + 1] * s));
                let q = [(p1.0 + w.0, p1.1 + w.1), (p1.0 - w.0, p1.1 - w.1), (p0.0 + v.0, p0.1 + v.1), (p0.0 - v.0, p0.1 - v.1)];
                out.push((p0.2, q, self.uv[self.age[k] as usize], semi));
            }
            k += 2;
        }
        if self.end == 0 && (self.len as i32 - 4) < self.head as i32 {
            self.end = 12;
        }
        if self.end != 0 {
            self.end -= 1;
            if self.end == 0 {
                return false;
            }
        }
        true
    }
}

/// RotMatrixYXZ: Ry · Rx · Rz of the angles `v` (the RotMatrix table's sine and cosine).
fn rot_yxz(t: &Tables, v: [i16; 3]) -> [[i64; 3]; 3] {
    let [x, y, z] = v.map(|a| a as i64);
    let (sx, cx, sy, cy, sz, cz) = (t.rsin(x), t.rcos(x), t.rsin(y), t.rcos(y), t.rsin(z), t.rcos(z));
    let (sysx, cysx) = ((sy * sx) >> 12, (cy * sx) >> 12);
    [
        [((cy * cz) >> 12) + ((sysx * sz) >> 12), -((cy * sz) >> 12) + ((sysx * cz) >> 12), (sy * cx) >> 12],
        [(cx * sz) >> 12, (cx * cz) >> 12, -sx],
        [-((sy * cz) >> 12) + ((cysx * sz) >> 12), ((sy * sz) >> 12) + ((cysx * cz) >> 12), (cy * cx) >> 12],
    ]
}

/// The effect scripts' command 5 (at node 3's world translation, Fox's; stepped in the
/// lightning's slots, +0x3F1 = 2): four arms of three bolts from the node, the 13 points of the web's point table (the
/// node, then the arms' 9 + 3) turned by two angle sets stepping (0x70, −0xA4, 0x4C) and (−0x90, 0x9C, 0x4C) a frame
/// (the first nine by the first set, the last three by the second), around the node jittered by
/// 0x1F − rand & 0x3F; each arm drawn as three sprites from the node out, sized 40 / 64 / 128 / 0 ·
/// H / (4 · otz) at its four points, at the depth of each bolt's inner point; its texels one of two sets (the web texel table +
/// 72 · (stream byte & 1)) an arm and frame; on the frames whose counter & 3 is 0 arms 0 and 2 are not drawn; 40
/// frames.
struct Web {
    side: usize,
    /// the node it stands on (+0x3CC: its world translation)
    node: i16,
    /// +0x3EC: the frames left; +0x3ED: the counter (from 1)
    left: u8,
    count: u8,
    /// the path's +0x1A8 / +0x1B0 angle sets and its 13 points (+0xC8)
    a: [i16; 3],
    b: [i16; 3],
    pts: [[i16; 3]; 13],
}

impl Web {
    fn new(t: &Tables, side: usize, node: i16) -> Self {
        let pts = std::array::from_fn(|k| std::array::from_fn(|i| t.tables.abs_i16(t.a(0x210) + 8 * k as u32 + 2 * i as u32)));
        Self { side, node, left: 0x28, count: 1, a: [0; 3], b: [0; 3], pts }
    }

    /// A frame; false once over.
    fn step(&mut self, t: &Tables, gte: &Gte, f: &Fighter, rnd: &mut Rand, out: &mut Vec<Sprite>) -> bool {
        self.left -= 1;
        if self.left == 0 {
            return false;
        }
        self.count = self.count.wrapping_add(1);
        for (v, d) in [(&mut self.a, [0x70, -0xA4, 0x4C]), (&mut self.b, [-0x90, 0x9C, 0x4C])] {
            for i in 0..3 {
                v[i] = v[i].wrapping_add(d[i]);
            }
        }
        // an arm's texels: the web texel table + 72 · (stream A & 1), the same four corners for its three sprites
        let uv: [[[u8; 2]; 4]; 4] = std::array::from_fn(|_| {
            let e = t.b(0x10E4) + 72 * (rnd.trand(t) as u32 & 1);
            std::array::from_fn(|j| [t.tables.abs_u8(e + 2 * j as u32), t.tables.abs_u8(e + 2 * j as u32 + 1)])
        });
        let j = 0x1F - (rnd.rand() & 0x3F);
        let node = f.node(self.node);
        let p0: [i64; 3] = std::array::from_fn(|i| (node[i] as i64 + j) as i16 as i64);
        let mut pts = vec![p0];
        for (m, range) in [(rot_yxz(t, self.a), 1..10), (rot_yxz(t, self.b), 10..13)] {
            for k in range {
                let v = self.pts[k].map(|c| c as i64);
                pts.push(std::array::from_fn(|i| ((((m[i][0] * v[0] + m[i][1] * v[1] + m[i][2] * v[2]) >> 12) as i16 as i64 + p0[i]) as i16) as i64));
            }
        }
        let proj = |p: [i64; 3]| gte.rtps(p.map(|c| c as i32)).map(|q| (q.sx, q.sy, if (1..0x7F0).contains(&q.otz) { q.otz } else { 0x7EF }));
        let Some(c) = proj(pts[0]) else { return true };
        let h = gte.h as i32;
        for arm in 0..4 {
            let mut p = vec![c];
            let mut size = vec![(h * 40 / (4 * c.2)) as f32];
            for k in 0..3 {
                let Some(q) = proj(pts[1 + 3 * arm + k]) else { return true };
                p.push(q);
                size.push(if k == 2 { 0.0 } else { (0x40 * (k as i32 + 1) * h / (4 * q.2)) as f32 });
            }
            if arm % 2 == 0 && self.count & 3 == 0 {
                continue;
            }
            for k in 0..3 {
                let (p0, p1) = (p[k], p[k + 1]);
                let a = angle((p1.0 - p0.0).round() as i32, (p1.1 - p0.1).round() as i32) as i64;
                let (cs, sn) = (t.rcos(a) as f32 / 4096.0, t.rsin(a) as f32 / 4096.0);
                let (v, w) = ((size[k] * cs, size[k] * sn), (size[k + 1] * cs, size[k + 1] * sn));
                let q = [(p1.0 + w.0, p1.1 + w.1), (p1.0 - w.0, p1.1 - w.1), (p0.0 + v.0, p0.1 + v.1), (p0.0 - v.0, p0.1 - v.1)];
                out.push((p0.2, q, uv[arm], false));
            }
        }
        true
    }
}

/// An emitter on a fighter's node (5 slots of 0x34 bytes, stepped after the
/// tasks): its fighter and node (the slot holds the node's pointer and reads it every frame), kind (+0x2F) and
/// particle type (+0x30), the jitter flag (+0x31), the frames it runs (+0x0C), the floor (+0x1C), the frames run
/// (+0x10), the spawn counter and its interval (+0x14 / +0x2D), the jitter's range (+0x18).
#[derive(Clone)]
struct Attached {
    side: usize,
    node: usize,
    kind: u8,
    ty: usize,
    jitter: bool,
    count: i32,
    floor: i32,
    frames: i32,
    counter: i32,
    interval: i32,
    range: i64,
}

impl Attached {
    /// The start (node, kind | jitter << 8, frames, floor) (op 0x18): kinds 8 / 9 a particle of their type every 8
    /// frames, 10 / 11 four of type 8 / 9 on the floor at the end, 12 / 13 one of type 8 / 9 every 3 frames; the first
    /// spawn at once (the counter starts at the interval).
    fn new(side: usize, node: i32, k: i32, count: i32, floor: i32) -> Self {
        let kind = k as u8;
        let (ty, interval, range) = match kind {
            8 | 9 => (kind, 7, 0xB4),
            10 | 11 => (kind - 2, 0x10, 0x5A),
            12 | 13 => (kind - 4, 2, 0x5A),
            _ => (kind, 0, 0),
        };
        Self {
            side,
            node: node as u8 as usize,
            kind,
            ty: ty as usize,
            jitter: (k >> 8) as u8 != 0,
            count,
            floor,
            frames: 0,
            counter: interval,
            interval,
            range,
        }
    }
}

/// The effects of the imported fighters of a match on screen.
pub struct Br1Fx {
    /// the particle records (40 of 0x98 bytes), the free ones (a stack and its
    /// count +0x1BC0) and the live ones in list order (head +0x37C, last +0x1BB8)
    pool: Vec<Particle>,
    free: Vec<usize>,
    live: Vec<usize>,
    /// the emitter slots (4)
    emitters: Vec<Emitter>,
    /// the effect tasks (7 slots) and this frame's flat quads of theirs (the crescents)
    tasks: Vec<Task>,
    flat: Vec<Flat>,
    draws: Vec<Draw>,
    rnd: Rand,
    /// the fight frame last stepped
    last: Option<u64>,
    /// the background's flash: frames left, colour
    flash: (i32, [u8; 3]),
    /// the hit's light flash: light #3's colour (s32, 4.12 in the high half), its step down, the frames run
    light: Option<([i32; 3], [i32; 3], u8)>,
    /// light #3's colour this frame (as the lighting reads it, before the step down), while the flash runs
    light_shown: Option<[i32; 3]>,
    /// the emitters on the fighters' nodes (5 slots)
    attached: [Option<Attached>; 5],
    /// the move-cancel flags by fighter (by player: a move cancel sets 2, cleared each frame after the tasks)
    cancel: [u8; SIDES],
    /// the camera's shake words (vertical and horizontal: `Request::Cam`), this frame's view offset
    /// and the fight frame they were last stepped to
    shake: [i16; 2],
    shake_ofs: [i32; 2],
    shake_last: Option<u64>,
    /// the camera's yaw this frame (the whirl faces it: G.101C +0xA)
    cam_yaw: i32,
    /// the effect scripts and the fighters' back colour scripts with the back colour they
    /// set per fighter (by player: None the stage's)
    scripts: [Option<Script>; 4],
    backs: [Option<BackScript>; SIDES],
    back: [[Option<i32>; 3]; SIDES],
    /// the beast form's lightning (command 3), command 5's web (the same four slots) and this frame's sprites of them
    crackles: Vec<Crackle>,
    webs: Vec<Web>,
    sprites: Vec<Sprite>,
}

impl Default for Br1Fx {
    fn default() -> Self {
        Self {
            pool: vec![Particle::blank(); POOL],
            free: (0..POOL).collect(),
            live: Vec::new(),
            emitters: Vec::new(),
            tasks: Vec::new(),
            flat: Vec::new(),
            draws: Vec::new(),
            rnd: Rand { seed: 1, tidx: 0 },
            last: None,
            flash: (0, [0; 3]),
            light: None,
            light_shown: None,
            attached: Default::default(),
            cancel: [0; SIDES],
            shake: [0; 2],
            shake_ofs: [0; 2],
            shake_last: None,
            cam_yaw: 0,
            scripts: Default::default(),
            backs: Default::default(),
            back: [[None; 3]; SIDES],
            crackles: Vec::new(),
            webs: Vec::new(),
            sprites: Vec::new(),
        }
    }
}

impl Br1Fx {
    /// The fight frames after the last one stepped up to `frame`: each one's requests from `log` (`Br1Match::fx_log`),
    /// then a frame of the particles; a frame number that went back (a rewind, the KO replay's start) or jumped
    /// starts afresh.
    pub fn advance(&mut self, t: &Tables, b: &Br1Scene, frame: u64, gte: &Gte, cam_yaw: i32) {
        self.cam_yaw = cam_yaw;
        let log = &b.fx_log;
        let first = match self.last {
            Some(l) if l <= frame && frame - l <= crate::fight::br1::bridge::FX_FRAMES => l + 1,
            _ => {
                *self = Self {
                    rnd: Rand { seed: self.rnd.seed, tidx: self.rnd.tidx },
                    shake: self.shake,
                    shake_ofs: self.shake_ofs,
                    shake_last: self.shake_last,
                    ..Self::default()
                };
                frame
            }
        };
        let pitch = gte.pitch as i64;
        for f in first..=frame {
            // (a step logs its requests under the frame number it starts from)
            for (_, side, r) in log.iter().filter(|(lf, _, _)| *lf + 1 == f) {
                self.request(t, b, *side as usize, r);
            }
            if self.flash.0 > 0 {
                self.flash.0 -= 1;
            }
            // the light flash, down by its step from its 11th frame, over at its 41st
            self.light_shown = self.light.map(|(c, _, _)| c.map(|v| v >> 16));
            if let Some((col, step, n)) = &mut self.light {
                *n += 1;
                if *n >= 11 {
                    for k in 0..3 {
                        col[k] = col[k].wrapping_sub(step[k]);
                    }
                }
                if *n >= 41 {
                    self.light = None;
                }
            }
            // the emitters (in the fighters' draw)
            let mut es = std::mem::take(&mut self.emitters);
            es.retain_mut(|e| self.emitter_step(t, b, e));
            self.emitters = es;
            // the effect tasks
            let fs: Vec<Fighter> = b.fs.iter().map(Fighter::of).collect();
            let mut flat = Vec::new();
            let cancel = self.cancel;
            let rnd = &mut self.rnd;
            let yaw = self.cam_yaw;
            self.tasks.retain_mut(|k| k.step(t, gte, &fs, cancel, rnd, yaw).map(|q| flat.extend(q)).is_some());
            self.flat = flat;
            // (the move-cancel flags cleared after the tasks)
            self.cancel = [0; SIDES];
            self.scripts_step(t, b);
            // the beast form's lightning
            let fs: Vec<Fighter> = b.fs.iter().map(Fighter::of).collect();
            let mut sprites = Vec::new();
            let rnd = &mut self.rnd;
            self.crackles.retain_mut(|c| c.step(t, gte, &fs[c.side], rnd, &mut sprites));
            self.webs.retain_mut(|w| w.step(t, gte, &fs[w.side], rnd, &mut sprites));
            self.sprites = sprites;
            // every live record in list order, children of the dying appended (and stepped this frame);
            // a record that died is unlinked and freed at its next visit (+0x47)
            let mut draws = Vec::new();
            let mut k = 0;
            while k < self.live.len() {
                let i = self.live[k];
                if self.pool[i].dead {
                    self.live.remove(k);
                    self.free.push(i);
                    continue;
                }
                let d = step(t, &mut self.pool[i], gte, pitch, &mut self.rnd, &fs);
                if self.pool[i].dead {
                    for (c, b3c) in kill_children(t, &self.pool[i], &mut self.rnd) {
                        self.spawn(c, b3c);
                    }
                } else if let Some(d) = d {
                    draws.push(d);
                }
                k += 1;
            }
            self.draws = draws;
            // the emitters on the nodes (after the particles: their spawns are stepped from the next frame)
            for k in 0..5 {
                if let Some(mut a) = self.attached[k].take() {
                    let f = &b.fs[a.side];
                    let node = f.forms[f.cur as usize & 1].md.get(a.node).map_or([0; 3], |d| d.wt.map(|v| s16(v as i64)));
                    if self.attached_step(t, &mut a, node) {
                        self.attached[k] = Some(a);
                    }
                }
            }
        }
        self.last = Some(frame);
    }

    /// A frame of emitter `e` (the emitters' slot loop, by kind); false once it has run its
    /// frames. Kind 1 (the hits' and guards' sparks): per particle a direction jittered by ±0x200 in yaw and pitch at
    /// speed `+0x1E · 10 + 0x100`, scaled by `(0x800 + rand & 0x7FF) >> 11`, from the position jittered by
    /// `0x20 − rand & 0x3F` per axis (the spawn's mode byte the pitch's jitter, written over after it by type: 6 mode 1,
    /// a quarter of them 10 larger by the byte table; 5 mode 5 with +0x3C 0x64 and damping 0xFA0; 21 mode 13 with +0x3C
    /// 0, +0x3E 0x378, gravity 0x200; the others mode 1).
    fn emitter_step(&mut self, t: &Tables, b: &Br1Scene, e: &mut Emitter) -> bool {
        let s4 = e.speed * 10 + 0x100;
        if e.kind == 1 {
            for _ in 0..e.per {
                let s1 = 0x200 - (self.rnd.rand() & 0x3FF);
                let a = e.yaw + s1;
                let mut v = [s4 * t.rcos(a) / 4096, 0, s4 * t.rsin(a) / 4096];
                let s1 = 0x200 - (self.rnd.rand() & 0x3FF);
                v[1] = s4 * t.rcos(e.pitch + s1) / 4096;
                let k = 0x800 + (self.rnd.rand() & 0x7FF);
                let v = v.map(|c| s16((s16(c) * k) >> 11));
                let p = [0, 1, 2].map(|i| s16(e.pos[i] + 0x20 - (self.rnd.rand() & 0x3F)));
                let Some(i) = self.spawn(Particle::new(t, e.ty, s1 as u8, p, v, e.clut), false) else { continue };
                match e.ty {
                    6 => {
                        if self.rnd.trand(t) & 3 == 0 {
                            self.pool[i].size = s16(self.pool[i].size + 10);
                        }
                        self.pool[i].mode = 1;
                    }
                    5 => (self.pool[i].mode, self.pool[i].b3c, self.pool[i].damp) = (5, 0x64, 0xFA0),
                    21 => {
                        let r = &mut self.pool[i];
                        (r.mode, r.b3c, r.b3e, r.grav) = (13, 0, 0x378, 0x200);
                    }
                    _ => self.pool[i].mode = 1,
                }
            }
        } else if matches!(e.kind, 8 | 13 | 15 | 18) {
            // kinds 8, 13, 15, 18: a random direction (libc rand for
            // the yaw, then for the pitch) at speed s4 (each component toward 0), scaled as kind 1's, from the emitter's
            // position, mode 1 (15: 9, 18: 13); 13 and 15 then +0x3C 0, +0x3E 0x378 and the damping 0xED8 + rand & 0xFF
            let (mode, damped) = match e.kind {
                13 => (1, true),
                15 => (9, true),
                18 => (13, false),
                _ => (1, false),
            };
            for _ in 0..e.per {
                let a = self.rnd.rand();
                let mut v = [s4 * t.rcos(a) / 4096, 0, s4 * t.rsin(a) / 4096];
                let b = self.rnd.rand();
                v[1] = s4 * t.rcos(b) / 4096;
                let k = 0x800 + (self.rnd.rand() & 0x7FF);
                let v = v.map(|c| s16((s16(c) * k) >> 11));
                if let Some(i) = self.spawn(Particle::new(t, e.ty, mode, e.pos, v, e.clut), false)
                    && damped
                {
                    (self.pool[i].b3c, self.pool[i].b3e) = (0, 0x378);
                    self.pool[i].damp = 0xED8 + (self.rnd.rand() & 0xFF);
                }
            }
        } else if e.kind == 2 {
            // kind 2: from a random point round the emitter (cos, sin, sin of three libc rands / 8, scaled
            // by (0xC00 + rand & 0x3FF) >> 12) moving in at a quarter of its offset, mode 2, no gravity, size + 0x19,
            // damping 0x14CC, its point the emitter's and +0x3A its distance (SquareRoot12)
            for _ in 0..e.per {
                let mut v = [0i64; 3];
                let r = self.rnd.rand();
                v[0] = t.rcos(r) / 8;
                let r = self.rnd.rand();
                v[1] = t.rsin(r) / 8;
                let r = self.rnd.rand();
                v[2] = t.rsin(r) / 8;
                let k = 0xC00 + (self.rnd.rand() & 0x3FF);
                let v = v.map(|c| s16((s16(c) * k) >> 12));
                let dist = (((v[0] * v[0] + v[1] * v[1] + v[2] * v[2]) as f64).sqrt()) as i64;
                let pos: [i64; 3] = std::array::from_fn(|i| s16(e.pos[i] + v[i]));
                let vel = v.map(|c| -(c >> 2));
                if let Some(i) = self.spawn(Particle::new(t, e.ty, 2, pos, vel, e.clut), false) {
                    let p = &mut self.pool[i];
                    (p.grav, p.damp, p.b3a, p.p) = (0, 0x14CC, dist, e.pos);
                    p.size = s16(p.size + 0x19);
                }
            }
        } else if e.kind == 3 {
            // kind 3 (the spark presets; ok_br1_converter's `vfx::spark_frames`, checked against recorded runs): every
            // fourth frame the speed by the preset's +5 (at least 2), the yaw by its +2; per particle (at least one a
            // frame) the yaw and the pitch jittered by speed · 4 + 0x30 − rand % (speed · 8 + 0x60), at speed · 20,
            // scaled by (0x400 + rand & 0xBFF) >> 11; mode 3, +0x3C the preset's +11, its +10 without gravity (damping
            // 0xDAC)
            let p = e.preset;
            let sx = |a: u32| t.tables.abs_u8(p + a) as i8 as i64;
            if e.n & 3 == 0 {
                e.speed += sx(5);
            }
            e.speed = e.speed.max(2);
            e.yaw = s16(e.yaw + sx(2));
            let (s4, s0, s5) = (e.speed * 20, e.speed * 8 + 0x60, e.speed * 4 + 0x30);
            for _ in 0..e.per.max(1) {
                let j = s5 - cmod(self.rnd.rand(), s0);
                let (vx, vz) = (s4 * t.rcos(e.yaw + j) / 4096, s4 * t.rsin(e.yaw + j) / 4096);
                let j = s5 - cmod(self.rnd.rand(), s0);
                let vy = s4 * t.rcos(e.pitch + j) / 4096;
                let k = 0x400 + (self.rnd.rand() & 0xBFF);
                let mut q = Particle::new(t, e.ty, 3, e.pos, [(vx * k) >> 11, (vy * k) >> 11, (vz * k) >> 11], e.clut);
                q.b3c = t.tables.abs_u8(p + 11) as i64;
                if t.tables.abs_u8(p + 10) != 0 {
                    (q.grav, q.damp) = (0, 0xDAC);
                }
                self.spawn(q, true);
            }
        } else if e.kind == 6 {
            // kind 6 (op 0x17 sub 2's case 8): one particle a frame, mode 6, at the tip of node +0x2A of its
            // fighter (`tip`); +0x28 0: on odd frames rising at (0x23 − frames run) · 6, gravity 0x880, +0x3C 0; else
            // in a direction jittered by 0xFF − rand & 0x1FF in yaw and pitch at speed 0x96 + rand & 0x1FF (each
            // component toward 0), size 0x50, +0x3C 1; then +0x3E its CLUT
            let f = Fighter::of(&b.fs[e.side]);
            let pos = tip(t, &f, e.node, 0).map(|v| v as i64);
            let made = if e.flag == 0 {
                if e.n & 1 == 0 {
                    None
                } else {
                    let v = [0, s16((0x23 - e.n as i64) * 6), 0];
                    self.spawn(Particle::new(t, e.ty, 6, pos, v, e.clut), false).inspect(|&i| (self.pool[i].grav, self.pool[i].b3c) = (0x880, 0))
                }
            } else {
                let s4 = (self.rnd.rand() & 0x1FF) + 0x96;
                let s1 = 0xFF - (self.rnd.rand() & 0x1FF);
                let (vx, vz) = (s16(s4 * t.rcos(e.yaw + s1) / 4096), s16(s4 * t.rsin(e.yaw + s1) / 4096));
                let s1 = 0xFF - (self.rnd.rand() & 0x1FF);
                let vy = s16(s4 * t.rcos(e.pitch + s1) / 4096);
                self.spawn(Particle::new(t, e.ty, 6, pos, [vx, vy, vz], e.clut), false).inspect(|&i| (self.pool[i].size, self.pool[i].b3c) = (0x50, 1))
            };
            if let Some(i) = made {
                (self.pool[i].fighter, self.pool[i].b3e) = (Some(e.side), e.clut as i64);
            }
        } else if e.kind == 16 {
            // kind 16 (op 0x17 sub 2's case 11): on three frames in four, the node's x / z against the last
            // ones (+0x28 / +0x2C, then set to them): once it moved more than 0x28 (SquareRoot12) and is not above
            // −0x63, on the floor there (+0x2A 0) a band (mode 11, its far end (x, −(byte & 0x80), z) + 3/32 of the
            // direction yaw + 0x2C8 − rand % 0x590) and a spark (type 6, CLUT 2, mode 1, size 0x28) thrown up at
            // 0x118 + byte, out along the yaw by 1/8
            if e.n & 3 != 0 {
                let core = &b.fs[e.side];
                let p = core.forms[core.cur as usize & 1].md.get(e.node as usize).map_or([0; 3], |d| d.wt);
                let (dx, dz) = ((e.pos[0] as i32).wrapping_sub(p[0]), (e.pos[2] as i32).wrapping_sub(p[2]));
                let d2 = dx.wrapping_mul(dx).wrapping_add(dz.wrapping_mul(dz));
                let dist = ((d2.wrapping_shl(12) as u32 as f64 * 4096.0).sqrt() as i64) >> 12;
                (e.pos[0], e.pos[2]) = (s16(p[0] as i64), s16(p[2] as i64));
                if dist >= 0x29 && p[1] >= -0x63 {
                    e.pos[1] = 0;
                    let a = e.yaw + self.rnd.sym(0x2C8);
                    let y = -(self.rnd.trand(t) & 0x80);
                    if let Some(i) = self.spawn(Particle::new(t, e.ty, 11, e.pos, [0; 3], e.clut), false) {
                        let (c, s) = (t.rcos(a), t.rsin(a));
                        let x = s16(e.pos[0] + (c >> 4) + (c >> 5));
                        let z = s16(e.pos[2] + (s >> 4) + (s >> 5));
                        let r = &mut self.pool[i];
                        r.vel[0] = (x & 0xFFFF) | (y << 16);
                        r.vel[1] = (r.vel[1] & !0xFFFF) | (z & 0xFFFF);
                    }
                    let vy = -(self.rnd.trand(t) + 0x118);
                    let v = [s16(t.rcos(e.yaw) >> 3), s16(vy), s16(t.rsin(e.yaw) >> 3)];
                    if let Some(i) = self.spawn(Particle::new(t, 6, 1, e.pos, v, 2), false) {
                        self.pool[i].size = 0x28;
                    }
                }
            }
        } else if e.kind == 19 {
            // kind 19 (the effect scripts' preset 2): kind 2's random point round the emitter, a still
            // particle there (mode 14) circling the emitter's point at that distance (+0x3A, shrinking by
            // +0x3C = distance / 8, damped), size 0x23, its angles three libc rands; then three jitter draws of 0x80 (+8 ..
            // +0xC, not read in mode 14)
            for _ in 0..e.per {
                let mut v = [0i64; 3];
                v[0] = t.rcos(self.rnd.rand()) / 8;
                v[1] = t.rsin(self.rnd.rand()) / 8;
                v[2] = t.rsin(self.rnd.rand()) / 8;
                let k = 0xC00 + (self.rnd.rand() & 0x3FF);
                let v = v.map(|c| s16((s16(c) * k) >> 12));
                let dist = (((v[0] * v[0] + v[1] * v[1] + v[2] * v[2]) as f64).sqrt()) as i64;
                let pos: [i64; 3] = std::array::from_fn(|i| s16(e.pos[i] + v[i]));
                if let Some(i) = self.glow(t, pos, 6, 2, 14) {
                    let a = [self.rnd.rand(), self.rnd.rand(), self.rnd.rand()];
                    for _ in 0..3 {
                        self.rnd.sym(0x80);
                    }
                    let p = &mut self.pool[i];
                    (p.b3a, p.size, p.p, p.b3c) = (dist, 0x23, e.pos, dist >> 3);
                    (p.vel[0], p.vel[1]) = ((a[0] & 0xFFFF) | a[1] << 16, a[2]);
                }
            }
        }
        e.n = e.n.wrapping_add(1);
        e.n < e.frames
    }

    /// The particle spawn with the particle `p` (`Particle::new`'s fields, `sets_b3c`: its +0x3C too): the last free record
    /// (none while fewer than 2 are free), what the spawn and its caller write over what the record held; linked last.
    fn spawn(&mut self, p: Particle, sets_b3c: bool) -> Option<usize> {
        if self.free.len() < 2 {
            return None;
        }
        let i = self.free.pop()?;
        let old = std::mem::replace(&mut self.pool[i], p);
        let r = &mut self.pool[i];
        (r.b3a, r.b3e, r.b40, r.rgb) = (old.b3a, old.b3e, old.b40, old.rgb);
        if !sets_b3c && r.ty != 16 {
            r.b3c = old.b3c;
        }
        self.live.push(i);
        Some(i)
    }

    /// A still particle without gravity, damping 0xE66 (its other half, ending the
    /// emitters of kind 10 first, has no emitter of that kind here to end).
    fn glow(&mut self, t: &Tables, pos: [i64; 3], ty: usize, clut: usize, mode: u8) -> Option<usize> {
        let i = self.spawn(Particle::new(t, ty, mode, pos, [0; 3], clut), false)?;
        (self.pool[i].grav, self.pool[i].damp) = (0, 0xE66);
        Some(i)
    }

    /// A frame of the emitter `a` on its node at `node` (by kind); false once it is over.
    fn attached_step(&mut self, t: &Tables, a: &mut Attached, node: [i64; 3]) -> bool {
        match a.kind {
            // every interval + 1 frames a still particle at the node (jittered by ±range / 2 in x and z by the byte
            // table), over after `count` frames
            8 | 9 => {
                a.counter += 1;
                if a.counter > a.interval {
                    a.counter = 0;
                    let mut p = node;
                    if a.jitter {
                        let h = a.range >> 1;
                        p[0] = s16(p[0] + h - cmod(self.rnd.trand(t), a.range));
                        p[2] = s16(p[2] + h - cmod(self.rnd.trand(t), a.range));
                    }
                    self.glow(t, p, a.ty, 3, 0);
                }
                a.frames += 1;
                a.frames <= a.count
            }
            // after `count` frames four particles 0x400 apart from a random angle, 0x28 above the floor under the node,
            // moving out at `(cos, sin) / 32 · (0x800 + rand & 0x7FF) >> 11`, from `(cos, sin) · 64 >> 12` out; then over
            10 | 11 => {
                a.frames += 1;
                if a.frames <= a.count {
                    return true;
                }
                let p = [node[0], s16(a.floor as i64 - 0x28), node[2]];
                let mut s0 = self.rnd.rand();
                for _ in 0..4 {
                    let (c, s) = (t.rcos(s0), t.rsin(s0));
                    let k = (self.rnd.rand() & 0x7FF) + 0x800;
                    let v = [s16(((c / 32) * k) >> 11), 0, s16(((s / 32) * k) >> 11)];
                    let q = [s16(p[0] + ((c << 6) >> 12)), p[1], s16(p[2] + ((s << 6) >> 12))];
                    s0 += 0x400;
                    self.spawn(Particle::new(t, a.ty, 1, q, v, 3), false);
                }
                false
            }
            // every interval + 1 frames a particle within ±0x32 of the node (a jitter draw per axis), over after `count`
            12 | 13 => {
                a.counter += 1;
                if a.counter > a.interval {
                    a.counter = 0;
                    let p = node.map(|v| s16(v + self.rnd.sym(0x32)));
                    self.spawn(Particle::new(t, a.ty, 1, p, [0; 3], 3), false);
                }
                a.frames += 1;
                a.frames <= a.count
            }
            _ => true,
        }
    }

    /// A frame of the effect scripts: each one's commands whose frame is its counter now, in order (the
    /// list over at a command 0), its counter on; then the back colour scripts: the entry now as the
    /// player's back colour, the next (round to the first after `count`), the frames left down, at 0 the stage's again.
    fn scripts_step(&mut self, t: &Tables, b: &Br1Scene) {
        for k in 0..4 {
            let Some(mut s) = self.scripts[k].take() else { continue };
            let mut alive = true;
            while t.tables.abs_u8(s.list + 1) == s.counter {
                let cmd = t.tables.abs_u8(s.list);
                self.script_cmd(t, b, &mut s, cmd);
                if t.tables.abs_u8(s.list + 2) == 0 {
                    alive = false;
                    break;
                }
                s.list += 2;
            }
            s.counter = s.counter.wrapping_add(1);
            if alive {
                self.scripts[k] = Some(s);
            }
        }
        for p in 0..SIDES {
            let Some(bs) = &mut self.backs[p] else { continue };
            let e = [0, 2, 4].map(|o| t.tables.abs_i16(bs.at + o) as i32);
            self.back[p] = e.map(|v| (v != -1).then_some(v));
            bs.n += 1;
            if bs.n >= bs.count {
                (bs.n, bs.at) = (0, bs.start);
            } else {
                bs.at += 6;
            }
            bs.left -= 1;
            if bs.left == 0 {
                self.back[p] = [None; 3];
                self.backs[p] = None;
            }
        }
    }

    /// The back colour script `script` of the back colour table for `frames` of fighter
    /// `side` (the player in a match; the team battle's fighter, T5).
    fn back_script(&mut self, t: &Tables, side: usize, frames: i32, script: u8) {
        let a = t.tables.abs_u32(t.b(0x4D8) + 4 * script as u32);
        let start = a + 2;
        self.backs[side] = (frames != 0).then_some(BackScript { start, at: start, count: t.tables.abs_i16(a), n: 0, left: frames as i16 });
    }

    /// A command of an effect script (by its number) with its parameters from
    /// the script's stream: 0 / 1 a back colour script (when the first byte is 0), 2 an effect task
    /// on the fighter's node (its parameters by type, switch; node 3, 2 for types 5, 0x0D, 0x13,
    /// the parameter for 0x1F), 3 (2 bytes), 4 an emitter preset (1 byte), 5 (none).
    fn script_cmd(&mut self, t: &Tables, b: &Br1Scene, s: &mut Script, cmd: u8) {
        let d = |k: u32| t.tables.abs_u8(s.data + k) as i32;
        let f = Fighter::of(&b.fs[s.side]);
        match cmd {
            0 | 1 => {
                if d(0) == 0 {
                    let (frames, script) = (d(1), d(2) as u8);
                    s.data += 3;
                    self.back_script(t, s.side, frames, script);
                }
            }
            2 => {
                let ty = d(0);
                let (len, node, a2, a3, p10, p14) = match ty {
                    1..=3 | 0x0B | 0x20 => (3, 3, d(1), d(2), 0, 0),
                    5 | 0x13 => (4, 2, d(1), d(2), d(3), 0),
                    7 => (1, 3, f.player as i32, 0, 0, 0),
                    8 | 0x16 | 0x18..=0x1A | 0x25 => (1, 3, 0, 0, 0, 0),
                    0x0A | 0x17 | 0x23 => (2, 3, d(1), 0, 0, 0),
                    0x0D => (5, 2, d(1), d(2), d(3), d(4)),
                    0x0E => (5, 3, d(1), d(2), d(3), d(4)),
                    0x1F => (2, d(1), d(1), 0, 0, 0),
                    _ => (0, 3, 0, 0, 0, 0),
                };
                s.data += len;
                if self.tasks.len() >= 7 {
                    return;
                }
                let node = node as i16;
                let pos = f.node(node);
                match ty {
                    0x02 => self.tasks.push(Task::Ring2(Box::new(Ring2::new(t, s.side, node, a2, a3)))),
                    0x0E if self.objects() < 6 => {
                        let k = Whirl::new(t, s.side, node, a2, a3, p10, p14, &mut self.rnd);
                        self.tasks.push(Task::Whirl(Box::new(k)));
                    }
                    0x17 if self.objects() < 6 => self.tasks.push(Task::Globe(Box::new(Globe::new(t, s.side, node)))),
                    0x08 => self.tasks.push(Task::Star7(Box::new(Star7::new(t, s.side, node)))),
                    0x01 => self.tasks.push(Task::Beams(Box::new(Beams::new(t, s.side, node, &mut self.rnd)))),
                    0x0A if self.objects() < 6 => self.tasks.push(Task::Band(Box::new(Band::new(t, s.side, node, a2)))),
                    0x0B if self.objects() < 6 => self.tasks.push(Task::Tornado(Box::new(Tornado::new(t, s.side, node, a2, a3)))),
                    0x1B => self.tasks.push(Task::Flame(Box::new(Flame::new(s.side, node)))),
                    0x16 if self.objects() < 6 => self.tasks.push(Task::Pie(Box::new(Pie::new(t, s.side, node, &mut self.rnd)))),
                    0x05 => self.tasks.push(Task::Aura(Box::new(Aura::new(t, s.side, node)))),
                    0x20 => self.tasks.push(Task::Rays(Box::new(Rays::new(t, s.side, node, a2, a3)))),
                    0x0D => self.tasks.push(Task::Crescent(Crescent::new(pos, a3, p10, p14, &mut self.rnd))),
                    0x13 if self.objects() < 6 => {
                        let k = Crystal::new(t, pos, a2 as u32, a3, p10, &mut self.rnd);
                        self.tasks.push(Task::Crystal(Box::new(k)));
                    }
                    _ => {}
                }
            }
            3 => {
                // the beast form's lightning along chain d0 in colour set d1 (4 at most)
                if self.crackles.len() + self.webs.len() < 4 {
                    let k = Crackle::new(t, s.side, &f, d(0) as u8, d(1) as u8, &mut self.rnd);
                    self.crackles.push(k);
                }
                s.data += 2;
            }
            5 => {
                // command 5 takes no parameter: the web at node 3's world translation (a free one of the
                // lightning's four slots)
                if self.crackles.len() + self.webs.len() < 4 {
                    self.webs.push(Web::new(t, s.side, 3));
                }
            }
            4 => {
                // an emitter preset at the fighter's node 3 (its MD +0x124): 0 converging type-6 sparks (kind 2),
                // 1 type 0x16 (kind 13) and type 0x11 (kind 15); 2 circling type-6 sparks (kind 19)
                let pos = f.node(3);
                match d(0) {
                    0 => self.emit(Emitter::new(pos, 6, 0, 0, 0x12C, 0x10, 4, 2, 2)),
                    1 => {
                        self.emit(Emitter::new(pos, 0x16, 0, 0, 0x1E, 0xE, 7, 2, 13));
                        self.emit(Emitter::new(pos, 0x11, 0, 0, 0x28, 6, 3, 2, 15));
                    }
                    // type 6, one a frame for 20 frames, CLUT 2 (the preset's number), kind 19
                    2 => self.emit(Emitter::new(pos, 6, 0, 0, 0, 0x14, 0x14, 2, 19)),
                    _ => {}
                }
                s.data += 1;
            }
            _ => {}
        }
    }

    /// The 3-D objects in use (6), the crystals' and the bursts'.
    fn objects(&self) -> usize {
        self.tasks
            .iter()
            .filter(|k| {
                !matches!(
                    k,
                    Task::Crescent(_) | Task::Ring2(_) | Task::Rays(_) | Task::Aura(_) | Task::Star7(_) | Task::Beams(_) | Task::Flame(_) | Task::Pulse(_)
                )
            })
            .count()
    }

    /// A new emitter in a free slot of the four (none free, none made).
    fn emit(&mut self, e: Emitter) {
        if self.emitters.len() < 4 {
            self.emitters.push(e);
        }
    }

    /// A request of the imported game's code from fighter `side` (its log entry's tag). The effects on a fighter take it
    /// from the tag, not from the fighter argument imported passes (F: its place in its pair, `script::ops`; the
    /// player for a move cancel), which equals the tag in a match but is 0 / 1 for the team battle's four (T5).
    fn request(&mut self, t: &Tables, b: &Br1Scene, side: usize, r: &Request) {
        match *r {
            // its sparks (type 6 of kind 1 at speed
            // = damage, damage − 10 of them (4 below damage 10, 12 above 22) over 4 frames, CLUT 2; the option G.BC+0x1E's
            // path for trail 2 is not taken), from damage 10 with a3 ≥ 0x1000 a burst (task 0x109 (pos, 3, a1, a2,
            // damage)), then its glow (a still particle (pos, 0, 4, 0): CLUT 4, mode 0, no gravity; size 0x64 +
            // 4 · damage, 0xF0 from damage 0x1F)
            Request::Fx { call: Call::HitSparks, pos, args } => {
                let dmg = (args[2] & 0xFFF) as i64;
                self.emit(Emitter::new(
                    pos,
                    0x106,
                    args[0],
                    args[1],
                    dmg as i32,
                    {
                        let c = dmg as i32 - 10;
                        if c < 0 { 4 } else { c.min(12) }
                    },
                    4,
                    2,
                    1,
                ));
                if args[2] >= 0x1000 && dmg >= 10 && self.tasks.len() < 7 && self.objects() < 6 {
                    let k = Burst::new(t, pos, args[0], args[1], dmg as i32, &mut self.rnd);
                    self.tasks.push(Task::Burst(Box::new(k)));
                }
                if let Some(i) = self.glow(t, pos.map(|v| v as i64), 0, 4, 0) {
                    self.pool[i].size = if dmg >= 0x1F { 0xF0 } else { dmg * 4 + 0x64 };
                }
            }
            // task 0x10C (pos, 0, 0xC8, 0xF, 1) (a hit with F.64 0x800000): the stars (type 0x0C: init
            // its init also starts an emitter of 0x10 type-6 sparks over 4 frames, speed 0x12, CLUT 2, kind 8)
            Request::Fx { call: Call::Task, pos, args } if args[0] & 0xFF == 0x0C => {
                if self.tasks.len() < 7 && self.objects() < 6 {
                    self.emit(Emitter::new(pos, 6, 0, 0, 0x12, 0x10, 4, 2, 8));
                    self.tasks.push(Task::Star(Box::new(Star::new(t, pos))));
                }
            }
            // the guard's sparks (type 6 of kind 1, speed and count
            // clamp(damage, 2, 4) over 2 frames, CLUT 0)
            Request::Fx { call: Call::GuardSparks, pos, args } => {
                let c = (args[2] & 0xFFF).clamp(2, 4);
                self.emit(Emitter::new(pos, 0x106, args[0], args[1], c, c, 2, 0, 1));
                // then task 0x113 (pos, 0x10, v, 0xC, 0): the crystal (when a task slot and a 3-D object are free)
                // and task 0x10D (pos, 0, (v >> 2) − 5, 16, v >> 3): the crescents, v = clamp(4 · damage, 0x46, 0x6E)
                let v = ((args[2] & 0xFFF) << 2).clamp(0x46, 0x6E);
                if self.tasks.len() < 7 && self.objects() < 6 {
                    let k = Crystal::new(t, pos, 0x10, v, 0xC, &mut self.rnd);
                    self.tasks.push(Task::Crystal(Box::new(k)));
                }
                if self.tasks.len() < 7 {
                    let k = Crescent::new(pos, (v >> 2) - 5, 16, v >> 3, &mut self.rnd);
                    self.tasks.push(Task::Crescent(k));
                }
            }
            // task `type | 0x10000` (F, timing, life, a, b) (op 0x0D): an attack's trail on fighter F (type 0x0F between nodes
            // a and b; type 0x11, op 0x17 sub 2's case 10, has the same init and update)
            Request::Call { call: Call::Task, args } if matches!(args[0] & 0xFF, 0x0F | 0x11) => {
                if self.tasks.len() < 7 && self.objects() < 6 {
                    let s = side;
                    if let Some(k) = Trail::new(t, s, &Fighter::of(&b.fs[s]), args[2], args[3], args[4], args[5]) {
                        self.tasks.push(Task::Trail(Box::new(k)));
                    }
                }
            }
            // (type 0x10: a beast's claws on node a)
            Request::Call { call: Call::Task, args } if args[0] & 0xFF == 0x10 => {
                if self.tasks.len() < 7 && self.objects() < 6 {
                    let s = side;
                    let life = if args[3] < 0 { 0xE10 } else { args[3] };
                    if args[2] & 0xFFFF != life {
                        let k = Claw::new(t, s, &Fighter::of(&b.fs[s]), args[2], life, args[4]);
                        self.tasks.push(Task::Claw(Box::new(k)));
                    }
                }
            }
            // (type 0x12: a human's hand ribbon on node a; Fox's and Uriko's a type 0x10 claw, its middle one only)
            Request::Call { call: Call::Task, args } if args[0] & 0xFF == 0x12 => {
                let s = side;
                let f = Fighter::of(&b.fs[s]);
                let life = if args[3] < 0 { 0xE10 } else { args[3] };
                if self.tasks.len() < 7 && args[2] & 0xFFFF != life {
                    if f.chr == 6 || f.chr == 8 {
                        // (the nested task 0x10 takes another slot while this one is held)
                        if self.tasks.len() < 6 && self.objects() < 6 {
                            let mut k = Claw::new(t, s, &f, args[2], life, args[4]);
                            (k.start, k.step) = (1, 2);
                            self.tasks.push(Task::Claw(Box::new(k)));
                        }
                    } else if self.objects() < 6 {
                        self.tasks.push(Task::Ribbon(Box::new(Ribbon::new(t, s, &f, args[2], life, args[4]))));
                    }
                }
            }
            // the floor-effect spawner (node, kind, yaw, floor) (op 0x17 subs 0, 1, 5): kind 0 a shockwave on the floor under the node
            // (task 0x1F at (x, floor, z)); the others (sub-emitters and kind-9 emitters) not yet
            Request::Call { call: Call::FloorEffect, args } if args[1] >> 8 == 0 => {
                if self.tasks.len() < 7 && self.objects() < 6 {
                    let n = Fighter::of(&b.fs[side]).node(args[0] as i16);
                    self.tasks.push(Task::Ring(Box::new(Ring::new(t, [n[0], args[3] as i16, n[2]]))));
                }
            }
            // an effect script n of fighter F (op 0x16; the transformation's is 1): an effect script in the first free slot: n 0 the
            // fighter's character's, 1 the tenth, 2..5, 0x100 fixed ones; 0, 1 and 4 with a back
            // colour script ((player, 0x3C or 0x64, 0))
            Request::Call { call: Call::EffectScript, args } => {
                let s = side;
                let f = Fighter::of(&b.fs[s]);
                let w = |a: u32| t.tables.abs_u32(a);
                let (list, data, back) = match args[0] {
                    0 => (w(t.b(0xE88) + 4 * f.chr as u32), w(t.b(0xEB0) + 4 * f.chr as u32), 0x3C),
                    1 => (w(t.b(0xEAC)), w(t.b(0xED4)), 0x64),
                    2 => (t.b(0xE08), t.b(0xE28), 0),
                    3 => (t.b(0xDF0), t.b(0xDFC), 0),
                    4 => (t.b(0xE50), t.b(0xE5C), 0x3C),
                    5 => (t.b(0xC2C), t.b(0xC34), 0),
                    0x100 => (t.b(0xE78), t.b(0xE80), 0),
                    _ => return,
                };
                if let Some(k) = self.scripts.iter().position(|s| s.is_none()) {
                    self.scripts[k] = Some(Script { list, data, side: s, counter: 0 });
                    if back != 0 {
                        self.back_script(t, s, back, 0);
                    }
                }
            }
            // a still particle (an imported stage's debris splashing, simulation
            // v18: type 0x10, CLUT 0xE, mode 4)
            Request::Fx { call: Call::StillParticle, pos, args } => {
                self.glow(t, pos.map(|v| v as i64), args[0] as usize, args[1] as usize, args[2] as u8);
            }
            // a spark preset (F, preset) (op 0x17 sub 3): a kind-3 emitter of the spark preset (12 bytes: +0 the
            // yaw / 16, +1 the pitch / 16, +4 the speed, +6 / +7 the total and frames, +8 the node; mirrored: the node by
            // the mirror table, the yaw 0x1000 − it) at the node, turned by F.46 + F.6C + 0x400; particle type and CLUT
            // from the preset's type table (+ 2 with the option G.BC + 0x1E, blood: not taken, as for the hits)
            Request::Call { call: Call::SparkPreset, args } => {
                let s = side;
                let core = &b.fs[s];
                let p = t.b(0x14) + 12 * (args[1] as u32 & 0xFF);
                let mut yaw = (t.tables.abs_u8(p) as i32) << 4;
                let mut node = t.tables.abs_u8(p + 8) as u32;
                if core.anim.mirror {
                    yaw = 0x1000 - yaw;
                    node = t.tables.abs_u8(t.b(0) + node) as u32;
                }
                let pos = Fighter::of(core).node(node as i16);
                let yaw = core.f.yaw as i32 + core.f.opp_angle as i32 + yaw + 0x400;
                let (ty, clut) = (t.tables.abs_u8(t.a(0x2BC)) as u16, t.tables.abs_u8(t.a(0x2BD)) as i32);
                let u = |k: u32| t.tables.abs_u8(p + k) as i32;
                let mut e = Emitter::new(pos, ty, yaw, u(1) << 4, u(4), u(6), u(7), clut, 3);
                e.preset = p;
                self.emit(e);
            }
            // op 0x17 sub 2 (F, a): by a's top nibble k, with its node nibble n (mirrored by table
            // the mirror table) and the parameters p, q: 0 the beast form's lightning on chain n (colour set 1),
            // 1 the floor-effect spawner's kind p at node n (kind 0: the floor ring), 3 a still particle (type 0, CLUT 4) of
            // size p · 16 at node n, 5 an emitter of kind p + 8 on node n + 1 for q frames, 6 a burst
            // (task 0x109: yaw F.46 + F.6C − p · 256, pitch q · 256, damage 30), 14 effect script n, 15 command
            // 5's web at node n; 7..13 below. Not implemented: 2 and 4 (emitter kinds 5 and p: no script of the four
            // fighters has them), 5's kinds past 13 (none of theirs either)
            Request::Call { call: Call::NodeEffect, args } => {
                let s = side;
                let core = &b.fs[s];
                let f = Fighter::of(core);
                let a = args[1];
                let (k, p, q) = ((a >> 12) & 0xF, (a >> 4) & 0xF, a & 0xF);
                let mut n = (a >> 8) & 0xF;
                if core.anim.mirror {
                    n = t.tables.abs_u8(t.b(0) + n as u32) as i32;
                }
                let pos = f.node(n as i16);
                match k {
                    0 if self.crackles.len() + self.webs.len() < 4 => {
                        let c = Crackle::new(t, s, &f, n as u8, 1 | (f.form & 1) << 7, &mut self.rnd);
                        self.crackles.push(c);
                    }
                    1 if p == 0 && self.tasks.len() < 7 && self.objects() < 6 => {
                        self.tasks.push(Task::Ring(Box::new(Ring::new(t, pos))));
                    }
                    3 => {
                        if let Some(i) = self.glow(t, pos.map(|v| v as i64), 0, 4, 0) {
                            self.pool[i].size = (p << 4) as i64;
                        }
                    }
                    // (kinds p + 8 past 13, 14.., are not implemented yet: none made, as no slot of theirs would end)
                    5 if p + 8 <= 13 => {
                        if let Some(slot) = self.attached.iter_mut().find(|x| x.is_none()) {
                            *slot = Some(Attached::new(s, n + 1, p + 8, q, 0));
                        }
                    }
                    6 if self.tasks.len() < 7 && self.objects() < 6 => {
                        let yaw = core.f.yaw as i32 + core.f.opp_angle as i32 - (p << 8);
                        let k = Burst::new(t, pos, yaw, q << 8, 0x1E, &mut self.rnd);
                        self.tasks.push(Task::Burst(Box::new(k)));
                    }
                    // the tasks of the four fighters' scripts (0x01 beams, 0x05 dark
                    // aura, 0x0A band, 0x1F floor ring)
                    7 if self.tasks.len() < 7 => match a & 0xFF {
                        0x01 => self.tasks.push(Task::Beams(Box::new(Beams::new(t, s, n as i16, &mut self.rnd)))),
                        0x05 => self.tasks.push(Task::Aura(Box::new(Aura::new(t, s, n as i16)))),
                        0x0A if self.objects() < 6 => self.tasks.push(Task::Band(Box::new(Band::new(t, s, n as i16, n)))),
                        0x1F if self.objects() < 6 => self.tasks.push(Task::Ring(Box::new(Ring::new(t, pos)))),
                        _ => {}
                    },
                    // 8: with n ≠ 0 an emitter of kind 6 ((node n, 0x10A, yaw F.46 + F.6C − 0x400,
                    // pitch 0x400, speed 0x14, 0x28 over 0x28 frames, CLUT 5), then +0x14 F, +0x28 n, +0x2A 0x13 (its
                    // node), +0x18 CLUT 6; with n = 0 none (that one, 15 frames on node 2, is the blood option's,
                    // G.BC + 0x1E, not taken as for the hits)
                    8 if n != 0 => {
                        let yaw = core.f.yaw as i32 + core.f.opp_angle as i32 - 0x400;
                        let mut e = Emitter::new(pos, 0x10A, yaw, 0x400, 0x14, 0x28, 0x28, 5, 6);
                        (e.side, e.flag, e.node, e.clut) = (s, n as i16, 0x13, 6);
                        self.emit(e);
                    }
                    // 11: an emitter of kind 16 on node n's translation ((the pointer, 0x12, yaw
                    // F.46 + F.6C − 0x400, 0, 0, 0x1E over 0x1E frames, CLUT 0))
                    11 => {
                        let yaw = core.f.yaw as i32 + core.f.opp_angle as i32 - 0x400;
                        let mut e = Emitter::new(pos, 0x12, yaw, 0, 0, 0x1E, 0x1E, 0, 16);
                        (e.side, e.node) = (s, n as i16);
                        self.emit(e);
                    }
                    // 13: in the beast form, on an odd libc rand, a drop (a still particle (the tip of node 19
                    // with v = −(byte & 5) more in z, type 0x13, CLUT 6, mode 12)) hanging from the fighter: +0x14 the
                    // tip, +0x30 F, +0x3C 0x400, +0x3E 0xFA, +0x40 v, +0x3A 0 (`drop_step`)
                    13 if f.form & 1 != 0 => {
                        if self.rnd.rand() & 1 != 0 {
                            let v = -(self.rnd.trand(t) & 5);
                            let tp = tip(t, &f, 19, v as i32).map(|c| c as i64);
                            if let Some(i) = self.glow(t, tp, 0x13, 6, 12) {
                                let p = &mut self.pool[i];
                                p.vel[0] = (tp[0] & 0xFFFF) | (tp[1] << 16);
                                p.vel[1] = (p.vel[1] & !0xFFFF) | (tp[2] & 0xFFFF);
                                p.fighter = Some(s);
                                (p.b3c, p.b3e, p.b40, p.b3a) = (0x400, 0xFA, v, 0);
                            }
                        }
                    }
                    // 9: task 0x21 on node n (at the node's translation) and the
                    // back colour script (player, 0x1E, 2)
                    9 => {
                        if self.tasks.len() < 7 {
                            self.tasks.push(Task::Pulse(Box::new(Pulse::new(t, s, n as i16))));
                        }
                        self.back_script(t, s, 0x1E, 2);
                    }
                    // 10: two trails (task 0x11 (F, 0, 5, a, b): nodes 0xB / 1 and 4 / 7, mirrored 0xE /
                    // 1 and 4 / 0xA), emitters of kind 8 on nodes 7 and 10 for 12 frames, the back colour
                    // script (player, 5, 0); its sound 0x5F is the simulation's (v18, `fight::sim::br1`)
                    10 => {
                        let pairs = if core.anim.mirror { [(0xE, 1), (4, 0xA)] } else { [(0xB, 1), (4, 7)] };
                        for (na, nb) in pairs {
                            self.request(t, b, s, &Request::Call { call: Call::Task, args: [0x11, s as i32, 0, 5, na, nb] });
                        }
                        for node in [7, 10] {
                            if let Some(slot) = self.attached.iter_mut().find(|x| x.is_none()) {
                                *slot = Some(Attached::new(s, node, 8, 0xC, 0));
                            }
                        }
                        self.back_script(t, s, 5, 0);
                    }
                    // 12: a still particle of type 0x15 (CLUT 0xF, mode 13) at node n, +0x3C 0, +0x3E 0x378, +0x40 0x2800,
                    // +0x14 0x80
                    12 => {
                        if let Some(i) = self.glow(t, pos.map(|v| v as i64), 0x15, 0xF, 13) {
                            let p = &mut self.pool[i];
                            (p.b3c, p.b3e, p.b40) = (0, 0x378, 0x2800);
                            p.vel[0] = (p.vel[0] & !0xFFFF) | 0x80;
                        }
                    }
                    14 => self.request(t, b, s, &Request::Call { call: Call::EffectScript, args: [n, s as i32, 0, 0, 0, 0] }),
                    15 if self.crackles.len() + self.webs.len() < 4 => self.webs.push(Web::new(t, s, n as i16)),
                    _ => {}
                }
            }
            // the player's moves cancelled (their trails end this frame)
            Request::Call { call: Call::MoveCancel, .. } => self.cancel[side] = 2,
            // a node emitter (node, kind, frames, floor) (op 0x18): an emitter on the node in the first free slot of the five
            Request::Call { call: Call::NodeEmitter, args } => {
                if let Some(s) = self.attached.iter_mut().find(|s| s.is_none()) {
                    *s = Some(Attached::new(side, args[0], args[1], args[2], args[3]));
                }
            }
            // the background's flash
            Request::Call { call: Call::BackgroundFlash, args } => self.flash = (args[0], [args[1] as u8, args[2] as u8, args[3] as u8]),
            // the hit's light flash: light #3 at min(damage · 128, 0xFFF) in the channels of `kind`
            // (0 white, 1 red, 2 blue, 3 red and green; the others none; the rest keep the record's colour)
            Request::Call { call: Call::HitLight, args } => {
                let a = (args[1].wrapping_shl(7)).min(0xFFF).wrapping_shl(16);
                // (the rest: the record's light #3 colour, 0 in every imported stage's record)
                let mut col = [0i32; 3];
                match args[0] {
                    0 => col = [a; 3],
                    1 => col[0] = a,
                    2 => col[2] = a,
                    3 => (col[0], col[1]) = (a, a),
                    _ => col = [0; 3],
                }
                self.light = Some((col, col.map(|c| c / 15), 0));
            }
            _ => {}
        }
    }

    /// The camera shake (stepped in the camera's frame) stepped to `frame`: each frame's `Request::Cam` words, the
    /// view offset `(h · 256 / 10000, v · 256 / 10000)` (x right, y down, view space: added to the view's translation),
    /// then the words decayed (v = −3v / 4, h = −(h / 4)); a rewind or jump starts from none.
    pub fn shake(&mut self, b: &Br1Scene, frame: u64) -> [i32; 2] {
        let first = match self.shake_last {
            Some(l) if l <= frame && frame - l <= crate::fight::br1::bridge::FX_FRAMES => l + 1,
            _ => {
                (self.shake, self.shake_ofs) = ([0; 2], [0; 2]);
                frame
            }
        };
        for f in first..=frame {
            for (_, _, r) in b.fx_log.iter().filter(|(lf, _, _)| *lf + 1 == f) {
                match *r {
                    Request::Cam { word: CamWord::Vertical, value } => self.shake[0] = value,
                    Request::Cam { word: CamWord::Horizontal, value } => self.shake[1] = value,
                    _ => {}
                }
            }
            let [v, h] = self.shake.map(|s| s as i32);
            self.shake_ofs = [(h << 8) / 10000, (v << 8) / 10000];
            self.shake = [(-3 * v / 4) as i16, (-(h / 4)) as i16];
        }
        self.shake_last = Some(frame);
        self.shake_ofs
    }

    /// The hit's light flash: light #3's colour (4.12), while it runs.
    pub fn light3(&self) -> Option<[i32; 3]> {
        self.light_shown
    }

    /// Fighter `p`'s back colour as its back colour script sets it (None per channel: the stage's).
    pub fn back(&self, p: usize) -> [Option<i32>; 3] {
        self.back[p]
    }

    /// The background's flash colour, while it lasts.
    pub fn flash(&self) -> Option<[u8; 3]> {
        (self.flash.0 > 0).then_some(self.flash.1)
    }

    /// This frame's particles and tasks as PS1 quads (far ones first, as the OT draws them).
    pub fn quads(&self, t: &Tables, gte: &Gte) -> Vec<PsxQuad> {
        let parts = self.draws.iter().map(|d| {
            let tt = &t.types[d.ty.min(t.types.len() - 1)];
            let z = gte.slot_depth(d.otz);
            let (cx, cy) = t.cluts.get(d.clut).copied().unwrap_or(t.cluts[0]);
            let c = |v: i64| if tt.raw { 128 } else { v.clamp(0, 255) as u8 };
            let q = PsxQuad {
                pos: d.quad.map(|(x, y)| gte.clip([x, y], z)),
                uv: d.uv.map(|(u, v)| [u as f32, v as f32]),
                rgb: [[c(d.rgb[0]), c(d.rgb[1]), c(d.rgb[2])]; 4],
                tpage: tpage_word(tt.tpage),
                clut: clut_word(cx, cy),
                textured: true,
                semi: tt.semi,
                sheet: None,
                clip: crate::output::layout::Clip::No,
            };
            (d.otz, q)
        });
        // the tasks' flat and gouraud quads (0x2A / 0x3A) in their blending (their DR_MODEs: added, or subtracted)
        let flats = self.flat.iter().map(|(q, rgb, otz, tp)| {
            let z = gte.slot_depth(*otz);
            let q = PsxQuad {
                pos: q.map(|(x, y)| gte.clip([x, y], z)),
                uv: [[0.0; 2]; 4],
                rgb: *rgb,
                tpage: if *tp == OPAQUE { ADD } else { *tp },
                clut: 0,
                textured: false,
                semi: *tp != OPAQUE,
                sheet: None,
                clip: crate::output::layout::Clip::No,
            };
            (*otz, q)
        });
        // the beast form's lightning: POLY_FT4 on tpage 0x26 (EFFECT.DAT's page (384, 0), added), CLUT 0x7808
        let sprites = self.sprites.iter().map(|(otz, q, uv, semi)| {
            let z = gte.slot_depth(*otz);
            let q = PsxQuad {
                pos: q.map(|(x, y)| gte.clip([x, y], z)),
                uv: uv.map(|[u, v]| [u as f32, v as f32]),
                rgb: [[128; 3]; 4],
                tpage: tpage_word(0x26),
                clut: clut_word(128, 480),
                textured: true,
                semi: *semi,
                sheet: None,
                clip: crate::output::layout::Clip::No,
            };
            (*otz, q)
        });
        let mut v: Vec<(i32, PsxQuad)> = parts.chain(flats).chain(sprites).collect();
        v.sort_by_key(|(otz, _)| -otz);
        v.into_iter().map(|(_, q)| q).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The tables of the imported game's install's static tables beside the engine (None without it).
    fn tables() -> Option<Tables> {
        crate::formats::tables::br1_export_tables().map(|t| Tables::of(&t))
    }

    /// The effects' cheap angle as recorded on chosen inputs (run s8t: each call's a0 / a1
    /// overwritten at its entry, v0 read at its `jr ra`): every octant, the axes (0 taken as 1), large ratios.
    #[test]
    fn angle_as_recorded() {
        const CASES: [(i32, i32, i32); 25] = [
            (0, 0, 1528),
            (0, 5, 1938),
            (5, 0, 1134),
            (0, -5, 110),
            (-5, 0, 2962),
            (100, 30, 1185),
            (100, -30, 863),
            (-100, 30, 2911),
            (-100, -30, 3233),
            (30, 100, 1887),
            (-30, 100, 2209),
            (30, -100, 161),
            (-30, -100, 3934),
            (-70, -71, 3583),
            (70, 70, 1528),
            (-70, 70, 2568),
            (1, -1, 520),
            (-1, -1, 3575),
            (-1, 1, 2568),
            (-2000, 1, 3064),
            (1, -2000, 8),
            (-3, -2000, 4087),
            (-2000, -3, 3080),
            (12345, -678, 988),
            (-678, 12345, 2084),
        ];
        for (x, y, want) in CASES {
            assert_eq!(angle(x, y), want, "({x}, {y})");
        }
    }

    /// The camera's shake words as the recorded frames hold them after a hit (run s8r,
    /// MITSUKO vs FOX: the vertical word from −615, the horizontal from −125, read each frame): the word a frame reads, then its
    /// decay (v = −3v / 4, h = −(h / 4)); the view offset `word · 256 / 10000`.
    #[test]
    fn shake_decay_as_recorded() {
        const V: [i16; 15] = [-615, 461, -345, 258, -193, 144, -108, 81, -60, 45, -33, 24, -18, 13, -9];
        const H: [i16; 5] = [-125, 31, -7, 1, 0];
        for (word, words, axis) in [(CamWord::Vertical, &V[..], 1), (CamWord::Horizontal, &H[..], 0)] {
            let mut b = Br1Match::default();
            b.fx_log.push((99, 0, Request::Cam { word, value: words[0] }));
            let mut fx = Br1Fx::default();
            for k in 0..words.len() - 1 {
                let ofs = fx.shake(&b.view(), 100 + k as u64);
                assert_eq!(ofs[axis], words[k] as i32 * 256 / 10000, "{word:?} frame {k}");
                assert_eq!(fx.shake[1 - axis], words[k + 1], "{word:?} frame {k}");
                assert_eq!(ofs[1 - axis], 0);
            }
        }
    }

    /// A colour slot's steps as recorded (run s8r: slot 0 each frame): a hit's burst (record 0,
    /// gouraud, the fade 0x1000 down by 0x100) and the floor ring (record 15, flat, 0x400 down by 0x35): (step, its
    /// frames, frames left, fade) after each frame (data-gated: the imported game's install).
    #[test]
    fn colours_step_as_recorded() {
        let Some(t) = tables() else {
            eprintln!("skipped: no okb_export/br1");
            return;
        };
        let burst: [(u32, i32, i32, i16); 14] = [
            (0, 3, 2, 3840),
            (0, 3, 1, 3584),
            (1, 2, 2, 3328),
            (1, 2, 1, 3072),
            (2, 5, 5, 2816),
            (2, 5, 4, 2560),
            (2, 5, 3, 2304),
            (2, 5, 2, 2048),
            (2, 5, 1, 1792),
            (3, 4, 4, 1536),
            (3, 4, 3, 1280),
            (3, 4, 2, 1024),
            (3, 4, 1, 768),
            (0, 3, 3, 512),
        ];
        let ring: [(u32, i32, i32, i16); 8] =
            [(0, 7, 6, 971), (0, 7, 5, 918), (0, 7, 4, 865), (0, 7, 3, 812), (0, 7, 2, 759), (0, 7, 1, 706), (1, 12, 12, 653), (1, 12, 11, 600)];
        for (model, gouraud, fade, step, want) in [(0, true, 0x1000, 0x100, &burst[..]), (15, false, 0x400, 0x35, &ring[..])] {
            let mut c = Colours::new(&t, model, gouraud, fade, step, 1, 0);
            for (k, w) in want.iter().enumerate() {
                let rgb = c.step(&t).expect("runs");
                assert_eq!(rgb.len(), 1);
                assert_eq!((c.idx, c.dur, c.left, c.fade), *w, "record {model} frame {k}");
            }
        }
    }

    /// The emitters' particles a frame (`total / frames`, +0x24) and their frames, as the recorded allocations and
    /// slots (out/s8r: hits 0x106 with total 2 / 3 / 10 over 4 frames, guards total 4 over 2): a slot runs its
    /// frames (active while its count +0x25 is below them), a guard's kind 1 spawning 2 sparks a frame (data-gated).
    #[test]
    fn emitter_count_as_recorded() {
        for (total, frames, per) in [(2, 4, 0), (3, 4, 0), (10, 4, 2), (4, 2, 2)] {
            let e = Emitter::new([0; 3], 0x106, 0, 0x500, 4, total, frames, 0, 1);
            assert_eq!((e.per, e.frames), (per, frames as u8), "{total} / {frames}");
        }
        let Some(t) = tables() else {
            eprintln!("skipped: no okb_export/br1");
            return;
        };
        let mut fx = Br1Fx::default();
        let mut e = Emitter::new([0; 3], 0x106, -1939, 1280, 4, 4, 2, 0, 1);
        assert!(fx.emitter_step(&t, &Br1Match::default().view(), &mut e));
        assert_eq!(fx.live.len(), 2);
        assert!(!fx.emitter_step(&t, &Br1Match::default().view(), &mut e));
        assert_eq!(fx.live.len(), 4);
        assert!(fx.live.iter().all(|&i| fx.pool[i].ty == 6));
    }

    /// Preset 2 (the rave's): kind 19, one type-6 particle a frame for 20 frames, each still (mode 14)
    /// circling the emitter's point at its spawn distance, size 0x23 (data-gated).
    #[test]
    fn emitter_kind_19() {
        let Some(t) = tables() else {
            eprintln!("skipped: no okb_export/br1");
            return;
        };
        let mut fx = Br1Fx::default();
        let mut e = Emitter::new([100, -500, 30], 6, 0, 0, 0, 0x14, 0x14, 2, 19);
        let mut frames = 1;
        while fx.emitter_step(&t, &Br1Match::default().view(), &mut e) {
            frames += 1;
        }
        assert_eq!((frames, fx.live.len()), (20, 20));
        for &i in &fx.live {
            let p = &fx.pool[i];
            assert_eq!((p.ty, p.mode, p.size, p.p, p.clut, p.grav), (6, 14, 0x23, [100, -500, 30], 2, 0));
            assert!(p.b3a > 0 && p.b3c == p.b3a >> 3);
        }
    }

    fn gte() -> Gte {
        let view = glam::Mat4::from_translation(glam::Vec3::new(0.0, 0.0, -10.0));
        Gte::new(view, glam::Mat4::perspective_rh(1.0, 4.0 / 3.0, 0.1, 100.0), [1.0, 1.0], 0)
    }

    /// Command 5's web (+0x3EC from 40, over when it reaches 0) and task 0x16 (d80 up to d82 = 40): the
    /// 40th frame ends each (data-gated).
    #[test]
    fn web_and_pie_last_40_frames() {
        let Some(t) = tables() else {
            eprintln!("skipped: no okb_export/br1");
            return;
        };
        let b = Br1Match::default();
        let f = Fighter::of(&b.fs[0]);
        let (g, mut rnd) = (gte(), Rand { seed: 1, tidx: 0 });
        let mut w = Web::new(&t, 0, 3);
        let mut sprites = Vec::new();
        let web = (1..100).find(|_| !w.step(&t, &g, &f, &mut rnd, &mut sprites));
        let mut p = Pie::new(&t, 0, 3, &mut rnd);
        let pie = (1..100).find(|_| p.step(&t, &g, &f).is_none());
        assert_eq!((web, pie), (Some(40), Some(40)));
    }

    /// Op 0x17 sub 2's argument `a`: the case in its top nibble, the node next (mirrored by the mirror table when S.128 is
    /// set), then p and q: case 3 a glow of size p · 16, case 5 an emitter of kind p + 8 on node n + 1 for q frames
    /// (none for the unimplemented kinds past 13), case 15 the web at node n (data-gated).
    #[test]
    fn cbb4_decode_and_mirror() {
        let Some(t) = tables() else {
            eprintln!("skipped: no okb_export/br1");
            return;
        };
        let call = |fx: &mut Br1Fx, b: &Br1Scene, a: i32| fx.request(&t, b, 0, &Request::Call { call: Call::NodeEffect, args: [0, a, 0, 0, 0, 0] });
        let mut b = Br1Match::default();
        let mut fx = Br1Fx::default();
        call(&mut fx, &b.view(), 0x32F0);
        assert_eq!(fx.live.iter().map(|&i| (fx.pool[i].ty, fx.pool[i].size)).collect::<Vec<_>>(), [(0, 0xF0)]);
        call(&mut fx, &b.view(), 0x5345);
        call(&mut fx, &b.view(), 0x5367);
        let a: Vec<_> = fx.attached.iter().flatten().map(|a| (a.node, a.kind, a.count)).collect();
        assert_eq!(a, [(4, 12, 5)], "kind 6 + 8 = 14 not made");
        call(&mut fx, &b.view(), 0xF500u32 as i16 as i32);
        assert_eq!(fx.webs.iter().map(|w| w.node).collect::<Vec<_>>(), [5], "0xF500 sign-extended: case 15, node 5");
        b.fs[0].anim.mirror = true;
        call(&mut fx, &b.view(), 0xF500u32 as i16 as i32);
        assert_eq!(fx.webs[1].node, t.tables.abs_u8(t.b(0) + 5) as i16, "mirrored");
    }

    /// Uriko's drop (op 0x17 sub 2's case 13, mode 12: `drop_step` with `tip`) against a recording: each
    /// record the recording logged at the update's entry, with its fighter's node 19 and chr / form, stepped once gives the
    /// record it logged at the exit (`fixtures/br1_drop.txt`, run s11e: 88 frames of three drops, from their
    /// hanging through their fall), or dies where the recorded one did, its far end on the floor (data-gated: the static tables'
    /// tip table). The capture's last entry (frame 2240: no exit, no later record, its far end at −298, so
    /// not this rule's death; the fighter's status changes about then) is left out.
    #[test]
    fn drop_as_recorded() {
        let Some(t) = tables() else {
            eprintln!("skipped: no okb_export/br1");
            return;
        };
        let hex = |s: &str| -> Vec<u8> { (0..s.len()).step_by(2).map(|k| u8::from_str_radix(&s[k..k + 2], 16).unwrap()).collect() };
        let h = |b: &[u8], o: usize| i16::from_le_bytes([b[o], b[o + 1]]) as i64;
        let w = |b: &[u8], o: usize| i32::from_le_bytes([b[o], b[o + 1], b[o + 2], b[o + 3]]) as i64;
        let lines: Vec<Vec<&str>> = include_str!("fixtures/br1_drop.txt").lines().filter(|l| !l.starts_with('#')).map(|l| l.split(' ').collect()).collect();
        let (mut stepped, mut died) = (0, 0);
        for (k, l) in lines.iter().enumerate().filter(|(k, l)| l[0] == "in" && k + 1 < lines.len()) {
            let (chr, form, node, rec) = (l[6].parse::<u8>().unwrap(), l[8].parse::<u8>().unwrap(), hex(l[10]), hex(l[12]));
            let mut md = vec![crate::fight::br1::object::Node::default(); 20];
            md[19].wm = std::array::from_fn(|i| std::array::from_fn(|j| h(&node, 2 * (3 * i + j)) as i32));
            md[19].wt = std::array::from_fn(|i| w(&node, 0x14 + 4 * i) as i32);
            let f = Fighter { md: &md, chr, form, player: 1 };
            let other = Fighter { md: &[], chr: 0, form: 0, player: 0 };
            let mut pt = Particle::blank();
            pt.mode = 12;
            pt.p = [h(&rec, 0), h(&rec, 2), h(&rec, 4)];
            pt.vel = [w(&rec, 0x14), w(&rec, 0x18), 0];
            (pt.b3a, pt.b3c, pt.b3e, pt.b40) = (h(&rec, 0x3A), h(&rec, 0x3C), h(&rec, 0x3E), h(&rec, 0x40));
            pt.fighter = Some(1);
            let alive = drop_step(&t, &mut pt, &[other, f]);
            match lines.get(k + 1).filter(|n| n[0] == "out") {
                Some(n) => {
                    let out = hex(n[4]);
                    assert!(alive, "line {k}: died, the original's lived");
                    let got = (pt.p, pt.vel[0] as i32 as i64, pt.vel[1] & 0xFFFF, [pt.b3a, pt.b3c, pt.b3e]);
                    let want = ([h(&out, 0), h(&out, 2), h(&out, 4)], w(&out, 0x14), w(&out, 0x18) & 0xFFFF, [h(&out, 0x3A), h(&out, 0x3C), h(&out, 0x3E)]);
                    assert_eq!(got, want, "line {k}");
                    stepped += 1;
                }
                None => {
                    assert!(!alive, "line {k}: lived, the original's died");
                    died += 1;
                }
            }
        }
        assert_eq!((stepped, died), (88, 2));
    }

    /// Op 0x17 sub 2's cases 8 to 11 (read from the code; their states, 53 and 60..62 of the four fighters and Uriko's 311, did
    /// not come up in the recorded runs): 8 with n = 0 makes nothing, with n = 1 an emitter of kind 6 on node 0x13 laying
    /// one type-10 particle (mode 6, size 0x50) a frame for 0x28 frames; 9 a task 0x21 drawn 6 frames; 10 two trails
    /// (type 0x11) and two node emitters; 11 an emitter of kind 16 for 0x1E frames (nothing laid while the node stays)
    /// (data-gated).
    #[test]
    fn cbb4_cases_8_to_11() {
        let Some(t) = tables() else {
            eprintln!("skipped: no okb_export/br1");
            return;
        };
        let call =
            |fx: &mut Br1Fx, b: &Br1Scene, a: u32| fx.request(&t, b, 0, &Request::Call { call: Call::NodeEffect, args: [0, a as i16 as i32, 0, 0, 0, 0] });
        let b = Br1Match::default();
        let mut fx = Br1Fx::default();
        call(&mut fx, &b.view(), 0x8000);
        assert!(fx.emitters.is_empty(), "case 8, n 0: the blood option's");
        call(&mut fx, &b.view(), 0x8100);
        let e = fx.emitters[0];
        assert_eq!((e.kind, e.ty, e.clut, e.node, e.flag, e.frames), (6, 0x0A, 6, 0x13, 1, 0x28));
        let mut es = std::mem::take(&mut fx.emitters);
        let frames = (1..100).find(|_| !fx.emitter_step(&t, &b.view(), &mut es[0])).unwrap();
        assert_eq!((frames, fx.live.len()), (0x28, 0x28.min(POOL - 1)));
        assert!(fx.live.iter().all(|&i| (fx.pool[i].ty, fx.pool[i].mode, fx.pool[i].size, fx.pool[i].fighter) == (0x0A, 6, 0x50, Some(0))));
        let mut fx = Br1Fx::default();
        call(&mut fx, &b.view(), 0x9300);
        let Some(Task::Pulse(p)) = fx.tasks.first_mut() else { panic!("case 9: no task 0x21") };
        let f = Fighter::of(&b.fs[0]);
        let drawn = (0..20).take_while(|_| p.step(&t, &gte(), &f).is_some()).count();
        assert_eq!(drawn, 6);
        let mut fx = Br1Fx::default();
        call(&mut fx, &b.view(), 0xA000);
        assert_eq!(fx.tasks.iter().filter(|k| matches!(k, Task::Trail(_))).count(), 2, "case 10: two trails");
        assert_eq!(fx.attached.iter().flatten().map(|a| (a.node, a.kind)).collect::<Vec<_>>(), [(7, 8), (10, 8)]);
        let mut fx = Br1Fx::default();
        call(&mut fx, &b.view(), 0xB700);
        let mut es = std::mem::take(&mut fx.emitters);
        assert_eq!((es[0].kind, es[0].ty, es[0].node), (16, 0x12, 7));
        let frames = (1..100).find(|_| !fx.emitter_step(&t, &b.view(), &mut es[0])).unwrap();
        assert_eq!((frames, fx.live.len()), (0x1E, 0));
    }

    /// The team battle's four (T5): a request's effects go on the fighter its log entry is tagged with, not on the
    /// place imported passes (fighter 2 at place 0, fighter 3 at place 1): op 0x17 sub 2's case 9 task and back colour
    /// script, an attack's trail (task type 0x0F) and claws (type 0x10), an effect script, the move
    /// cancel (by player) (data-gated).
    #[test]
    fn team_requests_on_the_tagged_fighter() {
        let Some(t) = tables() else {
            eprintln!("skipped: no okb_export/br1");
            return;
        };
        let fs = vec![crate::fight::br1::object::Br1Fighter::default(); 4];
        let b = Br1Scene { side: &[true; 4], fs: &fs, fx_log: &[] };
        let mut fx = Br1Fx::default();
        fx.request(&t, &b, 2, &Request::Call { call: Call::NodeEffect, args: [0, 0x9300, 0, 0, 0, 0] });
        assert!(matches!(fx.tasks.as_slice(), [Task::Pulse(p)] if p.side == 2));
        assert_eq!(fx.backs.each_ref().map(|s| s.is_some()), [false, false, true, false]);
        let mut fx = Br1Fx::default();
        fx.request(&t, &b, 3, &Request::Call { call: Call::Task, args: [0x1000F, 1, 0, 5, 6, 7] });
        fx.request(&t, &b, 2, &Request::Call { call: Call::Task, args: [0x10, 0, 0, 5, 6, 0] });
        let sides: Vec<usize> = fx
            .tasks
            .iter()
            .map(|k| match k {
                Task::Trail(k) => k.side,
                Task::Claw(k) => k.side,
                _ => panic!("a trail or claws"),
            })
            .collect();
        assert_eq!(sides, [3, 2]);
        fx.request(&t, &b, 3, &Request::Call { call: Call::EffectScript, args: [0, 1, 0, 0, 0, 0] });
        assert_eq!(fx.scripts[0].as_ref().map(|s| s.side), Some(3));
        assert_eq!(fx.backs.each_ref().map(|s| s.is_some()), [false, false, false, true]);
        fx.request(&t, &b, 3, &Request::Call { call: Call::MoveCancel, args: [1, 0, 0, 0, 0, 0] });
        assert_eq!(fx.cancel, [0, 0, 0, 2]);
    }
}
