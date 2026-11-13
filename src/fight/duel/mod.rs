//! The routines of one fighter against its opponent, by index, that the match (`fight::sim`) and the team battle
//! (`fight::team`, engine addition) share: its input and per-fighter step (here), the hit pairs (`hits`), the setup and
//! state checks of a fighter (`setup`), the imported parts (`br1`).
//!
//! State: none of its own: steps the fighters and the parts of the serialised `Match` it is lent
//!
//! The frame's per-fighter steps with the fighter `me` and its opponent `opp` given as indices into `fs` (all
//! fighters of the fight, indexed as `Loaded::fighters`), and the parts of the fight they write lent as `Parts`.
//! `Match::input_one` / `fighter_one` (`fight::sim::frame`) run them with `opp = 1 - me`; the team world (engine addition, the
//! team battle) runs them with each fighter's target.

use crate::fight::anim::playback;
use crate::fight::camera::Camera;
use crate::fight::fight_data::Loaded;
use crate::fight::fighter::{F5C, F6C, F58, F60, F64, Fighter, pair_mut};
use crate::fight::input;
use crate::fight::move_decision::{self, World};
use crate::fight::move_script::{self, Ctx};
use crate::fight::physics;

pub(crate) mod br1;
pub(crate) mod frame;
pub(crate) mod hits;
pub(crate) mod setup;

/// The parts of the fight a fighter's step writes besides the fighters.
pub(crate) struct Parts<'a> {
    pub world: &'a mut World,
    pub cam: &'a mut Camera,
    pub rand: &'a mut u32,
    /// the SFX queue
    pub sound_queue: &'a mut Vec<(i16, i16)>,
    pub custom: &'a crate::fight::rules::Custom,
    /// debug: (fighter, move) requested at the next move decision
    pub force_move: &'a mut Option<(usize, i16)>,
    /// the fighter's motors [large, small] when they run (the VIBRATION option on, a human player), else None
    pub vib: Option<&'a mut [(u8, u8); 2]>,
    pub round_state: u8,
    pub round_index: i8,
    pub frame: u64,
    /// the fighter's bank: 0 in a match; 1 for the team battle's P3 / P4, whose
    /// character sounds (slots 3 / 4) play from slots 7 / 8
    pub bank: crate::fight::side::Bank,
    /// the fighters whose attacks it may guard (the F5C guard bits from their F58): a match's opponent; the team
    /// battle's enemies still in the round
    pub enemies: &'a [usize],
}

impl Parts<'_> {
    /// A vibration request of motor `motor` (`Match::vibrate`).
    fn vibrate(&mut self, motor: usize, frames: u8, val: u8) {
        if let Some(v) = self.vib.as_deref_mut() {
            v[motor] = (frames, val);
        }
    }
}

/// The fighter faces screen-left, so Left/Right are exchanged: the sign of the 12-bit angle
/// camera yaw `cam_yaw` (+0x22, the committed +0x2A) + fighter yaw (+0x4E; the facing +0xD0 while F60 bit 4 / F5C
/// bit 6), inverted by F6C bit 13. Relative to the camera, so it follows the fighters when they cross over.
pub(crate) fn facing_left(f: &Fighter, cam_yaw: i16) -> bool {
    let yaw = if f.flags[F5C] & 0x40 != 0 || f.flags[F60] & 0x10 != 0 { f.yaw_base } else { f.yaw } as i32;
    let v = ((cam_yaw as i32 + yaw) << 20) >> 20;
    let left = v < 0;
    left != (f.flags[F6C] & 0x2000 != 0)
}

/// Input of fighter `me` against `opp` (the input step): its raw pad `pad` through its button map
/// `buttons`, or a CPU fighter's AI `ai`; `timer` the round timer, `cam_yaw` the camera's yaw.
#[allow(clippy::too_many_arguments)]
pub(crate) fn input(
    fs: &mut [Fighter],
    me: usize,
    opp: usize,
    data: &Loaded,
    pad: u16,
    buttons: &[u16; 8],
    ai: Option<&mut crate::fight::ai::Ai>,
    timer: i16,
    cam_yaw: i16,
    freeze: bool,
) {
    let swap = facing_left(&fs[me], cam_yaw);
    let mut logical = input::map_pad(pad, buttons);
    let (f, opp) = pair_mut(fs, me, opp);
    let opp = &*opp;
    let cpu = f.o_01 != 0;
    if cpu {
        // the AI routine replaces the pad (none while KO'd / frozen)
        logical = 0;
        if (f.o_00 | f.o_01) & 0x80 == 0
            && !freeze
            && let Some(ai) = ai
        {
            logical = ai.think(f, opp, &data.fighters[me].ani, &data.trig, timer);
        }
    }
    input::process(f, &data.fighters[me].ani, opp, logical, swap, cpu);
}

/// The red health's recovery rate of a fighter with script file `ani` (the fighter step's start, F8): (RECOVER SPEED `recover`
/// << 8) / ANI+0x2C + 1, none at RECOVER SPEED 0.
pub(crate) fn recover_rate(recover: u8, ani: &crate::formats::act_ani::Ani) -> u16 {
    let r = recover as i32;
    let pa = ani.param(4).max(1) as i32;
    if r == 0 { 0 } else { ((r << 8) / pa + 1) as u16 }
}

/// Fighter `me`'s step against `opp`: health and gauge, the state flags, the automatic
/// turn, the move decision, the move script, the frame step and the animation, from simulation version 13 the head
/// turn, the velocity. The move scripts' effect requests (ops 0x16 / 0x17 / 0x18) go to `effect_reqs`.
pub(crate) fn fighter(fs: &mut [Fighter], me: usize, opp: usize, data: &Loaded, p: &mut Parts, effect_reqs: &mut Vec<(usize, [u8; 6])>) {
    // health and gauge (the fighter step's start)
    let rate = recover_rate(p.custom.recover, &data.fighters[me].ani);
    let (c8, ac_over) = (fs[me].o_c8 as i32, fs[me].o_ac > fs[me].o_b0);
    crate::fight::hit::health(&mut fs[me], p.custom.charge as i16, rate);
    // vibration of the damage
    if c8 < 0 {
        p.vibrate(0, 5, (0x40 - c8).min(255) as u8);
    } else if c8 > 0 {
        p.vibrate(0, 15, (4 * c8 + 0x40).min(255) as u8);
        p.vibrate(1, 15, 1);
    }
    if ac_over {
        p.vibrate(0, 120, 255);
        p.vibrate(1, 120, 1);
    }
    {
        let f = &mut fs[me];
        f.o_24 = data.fighters[me].ani.param(2 + f.beast() as usize);
    }
    // per-frame state flags before the decision
    {
        // the opponent's F58 (the team battle: each enemy's, the bits ORed)
        let mut guard = 0;
        for &e in p.enemies {
            let opp58 = fs[e].flags[F58];
            if (opp58 as i32) < 0 {
                guard |= 0x1000; // opponent is attacking: guarding possible
            }
            if opp58 & 0x3_0000 != 0 && opp58 & 0x4_0800 == 0 {
                guard |= 0xA00;
            }
            if opp58 & 0x804_0000 == 0x800_0000 {
                guard |= 0x200;
            }
        }
        let f = &mut fs[me];
        // ground under the hips, ring-out, OUT / AIR bits
        physics::ground(f, &p.world.stage);
        f.flags[F5C] |= guard;
    }
    // automatic turn
    let opp_f60 = fs[opp].flags[F60];
    physics::auto_turn(&mut fs[me], opp_f60);
    // move decision
    let group = data.fighters[me].ani.group(fs[opp].chr as usize);
    {
        let (f, o) = pair_mut(fs, me, opp);
        let o = &*o;
        let r = move_decision::decide(f, o, p.world, &group, &data.trig);
        if crate::base::trace::flag("GUARDDBG") && !crate::base::trace::is_muted() && me == 0 && (o.flags[F58] as i32) < 0 {
            eprintln!("guard {} r {:#x} F5C {:08x} held {:04x} o_ce {} F60 {:08x}", p.frame, r, f.flags[F5C], f.held, f.o_ce, f.flags[F60]);
        }
        if r >= 0 {
            let mv = r & 0xFFF;
            f.move_req = mv as i16;
            if mv >= 0x90 {
                f.flags[F5C] &= 0xEFFF_BFFC;
                f.s_14 = (mv - 0x90) as i16;
            }
            if r & 0x1000 != 0 {
                f.flags[F5C] |= 0x40;
            } else {
                f.flags[F5C] &= !0x40;
            }
        }
    }
    // debug: a move forced from outside (tests), requested like a decided one
    if p.force_move.is_some_and(|(k, _)| k == me) {
        fs[me].move_req = p.force_move.take().unwrap().1;
    }
    // beast transformation accepted: both motors, 60 frames
    if fs[me].move_req == 0xBE && fs[me].move_cur != 0xBE {
        p.vibrate(0, 60, 255);
        p.vibrate(1, 60, 1);
    }
    // script, then frame step and the animation
    let fr = &data.fighters[me];
    let acts = data.acts(me, opp);
    let mut sounds = Vec::new();
    let mut effects = Vec::new();
    // the decision's sound and burst (sound request (0x400, 7, 0), burst op17 sub F), queued
    // before the script's; simulation version 4 on (before, the sound piled up and the burst was dropped)
    if data.gates().revert_burst {
        sounds.extend(p.world.sounds.drain(..).map(|(_, id)| (id as i16, 0)));
        effects.append(&mut p.world.effects);
    } else {
        p.world.effects.clear();
    }
    let mut ctx = Ctx {
        ani: &fr.ani,
        acts,
        trig: &data.trig,
        rand: &mut *p.rand,
        sounds: &mut sounds,
        cam: &mut *p.cam,
        ring: (p.world.stage.ring.hx as i32, p.world.stage.ring.hz as i32),
        walls: [0, 1, 2, 3].map(|k| p.world.stage.walls.present(k)),
        effects: &mut effects,
        need: p.world.c0_b4,
        effects_off: p.round_state >= 0xF,
        stage: data.stage_no as i8,
        round: p.round_index,
        fades: &mut p.world.p4b18,
        stage_fade: &mut p.world.g4a58,
        gates: data.gates(),
    };
    move_script::run(fs, me, opp, &mut ctx);
    move_script::voice_timers(&mut fs[me], &mut sounds);
    effect_reqs.extend(effects.into_iter().map(|b| (me, b)));
    for mut s in sounds {
        s.0 = p.bank.br2_sound(s.0);
        if p.sound_queue.len() < 32 && s.0 >= 0 {
            p.sound_queue.push(s);
        }
    }
    let f = &mut fs[me];
    f.step = f.o_86;
    if f.beast() && f.flags[F58] & 0xC000_0000 == 0x8000_0000 && f.move_req != 0xBE {
        f.step = f.o_fc;
    }
    // the node positions before this frame's animation (F->40, root used by the push-apart)
    f.prev_root = [f.m_t[0] as i16, f.m_t[1] as i16, f.m_t[2] as i16];
    playback::step(f, &acts);
    physics::compose_yaw(f, &data.trig);
    timers(f);
    // the head turn, from both heads of the last world walk (simulation version 13)
    if data.gates().head_turn {
        move_script::head_turn(fs, me, opp, &data.trig, &data.tables);
    }
    // velocity of this frame
    physics::velocity(&mut fs[me], &data.trig);
}

/// Fighter `f`'s model transform in glTF space (game (x, y, z) → (x, −y, −z) / 300, then its yaw): the renderer's,
/// and the logic's up to simulation version 2 (then `portable::transform`).
pub(crate) fn transform(f: &Fighter) -> glam::Mat4 {
    let t = f.m_t;
    let pos = glam::Vec3::new(t[0] as f32, -t[1] as f32, -t[2] as f32) / 300.0;
    let yaw = f.yaw as f32 * std::f32::consts::TAU / 4096.0;
    glam::Mat4::from_translation(pos) * glam::Mat4::from_rotation_y(yaw)
}

/// Fighter `me`'s (a main-game fighter, `f`) skeleton this frame: its node positions (game units) and world
/// rotations, posed from its pose records or its clip (`opp`: the opponent whose ACT a throw victim plays).
pub(crate) fn skeleton(f: &Fighter, data: &Loaded, me: usize, opp: usize) -> (Vec<[i32; 3]>, Vec<[[i32; 3]; 3]>) {
    let fr = &data.fighters[me];
    let m = &fr.models[f.beast() as usize];
    let rsqrt = data.tables.rsqrt_fn();
    if data.portable() {
        // simulation version 3 on: f64 with libm's trig, the same bits on every target
        use crate::base::portable as p;
        let g = if f.pose_set {
            let l = crate::fight::anim::pose::locals_portable(f, &fr.skels[f.beast() as usize], &data.trig, &rsqrt);
            crate::formats::gltf_pose::pose_locals_globals(m, &l, &f.shape())
        } else {
            crate::formats::gltf_pose::pose_globals(
                m,
                crate::fight::fight_data::clip_for_vs(m, &f.anim, data.clip_opp(opp)),
                playback::glb_time(f, &data.acts(me, opp)),
                f.anim.mirror,
                &f.shape(),
            )
        };
        let xf = p::transform(f.m_t, f.yaw);
        return (p::node_positions(&g, xf), p::node_rotations(&g, xf));
    }
    let g = if f.pose_set {
        let l = crate::fight::anim::pose::locals(f, &fr.skels[f.beast() as usize], &data.trig, &rsqrt);
        crate::formats::gltf_model::pose_locals_globals(m, &l, &f.shape())
    } else {
        let clip = crate::fight::fight_data::clip_for_vs(m, &f.anim, data.clip_opp(opp));
        let t = playback::glb_time(f, &data.acts(me, opp));
        crate::formats::gltf_model::pose_globals_scaled(m, clip, t, f.anim.mirror, &f.shape())
    };
    (physics::node_positions(&g, transform(f)), crate::fight::fx::script_link::node_rotations(&g, transform(f)))
}

/// Attack timers after the animation step.
pub(crate) fn timers(f: &mut Fighter) {
    f.flags[F6C] &= !0x800;
    if f.o_ba > 0 {
        f.o_ba -= f.step;
        if f.o_ba <= 0 {
            f.o_ba = 0;
            f.o_26 = 0; // attack becomes active
            f.flags[F6C] |= 0x800;
        }
    }
    if f.o_b8 > 0 {
        f.o_b8 -= f.o_86;
        if f.o_b8 <= 0 {
            f.o_b8 = 0;
            f.o_26 = -1; // active window over
        }
    }
    if f.o_b8 == 0 && f.o_ba == 0 && f.o_bc >= 0 {
        f.o_bc = f.o_bc.wrapping_add(f.step);
    }
    // the stagger timer counts down, twice as fast while mashing
    if f.o_d2 > 0 {
        if f.pressed & 0xF0E8 != 0 {
            f.o_d2 = f.o_d2.wrapping_sub(f.step.wrapping_mul(2));
        }
        f.o_d2 = f.o_d2.wrapping_sub(f.step);
        if f.o_d2 <= 0 {
            f.o_d2 = 0;
        }
        crate::fight::input::reset_generic(f);
        crate::fight::input::reset_attack(f);
    }
    // invulnerability (+0xD0), kept only while standing in neutral (move 1)
    if f.o_d0 > 0 {
        f.o_27 = -1;
        f.o_d0 -= f.step;
        if f.o_d0 <= 0 || f.move_req != 1 {
            f.o_d0 = 0;
            f.o_27 = 0;
        }
    }
    if (f.flags[F64] as i32) < 0 && f.o_b8 < 0xC0 && !(f.step < f.o_bc) {
        f.flags[F6C] |= 0x4000;
    } else {
        f.flags[F6C] &= !0x4000;
    }
}

/// Fighter `f` (script file `ani`) let go out of a throw: F58 0x100000 and F6C 0x860000 (held, attached) cleared and
/// move `state` started (its script is the caller's to run: `run_start_move`).
pub(crate) fn release(f: &mut Fighter, ani: &crate::formats::act_ani::Ani, state: i16) {
    f.flags[F58] &= !0x10_0000;
    f.flags[F6C] &= !0x86_0000;
    move_script::start_move(f, ani, state);
}
