//! The move-script interpreter: one script per fighter, one op a step.
//!
//! State: none of its own: runs the serialised `Fighter`
//!
//! Each fighter runs one script. A handler returns the next pc or "stop" (the pc is kept and the same
//! instruction runs again next frame). Offsets are relative to the start of the fighter's `.ANI`.
//! Field names follow the fighter/state-block offsets (e.g. `res` = state +0x94).
//!
//! Here: the instruction sizes, `Ctx`, `run` (a fighter's frame of script), `start_move` and the random table.
//! `step.rs` runs one instruction, `sound.rs` and `throw_damage.rs` hold the ops it hands on; `head_turn.rs` is the
//! end of the script step.

use crate::formats::act_ani::Ani;

mod head_turn;
mod sound;
mod step;
mod throw_damage;

pub use head_turn::{head_turn, square_root0};
pub use sound::{revert_sound, sound_remap, voice_timers};
use step::step;

/// Instruction sizes of the 48 opcodes (verified on every script of the disc, see ok_br2_converter::gamedata).
const SIZE: [u8; 48] = [
    2, 2, 2, 2, 4, 4, 2, 6, 4, 6, 6, 2, 4, 4, 4, 4, //
    4, 4, 2, 2, 4, 6, 2, 4, 6, 4, 4, 2, 2, 6, 4, 4, //
    4, 6, 4, 4, 2, 8, 4, 6, 4, 4, 2, 6, 2, 2, 2, 4,
];

pub fn op_size(b: &[u8], p: usize) -> usize {
    let (op, sub) = (b[p] as usize, b[p + 1]);
    match op {
        0x23 => {
            if sub & 0xF == 4 {
                6
            } else {
                4
            }
        }
        0x2E => match sub as i8 {
            0 | 5..=9 => 4,
            1..=4 => 8,
            _ => 2,
        },
        _ if op < 48 => SIZE[op] as usize,
        _ => 2,
    }
}

use crate::fight::anim::playback;
pub use crate::fight::fighter::{F5C, F6C, F54, F58, F60, F64, F68, Fighter};
use crate::fight::physics::Trig;

pub struct Ctx<'a> {
    pub ani: &'a Ani,
    pub acts: playback::Acts<'a>,
    pub trig: &'a Trig,
    pub rand: &'a mut u32,
    /// sound requests of this frame: (id after the P2 remap, delay in frames) for the SFX queue
    pub sounds: &'a mut Vec<(i16, i16)>,
    pub cam: &'a mut crate::fight::camera::Camera,
    /// ring half sizes and the walls present (a wall's hp != 0)
    pub ring: (i32, i32),
    pub walls: [bool; 4],
    /// effect opcodes 0x16 / 0x17 / 0x18 of this frame (raw bytes), started by the match
    pub effects: &'a mut Vec<[u8; 6]>,
    /// rounds needed to win the match (world.c0_b4)
    pub need: u8,
    /// M.B1: effects off (continue / game-over screens)
    pub effects_off: bool,
    /// M.98 stage, M.99 round index (−1 = sudden death)
    pub stage: i8,
    pub round: i8,
    /// fade records: per side and the stage darkening, {delay, mode, level, step}
    pub fades: &'a mut crate::fight::side::PerSlot<[i16; 4]>,
    pub stage_fade: &'a mut [i16; 4],
    /// the rules of the session's simulation version (`fight::version`)
    pub gates: crate::fight::version::Gates,
}

/// Next byte of the game's random byte table (an 8-bit counter).
pub fn rand(counter: &mut u32, table: &[u8]) -> i32 {
    let v = table[(*counter & 0xFF) as usize % table.len().max(1)] as i32;
    *counter = counter.wrapping_add(1) & 0xFF;
    v
}

/// Start move `n`: pc = ANI + entry[n], clear per-move flags.
pub fn start_move(f: &mut Fighter, ani: &Ani, n: i16) {
    let e = ani.entries.get(n as usize).copied().unwrap_or(0) as usize;
    f.move_req = n;
    f.move_cur = n;
    f.pc = e;
    f.loop_mark = e;
    f.sleep = 0;
    f.halted = e == 0;
    f.flags[F5C] &= 0xFFFC_7FD7;
    f.flags[F60] &= 0xA9DF_FFEE;
    f.flags[F6C] &= !0x2000;
    // object fields reset by the move start (the hand-mesh flags of the model are cosmetic, not kept here)
    f.o_8c = 0;
    f.o_8e = 0;
    f.o_26 = -1;
    f.hit_spheres.clear();
    f.o_27 = 0;
    f.o_ba = 0;
    f.o_b8 = 0;
    f.voice_timer = -1; // state +0x110
    // fists shown, open hands hidden (current form's list)
    f.hand_hide[(f.form & 1) as usize] = [0, 0, 1, 1];
}

enum Next {
    Go(usize),
    Stop,
    StopAt(usize),
}

/// Run fighter `me`'s script for one frame; `opp` is its opponent (the ops with an
/// opponent bit read or write it).
pub fn run(fs: &mut [Fighter], me: usize, opp: usize, ctx: &mut Ctx) {
    if fs[me].move_req != fs[me].move_cur {
        let n = fs[me].move_req;
        start_move(&mut fs[me], ctx.ani, n);
    }
    if fs[me].sleep > 0 {
        fs[me].sleep -= fs[me].anim.speed as i32; // state +0xCE
        return;
    }
    for _ in 0..256 {
        if fs[me].halted || fs[me].pc == 0 || fs[me].pc + 2 > ctx.ani.bytes.len() {
            return;
        }
        match step(fs, me, opp, ctx) {
            Next::Go(p) => fs[me].pc = p,
            Next::Stop => return,
            Next::StopAt(p) => {
                fs[me].pc = p;
                return;
            }
        }
    }
}
