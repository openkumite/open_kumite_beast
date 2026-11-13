//! The imported game's state-script interpreter and the script step of a fighter's frame.
//!
//! State: none of its own: runs the serialised `Br1Fighter`s
//!
//! The same machine as the main game's (`move_script`): a handler returns the next pc or stops (the pc stays, the
//! same instruction runs again next frame). Offsets are into the fighter's own ANM. `ops.rs` holds the 42 handlers.
//! What belongs to later stages (sounds, effects, the camera, the rave colour) is handed on as `Request`s in call
//! order; everything the handlers write into the fighter blocks is done here.

mod ops;

use super::object::Br1Fighter;
use super::physics::Ring;
use super::pose;
use super::res::Br1Res;
use serde::{Deserialize, Serialize};

/// Instruction sizes of the 42 opcodes (the census decodes every script of the disc with them).
pub const OPLEN: [u8; 42] = [
    2, 2, 2, 2, 4, 4, 2, 6, 4, 6, 6, 2, 4, 4, 4, 4, //
    4, 2, 2, 2, 4, 6, 2, 4, 6, 4, 4, 2, 2, 6, 4, 4, //
    4, 6, 4, 4, 2, 6, 4, 6, 2, 4,
];

/// Engine guard: instructions per frame before the interpreter gives up (the game has no limit; op 0x2A would spin).
const LIMIT: usize = 1024;

/// What a script asks of the systems outside the fighter blocks, in call order.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Request {
    /// A sound, the P2 remap applied
    Sound { id: i16, delay: i16 },
    /// a call of an effect / presentation routine with its arguments as the game passes them, except that a
    /// fighter block pointer is the side (0 / 1) and a model node pointer (MD + 0x34 + 0x50·n) is n
    Call { call: Call, args: [i32; 6] },
    /// a camera word (op 0x1B; the hit's shake) = value
    Cam { word: CamWord, value: i16 },
    /// a hit effect at a contact point (the hit apply's sparks: `call(pos, args…)`)
    Fx { call: Call, pos: [i16; 3], args: [i32; 4] },
}

/// The effect, presentation and sound routines the imported game's fight code calls, as `Request`s name them.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Call {
    /// a presentation call nothing answers (the main game's counterpart is a no-op too)
    Idle,
    /// the fighter's back colour (the rave's colour: player, r, g, b)
    BackColour,
    /// the stage sound (op 0x26 sub 7: its argument 0 / 1)
    StageSound,
    /// the background's flash (frames, r, g, b)
    BackgroundFlash,
    /// the hit's light flash (kind, damage)
    HitLight,
    /// an emitter on a fighter's node (node, kind | jitter << 8, frames, floor; op 0x18)
    NodeEmitter,
    /// a back colour script (player, frames, script)
    BackColourScript,
    /// a still particle without gravity (`Fx`: type, CLUT, mode)
    StillParticle,
    /// a guarded hit's sparks (`Fx`)
    GuardSparks,
    /// a clean hit's sparks and glow (`Fx`)
    HitSparks,
    /// the floor effects (node, kind, yaw, floor; op 0x17 subs 0, 1, 5)
    FloorEffect,
    /// an effect task (type, fighter, timing, life, a, b)
    Task,
    /// an effect script (n, fighter; op 0x16 and the transformation)
    EffectScript,
    /// the player's moves cancelled (their trails end this frame)
    MoveCancel,
    /// op 0x17 sub 2's node effects (fighter, argument)
    NodeEffect,
    /// a spark preset (fighter, preset; op 0x17 sub 3)
    SparkPreset,
    /// the form setup's bind pose rebuilt (side, form; op 0x1A, never used by the data)
    FormSetup,
    /// a skeleton node's scale (side, node, a, b; never used by the data)
    NodeScale,
}

/// The two camera words op 0x1B and the hits write (the camera's shake).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum CamWord {
    Vertical,
    Horizontal,
}

impl CamWord {
    /// The word's byte offset in the camera block.
    pub fn offset(self) -> usize {
        match self {
            CamWord::Vertical => 4,
            CamWord::Horizontal => 6,
        }
    }
}

/// The match around the two fighters (the globals G the handlers read).
pub struct Ctx<'a> {
    /// the fighters' static data, by side
    pub res: [&'a Br1Res; 2],
    /// the gameplay random counter (stream G)
    pub rand: &'a mut u8,
    /// G.94 stage, G.95 round
    pub stage: i8,
    pub round: i8,
    /// the walls present (a wall's hp word ≥ 0)
    pub walls: [bool; 4],
    /// rounds needed to win (`*(G.BC) + 4`)
    pub need: u8,
    /// G.AD: op 0x1B sub 1 off
    pub g_ad: u8,
    /// G.E8: the rave colour flag (bit 0)
    pub g_e8: u32,
    /// the requests of this frame, with the side that made them
    pub out: &'a mut Vec<(u8, Request)>,
    /// debug (the reference runner's op coverage): the pc of every instruction run
    pub trace: Option<&'a mut Vec<u32>>,
    /// engine addition (the import S7): the ring records {hips, edge} of a main-game stage standing in for the
    /// game's tables by `stage` (`ring`)
    pub rings: Option<[Ring; 2]>,
}

impl Ctx<'_> {
    /// The ring record of table `table` (`physics::RING_HIPS` / `RING_EDGE`) for this stage: the game's by G.94, or
    /// the main-game stage's (`rings`).
    pub fn ring(&self, table: &str) -> Ring {
        match self.rings {
            Some([hips, edge]) => {
                if table == super::physics::RING_EDGE {
                    edge
                } else {
                    hips
                }
            }
            None => Ring::of(&self.res[0].tables, table, self.stage),
        }
    }

    /// The next byte of the random table (`math.random_bytes`).
    pub fn rand(&mut self) -> i32 {
        let v = self.res[0].tables.table("math.random_bytes").u8(*self.rand as usize) as i32;
        *self.rand = self.rand.wrapping_add(1);
        v
    }

    fn push(&mut self, me: usize, r: Request) {
        self.out.push((me as u8, r));
    }

    /// A sound request (0x400, id, delay) with the P2 remap (an id ≥ 0xC0 of player 2 + 0x40).
    pub(in crate::fight::br1) fn sound(&mut self, player: u8, me: usize, id: i32, delay: i32) {
        let id = if id >= 0xC0 && player != 0 { id + 0x40 } else { id };
        self.push(me, Request::Sound { id: id as i16, delay: delay as i16 });
    }

    fn call(&mut self, me: usize, call: Call, args: &[i32]) {
        let mut a = [0; 6];
        a[..args.len()].copy_from_slice(args);
        self.push(me, Request::Call { call, args: a });
    }
}

/// MIPS `div` (`mflo`): truncating, and the R3000's results for a zero divisor (−1 / +1) and 0x80000000 / −1.
pub fn div(a: i32, b: i32) -> i32 {
    if b == 0 { if a >= 0 { -1 } else { 1 } } else { a.wrapping_div(b) }
}

/// Start state `n`: pc = ANM + (s16) states[n], the per-state flags and timers cleared, the hands reset.
pub fn start_state(f: &mut Br1Fighter, res: &Br1Res, n: i16) {
    // an unused entry (0xFFFF) or an index past the table gives a pc outside the file, where `run` then stops every
    // frame (engine guard: the game would execute the RAM before or after the ANM)
    let o = res.anm.header[1] as i64 + 2 * n as i64;
    let e = if (0..res.anm.bytes.len() as i64).contains(&o) { res.anm.i16(o as usize) } else { 0 };
    let pc = e as i32 as u32;
    f.s.sleep = 0;
    f.loop_mark = pc;
    f.pc = pc;
    f.f.f54 &= 0xFFFC_7FD7;
    f.f.f58 &= 0xA9DF_FFEF;
    f.f.v_speed = 0;
    f.f.v_decay = 0;
    f.f.attack = -1;
    f.spheres[0].flag = -1;
    f.f.hittable = 0;
    f.f.startup = 0;
    f.f.active = 0;
    f.f.move_time = 0;
    f.s.delayed_timer = -1;
    f.parts.reset_hands(f.cur as usize);
}

/// Switch to `form`'s lists (objects, model nodes, pose records, bone maps, spheres, weight); node 0's
/// and node 4's world transforms and records 0..18 carry over (GRE: also node 19).
pub fn model_switch(f: &mut Br1Fighter, form: usize) {
    let form = form & 1;
    let old = f.cur as usize & 1;
    let d = f.desc[form];
    f.f.objects = d.objects;
    f.f.nodes = d.nodes;
    f.f.body_spheres = d.body_spheres;
    f.f.hit_spheres = d.hit_spheres;
    f.f.weight = d.weight;
    f.cur = form as u8;
    if old == form {
        return;
    }
    let src = f.forms[old].clone();
    let dst = &mut f.forms[form];
    let mut carry = |n: usize| {
        if let (Some(s), Some(d)) = (src.md.get(n), dst.md.get_mut(n)) {
            (d.wm, d.wt) = (s.wm, s.wt);
        }
    };
    carry(0);
    carry(4);
    if f.f.chr == 1 {
        carry(19);
    }
    let recs = if f.f.chr == 1 { 20 } else { 19 };
    for n in 0..recs {
        if let (Some(s), Some(d)) = (src.rec.get(n), dst.rec.get_mut(n)) {
            *d = *s;
        }
    }
}

/// Run `me`'s script until a handler stops (or sleep while S.98 > 0).
pub fn run(fs: &mut [Br1Fighter; 2], me: usize, ctx: &mut Ctx) {
    let f = &mut fs[me];
    if f.s.sleep > 0 {
        f.s.sleep = f.s.sleep.wrapping_sub(f.anim.speed);
        return;
    }
    let len = ctx.res[me].anm.bytes.len();
    for _ in 0..LIMIT {
        let pc = fs[me].pc;
        // engine guard: a pc outside the file (the game reads RAM around the ANM) stops; widened so that a pc
        // like 0xFFFF_FFFF cannot wrap on a 32-bit usize (wasm32)
        if u64::from(pc) + 2 > len as u64 {
            return;
        }
        if let Some(t) = ctx.trace.as_deref_mut() {
            t.push(pc);
        }
        match ops::op(fs, me, ctx, pc) {
            Some(n) => fs[me].pc = n,
            None => return,
        }
    }
}

/// The clip timing and sampler, the local matrices of nodes 1..18, the turn request, F.46, node 0's
/// matrix and node 1 / 4's translations.
pub fn anim_step(fs: &mut [Br1Fighter; 2], me: usize, ctx: &Ctx) {
    let (o4c, o50) = (fs[1 - me].f.f4c, fs[1 - me].f.f50);
    let res = ctx.res[me];
    let banks = res.banks(Some(&ctx.res[1 - me].anm));
    let f = &mut fs[me];
    let cur = f.cur as usize & 1;
    let step = f.f.step;
    let skel = &res.skels[cur];
    let rs = res.tables.rsqrt_fn();
    let form = &mut f.forms[cur];
    pose::step(&mut f.anim, &mut form.rec, &banks, cur, step);
    let lm = pose::local_matrices(&form.rec, skel, &res.trig, &rs);
    for (n, m) in lm.iter().enumerate().take(pose::BASE_NODES).skip(1) {
        if let Some(d) = form.md.get_mut(n) {
            d.lm = *m;
        }
    }
    let w0 = form.md.first().map_or([0; 3], |d| d.wt);
    pose::turn(&mut f.anim, w0, &res.trig);
    f.f.yaw = pose::facing(&f.anim);
    let half = pose::node1_halved(f.f.f4c, f.f.f50, o4c, o50);
    let form = &mut f.forms[cur];
    let pos = form.md.first().map_or([0; 3], |d| d.lt);
    let lt = pose::local_translations(&f.anim, skel, pos, half);
    if let Some(d) = form.md.first_mut() {
        d.lm = pose::root_matrix([f.f.pitch, f.f.yaw, f.f.roll], &res.trig);
    }
    for n in [1, 4] {
        if let (Some(d), Some(t)) = (form.md.get_mut(n), lt.get(n)) {
            d.lt = *t;
        }
    }
}

/// `me`'s script step: the requested state started, the interpreter, the frame step, the animation step,
/// the per-move timers, the delayed sounds, the rave colour and the head turn.
pub fn step(fs: &mut [Br1Fighter; 2], me: usize, ctx: &mut Ctx) {
    let f = &mut fs[me];
    if f.s.state_req != f.s.state_cur {
        f.s.state_cur = f.s.state_req;
        let n = f.s.state_req;
        start_state(f, ctx.res[me], n);
    }
    run(fs, me, ctx);
    let f = &mut fs[me];
    // the step back to normal, ×3/2 while attacking under rave
    f.f.step = f.f.step_normal;
    if f.f.rave > 0 && f.f.f50 & 0xC000_0000 != 0 && f.f.f50 & 0x4000_0000 == 0 {
        f.f.step = (f.f.step_normal as i32 * 3 / 2) as i16;
    }
    anim_step(fs, me, ctx);
    let f = &mut fs[me];
    let step = f.f.step;
    // the move timers
    f.f.move_time = f.f.move_time.wrapping_add(step);
    if f.f.startup > 0 {
        f.f.startup = f.f.startup.wrapping_sub(step);
        if f.f.startup <= 0 {
            f.f.startup = 0;
            f.f.attack = 0;
        }
    }
    if f.f.active > 0 {
        f.f.active = f.f.active.wrapping_sub(f.f.step_normal);
        if f.f.active <= 0 {
            f.f.active = 0;
            f.f.attack = -1;
        }
    }
    if f.f.active == 0 && f.f.startup == 0 && f.f.recovery >= 0 {
        f.f.recovery = f.f.recovery.wrapping_add(step);
    }
    if f.f.no_throw > 0 {
        f.f.hittable = -1;
        f.f.no_throw = f.f.no_throw.wrapping_sub(step);
        if f.f.no_throw <= 0 || f.s.state_req != 1 {
            f.f.no_throw = 0;
            f.f.hittable = 0;
        }
    }
    if f.f.fc8 > 0 {
        if f.f.pressed & 0xF0E0 != 0 {
            f.f.fc8 = f.f.fc8.wrapping_sub(step.wrapping_mul(3));
        }
        f.f.fc8 = f.f.fc8.wrapping_sub(step);
        if f.f.fc8 <= 0 {
            f.f.fc8 = 0;
        }
    }
    // the startup sound (op 0x10's class), played while attacking
    if f.s.sound_timer > 0 {
        f.s.sound_timer = f.s.sound_timer.wrapping_sub(step);
        if f.s.sound_timer <= 0 {
            f.s.sound_timer = 0;
            let id = f.s.impact_own as i32;
            if id >= 0 && (f.f.f50 as i32) < 0 {
                ctx.sound(f.f.player, me, id, 0);
            }
        }
    }
    // the delayed sound of op 0x27 sub 1
    let f = &mut fs[me];
    if f.s.delayed_timer > 0 {
        f.s.delayed_timer = f.s.delayed_timer.wrapping_sub(f.f.step);
        if f.s.delayed_timer <= 0 {
            f.s.delayed_timer = 0;
            let id = f.s.delayed_sound as i32;
            if id >= 0 {
                ctx.sound(f.f.player, me, id, 0);
            }
        }
    }
    // the rave colour (presentation)
    let f = &mut fs[me];
    if f.f.rave != 0 {
        let p = f.f.player as i32 + 1;
        if f.f.rave < 0 {
            f.f.rave = 0;
            ctx.call(me, Call::BackColour, &[p, -1, -1, -1]);
        } else if ctx.g_e8 & 1 != 0 {
            ctx.call(me, Call::BackColour, &[p, -1, -1, -1]);
        } else {
            ctx.call(me, Call::BackColour, &[p, 0xFFF, 0xFFF, 0x7FF]);
        }
    }
    head(fs, me, ctx);
}

/// The head turn toward the opponent (`F.54 & 0x10000`, not KO'd), node 2's matrix turned.
fn head(fs: &mut [Br1Fighter; 2], me: usize, ctx: &Ctx) {
    let res = ctx.res[me];
    let w = pose::HeadInputs { own0: fs[me].world(0), own2: fs[me].world(2), opp0: fs[1 - me].world(0), opp2: fs[1 - me].world(2) };
    let f = &mut fs[me];
    let on = f.f.f54 & 0x1_0000 != 0 && f.f.status < 0x80;
    let sq = res.tables.sqrt_fn();
    let cur = f.cur as usize & 1;
    pose::head_turn(&mut f.anim, &f.forms[cur].rec, on, f.f.yaw, &w, &res.trig, &sq);
    if on && let Some(d) = f.forms[cur].md.get_mut(2) {
        d.lm = pose::turn_head(&d.lm, &f.anim, &res.trig);
    }
}

#[cfg(test)]
mod tests;
