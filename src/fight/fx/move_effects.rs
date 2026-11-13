//! The native move effects started by move-script ops 0x17 / 0x18 / 0x0D.
//!
//! State: serialised: part of `GameState` (field changes are `STATE_VERSION` changes)
//!
//! The remaining native move effects: plain C tasks started by move-script ops 0x17 / 0x18 / 0x0D.
//! Sparkle sweep (op17 sub 0), screen rays (sub 5), converging specks (sub 6),
//! rising background streaks (sub A), orb burst (sub B), claw / kick trails
//! (op18 type D and the automatic trail of op 0x0D) and Gado's ground comet (op18 type C, a = 1).
//! Logic runs in `tick`; `prims` turns the state into screen prims with the current camera.

use crate::fight::fx::effect_models::ModelPrim;
use crate::fight::fx::hit_effects::{Gte, rcos, rsin};
use crate::fight::fx::host::FxHost;
use crate::fight::fx::script_link::bonepos;
use crate::fight::fx::script_vm::{M3, Vm};
use crate::fight::side::{Bank, Fid, bank_bit};
use crate::formats::tables::Tables;
use serde::{Deserialize, Serialize};

const H: i32 = 360;

/// The move effects' data (`fx.move_effects`): the ring children's UV rectangles (+0), the trails' bone offsets (+0x30
/// the wrists and ankles, +0x60 + 0x30 per character the rest), their colours (+0x270, +0x2A8 Uriko's), the sparkle
/// schedules' addresses (+0x428), the rays' speeds (+0x444) and widths (+0x450), the streaks' CLUTs (+0x45C).
pub(crate) const MOVE_EFFECTS: &str = "fx.move_effects";

/// MIPS `div` (truncation, division by 0 → ∓1).
fn tdiv(a: i32, b: i32) -> i32 {
    if b == 0 {
        return if a >= 0 { -1 } else { 1 };
    }
    a.wrapping_div(b)
}

fn s16(v: i32) -> i32 {
    v as i16 as i32
}

fn bone_pos(vm: &Vm, side: usize, b: usize) -> [i32; 3] {
    let a = bonepos(side, b);
    [0, 1, 2].map(|k| vm.mem.s32(a + 4 * k))
}

fn bone_mat(vm: &Vm, side: usize, b: usize) -> M3 {
    vm.read_m3(bonepos(side, b) - 0x14)
}

/// atan_xz: (x, z) ∝ (sin a, −cos a).
fn atan_xz(x: i32, y: i32) -> i32 {
    let (x, y) = (if x == 0 { 1 } else { x }, if y == 0 { 1 } else { y });
    let (ax, ay) = (x.abs(), y.abs());
    if ay < ax {
        let n = if y > 0 { (y << 9) + (ax << 3) } else { (y << 9) - (ax << 3) };
        let v = n / ax;
        if x > 0 { v + 0x400 } else { 0xC00 - v }
    } else {
        let n = if x > 0 { (x << 9) + (ay << 3) } else { (x << 9) - (ay << 3) };
        let v = n / ay;
        if y > 0 {
            0x800 - v
        } else if x > 0 {
            v
        } else {
            v + 0xFFF
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct Sparkle {
    side: usize,
    recs: Vec<[i16; 4]>,
    cur: usize,
    frame: i16,
    loop_n: i16,
    mir: bool,
    clut: i32,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct Rays {
    side: usize,
    bone: usize,
    life: i32,
    ang: [i32; 6],
    spd: [i32; 6],
    wid: [i32; 6],
    /// the angles / widths drawn this frame (before this frame's step) and the joint
    draw: ([i32; 6], [i32; 6], [i32; 3]),
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct Converge {
    side: usize,
    bone: usize,
    life: i32,
    off: [[i32; 2]; 16],
    k: [i32; 16],
    dk: [i32; 16],
    at: [i32; 3],
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct Streaks {
    life: i32,
    x: [i32; 30],
    y: [i32; 30],
    spd: [i32; 30],
    cluts: [u16; 8],
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct Orbs {
    t: i32,
    size: [i32; 10],
    p: [[i32; 3]; 10],
    v: [[i32; 3]; 10],
    rgb: [[u8; 10]; 2],
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct Trail {
    side: usize,
    bone: usize,
    life: i32,
    delay: i32,
    nseg: i32,
    scale: i32,
    fade_t: i32,
    fade_dt: i32,
    fade_n: i32,
    off: [[i32; 3]; 6],
    /// per ribbon: cur tip, cur base, prev tip, prev base, prev2 tip, prev2 base
    e: [[[i32; 3]; 6]; 3],
    /// per ribbon: 18 points (4 slots of 4 + the wrap copy)
    q: [[[i32; 3]; 18]; 3],
    /// slot point index newest → oldest
    idx: [usize; 4],
    rgb: [[[u8; 3]; 21]; 2],
    col: [[i32; 3]; 7],
    clut: u16,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct Comet {
    t: i32,
    t0: i32,
    vx: i32,
    vz: i32,
    pts: [[i32; 3]; 3],
    rgb: [u8; 2],
    clut: u16,
    v0: u8,
}

/// Blood from a joint, one record of `<CHR>EFT.DAT` entry 20.
#[derive(Clone, Debug, Serialize, Deserialize)]
struct Blood {
    side: usize,
    life: i32,
    acc: i32,
    rate: i32,
    bone: usize,
    size: i32,
    dsize: i32,
    yaw: i32,
    dyaw: i32,
    pitch: i32,
    dpitch: i32,
    speed: i32,
    dspeed: i32,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
enum Task {
    Blood(Blood),
    Sparkle(Sparkle),
    Rays(Rays),
    Converge(Box<Converge>),
    Streaks(Box<Streaks>),
    Orbs(Box<Orbs>),
    Trail(Box<Trail>),
    Comet(Comet),
    /// a comet of the team battle's fighters 2 / 3 (engine addition, T4): VRAM bank 1, CLUT bit 15 set at `prims` (the
    /// task keeps the game's CLUT; a variant of its own, so a match's `Comet` serialises as before)
    CometBank1(Comet),
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct State {
    tasks: Vec<Task>,
}

fn rand(m: &mut FxHost) -> i32 {
    m.fx.rand()
}

/// Whether `bone` is a halfword sign-extended to a `usize` (`arg as i16 as usize`, op 0x17 subs 5 / 6).
fn halfword(bone: usize) -> bool {
    i16::try_from(bone as isize).is_ok()
}

impl State {
    /// A decoded state's tasks fit a fight of `n` fighters and the move effects' data (engine addition: `Match::check`,
    /// `TeamFight::check`): every fighter index below `n`; a sparkle's schedule one of the data's, its record in it and
    /// no loop count (the data's schedules hold no 0xFC00 loop, so `sparkle_tick` never runs off the end or round in
    /// circles); a trail's slot points inside its 18 (`trail_tick`, `prims`: `n + 5`); each bone within what its op can
    /// give (R2: `bonepos` stays in range): blood's 0..=23 (its clamp), a trail's a byte (op 0x0D's node, the 0x7F
    /// field), rays' and converge's a halfword sign-extended (the op's argument, `arg as i16 as usize`: the game reads it signed,
    /// with an arithmetic shift, `bonepos` wraps as its address add does).
    pub fn check(&self, n: usize, tables: &Tables) -> Result<(), String> {
        let ok = |t: &Task| match t {
            Task::Blood(b) => b.side < n && b.bone <= 23,
            Task::Sparkle(s) => s.side < n && s.cur < s.recs.len() && s.loop_n == 0 && (0..3).any(|i| schedule(tables, i) == s.recs),
            Task::Rays(r) => r.side < n && halfword(r.bone),
            Task::Converge(c) => c.side < n && halfword(c.bone),
            Task::Trail(t) => t.side < n && t.bone <= 0xFF && t.idx.iter().all(|&i| i <= 12),
            Task::Streaks(_) | Task::Orbs(_) | Task::Comet(_) | Task::CometBank1(_) => true,
        };
        if self.tasks.iter().all(ok) { Ok(()) } else { Err("move effects".into()) }
    }
}

/// Sparkle schedule `idx` (0..3): records up to the 0xFF00 end, at most 64.
fn schedule(tables: &Tables, idx: u32) -> Vec<[i16; 4]> {
    let mut a = tables.table(MOVE_EFFECTS).u32(0x428 + 4 * idx as usize);
    let mut recs = Vec::new();
    for _ in 0..64 {
        let r = [0, 1, 2, 3].map(|k| tables.abs_i16(a + 2 * k));
        recs.push(r);
        if r[0] as u16 & 0xFF00 == 0xFF00 {
            break;
        }
        a += 8;
    }
    recs
}

/// op17 sub 0: sprites popping on the joints along schedule `a1 & 0x7F`, CLUT row `clut`.
pub fn sparkle(m: &mut FxHost, tables: &Tables, side: usize, a1: u8, clut: i32) {
    let idx = (a1 & 0x7F) as u32;
    if idx >= 3 {
        return;
    }
    let recs = schedule(tables, idx);
    m.fxn.mv.tasks.push(Task::Sparkle(Sparkle { side, recs, cur: 0, frame: 0, loop_n: 0, mir: a1 & 0x80 != 0, clut }));
}

/// op17 sub 7 → blood_spawn(F, ch, lo): record `lo & 0x7F` of character `ch`'s blood table.
pub fn blood(m: &mut FxHost, data: &crate::fight::fight_data::Loaded, side: usize, ch: i32, lo: u8) {
    use crate::fight::fighter::F58;
    if m.fighters.iter().any(|g| g.flags[F58] & 0xC000_0000 != 0 && g.form == 0) {
        return;
    }
    // EffTab[59] / [79]: entry 20 of P1's / P2's EFT.DAT (P2 wins on a mirror match; the team battle's fighters 2 / 3:
    // EffTab[119] / [139], the last match wins)
    let mut tab = None;
    for p in 0..m.fighters.len() {
        if m.fighters[p].chr as i32 == ch {
            tab = data.eff_tab.get(crate::fight::fight_data::eft_base(p) + 19).and_then(|t| t.as_ref());
        }
    }
    let Some(tab) = tab else { return };
    let off = 24 * (lo & 0x7F) as usize;
    let Some(r) = tab.get(off..off + 24) else { return };
    let w = |k: usize| i16::from_le_bytes([r[2 * k], r[2 * k + 1]]) as i32;
    let mut b = Blood {
        side,
        life: w(0),
        acc: 0,
        rate: w(2),
        bone: w(3).clamp(0, 23) as usize,
        size: w(4),
        dsize: w(5),
        yaw: w(6),
        dyaw: w(7),
        pitch: w(8),
        dpitch: w(9),
        speed: w(10),
        dspeed: w(11),
    };
    if b.size >= 0x2E {
        b.size = 0x2D; // normal mode (the size-mode word is 0)
    }
    b.speed = b.speed.min(0x190);
    if m.fighters[side].anim.mirror {
        b.bone = crate::fight::fx::script_link::REMAP.get(b.bone).copied().unwrap_or(b.bone as u8) as usize;
        if lo & 0x80 != 0 {
            b.yaw = 0x1000 - b.yaw;
        }
    }
    b.pitch = s16(b.pitch - 0x400);
    b.yaw = s16(b.yaw - 0x400);
    m.fxn.mv.tasks.push(Task::Blood(b));
}

fn blood_tick(m: &mut FxHost, tables: &Tables, b: &mut Blood) -> bool {
    b.life -= 1;
    if b.life < 0 {
        return false;
    }
    // normal mode: drops only on even life values, at 80 % size
    if b.life & 1 == 0 {
        let a3 = b.size * 8 / 10;
        if (b.acc >> 3) < ((b.acc + b.rate) >> 3) {
            let pos = bone_pos(m.vm, b.side, b.bone).map(|v| v as i16);
            let yaw = b.yaw + m.fighters[b.side].yaw as i32;
            m.fx.sprite_spawn(tables, pos, 5, yaw, b.pitch, (a3 << 16) | (b.speed / 16));
            b.size = s16(b.size + b.dsize);
            b.yaw = s16(b.yaw + b.dyaw);
            b.pitch = s16(b.pitch + b.dpitch);
            b.speed = s16(b.speed + b.dspeed);
            b.speed = b.speed.max(0xA0);
            b.size = b.size.max(0xF);
        }
    }
    b.acc = s16(b.acc + b.rate);
    true
}

/// op17 sub 5: six additive light rays spinning out of `bone`.
pub fn rays(m: &mut FxHost, tables: &Tables, side: usize, bone: usize) {
    let r = rand(m);
    let ang = [0, 1, 2, 3, 4, 5].map(|i| r + i * 0x2AA);
    let me = tables.table(MOVE_EFFECTS);
    let spd = [0, 1, 2, 3, 4, 5].map(|i| me.i16(0x444 + 2 * i) as i32);
    let wid = [0, 1, 2, 3, 4, 5].map(|i| me.i16(0x450 + 2 * i) as i32);
    m.fxn.mv.tasks.push(Task::Rays(Rays { side, bone, life: 20, ang, spd, wid, draw: (ang, wid, [0; 3]) }));
}

/// op17 sub 6: 16 specks converging on `bone`.
pub fn converge(m: &mut FxHost, side: usize, bone: usize) {
    let (mut r1, mut r2) = (rand(m), rand(m));
    let mut c = Converge { side, bone, life: 16, off: [[0; 2]; 16], k: [0x1000; 16], dk: [0; 16], at: [0; 3] };
    for i in 0..16 {
        r1 += 0x63;
        r2 += 0x85;
        c.off[i] = [(r1 & 0x1FF) - 0x100, (r2 & 0x1FF) - 0x100];
        c.dk[i] = (rand(m) & 0x3F) + 10;
    }
    m.fxn.mv.tasks.push(Task::Converge(Box::new(c)));
}

/// op17 sub A: 30 rising flame streaks over the whole background for `frames`.
pub fn streaks(m: &mut FxHost, tables: &Tables, frames: i32) {
    let mut s = Streaks { life: frames, x: [0; 30], y: [0; 30], spd: [0; 30], cluts: [0; 8] };
    for i in 0..30 {
        s.x[i] = rand(m) & 0x1FF;
        s.y[i] = rand(m) & 0x1FF;
        s.spd[i] = if i & 3 == 2 { (rand(m) & 0x3F) + 0x20 } else { (rand(m) & 0x3F) + 8 };
    }
    for k in 0..8 {
        s.cluts[k] = tables.table(MOVE_EFFECTS).u16(0x45C + 2 * k);
    }
    m.fxn.mv.tasks.push(Task::Streaks(Box::new(s)));
}

/// op17 sub B: 10 red orbs flying out of `pos`.
pub fn orbs(m: &mut FxHost, pos: [i32; 3]) {
    let mut o = Orbs { t: 26, size: [0; 10], p: [[0; 3]; 10], v: [[0; 3]; 10], rgb: [[0xA2; 10]; 2] };
    for i in 0..10 {
        o.size[i] = (rand(m) & 0x1F) + 8;
        o.p[i] = pos.map(s16);
        for k in 0..3 {
            o.v[i][k] = 0x20 - (rand(m) & 0x3F);
        }
    }
    m.fxn.mv.tasks.push(Task::Orbs(Box::new(o)));
}

/// Three claw / kick ribbons following `bone`; `packed` = fade frames << 16 | length scale.
pub fn trail(m: &mut FxHost, tables: &Tables, side: usize, bone: usize, life: i32, delay: i32, packed: i32) {
    let f = &m.fighters[side];
    let mut seg = packed >> 16;
    if seg < 2 {
        seg = 12;
    }
    let s = packed & 0xFFFF;
    let me = tables.table(MOVE_EFFECTS);
    let base = if bone == 7 || bone == 10 { 0x30 } else { 0x60 + f.chr as usize * 0x30 };
    let mut off = [[0; 3]; 6];
    for (e, o) in off.iter_mut().enumerate() {
        let a = base + 8 * e;
        *o = [me.i16(a) as i32, me.i16(a + 2) as i32, me.i16(a + 4) as i32];
        if s != 0 && e & 1 == 1 {
            let k = (0x100_0000u32 / s as u32) as i32;
            o[1] = (o[1] * k) >> 12;
        }
    }
    let ca = if f.chr == 10 { 0x2A8 } else { 0x270 };
    let col = [0, 1, 2, 3, 4, 5, 6].map(|i| [0, 1, 2].map(|c| me.i16(ca + 8 * i + 2 * c) as i32));
    let rgb = std::array::from_fn(|p| col[p % 7].map(|v| v as u8));
    let t = Trail {
        side,
        bone,
        life: life + 3,
        delay: delay.max(0), // a negative delay (auto path) would sample stale history: treated as 0
        nseg: 0,
        scale: s,
        fade_t: 0x1000,
        fade_dt: 0x1000 / seg,
        fade_n: seg,
        off,
        e: [[[0; 3]; 6]; 3],
        q: [[[0; 3]; 18]; 3],
        idx: [0, 4, 8, 12],
        rgb: [rgb; 2],
        col,
        clut: if f.player != 0 { 0x7BC7 } else { 0x7BC6 },
    };
    m.fxn.mv.tasks.push(Task::Trail(Box::new(t)));
}

/// GAD: a comet streak from `pos` along the facing for `frames`.
pub fn comet(m: &mut FxHost, side: usize, pos: [i32; 3], frames: i32) {
    let f = &m.fighters[side];
    let (vx, vz) = (rcos(f.yaw as i32 + 0x400) >> 6, rsin(f.yaw as i32 + 0x400) >> 6);
    let p = pos.map(s16);
    let mut b = p;
    b[0] = s16(b[0] + 8 * vx);
    b[2] = s16(b[2] + 8 * vz);
    let (clut, v0) = if f.player != 0 { (0x7847, 0x40) } else { (0x7846, 0) };
    let c = Comet { t: frames, t0: frames, vx, vz, pts: [p, b, p], rgb: [0x7F; 2], clut, v0 };
    // the team battle's fighters 2 / 3 (engine addition): VRAM bank 1 (`Task::CometBank1`)
    m.fxn.mv.tasks.push(if Fid::of(side).bank() == Bank::B1 { Task::CometBank1(c) } else { Task::Comet(c) });
}

/// The trail's sample of offset `e`: `(R·off) >> 12 + low16(t)` with R = BONEMAT (· diag(1, s, 1)).
fn sample(mt: &M3, t: [i32; 3], scale: i32, off: [i32; 3]) -> [i32; 3] {
    let r = if scale != 0 { Vm::mul(mt, &[[0x1000, 0, 0], [0, scale, 0], [0, 0, 0x1000]]) } else { *mt };
    [0, 1, 2].map(|i| s16((((r[i][0] * off[0] + r[i][1] * off[1] + r[i][2] * off[2]) >> 12).clamp(-0x8000, 0x7FFF)) + t[i]))
}

/// One frame of every move effect (pool 1 lists 8, 11, 12, 14). Sparkles spawn kind-3 sprites into `m.fx`.
pub fn tick(m: &mut FxHost, tables: &Tables) {
    let frozen = m.vm.paused;
    let buf = m.vm.buf;
    let mut keep = Vec::new();
    for task in std::mem::take(&mut m.fxn.mv.tasks) {
        let alive = match task {
            Task::Blood(mut b) => (frozen || blood_tick(m, tables, &mut b)).then_some(Task::Blood(b)),
            Task::Sparkle(mut s) => {
                let ok = sparkle_tick(m, tables, &mut s, frozen);
                ok.then_some(Task::Sparkle(s))
            }
            Task::Rays(mut r) => {
                let old = r.life;
                if !frozen {
                    r.life -= 1;
                }
                if !frozen && old < 0 {
                    None
                } else {
                    r.draw = (r.ang, r.wid, bone_pos(m.vm, r.side, r.bone).map(s16));
                    if !frozen {
                        for i in 0..6 {
                            r.ang[i] += r.spd[i];
                            r.spd[i] = (r.spd[i] * 1875) >> 11;
                            r.wid[i] = (r.wid[i] * 1875) >> 11;
                        }
                    }
                    Some(Task::Rays(r))
                }
            }
            Task::Converge(mut c) => {
                if !frozen {
                    c.life -= 1;
                }
                if !frozen && c.life == 0 {
                    None
                } else {
                    c.at = bone_pos(m.vm, c.side, c.bone).map(s16);
                    if !frozen {
                        for i in 0..16 {
                            for k in 0..2 {
                                c.off[i][k] = s16((c.off[i][k] * c.k[i]) >> 12);
                            }
                            c.k[i] -= c.dk[i];
                        }
                    }
                    Some(Task::Converge(c))
                }
            }
            Task::Streaks(mut s) => {
                if !frozen {
                    s.life -= 1;
                }
                if !frozen && s.life < 0 {
                    None
                } else {
                    if !frozen {
                        for i in 0..30 {
                            s.y[i] -= s.spd[i];
                            if s.y[i] < -0x40 {
                                s.y[i] = 0x200;
                                s.x[i] = rand(m) & 0x1FF;
                            }
                        }
                    }
                    Some(Task::Streaks(s))
                }
            }
            Task::Orbs(mut o) => {
                if !frozen {
                    o.t -= 1;
                }
                if !frozen && o.t < 0 {
                    None
                } else {
                    if !frozen {
                        for i in 0..10 {
                            if o.t < 10 {
                                o.rgb[buf][i] = o.rgb[buf][i].wrapping_sub(0x20);
                            }
                            if o.t >= 11 {
                                o.size[i] += 4;
                                for k in 0..3 {
                                    o.v[i][k] = o.v[i][k] * 3850 / 4096;
                                }
                            }
                            for k in 0..3 {
                                o.p[i][k] = s16(o.p[i][k] + o.v[i][k]);
                            }
                        }
                    }
                    Some(Task::Orbs(o))
                }
            }
            Task::Trail(mut t) => trail_tick(m, &mut t, frozen, buf).then_some(Task::Trail(t)),
            Task::Comet(c) => comet_tick(c, frozen, buf).map(Task::Comet),
            Task::CometBank1(c) => comet_tick(c, frozen, buf).map(Task::CometBank1),
        };
        if let Some(t) = alive {
            keep.push(t);
        }
    }
    keep.append(&mut m.fxn.mv.tasks);
    m.fxn.mv.tasks = keep;
}

fn sparkle_tick(m: &mut FxHost, tables: &Tables, s: &mut Sparkle, frozen: bool) -> bool {
    if frozen {
        return true;
    }
    loop {
        let r = s.recs[s.cur];
        let n = r[0] & 0xFF;
        match r[0] as u16 & 0xFF00 {
            0xFE00 => {
                s.cur = r[1] as usize;
                s.frame = s.recs[s.cur][0];
            }
            0xFC00 => {
                if n < s.loop_n {
                    s.cur += 1;
                } else {
                    s.cur = r[1] as usize;
                    s.loop_n += 1;
                    s.frame = s.recs[s.cur][0];
                }
            }
            0xFF00 => {
                if n < s.loop_n {
                    s.cur += 1;
                    return true;
                }
                return false;
            }
            _ => break,
        }
    }
    while s.cur < s.recs.len() && s.frame == s.recs[s.cur][0] {
        let r = s.recs[s.cur];
        let mut b = r[2] as usize;
        if s.mir {
            b = crate::fight::fx::script_link::REMAP.get(b).copied().unwrap_or(b as u8) as usize;
        }
        for _ in 0..r[1] {
            if r[3] == 0 {
                m.fx.sprite_follow(tables, s.side, b, s.clut);
            }
        }
        s.cur += 1;
    }
    s.frame += 1;
    true
}

/// The comet's task per frame: the head moves on, the colour fades with the time left; None once it ran out.
fn comet_tick(mut c: Comet, frozen: bool, buf: usize) -> Option<Comet> {
    if !frozen {
        c.t -= 1;
    }
    if !frozen && c.t < 0 {
        return None;
    }
    if !frozen {
        let f = (c.t << 12) / c.t0.max(1);
        c.pts[2][0] = s16(c.pts[2][0] + c.vx);
        c.pts[2][2] = s16(c.pts[2][2] + c.vz);
        c.rgb[buf] = ((255 * f) >> 12).clamp(0, 255) as u8;
    }
    Some(c)
}

/// The trail's task per frame; false = the task ends.
fn trail_tick(m: &mut FxHost, t: &mut Trail, frozen: bool, buf: usize) -> bool {
    let mut sampled = false;
    if !frozen {
        if m.fighters[t.side].fx_cancel != 0 {
            return false;
        }
        if t.life == 0 {
            for k in 0..3 {
                t.q[k][0] = t.q[k][1];
            }
        }
        t.life -= 1;
        if t.life <= 0 {
            t.fade_n -= 1;
            if t.fade_n < 0 {
                return false;
            }
        } else {
            if t.delay > 0 {
                t.delay -= 1;
                return true; // nothing drawn (prims skip a delayed trail)
            }
            t.nseg += 1;
            let mt = bone_mat(m.vm, t.side, t.bone);
            let tr = bone_pos(m.vm, t.side, t.bone);
            if t.delay == 0 {
                for k in 0..3 {
                    for j in 0..2 {
                        let v = sample(&mt, tr, t.scale, t.off[2 * k + j]);
                        t.e[k][j] = v;
                        t.e[k][j + 2] = v;
                    }
                }
                t.delay = -1;
            }
            t.idx = [t.idx[3], t.idx[0], t.idx[1], t.idx[2]];
            for k in 0..3 {
                t.e[k][4] = t.e[k][2];
                t.e[k][5] = t.e[k][3];
                t.e[k][2] = t.e[k][0];
                t.e[k][3] = t.e[k][1];
                for j in 0..2 {
                    t.e[k][j] = sample(&mt, tr, t.scale, t.off[2 * k + j]);
                }
                let n = t.idx[0];
                let e = t.e[k];
                t.q[k][n] = e[0];
                t.q[k][n + 1] = e[1];
                t.q[k][n + 2] = [0, 1, 2].map(|c| (e[0][c] + e[2][c]) / 2);
                t.q[k][n + 3] = [0, 1, 2].map(|c| (e[1][c] + e[3][c]) / 2);
            }
            sampled = true;
        }
    }
    if frozen || sampled {
        // B-spline of the previous sample into the next older slot's edge (and the wrap copy)
        for k in 0..3 {
            for j in 0..2 {
                let e = t.e[k];
                let b = [0, 1, 2].map(|c| ((512 * e[j][c] + 3072 * e[j + 2][c] + 512 * e[j + 4][c]) >> 12).clamp(-0x8000, 0x7FFF));
                let n = t.idx[0];
                t.q[k][n + 4 + j] = b;
                if n == 12 {
                    t.q[k][t.idx[1] + j] = b;
                }
            }
        }
    }
    if !frozen && t.life < 0 {
        t.fade_t -= t.fade_dt;
        for i in 0..7 {
            let c = t.col[i].map(|v| ((v * t.fade_t) >> 12) as u8);
            for p in [i, i + 7, i + 14] {
                t.rgb[buf][p] = c;
            }
        }
    }
    true
}

fn quad(xy: [[i32; 2]; 4]) -> [[f32; 2]; 4] {
    xy.map(|p| [p[0] as f32, p[1] as f32])
}

/// The move effects' screen prims for the model list (`buf` = the double-buffer copy of this frame).
pub fn prims(st: &State, gte: &Gte, buf: usize) -> Vec<ModelPrim> {
    let mut out = Vec::new();
    let proj = |p: [i32; 3]| gte.rtps(p).map(|q| (q.sx.trunc() as i32, q.sy.trunc() as i32, q.otz));
    let tex = |xy: [[i32; 2]; 4], uv: [[u8; 2]; 4], rgb: [u8; 3], tpage: u16, clut: u16, otz: i32| ModelPrim {
        xy: quad(xy),
        otz,
        rgb: [rgb; 4],
        uv,
        tpage,
        clut,
        textured: true,
        semi: true,
    };
    for task in &st.tasks {
        match task {
            Task::Sparkle(_) | Task::Blood(_) => {}
            Task::Rays(r) => {
                let Some((sx, sy, _)) = proj(r.draw.2) else { continue };
                let (ang, wid) = (r.draw.0, r.draw.1);
                for i in 0..6 {
                    let a = ang[i];
                    let p1 = [sx + ((rcos(a) * 512) >> 12), sy + ((rsin(a) * 512) >> 12)];
                    let p2 = [sx + ((rcos(a + wid[i]) * 356) >> 12), sy + ((rsin(a + wid[i]) * 356) >> 12)];
                    out.push(ModelPrim {
                        xy: quad([[sx, sy], p1, p2, p2]),
                        otz: 3,
                        rgb: [[0, 0, 0], [0xFF, 0xFF, 0xD2], [0, 0, 0x80], [0, 0, 0x80]],
                        uv: [[0; 2]; 4],
                        tpage: 0x20,
                        clut: 0,
                        textured: false,
                        semi: true,
                    });
                }
            }
            Task::Converge(c) => {
                let Some((sx, sy, _)) = proj(c.at) else { continue };
                for i in 0..16 {
                    let (x, y) = (sx + c.off[i][0], sy + c.off[i][1]);
                    let uv = [[0x40, 0x80], [0x48, 0x80], [0x40, 0x88], [0x48, 0x88]];
                    out.push(tex([[x, y], [x + 20, y], [x, y + 20], [x + 20, y + 20]], uv, [0x7F; 3], 0x3F, 0x7918, 3));
                }
            }
            Task::Streaks(s) => {
                // absolute frame-buffer pixels of a 512×480 field, stretched over the whole view (`Gte::clip`
                // scales y by the Vert+ gain: undone here)
                let sx = 480.0 * gte.fov[0] / 512.0;
                for i in 0..30 {
                    let uv = match i & 3 {
                        0 | 1 => [[0xA0, 0xC0], [0xA7, 0xC0], [0xA0, 0xDF], [0xA7, 0xDF]],
                        2 => [[0xA8, 0xC0], [0xAF, 0xC0], [0xA8, 0xDF], [0xAF, 0xDF]],
                        _ => [[0x70, 0xA0], [0x7F, 0xA0], [0x70, 0xAF], [0x7F, 0xAF]],
                    };
                    let (x0, x1) = ((s.x[i] - 256) as f32 * sx, (s.x[i] + 24 - 256) as f32 * sx);
                    let (y0, y1) = ((s.y[i] - 240) as f32 / gte.fov[1], (s.y[i] + 96 - 240) as f32 / gte.fov[1]);
                    out.push(ModelPrim {
                        xy: [[x0, y0], [x1, y0], [x0, y1], [x1, y1]],
                        otz: 0x80F,
                        rgb: [[0x2A; 3]; 4],
                        uv,
                        tpage: 0x39,
                        clut: s.cluts[(s.y[i] & 7) as usize],
                        textured: true,
                        semi: true,
                    });
                }
            }
            Task::Orbs(o) => {
                for i in 0..10 {
                    let Some((sx, sy, otz)) = proj(o.p[i]) else { continue };
                    let s = tdiv(o.size[i] * H, otz * 4);
                    let uv = [[0xC0, 0x20], [0xDF, 0x20], [0xC0, 0x3F], [0xDF, 0x3F]];
                    out.push(tex([[sx, sy], [sx + s, sy], [sx, sy + s], [sx + s, sy + s]], uv, [o.rgb[buf][i]; 3], 0x39, 0x789A, 4));
                }
            }
            Task::Trail(t) => {
                if t.delay > 0 || t.nseg == 0 {
                    continue;
                }
                let mut pr = 0;
                for k in 0..3 {
                    let s: Vec<_> = t.q[k].iter().map(|&p| proj(p)).collect();
                    for seg in 0..4usize {
                        if seg as i32 >= t.nseg {
                            break;
                        }
                        let zp = s[3 * seg + 2];
                        let otz = zp.map(|q| q.2).unwrap_or(0).clamp(3, 0x80F);
                        let mut n = t.idx[seg];
                        for _ in 0..if seg == 3 { 1 } else { 2 } {
                            let pts = [s[n], s[n + 2], s[n + 1], s[n + 3]];
                            if let [Some(a), Some(b), Some(c), Some(d)] = pts {
                                let uv = [[0xA0, 0xA0], [0xBF, 0xA0], [0xA0, 0xAF], [0xBF, 0xAF]];
                                out.push(tex(
                                    [[a.0, a.1], [b.0, b.1], [c.0, c.1], [d.0, d.1]],
                                    uv,
                                    t.rgb[buf][pr.min(20)],
                                    0x39,
                                    t.clut | bank_bit(t.side),
                                    otz,
                                ));
                            }
                            pr += 1;
                            n += 2;
                        }
                    }
                }
            }
            Task::Comet(c) | Task::CometBank1(c) => {
                // fighters 2 / 3 of the team battle: VRAM bank 1, CLUT bit 15 (the PS1 GPU does not read it)
                let bank = if matches!(task, Task::CometBank1(_)) { Bank::B1 } else { Bank::B0 }.clut_bit();
                let s: Vec<_> = c.pts.iter().map(|&p| proj(p)).collect();
                let (Some(a), Some(b), Some(cc)) = (s[0], s[1], s[2]) else { continue };
                let sp = [a, b, cc];
                let mut ang = [0; 3];
                ang[0] = atan_xz(b.0 - a.0, b.1 - a.1);
                ang[1] = atan_xz(cc.0 - b.0, cc.1 - b.1);
                ang[2] = ang[1];
                let w = [356, 228, 118];
                let w = [0, 1, 2].map(|i| tdiv(H * w[i], sp[i].2 * 4));
                let off = |i: usize| ((rcos(ang[i]) * w[i]) >> 12, (rsin(ang[i]) * w[i]) >> 12);
                for i in 0..2 {
                    if ((sp[i].2 - 4) as u32) >= 0x80B {
                        continue;
                    }
                    let (o0, o1) = (off(i), off(i + 1));
                    let (p, q) = (sp[i], sp[i + 1]);
                    let xy = [[p.0 + o0.0, p.1 + o0.1], [q.0 + o1.0, q.1 + o1.1], [p.0 - o0.0, p.1 - o0.1], [q.0 - o1.0, q.1 - o1.1]];
                    let v0 = c.v0;
                    let uv = if i == 0 {
                        [[0x90, v0], [0xA0, v0], [0x90, v0 + 0x20], [0xA0, v0 + 0x20]]
                    } else {
                        [[0xA0, v0], [0xC0, v0], [0xA0, v0 + 0x20], [0xC0, v0 + 0x20]]
                    };
                    out.push(tex(xy, uv, [c.rgb[buf]; 3], 0x3F, c.clut | bank, sp[i].2));
                }
            }
        }
    }
    out
}

/// Debug: one line per live move effect.
pub fn debug_line(st: &State) -> String {
    st.tasks
        .iter()
        .map(|t| match t {
            Task::Blood(b) => format!(" blood[life{} bone{}]", b.life, b.bone),
            Task::Sparkle(s) => format!(" sparkle[f{} rec{}]", s.frame, s.cur),
            Task::Rays(r) => format!(" rays[{}]", r.life),
            Task::Converge(c) => format!(" converge[{}]", c.life),
            Task::Streaks(s) => format!(" streaks[{}]", s.life),
            Task::Orbs(o) => format!(" orbs[{}]", o.t),
            Task::Trail(t) => format!(" trail[b{} life{} delay{} nseg{} fade{}]", t.bone, t.life, t.delay, t.nseg, t.fade_n),
            Task::Comet(c) | Task::CometBank1(c) => format!(" comet[{}]", c.t),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn blood(side: usize) -> Task {
        Task::Blood(Blood { side, life: 1, acc: 0, rate: 0, bone: 0, size: 0, dsize: 0, yaw: 0, dyaw: 0, pitch: 0, dpitch: 0, speed: 0, dspeed: 0 })
    }

    fn sparkle(recs: Vec<[i16; 4]>, cur: usize, loop_n: i16) -> Task {
        Task::Sparkle(Sparkle { side: 0, recs, cur, frame: 0, loop_n, mir: false, clut: 0 })
    }

    fn trail(side: usize, idx: [usize; 4]) -> Task {
        Task::Trail(Box::new(Trail {
            side,
            bone: 0,
            life: 1,
            delay: 0,
            nseg: 0,
            scale: 0,
            fade_t: 0,
            fade_dt: 0,
            fade_n: 0,
            off: [[0; 3]; 6],
            e: [[[0; 3]; 6]; 3],
            q: [[[0; 3]; 18]; 3],
            idx,
            rgb: [[[0; 3]; 21]; 2],
            col: [[0; 3]; 7],
            clut: 0,
        }))
    }

    /// A decoded state's move effect naming a fighter the fight does not have, a sparkle off the data's schedules (or
    /// past its end, or looping), a trail's slot past its points and (R2) a bone past what its op gives are refused (Err,
    /// where `tick` / `prims` would index past them or never end).
    #[test]
    fn decoded_move_effects_out_of_range_are_refused() {
        // made-up data: schedule 1 at +0x100, two records and the end
        let (origin, mut bytes) = (0x8000_1000u32, vec![0u8; 0x46C]);
        bytes[0x42C..0x430].copy_from_slice(&(origin + 0x100).to_le_bytes());
        let recs: [[i16; 4]; 3] = [[0, 1, 7, 0], [4, 1, 6, 0], [0xFF00u16 as i16, 0, 0, 0]];
        for (k, r) in recs.iter().enumerate() {
            for (j, v) in r.iter().enumerate() {
                let o = 0x100 + 8 * k + 2 * j;
                bytes[o..o + 2].copy_from_slice(&v.to_le_bytes());
            }
        }
        let tables = Tables::build([(MOVE_EFFECTS.into(), crate::formats::tables::Elem::Blob, 0x46C, origin, bytes)]).unwrap();
        assert_eq!(schedule(&tables, 1), recs);
        let check = |t: Task, n: usize| State { tasks: vec![t] }.check(n, &tables);
        assert!(check(blood(1), 2).is_ok());
        assert!(check(blood(2), 2).is_err());
        assert!(check(blood(3), 4).is_ok());
        assert!(check(blood(4), 4).is_err());
        assert!(check(sparkle(recs.to_vec(), 2, 0), 2).is_ok());
        assert!(check(sparkle(recs.to_vec(), 3, 0), 2).is_err());
        assert!(check(sparkle(recs.to_vec(), 0, 1), 2).is_err());
        // a loop back to itself: 0xFE00 to record 0
        assert!(check(sparkle(vec![[0xFE00u16 as i16, 0, 0, 0]], 0, 0), 2).is_err());
        assert!(check(sparkle(Vec::new(), 0, 0), 2).is_err());
        assert!(check(trail(0, [12, 0, 4, 8]), 2).is_ok());
        assert!(check(trail(0, [13, 0, 4, 8]), 2).is_err());
        assert!(check(trail(2, [0, 4, 8, 12]), 2).is_err());
        // (R2) a bone past what its op gives (`bonepos` would run past the address space)
        let bone = |t: Task, b: usize| match t {
            Task::Blood(mut x) => {
                x.bone = b;
                Task::Blood(x)
            }
            Task::Trail(mut x) => {
                x.bone = b;
                Task::Trail(x)
            }
            t => t,
        };
        let rays = |b: usize| Task::Rays(Rays { side: 0, bone: b, life: 1, ang: [0; 6], spd: [0; 6], wid: [0; 6], draw: ([0; 6], [0; 6], [0; 3]) });
        let converge = |b: usize| Task::Converge(Box::new(Converge { side: 0, bone: b, life: 1, off: [[0; 2]; 16], k: [0; 16], dk: [0; 16], at: [0; 3] }));
        assert!(check(bone(blood(0), 23), 2).is_ok() && check(bone(blood(0), 24), 2).is_err());
        assert!(check(bone(trail(0, [0, 4, 8, 12]), 0xFF), 2).is_ok() && check(bone(trail(0, [0, 4, 8, 12]), 0x100), 2).is_err());
        // rays' and converge's bone as op 0x17 subs 5 / 6 make it (`arg as i16 as usize`): a negative argument too
        let made = |arg: u32| arg as i16 as usize;
        assert!(check(rays(0x7FFF), 2).is_ok() && check(rays(made(0x8000)), 2).is_ok() && check(rays(made(0xFFFF)), 2).is_ok());
        assert!(check(rays(0x8000), 2).is_err() && check(rays(usize::MAX / 2), 2).is_err());
        assert!(check(converge(made(0xFFFF)), 2).is_ok() && check(converge(0x8000), 2).is_err());
    }
}
