//! The 42 op handlers of the imported game's state scripts, one match over the opcode.
//!
//! State: none of its own: updates the serialised `Br1Fighter`s
//!
//! `op` runs the instruction at `pc` and returns the next pc, or None to stop (the handler's 0). The form gate of a
//! sub byte comes in two shapes: `(s8 sub) >> 4` compared with 1 (human only) / 2 (beast only) for ops 07..10, 1D;
//! the same `& 3` for ops 05, 1F, 21..27 (whose bit 0x40 may point the test at the opponent). Ops 04 / 0A gate on
//! `sub & 0xC0`. A gated op returns its next pc, except op 24 (pc + 4, as the game does).

use super::{Call, CamWord, Ctx, Request, div, model_switch, start_state};
use crate::fight::br1::object::Br1Fighter;
use crate::fight::br1::pose::{self, Src};
use crate::fight::physics::wrap12;

/// The gate `g` (1 human only, 2 beast only) closed for a fighter of form `beast`.
fn gated(g: i32, beast: bool) -> bool {
    (g == 1 && beast) || (g == 2 && !beast)
}

/// Low RAM 0x00..0x0F as the console's BIOS leaves it (every recorded RAM dump holds these words).
const LOW_RAM: [u8; 16] = [0xFF, 0, 0, 0, 0x80, 0x0C, 0x5A, 0x27, 0x08, 0, 0x40, 0x03, 0, 0, 0, 0];

/// CMN.ACT's table read past its end (S.B0[k], k ≥ its count): the raw word of the file there (only the table's
/// entries are relocated) as a pointer; one into the static tables' image is that clip there (the scripts' clip 143: the word
/// 0x00720DE6, a pointer into the image's read-only strings, played as keys — constant in every RAM dump). Its image file
/// offset; None for an entry of the table, or a pointer elsewhere (not modelled: the null clip).
pub(super) fn image_clip(res: &crate::fight::br1::res::Br1Res, k: i32) -> Option<usize> {
    let k = usize::try_from(k).ok().filter(|&k| k >= res.cmn_clips.count())?;
    let w = u32::from_le_bytes(res.cmn.get(8 + 4 * k..12 + 4 * k)?.try_into().ok()?);
    crate::fight::br1::address_map::image_offset(w, res.image_len)
}

/// The clip start with S.B8 = 0: op 0x05 sub 0 on an empty CMN.ACT slot (the scripts of all nine fighters
/// name clips 54 and 55, which no fighter and CMN.ACT have) reads its clip header and key list at address 0.
/// The timing fields get low RAM's words; the animation step then skips the fighter (S.B8 == 0), so the clip never
/// ends. (`pose::start` has done the root bake and the mirror toggle.)
fn null_clip(a: &mut pose::Anim, step: i16) {
    let w = |o: usize| i16::from_le_bytes([LOW_RAM[o], LOW_RAM[o + 1]]);
    a.elapsed = 0;
    a.total = w(0).wrapping_shl(4);
    let d0 = w(6);
    a.next_dur = if d0 == 1 { step } else { d0.wrapping_shl(4) };
    a.key = 1;
    // the key setup on key 1 at S.BC + 4 = 8: S.C0 = 0 + off (a pointer into low RAM: kept as `target`)
    a.d0 = a.next_dur;
    if a.next_dur != 0 {
        a.left = a.next_dur;
        a.speed = step;
        a.target = w(8) as u16 as usize;
        let dur = w(10);
        a.next_dur = if dur == 1 { step } else { dur.wrapping_shl(4) };
    } else {
        a.speed = 0;
        a.left = 0;
    }
}

pub(super) fn op(fs: &mut [Br1Fighter; 2], me: usize, ctx: &mut Ctx, pc: u32) -> Option<u32> {
    let res = ctx.res[me];
    let anm = &res.anm;
    let p = pc as usize;
    let (code, sub) = (anm.u8(p), anm.u8(p + 1));
    let s8 = sub as i8 as i32;
    let g4 = s8 >> 4;
    let g3 = g4 & 3;
    let len = super::OPLEN.get(code as usize).copied().unwrap_or(2) as u32;
    let next = pc.wrapping_add(len);
    let a = anm.i16(p + 2) as i32;
    let b = anm.i16(p + 4) as i32;
    let au = anm.u16(p + 2) as u32;
    let opp = 1 - me;
    let beast = fs[me].beast();
    let f = &mut fs[me];
    match code {
        // stop / next / return by the result
        0x00 => match s8 {
            0 => None,
            1 => Some(next),
            3 if f.s.res == 0 => Some(f.ret),
            4 if f.s.res != 0 => Some(f.ret),
            5 if f.s.res >= 0 => Some(f.ret),
            6 if f.s.res < 0 => Some(f.ret),
            3..=6 => Some(next),
            7 => {
                f.pc = next;
                None
            }
            8 => {
                // a presentation call nothing answers (the main game's counterpart is a no-op too)
                ctx.call(me, Call::Idle, &[]);
                Some(next)
            }
            _ => Some(f.ret),
        },
        // loop mark
        0x01 => match s8 {
            0 => Some(f.loop_mark),
            1 => {
                f.loop_mark = next;
                Some(next)
            }
            _ => Some(next),
        },
        // wait
        0x02 => {
            if s8 >= 0 {
                return if (f.anim.key as i32) < s8 { None } else { Some(next) };
            }
            let o54 = fs[opp].f.f54;
            let f = &fs[me];
            let ended = f.anim.ended();
            let go = match s8 {
                -1 => ended,
                -2 => f.world(0)[1] >= f.f.floor as i32,
                -3 => o54 & 0x80 == 0,
                -4 => f.f.f54 & 8 != 0 || ended,
                -5 => (f.f.floor as i32).wrapping_sub(f.hips_y()) < 0xE7,
                -6 => f.f.fc8 == 0 || ended,
                -7 => f.f.f54 & 8 != 0 || f.f.recovery > 0,
                _ => false,
            };
            if go { Some(next) } else { None }
        }
        // sleep sub frames (no 0xFF form)
        0x03 => {
            f.pc = next;
            f.s.sleep = (sub as i16) << 4;
            None
        }
        // jump / call to ANM + u16
        0x04 => {
            let gate = sub & 0xC0;
            if (gate == 0x40 && beast) || (gate == 0x80 && !beast) {
                return Some(next);
            }
            let t_chr = fs[if gate == 0xC0 { opp } else { me }].f.chr as i32;
            let f = &mut fs[me];
            let r = f.s.res;
            let (jump, call) = match sub & 0x3F {
                1 => (r == 0, false),
                2 => (r != 0, false),
                3 => (r >= 0, false),
                4 => (r < 0, false),
                5 => (true, true),
                6 => (r == 0, true),
                7 => (r != 0, true),
                8 => (r >= 0, true),
                9 => (r < 0, true),
                10 => (!beast, false),
                11 => (beast, false),
                12 => (f.anim.ended(), false),
                13 => (f.f.status & 0x80 != 0, false),
                14 => (f.f.rave != 0, false),
                c @ 16..=24 => (t_chr == c as i32 - 16, false),
                _ => (true, false),
            };
            if !jump {
                return Some(next);
            }
            if call {
                f.ret = next;
            }
            Some(au)
        }
        // clip / state control
        0x05 => {
            if gated(g3, beast) {
                return Some(next);
            }
            match s8 & 0xF {
                0 => {
                    let count = anm.clips.count() as i32;
                    let mut k = a;
                    if k == -1 {
                        k = f.s.clip_cycle as i32;
                        if k >= count {
                            f.s.clip_cycle = 0;
                            k = 0;
                        }
                        f.s.clip_cycle = f.s.clip_cycle.wrapping_add(1);
                    }
                    if k >= count {
                        return Some(next);
                    }
                    // one fall-through lookup: the own table, below 0x100 CMN.ACT's (taken as it is), else the
                    // opponent's (a throw victim playing the thrower's clip); none → state 1
                    let own = usize::try_from(k).ok().and_then(|i| anm.clips.get(i));
                    let (src, clip, scale) = if let Some(c) = own {
                        (Src::Own, Some(c.offset), 0x80)
                    } else if k < 0x100 {
                        match image_clip(res, k) {
                            Some(o) => (Src::Image, Some(o), f.anim.root_scale),
                            None => {
                                let c = usize::try_from(k).ok().and_then(|i| res.cmn_clips.get(i)).map(|c| c.offset);
                                (Src::Cmn, c, f.anim.root_scale)
                            }
                        }
                    } else if let Some(c) = ctx.res[opp].anm.clips.get(k as usize) {
                        (Src::Opp, Some(c.offset), f.anim.root_scale)
                    } else {
                        f.s.state_req = 1;
                        f.s.state_cur = 1;
                        start_state(f, res, 1);
                        return Some(f.pc);
                    };
                    (f.anim.src, f.anim.clip, f.anim.scale) = (src, clip, scale);
                    f.anim.id = k as i16;
                    let banks = res.banks(Some(&ctx.res[opp].anm));
                    let cur = f.cur as usize & 1;
                    let step = f.f.step;
                    pose::start(&mut f.anim, &mut f.forms[cur].rec, &banks, &res.trig, step);
                    if clip.is_none() {
                        null_clip(&mut f.anim, step);
                    }
                    Some(next)
                }
                1 => {
                    // bit 0x40 (the opponent's block) is never used by the data: the own block
                    let n = if a == -1 { f.s.next_state } else { a as i16 };
                    f.s.state_req = n;
                    f.s.state_cur = n;
                    start_state(f, res, n);
                    Some(f.pc)
                }
                2 => {
                    f.s.next_state = a as i16;
                    Some(next)
                }
                3 => {
                    f.s.land_state = a as i16;
                    Some(next)
                }
                4 => {
                    f.s.s1e = a as i16;
                    Some(next)
                }
                _ => Some(next),
            }
        }
        // mirror S.128 and "flip at the next clip" S.A4
        0x06 => {
            let (oa4, o128) = (fs[opp].anim.flip_next, fs[opp].anim.mirror);
            let f = &mut fs[me];
            match s8 {
                -3 => (f.anim.flip_next, f.anim.mirror) = (oa4, o128),
                -2 => (f.anim.flip_next, f.anim.mirror) = (false, false),
                -1 => {
                    f.anim.flip_next = false;
                    f.anim.mirror ^= true;
                }
                _ => f.anim.flip_next = sub != 0,
            }
            Some(next)
        }
        // the base position S.DC / DE / E0
        0x07 => {
            if gated(g4, beast) {
                return Some(next);
            }
            let o0 = fs[opp].world(0);
            let f = &mut fs[me];
            let t = &res.trig;
            let pos = &mut f.anim.pos;
            match s8 & 0xF {
                0 => (pos[0], pos[2]) = (a as i16, b as i16),
                1 => {
                    let y = f.anim.yaw_base as i32;
                    let (c, s) = (t.cos(y), t.sin(y));
                    let pos = &mut f.anim.pos;
                    pos[0] = pos[0].wrapping_add((a.wrapping_mul(c).wrapping_sub(b.wrapping_mul(s)) >> 16) as i16);
                    pos[2] = pos[2].wrapping_add((a.wrapping_mul(s).wrapping_add(b.wrapping_mul(c)) >> 16) as i16);
                }
                2 => *pos = [o0[0].wrapping_add(a) as i16, o0[1] as i16, o0[2].wrapping_add(b) as i16],
                3 => {
                    let hy = f.hips_y();
                    f.anim.pos[1] = hy.wrapping_add(a) as i16;
                    f.anim.t1[1] = (((f.anim.bind1[1] as i32 + b) << 4) as i16).wrapping_neg();
                    f.anim.t4[1] = 0;
                }
                4 => {
                    if a < pos[1] as i32 - f.f.floor as i32 {
                        pos[1] = (f.f.floor as i32 + a) as i16;
                    }
                }
                5 => pos[1] = (f.f.floor as i32 + a) as i16,
                6 => {
                    // the ring's half sizes from the stage's ring record (+4 x, +6 z); later bits win
                    let (hx, hz) = ctx.ring(super::super::physics::RING_HIPS).half();
                    let e = f.f.f64;
                    if e & 1 != 0 {
                        pos[0] = (hx - a) as i16;
                    }
                    if e & 2 != 0 {
                        pos[0] = (a - hx) as i16;
                    }
                    if e & 4 != 0 {
                        pos[2] = (hz - a) as i16;
                    }
                    if e & 8 != 0 {
                        pos[2] = (a - hz) as i16;
                    }
                }
                7 => {
                    let h = f.world(4);
                    (f.anim.pos[0], f.anim.pos[2]) = (h[0] as i16, h[2] as i16);
                }
                _ => {}
            }
            Some(next)
        }
        // facing S.D8 and the turn request S.10C
        0x08 => {
            if gated(g4, beast) {
                return Some(next);
            }
            let o46 = fs[opp].f.yaw as i32;
            let f = &mut fs[me];
            let (yaw, turn) = (f.f.yaw as i32, f.anim.turn as i32);
            match s8 & 0xF {
                0 => f.anim.yaw_base = a as i16,
                1 => f.anim.yaw_base = wrap12(f.anim.yaw_base as i32 + a),
                2 => f.anim.yaw_base = wrap12(o46 + a),
                3 => f.anim.turn = wrap12(f.f.opp_angle as i32 + a),
                4 => f.anim.turn = wrap12(f.f.knock_dir as i32 - (yaw - 0x800) + a),
                5 => f.anim.turn = wrap12(f.f.centre_angle as i32 + a),
                6 => {
                    // the wall's angle, the bits in this order (a later one wins)
                    let e = f.f.f64;
                    let mut w = 0;
                    for (bit, v) in [(0x10, -0x400), (0x20, 0x400), (0x40, 0), (0x80, -0x800), (1, -0x400), (2, 0x400), (4, 0), (8, -0x800)] {
                        if e & bit != 0 {
                            w = v;
                        }
                    }
                    f.anim.turn = wrap12(w - (yaw - turn) + a);
                }
                7 => {
                    // only within ±0x555 of the opponent: turn toward it by at most a
                    let o = f.f.opp_angle as i32;
                    if o.abs() >= 0x556 {
                        return Some(next);
                    }
                    f.anim.turn = wrap12(if o < -a {
                        turn - a
                    } else if a < o {
                        turn + a
                    } else {
                        turn + o
                    });
                }
                9 => {
                    let h = f.world(4);
                    (f.anim.pos[0], f.anim.pos[2]) = (h[0] as i16, h[2] as i16);
                    f.anim.turn = wrap12(f.f.knock_dir as i32 - (yaw - 0x800) + a);
                }
                _ => {}
            }
            Some(next)
        }
        // velocity sources
        0x09 => {
            if gated(g4, beast) {
                return Some(next);
            }
            let x = &mut f.f;
            match s8 & 0xF {
                0 => (x.v_angle, x.v_speed) = (a as i16, b as i16),
                1 => (x.launch_angle, x.launch_speed) = (a as i16, b as i16),
                2 => (x.vy, x.gravity) = (a as i16, b as i16),
                3 => x.v_decay = a as i16,
                4 => {
                    x.launch_angle = x.opp_angle;
                    let d = (x.opp_dist as i32).min(a);
                    x.launch_speed = div(d << 7, b) as i16;
                }
                5 => (x.launch_angle, x.launch_speed) = (wrap12(x.centre_angle as i32 + a), b as i16),
                6 => x.launch_angle = a as i16,
                _ => {}
            }
            Some(next)
        }
        // the flag words
        0x0A => {
            let gate = sub & 0xC0;
            if (gate == 0x40 && beast) || (gate == 0x80 && !beast) {
                return Some(next);
            }
            let v = au | (anm.u16(p + 4) as u32) << 16;
            let t = if gate == 0xC0 { opp } else { me };
            let s = (sub & 0x3F) as usize;
            if s >= 0x23 {
                return Some(next);
            }
            let x = &mut fs[t].f;
            let (w, k) = match s {
                0..0x19 => ([0x50, 0x58, 0x54, 0x5C, 0x60][s % 5], s / 5),
                0x19..0x1E => (0x64, s - 0x19),
                _ => (0x4C, s - 0x1E),
            };
            let word = match w {
                0x4C => &mut x.f4c,
                0x50 => &mut x.f50,
                0x54 => &mut x.f54,
                0x58 => &mut x.f58,
                0x5C => &mut x.f5c,
                0x60 => &mut x.f60,
                _ => &mut x.f64,
            };
            match k {
                0 => *word = v,
                1 => *word &= v,
                2 => *word |= v,
                3 => *word ^= v,
                _ => {
                    let r = (*word & v) as i32;
                    fs[me].s.res = r;
                }
            }
            if s == 0 {
                // F.50 set: the step back to normal (×3/2 attacking under rave)
                let x = &mut fs[t].f;
                x.step = x.step_normal;
                if x.rave > 0 && v & 0xC000_0000 != 0 && v & 0x4000_0000 == 0 {
                    x.step = (x.step_normal as i32 * 3 / 2) as i16;
                }
            }
            Some(next)
        }
        0x0B => Some(next),
        // clip timing
        0x0C => {
            let step = f.f.step;
            let blend = |n: i32| if n < 2 { step } else { (n << 4) as i16 };
            match s8 {
                0 => (f.anim.d0, f.anim.left) = (blend(a), blend(a)),
                1 => {
                    f.anim.key = a as i16;
                    // key a's duration from the key list S.BC (= the clip + 4)
                    let banks = res.banks(Some(&ctx.res[opp].anm));
                    let d = match (f.anim.clip, banks.bank(f.anim.src)) {
                        (Some(c), Some(bank)) => pose::rd16(bank.bytes, (c as isize + 6 + 4 * a as isize) as usize) as i32,
                        _ => 0,
                    };
                    f.anim.next_dur = if d < 2 { step } else { (d << 4) as i16 };
                    f.anim.key = f.anim.key.wrapping_add(1);
                    pose::key_setup(&mut f.anim, &banks, step);
                }
                2 => {
                    let n = f.f.guard_stun as i32;
                    (f.anim.d0, f.anim.left) = (blend(n), blend(n));
                }
                3 => f.anim.next_dur = f.anim.next_dur.wrapping_add((a << 4) as i16),
                4 => {
                    let n = (f.f.push_speed as i32 / 512 + a).min(15);
                    (f.anim.d0, f.anim.left) = (blend(n), blend(n));
                }
                _ => {}
            }
            Some(next)
        }
        // an attack sphere and its trail
        0x0D => {
            if gated(g4, beast) {
                return Some(next);
            }
            let map = res.anm.bone_map(f.cur as usize, f.anim.mirror);
            let mut t0 = s8;
            for i in 0..8 {
                if f.spheres[i].flag == -1 {
                    f.spheres[i].flag = 0;
                    let n = map.get(anm.u8(p + 2) as usize).copied().unwrap_or(0).wrapping_add(1);
                    f.spheres[i].node = n;
                    f.spheres[i].radius = anm.u8(p + 3) as u16;
                    if i + 1 < 8 {
                        f.spheres[i + 1].flag = -1;
                    }
                    t0 = n as i8 as i32;
                    break;
                }
            }
            // the trail's timing
            let step = f.f.step as i32;
            let tt = f.s.trail_time as i32;
            let (mut at, lt) = if tt == -1 {
                (8, 0)
            } else if tt >= 0 {
                (div(tt << 4, step), 0)
            } else {
                (8, div((!tt) << 4, step))
            };
            let mut x = f.f.active as i32;
            if x >= 0 {
                x = div(x, step);
                if x < at {
                    at = 0;
                }
            }
            let tm = at | lt << 16;
            let (chr, mirror, human) = (f.f.chr as i32, f.anim.mirror, !beast);
            let side = me as i32;
            let mut fx = |id: u32, n1: i32, n2: i32| ctx.call(me, Call::Task, &[id as i32, side, tm, x, n1, n2]);
            let mode = f.s.trail_mode as i32;
            match mode {
                -1 => match t0 {
                    7 => fx(0x1000F, 6, 7),
                    10 => fx(0x1000F, 9, 10),
                    13 | 16 => fx(if human { 0x10012 } else { 0x10010 }, t0, chr),
                    _ => {}
                },
                1 | 2 if matches!(t0, 7 | 10 | 13 | 16) => fx(if mode == 1 { 0x10012 } else { 0x10010 }, t0, chr),
                3 if matches!(t0, 7 | 10 | 13 | 16) => fx(0x1000F, t0 - 1, t0),
                4..=7 => {
                    let n = match (mode, mirror) {
                        (4, false) | (5, true) => 12,
                        (4, true) | (5, false) => 15,
                        (6, false) | (7, true) => 6,
                        _ => 9,
                    };
                    fx(0x10012, n, chr);
                    fs[me].s.trail_mode = 0;
                }
                8 | 9 => fx(if human { 0x10012 } else { 0x10010 }, if mode == 8 { 13 } else { 16 }, chr),
                10 => fx(0x1000F, 6, 7),
                11 => fx(0x1000F, 9, 10),
                _ => {}
            }
            Some(next)
        }
        // damage (one s16) and the gauge it queues
        0x0E => {
            if gated(g4, beast) {
                return Some(next);
            }
            let gain = |x: &mut crate::fight::br1::object::Block| {
                x.gauge_queued = x.gauge_queued.wrapping_add(((x.attack_damage >> 3) as i16).wrapping_mul(60));
            };
            let x = &mut f.f;
            match sub & 0xF {
                0 => {
                    x.attack_damage = a as u8;
                    if x.form & 0x10 == 0 {
                        gain(x);
                    }
                }
                1 => x.attack_damage = x.attack_damage.wrapping_add(a as u8),
                2 => {
                    x.attack_damage = x.attack_damage.wrapping_add(a as u8);
                    if x.form & 0x10 == 0 {
                        gain(x);
                    }
                }
                _ => {}
            }
            f.s.damage_sum = f.s.damage_sum.wrapping_add(a as i16);
            Some(next)
        }
        // knockback
        0x0F => {
            if gated(g4, beast) {
                return Some(next);
            }
            f.f.knock_strength = anm.u8(p + 2);
            let k = (a >> 4) as i16;
            f.f.knock_angle = if f.anim.mirror { k.wrapping_neg() } else { k };
            Some(next)
        }
        // attack phases and the impact-sound class
        0x10 => {
            if gated(g4, beast) {
                return Some(next);
            }
            let c = f.f.f5c;
            let (oc, own) = if c & 0x200 != 0 {
                (0x40, 0x53)
            } else if c & 0x400 != 0 {
                (0x43, 0x56)
            } else if c & 0x1000_0000 != 0 {
                (0x46, 0x53)
            } else {
                (-1, -1)
            };
            fs[opp].s.impact_class = oc;
            let f = &mut fs[me];
            f.s.impact_own = own;
            if own >= 0 {
                match f.f.f5c & 0x38 {
                    0x10 => f.s.impact_own = own + 1,
                    0x20 => f.s.impact_own = own + 2,
                    _ => {}
                }
            }
            f.s.trail_mode = -1;
            f.s.trail_time = -1;
            f.f.attack = -1;
            f.spheres[0].flag = -1;
            f.f.recovery = if sub & 0xF != 0 { -1 } else { 0 };
            let st = anm.u8(p + 2) as i8;
            if st < 0 {
                ctx.call(me, Call::MoveCancel, &[f.f.player as i32]);
                f.f.startup = -1;
            } else {
                f.f.startup = (st as i16) << 4;
                if st == 0 {
                    f.f.attack = 0;
                }
            }
            let ac = anm.u8(p + 3) as i8;
            f.f.active = if ac < 0 { -1 } else { (ac as i16) << 4 };
            f.s.sound_timer = f.f.startup;
            Some(next)
        }
        // the auto-turn request; 0x28 also sets the "turn back" bit F.58 & 8
        0x11 | 0x28 => {
            f.s.auto_turn = s8 as i16;
            if code == 0x11 {
                f.f.f58 &= !8;
            } else {
                f.f.f58 |= 8;
            }
            if f.s.auto_turn == 0 {
                f.f.f54 &= !0x2_0000;
            } else {
                f.f.f54 |= 0x2_0000;
            }
            Some(next)
        }
        // the form byte and the model switch
        0x12 => {
            f.f.form = sub;
            model_switch(f, (sub & 1) as usize);
            Some(next)
        }
        // hittable
        0x13 => {
            f.f.hittable = sub as i8;
            Some(next)
        }
        // timers; sub 0 by the form's factor K
        0x14 => {
            match s8 {
                0 => {
                    if f.f.down_timer >= 0 {
                        f.f.down_timer = (a.wrapping_mul(f.factor()) as u32 >> 3) as i16;
                    }
                }
                1 => f.f.no_throw = (a << 4) as i16,
                2 => f.f.fc8 = (a << 4) as i16,
                3 => f.f.hit_stun = (a << 4) as i16,
                _ => {}
            }
            Some(next)
        }
        0x15 => Some(next),
        // screen effects at once
        0x16 => {
            match s8 {
                2 => ctx.call(me, Call::MoveCancel, &[f.f.player as i32]),
                n => ctx.call(me, Call::EffectScript, &[n, me as i32]),
            }
            Some(next)
        }
        // emitters at once
        0x17 => {
            let floor = if f.hips_y() > 0 { f.f.floor as i32 } else { 0 };
            let (side, yaw) = (me as i32, f.f.yaw as i32);
            match s8 {
                0 => ctx.call(me, Call::FloorEffect, &[a, 0, 0, floor]),
                1 => ctx.call(me, Call::FloorEffect, &[a, 0x100, yaw - 0x400, 0]),
                2 => ctx.call(me, Call::NodeEffect, &[side, a]),
                3 => ctx.call(me, Call::SparkPreset, &[side, a]),
                4 => ctx.call(me, Call::Task, &[0x21, side, 0, 0, 0, 0]),
                5 => ctx.call(me, Call::FloorEffect, &[a, 0x100, yaw + 0x400, 0]),
                6 => ctx.call(me, Call::Task, &[0x23, side, 0x41770, 1, 3, 0x10]),
                _ => {}
            }
            Some(next)
        }
        // emitters at once
        0x18 => {
            let k = match s8 {
                0 => Some((0x108, 0)),
                1 => Some((0xD, 0)),
                2 => Some((0x10A, if f.hips_y() > 0 { f.f.floor as i32 } else { 0 })),
                3 => Some((9, 0)),
                _ => None,
            };
            if let Some((k, fl)) = k {
                ctx.call(me, Call::NodeEmitter, &[a, k, b, fl]);
            } else if s8 == 4 {
                ctx.call(me, Call::BackColourScript, &[f.f.player as i32, a, b]);
            }
            Some(next)
        }
        // follow-up / throw damage on this fighter (the opponent = the attacker), then the revert
        0x19 => {
            op19(fs, me, ctx, s8, au as u16);
            Some(next)
        }
        // rebuild the bind pose of form (2a + F.5) & 1 (never used by the data)
        0x1A => {
            let form = (2 * a + f.f.form as i32) & 1;
            ctx.call(me, Call::FormSetup, &[me as i32, form]);
            Some(next)
        }
        // camera shake words
        0x1B => {
            match s8 {
                0 => ctx.push(me, Request::Cam { word: CamWord::Horizontal, value: f.f.weight }),
                1 if ctx.g_ad == 0 => ctx.push(me, Request::Cam { word: CamWord::Vertical, value: f.f.weight }),
                _ => {}
            }
            Some(next)
        }
        // commit the root motion
        0x1C => {
            let cur = f.cur as usize & 1;
            pose::root_bake(&mut f.anim, &mut f.forms[cur].rec, &res.trig);
            Some(next)
        }
        // node scale (never used by the data)
        0x1D => {
            if gated(g4, beast) {
                return Some(next);
            }
            ctx.call(me, Call::NodeScale, &[me as i32, s8 & 0xF, a, b << 5]);
            Some(next)
        }
        // object visibility
        0x1E => {
            let cur = f.cur as usize;
            f.parts.set(cur, sub, au as u16, f.anim.mirror);
            Some(next)
        }
        // launch parameters (the gate may test the opponent; the own fields are written)
        0x1F => {
            let tb = fs[if s8 & 0x40 != 0 { opp } else { me }].beast();
            if gated(g3, tb) {
                return Some(next);
            }
            let x = &mut fs[me].f;
            match s8 & 0xF {
                0 => x.push_speed = ((x.push_speed as i32).wrapping_mul(a) >> 7) as i16,
                1 => x.knock_dir = x.yaw.wrapping_add(a as i16),
                2 => x.knock_dir = x.yaw.wrapping_add(x.centre_angle).wrapping_add(a as i16),
                3 => {
                    x.push_decay = a as i16;
                    let l = x.launch_speed;
                    x.push_speed = if l < 0 { x.push_speed.wrapping_sub(l) } else { x.push_speed.wrapping_add(l) };
                    x.launch_speed = 0;
                }
                4 => x.push_decay = a as i16,
                _ => {}
            }
            Some(next)
        }
        // the forced throw candidate
        0x20 => {
            f.s.att_flags = 0x8000;
            f.s.att_prio = 0xFFF;
            f.s.att_mv = (au as u16).wrapping_add(0xB0) as i16;
            Some(next)
        }
        // a pad test into the result (always the own pads)
        0x21 => {
            let tb = fs[if s8 & 0x40 != 0 { opp } else { me }].beast();
            if gated(g3, tb) {
                return Some(next);
            }
            if s8 & 0x80 != 0 {
                return Some(next);
            }
            let f = &mut fs[me];
            let w = match s8 & 0xF {
                1 => f.f.held,
                2 => f.f.pressed,
                3 => f.f.recent,
                4 => f.f.long,
                5 => f.f.dbl,
                _ => return Some(next),
            };
            f.s.res = (w as u32 & au) as i32;
            Some(next)
        }
        // a value of the target minus a into the result
        0x22 => {
            let t = if s8 & 0x40 != 0 { opp } else { me };
            if gated(g3, fs[t].beast()) {
                return Some(next);
            }
            let x = &fs[t];
            let v = match s8 & 0xF {
                0 => x.world(0)[1],
                1 => x.f.vy as i32,
                2 => x.s.state_req as i32,
                3 => x.anim.id as i32,
                4 => ctx.round as i32,
                5 => ctx.stage as i32,
                6 => x.f.launch_speed as i32,
                _ => return Some(next),
            };
            fs[me].s.res = v.wrapping_sub(a);
            Some(next)
        }
        // the trail mode / time of op 0x0D
        0x23 => {
            let tb = fs[if s8 & 0x40 != 0 { opp } else { me }].beast();
            if gated(g3, tb) {
                return Some(next);
            }
            let f = &mut fs[me];
            match s8 & 0xF {
                0 => f.s.trail_mode = a as i16,
                1 => f.s.trail_time = a as i16,
                2 => f.s.trail_time = !(a as i16),
                _ => {}
            }
            Some(next)
        }
        // wall contact into the result
        0x24 => {
            let t = if s8 & 0x40 != 0 { opp } else { me };
            if gated(g3, fs[t].beast()) {
                return Some(pc.wrapping_add(4));
            }
            let k = s8 & 0xF;
            if k > 1 {
                return Some(next);
            }
            fs[me].s.res = 0;
            let (me4c, me50, hy) = (fs[me].f.f4c, fs[me].f.f50, fs[me].hips_y());
            let ok = if me4c & 8 != 0 {
                hy < -0x7F
            } else if fs[opp].f.f4c & 8 != 0 {
                if me50 & 0x4010_0000 != 0 { hy < -0x7F } else { hy < -0xFF }
            } else {
                true
            };
            if !ok || hy < -0x280 || fs[t].f.f54 & 0x100_0000 == 0 {
                return Some(next);
            }
            let w = fs[t].f.f64;
            let hit = if k == 0 { (0..4).any(|i| w & (1 << i) != 0 && ctx.walls[i]) } else { true };
            if hit {
                fs[me].s.res = 1;
            }
            Some(next)
        }
        // chain-bone behaviour bits (record +6 of node a)
        0x25 => {
            let tb = fs[if s8 & 0x40 != 0 { opp } else { me }].beast();
            if gated(g3, tb) {
                return Some(next);
            }
            let m = anm.u16(p + 4);
            let f = &mut fs[me];
            let cur = f.cur as usize & 1;
            if let Some(r) = usize::try_from(a).ok().and_then(|n| f.forms[cur].rec.get_mut(n)) {
                match s8 & 0xF {
                    0 => r.twist = (r.twist as u16 | m) as i16,
                    1 => r.twist = (r.twist as u16 & !m) as i16,
                    _ => {}
                }
            }
            Some(next)
        }
        // sounds
        0x26 => {
            if gated(g3, beast) {
                return Some(next);
            }
            let pl = f.f.player;
            match s8 & 0xF {
                0 => ctx.sound(pl, me, a, 0),
                1 => {
                    let c = f.s.impact_class as i32;
                    let bin = |b: [i32; 4]| match f.f.f60 & 0x3C0 {
                        0x40 => Some(b[0]),
                        0x80 => Some(b[1]),
                        0x100 => Some(b[2]),
                        0x200 => Some(b[3]),
                        _ => None,
                    };
                    let id = match c {
                        -1 => None,
                        0x40 => bin([0x40, 0x41, 0x42, 0x42]),
                        0x43 => bin([0x43, 0x44, 0x45, 0x45]),
                        0x46 => bin([0x46, 0, 1, 1]),
                        c => Some(c),
                    };
                    if let Some(id) = id {
                        ctx.sound(pl, me, id, 0);
                    }
                }
                2 => fs[opp].s.impact_class = a as i16,
                3 => f.s.impact_own = a as i16,
                4 => f.s.sound_timer = (a << 4) as i16,
                5 => {
                    let r = ctx.rand();
                    ctx.sound(pl, me, r % 3 + if beast { 0x64 } else { 0x61 }, 0);
                }
                6 => {
                    // human 0xC3; beast by character: GRE, EVE, MIT 0xCB, URI none, the others 0xCA
                    let id = match (beast, f.f.chr) {
                        (_, 9..) => -1,
                        (false, _) => 0xC3,
                        (true, 1 | 2 | 4) => 0xCB,
                        (true, 8) => -1,
                        (true, _) => 0xCA,
                    };
                    if id >= 0 {
                        ctx.sound(pl, me, id, 0);
                    }
                }
                7 => ctx.call(me, Call::StageSound, &[0]),
                _ => {}
            }
            Some(next)
        }
        // a sound with a delay / the delayed sound slot
        0x27 => {
            if gated(g3, beast) {
                return Some(next);
            }
            match s8 & 0xF {
                0 => {
                    let d = div(b << 4, f.f.step as i32);
                    ctx.sound(f.f.player, me, a, d);
                }
                1 => {
                    f.s.delayed_sound = a as i16;
                    f.s.delayed_timer = (b << 4) as i16;
                }
                _ => {}
            }
            Some(next)
        }
        // random branch
        0x29 => {
            if ctx.rand() < sub as i32 {
                Some(pc.wrapping_add(4))
            } else {
                Some(au)
            }
        }
        // op 0x2A returns the same pc: the game spins; the rest of the table is not code
        _ => None,
    }
}

/// Op 0x19 on fighter `me` (the victim; t = the attacker): repeat damage (subs 0 / 1), throw damage
/// (2), fixed damage (3); then the revert to human form when the beast is KO'd for good or its gauge ran out.
fn op19(fs: &mut [Br1Fighter; 2], me: usize, ctx: &mut Ctx, sub: i32, arg: u16) {
    let t = 1 - me;
    let t_damage = fs[t].f.attack_damage as i32;
    let t_transformed = fs[t].f.form & 0x10 != 0;
    let f = &mut fs[me];
    let k = f.factor();
    let x = &mut f.f;
    match sub {
        0 | 1 => 'rep: {
            if sub == 0 {
                x.hit_stun = 0x140;
            }
            if (x.max_hp as i32) < x.damage as i32 + x.queued_damage as i32 {
                break 'rep;
            }
            if x.queued_damage != 0 {
                x.last_damage = x.queued_damage;
            }
            let v = if sub == 0 { (x.last_damage as i32 * 3) / 10 } else { (x.last_damage as i32 / 10) as i16 as i32 };
            if x.down_timer >= 0 {
                x.down_timer = x.down_timer.wrapping_add((v.wrapping_mul(k).wrapping_shl(4) >> 7) as i16);
            }
            x.queued_damage = x.queued_damage.wrapping_add(v as i16);
            if (x.max_hp as i32) < x.damage as i32 + x.queued_damage as i32 {
                x.queued_damage = x.max_hp.wrapping_sub(x.damage);
            }
        }
        2 => {
            let prod = t_damage.wrapping_mul(arg as i16 as i32);
            let v = prod >> 7;
            if x.down_timer >= 0 {
                x.down_timer = (v.wrapping_mul(k) as u32 >> 3) as i16;
            }
            x.f54 &= 0xF7FF_FFFF;
            x.hit_stun = 0;
            x.queued_damage = x.queued_damage.wrapping_add(v as i16);
            let own_transformed = x.form & 0x10 != 0;
            if !t_transformed {
                let g = &mut fs[t].f.gauge_queued;
                *g = g.wrapping_add(((v / 2) * 60) as i16);
            }
            let x = &mut fs[me].f;
            if !own_transformed {
                x.gauge_queued = x.gauge_queued.wrapping_add(((v / 3) * 60) as i16);
            } else {
                x.gauge_queued = x.gauge_queued.wrapping_sub((((3 * v) / 2) * 60) as i16);
            }
        }
        3 => {
            let v = arg as i16 as i32;
            if x.down_timer >= 0 {
                x.down_timer = (v.wrapping_mul(k) as u32 >> 3) as i16;
            }
            x.f54 &= 0xF7FF_FFFF;
            x.hit_stun = 0;
            x.queued_damage = x.queued_damage.wrapping_add(v as i16);
        }
        _ => {}
    }
    // the revert (never Uriko)
    let f = &mut fs[me];
    if !f.beast() || f.f.chr == 8 {
        return;
    }
    let ko_for_good = f.f.status & 0x80 != 0 && f.f.rounds_lost >= ctx.need;
    if !ko_for_good && f.f.gauge >= 0 {
        return;
    }
    ctx.sound(f.f.player, me, 4, 0);
    ctx.call(me, Call::EffectScript, &[1, me as i32]);
    f.f.gauge = 0;
    f.f.gauge_queued = 0;
    f.f.form = 0;
    f.f.damage_perm = f.f.damage;
    model_switch(f, 0);
}
