//! The reference runner's core (port addition, debug only, never a game mode): imported fighters loaded from the
//! original's snapshots, the port's script step run on them and the result compared field by field.
//!
//! Original: the snapshots `.research/br1/specs/s4/steplog.lua` takes in PCSX-Redux at every entry of the script step
//! 0x8018118C and at its return into 0x8017B608 (0x8017BFB8), both fighters each time
//! Spec: docs/specs/br1_script_ops.md §16
//! State: none (reads byte slices; `bin/br1test.rs` reads the files)
//!
//! Between a step's entry and its return nothing else runs, so the difference of the two snapshots is exactly what
//! the step wrote. `Br1Fighter`'s serde form names every field, so a diff of two fighters is a list of field paths.

use super::object::{Block, Br1Fighter, FormDesc, Node, Sphere, State};
use super::pose::{Anim, M, Parts, Rec, Src};
use super::res::Br1Res;
use super::script::{Call, Ctx, Request};
use serde_json::Value;

const SPAD: usize = 0x400;
const STATES: usize = 0x280;
const NODES: usize = 26;
const FORM: usize = NODES * (0x18 + 0x50 + 4);
const FIGHTER: usize = 0x38 + 2 * FORM + 0x20;
/// One snapshot record of `steps.bin`.
pub const RECORD: usize = 16 + SPAD + STATES + 4 + 8 + 16 + 16 + 2 * FIGHTER;
/// The fighter blocks F of P1 / P2 in the scratchpad.
pub const FIGHTERS: [u32; 2] = [0x1F80_00F4, 0x1F80_01E0];

fn u16at(b: &[u8], o: usize) -> u16 {
    b.get(o..o + 2).map_or(0, |s| u16::from_le_bytes([s[0], s[1]]))
}
fn i16at(b: &[u8], o: usize) -> i16 {
    u16at(b, o) as i16
}
fn u32at(b: &[u8], o: usize) -> u32 {
    b.get(o..o + 4).map_or(0, |s| u32::from_le_bytes([s[0], s[1], s[2], s[3]]))
}

/// One fighter's RAM in a snapshot.
pub struct SnapFighter<'a> {
    pub desc: &'a [u8],
    /// per form: pose records, model nodes, objects' +0xC / +0xE halfwords
    pub forms: [(&'a [u8], &'a [u8], &'a [u8]); 2],
    pub spheres: &'a [u8],
}

/// One snapshot: tag 1 = entry of the step, 2 = its return.
pub struct Snap<'a> {
    pub tag: u32,
    pub frame: u32,
    /// the fighter whose step it is
    pub side: usize,
    pub spad: &'a [u8],
    pub states: &'a [u8],
    pub rand: u8,
    pub walls: [i16; 4],
    pub cam: &'a [u8],
    pub gbc: &'a [u8],
    pub fighters: [SnapFighter<'a>; 2],
}

/// The records of a `steps.bin`.
pub fn parse(b: &[u8]) -> Result<Vec<Snap<'_>>, String> {
    if !b.len().is_multiple_of(RECORD) {
        return Err(format!("{} bytes: not a whole number of {RECORD}-byte records", b.len()));
    }
    let mut out = Vec::with_capacity(b.len() / RECORD);
    for r in b.as_chunks::<RECORD>().0 {
        if u32at(r, 0) != 0x4352_3453 {
            return Err("not a steps.bin record (magic)".into());
        }
        let fb = u32at(r, 12);
        if !FIGHTERS.contains(&fb) {
            return Err(format!("fighter {fb:#x}"));
        }
        out.push(record(r));
    }
    Ok(out)
}

/// One snapshot of `RECORD` bytes (the magic not checked; `side` 0 when the record names no fighter).
pub fn record(r: &[u8; RECORD]) -> Snap<'_> {
    let fb = u32at(r, 12);
    let side = FIGHTERS.iter().position(|&a| a == fb).unwrap_or(0);
    let mut o = 16;
    let mut take = |n: usize| {
        let s = &r[o..o + n];
        o += n;
        s
    };
    let spad = take(SPAD);
    let states = take(STATES);
    let rand = take(4)[0];
    let w = take(8);
    let walls = std::array::from_fn(|i| i16at(w, 2 * i));
    let cam = take(16);
    let gbc = take(16);
    let mut fighter = || {
        let desc = take(0x38);
        let mut form = || (take(NODES * 0x18), take(NODES * 0x50), take(NODES * 4));
        let forms = [form(), form()];
        SnapFighter { desc, forms, spheres: take(0x20) }
    };
    let fighters = [fighter(), fighter()];
    Snap { tag: u32at(r, 4), frame: u32at(r, 8), side, spad, states, rand, walls, cam, gbc, fighters }
}

impl Snap<'_> {
    /// Fighter k's block F.
    pub fn f(&self, k: usize) -> &[u8] {
        let o = (FIGHTERS[k] & 0x3FF) as usize;
        &self.spad[o..o + 0xEC]
    }
    /// Fighter k's script state S.
    pub fn s(&self, k: usize) -> &[u8] {
        &self.states[0x140 * k..0x140 * (k + 1)]
    }
    /// The fighter's character (F.03).
    pub fn chr(&self, k: usize) -> usize {
        self.f(k)[3] as usize
    }
}

fn mat(b: &[u8], o: usize) -> M {
    std::array::from_fn(|i| std::array::from_fn(|j| i16at(b, o + 6 * i + 2 * j) as i32))
}
fn v3(b: &[u8], o: usize) -> [i16; 3] {
    std::array::from_fn(|i| i16at(b, o + 2 * i))
}

/// Fighter `k` of snapshot `sn` as the port's state; `res` by side.
pub fn load(sn: &Snap, k: usize, res: [&Br1Res; 2]) -> Br1Fighter {
    let (fb, sb) = (sn.f(k), sn.s(k));
    let r = res[k];
    let base = u32at(fb, 0x2C);
    let obase = u32at(sn.f(1 - k), 0x2C);
    let cmn = u32at(sb, 0xB0).wrapping_sub(8);
    let olen = res[1 - k].anm.bytes.len();
    // a pointer into the own ANM, CMN.ACT or the opponent's ANM → (source, file offset)
    let loc = |a: u32| -> Option<(Src, usize)> {
        let inr = |b: u32, n: usize| a.wrapping_sub(b) < n as u32;
        if inr(base, r.anm.bytes.len()) {
            Some((Src::Own, (a - base) as usize))
        } else if inr(cmn, r.cmn.len()) {
            Some((Src::Cmn, (a - cmn) as usize))
        } else if inr(obase, olen) {
            Some((Src::Opp, (a - obase) as usize))
        } else {
            // the EXE image through the RAM mirrors (`script::ops::image_clip`)
            super::address_map::image_offset(a, r.image_len).map(|o| (Src::Image, o))
        }
    };
    let b8 = u32at(sb, 0xB8);
    let (src, clip) = if b8 == 0 { (Src::Own, None) } else { loc(b8).map_or((Src::Own, None), |(s, o)| (s, Some(o))) };
    let anim = Anim {
        flip_next: sb[0xA4] != 0,
        id: i16at(sb, 0xAA),
        src,
        clip,
        // (a null clip's target points into low RAM: kept as the address)
        target: {
            let c0 = u32at(sb, 0xC0);
            loc(c0).map_or(if c0 < 0x10 { c0 as usize } else { 0 }, |(_, o)| o)
        },
        root_scale: i16at(sb, 0xC4),
        scale: i16at(sb, 0xC6),
        keep_root: i16at(sb, 0xC8),
        total: i16at(sb, 0xCA),
        elapsed: i16at(sb, 0xCC),
        key: i16at(sb, 0xCE),
        d0: i16at(sb, 0xD0),
        left: i16at(sb, 0xD2),
        next_dur: i16at(sb, 0xD4),
        speed: i16at(sb, 0xD6),
        yaw_base: i16at(sb, 0xD8),
        yaw: i16at(sb, 0xDA),
        pos: v3(sb, 0xDC),
        root: v3(sb, 0xE4),
        bind1: v3(sb, 0xEC),
        t1: v3(sb, 0xF4),
        bind4: v3(sb, 0xFC),
        t4: v3(sb, 0x104),
        turn: i16at(sb, 0x10C),
        mirror: sb[0x128] != 0,
        head_yaw: i16at(sb, 0x12E),
        head_pitch: i16at(sb, 0x130),
    };
    let sf = &sn.fighters[k];
    let d = sf.desc;
    let desc: [FormDesc; 2] = std::array::from_fn(|i| FormDesc {
        objects: d[0x1C * i],
        nodes: d[0x1C * i + 1],
        body_spheres: d[0x1C * i + 2],
        hit_spheres: d[0x1C * i + 3],
        weight: i16at(d, 0x1C * i + 0x18),
        factor: i16at(d, 0x1C * i + 0x1A),
    });
    let md_ptr = u32at(fb, 0x1C);
    let cur = (0..2).find(|&i| u32at(d, 0x1C * i + 0x10) == md_ptr).unwrap_or(0) as u8;
    let mut parts = Parts { hidden: [0; 2] };
    let forms = std::array::from_fn(|i| {
        let (rb, mb, ob) = sf.forms[i];
        let n = (desc[i].nodes as usize).min(NODES);
        for o in 0..NODES {
            if u16at(ob, 4 * o) != 0 {
                parts.hidden[i] |= 1 << o;
            }
        }
        super::object::Form {
            rec: (0..n).map(|j| Rec { dir: v3(rb, 0x18 * j), twist: i16at(rb, 0x18 * j + 6) }).collect(),
            md: (0..n)
                .map(|j| {
                    let o = 0x50 * j;
                    Node {
                        lm: mat(mb, o + 4),
                        lt: v3(mb, o + 0x18).map(|v| v as i32),
                        wm: mat(mb, o + 0x20),
                        wt: std::array::from_fn(|c| u32at(mb, o + 0x34 + 4 * c) as i32),
                    }
                })
                .collect(),
        }
    });
    let spheres = std::array::from_fn(|i| Sphere { flag: sf.spheres[4 * i] as i8, node: sf.spheres[4 * i + 1], radius: u16at(sf.spheres, 4 * i + 2) });
    Br1Fighter {
        f: Block::read(fb),
        s: State::read(sb),
        pc: u32at(sb, 0x8C).wrapping_sub(base),
        loop_mark: u32at(sb, 0x90).wrapping_sub(base),
        ret: u32at(sb, 0x94).wrapping_sub(base),
        anim,
        spheres,
        cur,
        forms,
        desc,
        parts,
        ..Default::default()
    }
}

/// The match globals of a snapshot for the step.
pub struct Globals {
    pub stage: i8,
    pub round: i8,
    pub walls: [bool; 4],
    pub need: u8,
    pub g_ad: u8,
    pub g_e8: u32,
}

pub fn globals(sn: &Snap) -> Globals {
    Globals {
        stage: sn.spad[0x94] as i8,
        round: sn.spad[0x95] as i8,
        walls: sn.walls.map(|w| w >= 0),
        need: sn.gbc[4],
        g_ad: sn.spad[0xAD],
        g_e8: u32at(sn.spad, 0xE8),
    }
}

/// Run fighter `me`'s script step (0x8018118C) on `fs`: the requests made and the random counter after it; the pc of
/// every instruction run into `trace`.
pub fn run_step(fs: &mut [Br1Fighter; 2], me: usize, res: [&Br1Res; 2], g: &Globals, rand: u8, trace: &mut Vec<u32>) -> (Vec<(u8, Request)>, u8) {
    let mut out = Vec::new();
    let mut r = rand;
    let mut ctx = Ctx {
        res,
        rand: &mut r,
        stage: g.stage,
        round: g.round,
        walls: g.walls,
        need: g.need,
        g_ad: g.g_ad,
        g_e8: g.g_e8,
        out: &mut out,
        trace: Some(trace),
        rings: None,
    };
    super::script::step(fs, me, &mut ctx);
    (out, r)
}

/// The paths (`f.f54`, `anim.left`, `forms[0].md[3].lm`, …) where `a` and `b` differ, with both values.
pub fn diff(a: &Br1Fighter, b: &Br1Fighter) -> Vec<(String, String, String)> {
    let (va, vb) = (serde_json::to_value(a).unwrap_or(Value::Null), serde_json::to_value(b).unwrap_or(Value::Null));
    let mut out = Vec::new();
    walk(&va, &vb, String::new(), &mut out);
    out
}

fn walk(a: &Value, b: &Value, path: String, out: &mut Vec<(String, String, String)>) {
    match (a, b) {
        (Value::Object(x), Value::Object(y)) => {
            for (k, v) in x {
                let p = if path.is_empty() { k.clone() } else { format!("{path}.{k}") };
                walk(v, y.get(k).unwrap_or(&Value::Null), p, out);
            }
        }
        // matrices and 3-vectors as one value; longer lists element by element
        (Value::Array(x), Value::Array(y))
            if x.len() == y.len() && x.iter().any(|v| v.is_object() || v.is_array() && v.as_array().is_some_and(|w| w.len() > 3)) =>
        {
            for (i, (v, w)) in x.iter().zip(y).enumerate() {
                walk(v, w, format!("{path}[{i}]"), out);
            }
        }
        _ if a != b => out.push((path, a.to_string(), b.to_string())),
        _ => {}
    }
}

/// `port` with every field where `prev` and `now` (two of the original's states) differ taken from `now`: what the
/// rest of the frame changed between two steps, fed into the port's own state.
pub fn feed(port: &Br1Fighter, prev: &Br1Fighter, now: &Br1Fighter) -> Br1Fighter {
    let (mut p, a, b) = (
        serde_json::to_value(port).unwrap_or(Value::Null),
        serde_json::to_value(prev).unwrap_or(Value::Null),
        serde_json::to_value(now).unwrap_or(Value::Null),
    );
    patch(&mut p, &a, &b);
    serde_json::from_value(p).unwrap_or_else(|_| now.clone())
}

fn patch(p: &mut Value, a: &Value, b: &Value) {
    if a == b {
        return;
    }
    match (p, a, b) {
        (Value::Object(x), Value::Object(y), Value::Object(z)) => {
            for (k, v) in z {
                match (x.get_mut(k), y.get(k)) {
                    (Some(xv), Some(yv)) => patch(xv, yv, v),
                    (_, _) => {
                        x.insert(k.clone(), v.clone());
                    }
                }
            }
        }
        (Value::Array(x), Value::Array(y), Value::Array(z)) if x.len() == z.len() && y.len() == z.len() => {
            for ((xv, yv), zv) in x.iter_mut().zip(y).zip(z) {
                patch(xv, yv, zv);
            }
        }
        (p, _, b) => *p = b.clone(),
    }
}

/// One call the original made inside a step (`events.txt`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Event {
    pub frame: u32,
    pub fighter: u32,
    pub name: String,
    /// a0..a3, [sp+0x10], [sp+0x14]
    pub args: [u32; 6],
    /// the return address: the caller
    pub ra: u32,
}

/// `events.txt`'s lines.
pub fn parse_events(text: &str) -> Vec<Event> {
    text.lines()
        .filter_map(|l| {
            let w: Vec<&str> = l.split_whitespace().collect();
            let hex = |s: &str| u32::from_str_radix(s.split('=').nth(1)?, 16).ok();
            let args = [hex(w.get(3)?)?, hex(w.get(4)?)?, hex(w.get(5)?)?, hex(w.get(6)?)?, hex(w.get(7)?)?, hex(w.get(8)?)?];
            let ra = hex(w.get(9)?)?;
            Some(Event { frame: w.first()?.parse().ok()?, fighter: u32::from_str_radix(w.get(1)?, 16).ok()?, name: w.get(2)?.to_string(), args, ra })
        })
        .collect()
}

/// The calls of the original that are `Request`s in the port, as the port's requests: fighter block pointers → the
/// side, model node pointers → the node (`md` = each fighter's current model nodes address, F.1C). Only the calls
/// the script code makes (the handlers and the step, 0x8017D508..0x80181754); the emitters' own calls are theirs.
pub fn event_request(e: &Event, md: [u32; 2]) -> Option<Request> {
    if !(0x8017_D508..0x8018_1754).contains(&e.ra) {
        return None;
    }
    let side = |p: u32| FIGHTERS.iter().position(|&a| a == p).map_or(p as i32, |s| s as i32);
    let node = |p: u32| {
        md.iter().find_map(|&m| {
            let d = p.wrapping_sub(m.wrapping_add(0x34));
            (d < 0x50 * NODES as u32 && d % 0x50 == 0).then_some((d / 0x50) as i32)
        })
    };
    let a = e.args.map(|v| v as i32);
    let call = match e.name.as_str() {
        "sound" => return Some(Request::Sound { id: a[1] as i16, delay: a[2] as i16 }),
        "fx40880" => return Some(Request::Call { call: Call::Task, args: [a[0], side(e.args[1]), a[2], a[3], a[4], a[5]] }),
        "fx3FB24" => return Some(Request::Call { call: Call::FloorEffect, args: [node(e.args[0])?, a[1], a[2], a[3], 0, 0] }),
        "fx3E3BC" => return Some(Request::Call { call: Call::NodeEmitter, args: [node(e.args[0])?, a[1], a[2], a[3], 0, 0] }),
        "fx3EA60" => return Some(Request::Call { call: Call::BackColourScript, args: [a[0], a[1], a[2], 0, 0, 0] }),
        "fx4B3C4" => return Some(Request::Call { call: Call::EffectScript, args: [a[0], side(e.args[1]), 0, 0, 0, 0] }),
        "fx4CB7C" => Call::MoveCancel,
        "fx4CBB4" => return Some(Request::Call { call: Call::NodeEffect, args: [side(e.args[0]), a[1], 0, 0, 0, 0] }),
        "fx4D370" => return Some(Request::Call { call: Call::SparkPreset, args: [side(e.args[0]), a[1], 0, 0, 0, 0] }),
        "fx37A80" => Call::StageSound,
        "rave" => return Some(Request::Call { call: Call::BackColour, args: [a[0], a[1], a[2], a[3], 0, 0] }),
        _ => return None,
    };
    Some(Request::Call { call, args: [a[0], 0, 0, 0, 0, 0] })
}
