//! The frame runner's core (port addition, debug only, never a game mode): the imported game's whole fight frame, stage
//! by stage, against the original's snapshots.
//!
//! Original: the snapshots `.research/br1/specs/s5/framelog.lua` takes in PCSX-Redux at every stage boundary of the
//! fight frame's logic (0x80172660): the inputs 0x80182A24, each fighter's frame 0x8017B608 with its script step
//! 0x8018118C, the pair 0x80183EE4 before and after the hits, the frame's end
//! Spec: docs/specs/br1_decision_input_physics.md §9
//! State: none (reads byte slices; `bin/br1test.rs` reads the files)
//!
//! A record is S4's snapshot (`reference::record`) and the RAM S5's routines read besides: the command and chain
//! slots, the button counters and configs, the camera's angle, the options, the walls, the objects 0 / 2, the AI's
//! pads, the raw pads as read, the KO replay recorder's mode.

use super::decision::Opts;
use super::frame::{self, Later, World};
use super::object::Br1Fighter;
use super::physics::Walls;
use super::reference::{self, Snap};
use super::res::Br1Res;
use super::script::{self, Ctx, Request};

/// The S5 part of a record.
pub const EXTRA: usize = 0x200 + 0x280 + 0x100 + 0x20 + 8 + 0x40 + 0x80 + 2 * 2 * 40 + 4 + 4 + 4;
/// One record of `frames.bin`.
pub const RECORD: usize = reference::RECORD + EXTRA;
/// The record format this build reads: the magic 'S5F' + the version byte ('S5FR' was version 1, before the KO replay
/// recorder's mode).
pub const MAGIC: u32 = 0x3246_3553;
const _: () = assert!(RECORD >= reference::RECORD);

fn u16at(b: &[u8], o: usize) -> u16 {
    b.get(o..o + 2).map_or(0, |s| u16::from_le_bytes([s[0], s[1]]))
}
fn u32at(b: &[u8], o: usize) -> u32 {
    b.get(o..o + 4).map_or(0, |s| u32::from_le_bytes([s[0], s[1], s[2], s[3]]))
}

/// One snapshot of `frames.bin`.
pub struct Rec<'a> {
    pub snap: Snap<'a>,
    /// the fighter of tags 2..5 (else 0)
    pub fighter: u32,
    pub cmd: &'a [u8],
    pub chain: &'a [u8],
    pub counters: &'a [u8],
    pub buttons: &'a [u8],
    pub cam: &'a [u8],
    pub opts: &'a [u8],
    pub walls: &'a [u8],
    pub objs: &'a [u8],
    pub ai: &'a [u8],
    /// the raw pads as the input read them (G.68 + 2·player at 0x80183178), this frame's from tag 2 on
    pub pads_read: &'a [u8],
    /// the KO replay recorder's mode `*(G.C4)` (0x8012B90C: 3 / 4 / 5 play the recorded pads back)
    pub replay: i16,
}

/// The records of a `frames.bin` (none for an empty file).
pub fn parse(b: &[u8]) -> Result<Vec<Rec<'_>>, String> {
    if b.is_empty() {
        return Ok(Vec::new());
    }
    let magic = u32at(b, 0);
    if magic & 0xFF_FFFF != 0x46_3553 {
        return Err("not a frames.bin (magic)".into());
    }
    if magic != MAGIC {
        let v = match magic >> 24 {
            0x52 => "1".to_string(),
            c => (c as u8 as char).to_string(),
        };
        return Err(format!("frames.bin format version {v}; this build reads version 2"));
    }
    if !b.len().is_multiple_of(RECORD) {
        return Err(format!("{} bytes: not a whole number of {RECORD}-byte records", b.len()));
    }
    let mut out = Vec::with_capacity(b.len() / RECORD);
    for r in b.as_chunks::<RECORD>().0 {
        if u32at(r, 0) != MAGIC {
            return Err("a frames.bin record of another version (magic)".into());
        }
        let mut o = reference::RECORD;
        let mut take = |n: usize| {
            let s = &r[o..o + n];
            o += n;
            s
        };
        out.push(Rec {
            snap: reference::record(r.first_chunk().expect("a frames.bin record holds a steps.bin one")),
            fighter: u32at(r, 12),
            cmd: take(0x200),
            chain: take(0x280),
            counters: take(0x100),
            buttons: take(0x20),
            cam: take(8),
            opts: take(0x40),
            walls: take(0x80),
            objs: take(2 * 2 * 40),
            ai: take(4),
            pads_read: take(4),
            replay: u16at(take(4), 0) as i16,
        });
    }
    Ok(out)
}

/// Fighter `k` of record `r` as the port's state.
pub fn load(r: &Rec, k: usize, res: [&Br1Res; 2]) -> Br1Fighter {
    let mut f = reference::load(&r.snap, k, res);
    let p = f.f.player as usize & 1;
    let i = &mut f.input;
    for b in 0..16 {
        i.hold[b] = u32at(r.counters, 0x80 * p + 4 * b);
        i.since[b] = u32at(r.counters, 0x80 * p + 0x40 + 4 * b);
    }
    for (n, s) in i.cmd.iter_mut().enumerate() {
        *s = std::array::from_fn(|j| u16at(r.cmd, 0x100 * p + 8 * n + 2 * j) as i16);
    }
    for (n, s) in i.chain.iter_mut().enumerate() {
        *s = std::array::from_fn(|j| u16at(r.chain, 0x140 * p + 10 * n + 2 * j) as i16);
    }
    i.buttons = std::array::from_fn(|j| u16at(r.buttons, 0x10 * p + 2 * j));
    for form in 0..2 {
        for (slot, o) in [0usize, 12].into_iter().enumerate() {
            let b = 80 * k + 40 * form + o;
            f.objs[form][slot] = std::array::from_fn(|c| u32at(r.objs, b + 4 * c) as i32);
        }
    }
    f
}

/// Fighter `k`'s linked nodes per form in record `r` (what `Br1Res::linked` must give: the frame runs check it).
pub fn linked(r: &Rec, k: usize) -> [u32; 2] {
    let fr = &r.snap.fighters[k];
    [0, 1].map(|form| links(fr.forms[form].1, u32at(fr.desc, 0x1C * form + 0x10)))
}

/// The nodes reached from node 0 through the MD child (+0x44) / sibling (+0x48) links; `md` = the nodes' bytes,
/// `base` their address.
fn links(md: &[u8], base: u32) -> u32 {
    let node = |a: u32| {
        let d = a.wrapping_sub(base);
        (a != 0 && d.is_multiple_of(0x50) && d / 0x50 < 26).then_some((d / 0x50) as usize)
    };
    let mut mask = 1u32;
    let mut stack = vec![node(u32at(md, 0x44))];
    while let Some(x) = stack.pop() {
        let Some(n) = x else { continue };
        if mask & (1 << n) != 0 {
            continue;
        }
        mask |= 1 << n;
        stack.push(node(u32at(md, 0x50 * n + 0x48)));
        stack.push(node(u32at(md, 0x50 * n + 0x44)));
    }
    mask
}

/// Both fighters of a record.
pub fn load2(r: &Rec, res: [&Br1Res; 2]) -> [Br1Fighter; 2] {
    [load(r, 0, res), load(r, 1, res)]
}

/// The match around the fighters in a record.
pub fn world(r: &Rec) -> World {
    let sp = r.snap.spad;
    World {
        pads: [u16at(sp, 0x68), u16at(sp, 0x6A)],
        cam: u16at(r.cam, 2) as i16,
        ai: [u16at(r.ai, 0), u16at(r.ai, 2)],
        stage: sp[0x94] as i8,
        opts: Opts { g98: sp[0x98], need: r.snap.gbc[4], opt38: u32at(r.opts, 0x38), rave: true },
        opt2c: u32at(r.opts, 0x2C),
        frozen: u16at(sp, 0x7E) != 0,
        replay: [None; 2],
    }
}

/// The walls' state in a record (0x801CCF40 + 0x44 ..).
pub fn walls(r: &Rec) -> Walls {
    let w = |o: usize| u16at(r.walls, o) as i16;
    Walls {
        w44: w(0x44),
        hp: std::array::from_fn(|k| w(0x5A + 2 * k)),
        hit: std::array::from_fn(|k| w(0x6A + 2 * k)),
        lock: std::array::from_fn(|k| w(0x72 + 2 * k)),
    }
}

/// A stage of the frame: between two consecutive records.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Stage {
    /// 0x80182A24: both inputs and the throws
    Inputs,
    /// 0x8017B608 up to the script step (health, auto-turn, ground, flags, decision), by side
    Decide(usize),
    /// the script step 0x8018118C (S4's), by side
    Script(usize),
    /// 0x8017BFB8.. after it (the KO voice, the velocity), by side
    Velocity(usize),
    /// 0x80183EE4 up to the hits (integrate, landing)
    Move,
    /// 0x80183F88..0x801840CC: the hits
    Hits,
    /// 0x801840CC..: the push-apart, the walls, the angles (the spring chains of nodes 19.. fed)
    Push,
    /// from the logic's end to the next frame's (the spring chains, the camera, the round loop: fed)
    Between,
    /// P1's frame end → P2's frame start: nothing runs
    Gap,
}

impl Stage {
    /// Whether the port runs this stage (else it is fed from the original).
    pub fn implemented(self) -> bool {
        !matches!(self, Stage::Between | Stage::Gap)
    }
}

/// Whether the KO replay plays its recorded pads back at record `a` (the recorder 0x8012B90C in mode 3..5: the round
/// flow's, replaced by the main game's; the port's recorder passes the pads through).
pub fn playback(a: &Rec) -> bool {
    (3..=5).contains(&a.replay)
}

/// Whether the original's KO "guts" save let fighter `f` survive between `x` and `y` (the 0x200000 bit newly set,
/// still fighting, the damage at the max): dropped in the port (a user decision), which KOs instead.
pub fn guts(x: &Br1Fighter, y: &Br1Fighter) -> bool {
    x.f.f54 & 0x20_0000 == 0 && y.f.f54 & 0x20_0000 != 0 && y.f.status == 1 && y.f.damage == y.f.max_hp
}

/// The stage from record `a` to the next record `b`.
pub fn stage(a: &Rec, b: &Rec) -> Option<Stage> {
    let side = |r: &Rec| reference::FIGHTERS.iter().position(|&x| x == r.fighter);
    Some(match (a.snap.tag, b.snap.tag) {
        (1, 2) => Stage::Inputs,
        (2, 3) => Stage::Decide(side(a)?),
        (3, 4) => Stage::Script(side(a)?),
        (4, 5) => Stage::Velocity(side(a)?),
        (5, 2) => Stage::Gap,
        (5, 6) => Stage::Move,
        (6, 7) => Stage::Hits,
        (7, 8) => Stage::Push,
        (8, 1) | (8, 8) => Stage::Between,
        _ => return None,
    })
}

/// A stage the port does not run, in a carried run: what the original changed in it (`orig` = its fighters at `a`
/// and at the next record `b`) fed into the port's state; the walls taken when the original's changed, the random
/// counter advanced as the original's.
pub fn feed(fs: &mut [Br1Fighter; 2], w: &mut Walls, rand: &mut u8, orig: (&[Br1Fighter; 2], &[Br1Fighter; 2]), a: &Rec, b: &Rec) {
    let (oa, ob) = orig;
    *fs = std::array::from_fn(|k| if oa[k] == ob[k] { fs[k].clone() } else { reference::feed(&fs[k], &oa[k], &ob[k]) });
    let wb = walls(b);
    if walls(a) != wb {
        *w = wb;
    }
    *rand = rand.wrapping_add(b.snap.rand.wrapping_sub(a.snap.rand));
}

/// `Later` of the runner: the spring chains taken from the original (`prev` → `now`).
pub struct Fed<'a> {
    pub prev: &'a [Br1Fighter; 2],
    pub now: &'a [Br1Fighter; 2],
}

impl Later for Fed<'_> {
    fn chains(&mut self, fs: &mut [Br1Fighter; 2]) {
        // 0x80158458: nodes 19.. and their pose records
        for k in 0..2 {
            for form in 0..2 {
                let (a, b) = (&self.prev[k].forms[form], &self.now[k].forms[form]);
                let p = &mut fs[k].forms[form];
                for n in 19..p.md.len().min(a.md.len()).min(b.md.len()) {
                    if a.md[n] != b.md[n] {
                        p.md[n] = b.md[n];
                    }
                }
                for n in 19..p.rec.len().min(a.rec.len()).min(b.rec.len()) {
                    if a.rec[n] != b.rec[n] {
                        p.rec[n] = b.rec[n];
                    }
                }
            }
        }
    }
}

/// Run the port's `stage` on `fs` (the record `a`'s match around it; `orig` = the original's fighters at `a` and at the
/// next record, and that record: they feed what the port does not run). Returns the requests (the frame oracle
/// are compared with the call log through `call_request`, the camera words through `cam_after`); `trace` collects the
/// decision's sections reached; `ko` = the decide stage KO'd the fighter this frame (for the velocity stage).
#[allow(clippy::too_many_arguments)]
pub fn run(
    st: Stage,
    fs: &mut [Br1Fighter; 2],
    walls: &mut Walls,
    rand: &mut u8,
    a: &Rec,
    orig: (&[Br1Fighter; 2], &[Br1Fighter; 2], &Rec),
    res: [&Br1Res; 2],
    trace: Option<&mut Vec<u32>>,
    ko: bool,
) -> Vec<(u8, Request)> {
    let g = reference::globals(&a.snap);
    let mut w = world(a);
    if st == Stage::Inputs {
        // the pads as read inside the stage (a VBlank may change G.68 between the snapshots and the read), and the
        // AI's pad as its output (S10's AI runs inside the input)
        let b = orig.2;
        w.pads = [u16at(b.pads_read, 0), u16at(b.pads_read, 2)];
        w.ai = world(b).ai;
    }
    let mut out = Vec::new();
    let mut ctx = Ctx {
        res,
        rand,
        stage: g.stage,
        round: g.round,
        walls: walls.hp.map(|h| h >= 0),
        need: g.need,
        g_ad: g.g_ad,
        g_e8: g.g_e8,
        out: &mut out,
        trace,
        rings: None,
    };
    match st {
        Stage::Inputs => {
            frame::inputs(fs, &mut ctx, &w);
        }
        Stage::Decide(k) => {
            frame::before_script(fs, k, &mut ctx, &w, walls);
        }
        Stage::Script(k) => script::step(fs, k, &mut ctx),
        // `ko`: this frame's health KO'd the fighter (the decide stage's result: the KO voice)
        Stage::Velocity(k) => frame::after_script(fs, k, &mut ctx, ko),
        Stage::Move => frame::pair_move(fs, res),
        Stage::Hits => super::hit::hits(fs, &mut ctx, w.opts.g98),
        Stage::Push => {
            super::physics::after_hits(fs, res, super::physics::Ring::of(&res[0].tables, super::physics::RING_HIPS, w.stage), walls);
            Fed { prev: orig.0, now: orig.1 }.chains(fs);
        }
        Stage::Between | Stage::Gap => {}
    }
    out
}

/// The camera words of record `a` with the stage's camera requests (`Request::Cam`) applied: compare with the next
/// record's.
pub fn cam_after(a: &Rec, reqs: &[(u8, Request)]) -> Vec<u8> {
    let mut cam = a.snap.cam.to_vec();
    for (_, r) in reqs {
        if let Request::Cam { word, value } = r {
            let o = word.offset();
            if let Some(c) = cam.get_mut(o..o + 2) {
                c.copy_from_slice(&value.to_le_bytes());
            }
        }
    }
    cam
}

/// One call of the original's call log (`calls.txt`: the sound, effect and random routines while the logic ran).
#[derive(Clone, Debug)]
pub struct Call {
    /// the records written before it: the call belongs to the stage from record `rec − 1` to `rec`
    pub rec: usize,
    pub event: reference::Event,
    /// the 3 halfwords at a0 / a1 (an effect's position argument)
    pub pa: [i16; 3],
    pub pb: [i16; 3],
}

/// `calls.txt`'s lines.
pub fn parse_calls(text: &str) -> Vec<Call> {
    let halves = |s: &str| -> [i16; 3] {
        let b: Vec<u8> = (0..6).filter_map(|i| s.get(2 * i..2 * i + 2).and_then(|h| u8::from_str_radix(h, 16).ok())).collect();
        std::array::from_fn(|i| b.get(2 * i..2 * i + 2).map_or(0, |p| i16::from_le_bytes([p[0], p[1]])))
    };
    text.lines()
        .filter_map(|l| {
            let w: Vec<&str> = l.split_whitespace().collect();
            let rec = w.first()?.parse().ok()?;
            // as an S4 event line: frame, fighter (none), name, the arguments, ra
            let line = format!("{} 0 {}", w.get(1)?, w[2..10.min(w.len())].join(" "));
            let event = reference::parse_events(&line).pop()?;
            let field = |k: &str| w.iter().find_map(|t| t.strip_prefix(k)).map_or([0; 3], halves);
            Some(Call { rec, event, pa: field("pa="), pb: field("pb=") })
        })
        .collect()
}

/// The original's call as the port's request: the script step's calls as S4's runner maps them, the frame code's
/// (the decision, the fighter frame, the throw test, the hit apply); None for the calls the port does not make (the
/// effects' own, the random routine, the round loop's).
pub fn call_request(c: &Call, md: [u32; 2]) -> Option<Request> {
    let e = &c.event;
    if let Some(r) = reference::event_request(e, md) {
        return Some(r);
    }
    const CODE: [(u32, u32); 4] = [(0x8017_9694, 0x8017_B608), (0x8017_B608, 0x8017_C204), (0x8018_2D18, 0x8018_30C0), (0x8014_EFFC, 0x8015_03F0)];
    if !CODE.iter().any(|&(a, b)| (a..b).contains(&e.ra)) {
        return None;
    }
    let a = e.args.map(|v| v as i32);
    let side = |p: u32| reference::FIGHTERS.iter().position(|&x| x == p).map_or(p as i32, |s| s as i32);
    Some(match e.name.as_str() {
        "sound" => Request::Sound { id: a[1] as i16, delay: a[2] as i16 },
        "fx4B3C4" => Request::Call { call: script::Call::EffectScript, args: [a[0], side(e.args[1]), 0, 0, 0, 0] },
        "fx4CB7C" => Request::Call { call: script::Call::MoveCancel, args: [a[0], 0, 0, 0, 0, 0] },
        "flash" => Request::Call { call: script::Call::BackgroundFlash, args: [a[0], a[1], a[2], a[3], 0, 0] },
        // 0x8013E1A8 is the hit's light flash (light #3's colour, `present::br1_fx`); the S5 / S6 logs call it "hitsnd"
        "hitflash" | "hitsnd" => Request::Call { call: script::Call::HitLight, args: [a[0], a[1], 0, 0, 0, 0] },
        "spark" => Request::Fx { call: script::Call::HitSparks, pos: c.pa, args: [a[1], a[2], a[3], a[4]] },
        "gspark" => Request::Fx { call: script::Call::GuardSparks, pos: c.pa, args: [a[1], a[2], a[3], 0] },
        "fx40880" => Request::Fx { call: script::Call::Task, pos: c.pb, args: [a[0], a[3], a[4], a[5]] },
        _ => return None,
    })
}

/// The port's requests of a stage in the form `call_request` gives the original's (the camera words are compared
/// apart; the script step's internal calls are not logged).
pub fn comparable(reqs: &[(u8, Request)]) -> Vec<Request> {
    reqs.iter()
        .map(|r| r.1)
        .filter(|r| !matches!(r, Request::Cam { .. } | Request::Call { call: script::Call::Idle | script::Call::FormSetup | script::Call::NodeScale, .. }))
        .collect()
}

/// The random draws of the port's code among a stage's calls (op 29 / 26.05 in the script step, the decision's and
/// the hit code's: none today): the counter's expected advance. The effects' own draws (0x80140AA4 under op 17.02's
/// 0x8014CBB4, the particles) are presentation, not the port's.
pub fn logic_draws(calls: &[&Call]) -> u8 {
    const CODE: [(u32, u32); 3] = [(0x8017_D508, 0x8018_1754), (0x8017_9694, 0x8017_C204), (0x8014_EFFC, 0x8015_03F0)];
    calls.iter().filter(|c| c.event.name == "rand" && CODE.iter().any(|&(a, b)| (a..b).contains(&c.event.ra))).count() as u8
}
