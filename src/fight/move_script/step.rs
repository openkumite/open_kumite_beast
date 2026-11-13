//! The move-script op dispatch `step`: one instruction of a fighter's script.
//!
//! State: none of its own: updates the serialised `Fighter`
//!
//! The op dispatch `step`: one instruction of a fighter's script (the op handlers, one match
//! over the opcode as one function), plus its helpers `trace` (an op not implemented) and `gated` (the form bits
//! of a sub-op). Op 19's damage is `throw_damage.rs`, ops 26 / 27 `sound.rs`.

use super::{Ctx, Next, op_size, rand, revert_sound, sound::sound_op, start_move, throw_damage::throw_damage};
use crate::fight::anim::playback;
use crate::fight::fighter::{F5C, F6C, F58, Fighter};
use crate::fight::physics::{self, wrap12};

pub(super) fn step(fs: &mut [Fighter], me: usize, opp: usize, ctx: &mut Ctx) -> Next {
    let ani = ctx.ani;
    let pc = fs[me].pc;
    let (op, sub) = (ani.u8(pc), ani.u8(pc + 1));
    let s8 = sub as i8;
    let size = op_size(&ani.bytes, pc);
    let next = pc + size;
    let f = &mut fs[me];
    match op {
        0x00 => match s8 {
            0 => Next::Stop,
            1 => Next::Go(next),
            3 if f.res == 0 => Next::Go(f.ret),
            4 if f.res != 0 => Next::Go(f.ret),
            5 if f.res >= 0 => Next::Go(f.ret),
            6 if f.res < 0 => Next::Go(f.ret),
            3..=6 => Next::Go(next),
            7 => Next::StopAt(next),
            8 => Next::Go(next), // a native call that does nothing
            _ => Next::Go(f.ret),
        },
        0x01 => match sub {
            0 => Next::Go(f.loop_mark),
            1 => {
                f.loop_mark = next;
                Next::Go(next)
            }
            _ => Next::Go(next),
        },
        0x02 => {
            if s8 >= 0 {
                if (f.anim.key as i32) < s8 as i32 { Next::Stop } else { Next::Go(next) }
            } else {
                // conditions (cond6..cond0 = -1..-7); <= -8 waits forever
                let o5c = fs[opp].flags[F5C];
                let f = &fs[me];
                let ended = f.anim.ended();
                let go = match s8 {
                    -1 => ended,
                    -2 => f.m_t[1] >= f.o_94 as i32,              // landed
                    -3 => o5c & 0x80 == 0,                        // opponent not airborne
                    -4 => f.flags[F5C] & 0x28 != 0 || ended,      // my attack touched, or anim end
                    -5 => (f.o_94 as i32) - f.hips[1] < 0xE7,     // hips near the floor
                    -6 => f.o_d2 == 0 || ended,                   // stagger over
                    -7 => f.flags[F5C] & 0x28 != 0 || f.o_bc > 0, // touched, or recovery running
                    _ => false,
                };
                if go { Next::Go(next) } else { Next::Stop }
            }
        }
        0x03 => {
            f.sleep = if sub == 0xFF { f.o_be as i32 * 16 } else { sub as i32 * 16 };
            Next::StopAt(next)
        }
        0x04 => {
            let gate = sub >> 6;
            let me_beast = f.beast();
            if (gate == 1 && me_beast) || (gate == 2 && !me_beast) {
                return Next::Go(next);
            }
            let t = &fs[if gate == 3 { opp } else { me }];
            let res = fs[me].res;
            let target = ani.u16(pc + 2) as usize;
            let (jump, call) = match sub & 0x3F {
                0 | 15 => (true, false),
                1 => (res == 0, false),
                2 => (res != 0, false),
                3 => (res >= 0, false),
                4 => (res < 0, false),
                5 => (true, true),
                6 => (res == 0, true),
                7 => (res != 0, true),
                8 => (res >= 0, true),
                9 => (res < 0, true),
                10 => (!t.beast(), false),
                11 => (t.beast(), false),
                12 => (fs[me].anim.ended(), false), // always the own animation
                13 => (t.o_00 & 0x80 != 0, false),  // KO'd
                14 => (t.form & 0x10 != 0 || t.gauge >= 0x1E00, false),
                c @ 16..=26 => (t.chr == (c - 16), false),
                _ => (true, false), // 27..63: unconditional jump
            };
            if call && jump {
                fs[me].ret = next;
            }
            Next::Go(if jump { target } else { next })
        }
        0x05 => {
            let gate = (sub >> 4) & 3;
            if (gate == 1 && f.beast()) || (gate == 2 && !f.beast()) {
                return Next::Go(next);
            }
            let arg = ani.i16(pc + 2);
            match sub & 0xF {
                0 => {
                    // arg -1 = the animation viewer of move 0 (own ACT 0, 1, 2, ...)
                    let count = ctx.acts.own.count() as i16;
                    let mut arg = arg;
                    if arg == -1 {
                        arg = f.anim_cycle;
                        if arg >= count {
                            f.anim_cycle = 0;
                            arg = 0;
                        }
                        f.anim_cycle = f.anim_cycle.wrapping_add(1);
                    }
                    if arg >= count {
                        return Next::Go(next); // out of range: nothing happens
                    }
                    if playback::locate(arg as i32, ctx.acts.own, ctx.acts.cmn).is_none() {
                        start_move(f, ani, 1);
                        return Next::Go(f.pc);
                    }
                    playback::start(f, ctx.trig, &ctx.acts, arg as i32);
                    Next::Go(next)
                }
                1 => {
                    let n = if arg == -1 { f.s_98 } else { arg };
                    start_move(f, ani, n);
                    Next::Go(f.pc)
                }
                2 => {
                    f.s_98 = arg; // move to continue with
                    Next::Go(next)
                }
                3 => {
                    f.s_9a = arg; // move to take on landing (returned by the decision)
                    Next::Go(next)
                }
                4 => {
                    f.s_16 = arg; // S.16
                    Next::Go(next)
                }
                6 => {
                    f.s_14 = arg; // S.14: the attack index used by the chain matching
                    Next::Go(next)
                }
                5 => {
                    // animation of the opponent's ACT (throw victims); none → move 1
                    if arg >= 0 && ctx.acts.opp.offset(arg as usize).is_some() {
                        playback::start_opp(f, ctx.trig, &ctx.acts, arg as i32);
                        Next::Go(next)
                    } else {
                        start_move(f, ani, 1);
                        Next::Go(f.pc)
                    }
                }
                7 => {
                    // chain_moves[chain_len - 1] = arg
                    let k = (f.chain_len as usize).wrapping_sub(1);
                    if let Some(c) = f.chain_moves.get_mut(k) {
                        *c = arg;
                    }
                    Next::Go(next)
                }
                _ => {
                    trace(f, pc, op, sub);
                    Next::Go(next)
                }
            }
        }
        0x0A => {
            // sub & 0xC0: 0x40 human only, 0x80 beast only, 0xC0 the opponent's flags
            match sub & 0xC0 {
                0x40 if f.beast() => return Next::Go(next),
                0x80 if !f.beast() => return Next::Go(next),
                _ => {}
            }
            let v = ani.u16(pc + 2) as u32 | (ani.u16(pc + 4) as u32) << 16;
            let t = if sub & 0xC0 == 0xC0 { opp } else { me };
            let s = (sub & 0x3F) as usize;
            let (w, k) = if s < 0x19 {
                ([1, 3, 2, 4, 5][s % 5], s / 5)
            } else if s < 0x1E {
                (6, s - 0x19)
            } else {
                (0, s - 0x1E)
            };
            let fl = &mut fs[t].flags[w];
            match k {
                0 => *fl = v,
                1 => *fl &= v,
                2 => *fl |= v,
                3 => *fl ^= v,
                _ => fs[me].res = (*fl & v) as i32,
            }
            Next::Go(next)
        }
        0x06 => {
            // side flip (mirror the following poses)
            let (o9c, o100) = (fs[opp].anim.flip_next, fs[opp].anim.mirror);
            let f = &mut fs[me];
            match s8 {
                -4 => {
                    if f.anim.flip_next {
                        f.anim.flip_next = false;
                        f.anim.mirror ^= true;
                        f.swap_hands();
                    }
                }
                -3 => {
                    f.anim.flip_next = o9c;
                    if f.anim.mirror != o100 {
                        f.anim.mirror = o100;
                        f.swap_hands();
                    }
                }
                -2 => {
                    f.anim.flip_next = false;
                    if f.anim.mirror {
                        f.anim.mirror = false;
                        f.swap_hands();
                    }
                }
                -1 => {
                    f.anim.flip_next = false;
                    f.anim.mirror ^= true;
                    f.swap_hands();
                }
                0 | 1 => f.anim.flip_next = sub != 0,
                _ => {} // other subs do nothing
            }
            Next::Go(next)
        }
        0x07 => {
            if gated(sub, fs[me].beast()) {
                return Next::Go(next);
            }
            let (a, b) = (ani.i16(pc + 2) as i32, ani.i16(pc + 4) as i32);
            let t = ctx.trig;
            let (ox, oy, oz) = (fs[opp].m_t[0], fs[opp].m_t[1], fs[opp].m_t[2]);
            let f = &mut fs[me];
            match sub & 0xF {
                0 => {
                    f.pos[0] = a as i16;
                    f.pos[2] = b as i16;
                }
                1 => {
                    let yw = f.yaw_base as i32;
                    let (c, s) = (t.cos(yw), t.sin(yw));
                    f.pos[0] = f.pos[0].wrapping_add(((a * c - b * s) >> 16) as i16);
                    f.pos[2] = f.pos[2].wrapping_add(((a * s + b * c) >> 16) as i16);
                }
                2 => {
                    f.pos[0] = (ox + a) as i16;
                    f.pos[1] = oy as i16;
                    f.pos[2] = (oz + b) as i16;
                }
                4 => {
                    if a < f.pos[1] as i32 - f.o_94 as i32 {
                        f.pos[1] = f.o_94.wrapping_add(a as i16);
                    }
                }
                5 => f.pos[1] = f.o_94.wrapping_add(a as i16),
                7 => {
                    f.pos[0] = f.hips[0] as i16;
                    f.pos[2] = f.hips[2] as i16;
                }
                8 => {
                    let ang = a + f.yaw as i32;
                    let (c, s) = (t.cos(ang), t.sin(ang));
                    let dx = b * c;
                    let dz = b * s;
                    f.pos[0] = f.pos[0].wrapping_add(((if dx < 0 { dx + 0xFFF } else { dx }) >> 12) as i16);
                    f.pos[2] = f.pos[2].wrapping_add(((if dz < 0 { dz + 0xFFF } else { dz }) >> 12) as i16);
                }
                6 => {
                    // stand `a` units inside the wall being touched
                    let (hx, hz) = ctx.ring;
                    let e = f.flags[F6C];
                    if e & 1 != 0 {
                        f.pos[0] = (hx - a) as i16;
                    } else if e & 2 != 0 {
                        f.pos[0] = (a - hx) as i16;
                    } else if e & 4 != 0 {
                        f.pos[2] = (hz - a) as i16;
                    } else if e & 8 != 0 {
                        f.pos[2] = (a - hz) as i16;
                    }
                }
                3 => {
                    // the base position jumps to the hips, `b` higher; the pelvis stays where it was on
                    // screen (pose record rewrite) and blends into the next animation (drawing offset, fight/sim/frame.rs)
                    f.pelvis_lock = Some(f.hips);
                    f.pos = [f.hips[0] as i16, (f.hips[1] - b) as i16, f.hips[2] as i16];
                }
                _ => trace(f, pc, op, sub),
            }
            Next::Go(next)
        }
        0x08 => {
            if gated(sub, fs[me].beast()) {
                return Next::Go(next);
            }
            let a = ani.i16(pc + 2) as i32;
            let opp_yaw = fs[opp].yaw as i32;
            let f = &mut fs[me];
            match sub & 0xF {
                0 => f.yaw_base = a as i16,
                1 => f.yaw_base = wrap12(f.yaw_base as i32 + a),
                2 => f.yaw_base = wrap12(opp_yaw + a),
                3 => f.s_e4 = wrap12(f.o_74 as i32 + a),
                4 => f.s_e4 = wrap12(f.o_c4 as i32 - (f.yaw as i32 - 0x800) + a),
                5 => f.s_e4 = wrap12(f.o_70 as i32 + a),
                6 => {
                    let v = f.flags[F6C];
                    let mut a3 = 0;
                    if v & 0x10 != 0 {
                        a3 = -0x400;
                    }
                    if v & 0x20 != 0 {
                        a3 = 0x400;
                    }
                    if v & 0x40 != 0 {
                        a3 = 0;
                    }
                    if v & 0x80 != 0 {
                        a3 = -0x800;
                    }
                    if v & 1 != 0 {
                        a3 = -0x400;
                    }
                    if v & 2 != 0 {
                        a3 = 0x400;
                    }
                    if v & 4 != 0 {
                        a3 = 0;
                    }
                    if v & 8 != 0 {
                        a3 = -0x800;
                    }
                    f.s_e4 = wrap12(a3 - (f.yaw as i32 - f.s_e4 as i32) + a);
                }
                7 => {
                    // turn toward the opponent by at most `a`
                    if f.flags[F5C] & 0x10_0000 == 0 {
                        let r = f.o_74 as i32;
                        f.s_e4 = wrap12(if r < -a {
                            f.s_e4 as i32 - a
                        } else if a < r {
                            f.s_e4 as i32 + a
                        } else {
                            f.s_e4 as i32 + r
                        });
                    }
                }
                9 => {
                    f.pos[0] = f.hips[0] as i16;
                    f.pos[2] = f.hips[2] as i16;
                    f.s_e4 = wrap12(f.o_c4 as i32 - (f.yaw as i32 - 0x800) + a);
                }
                10 => f.s_e4 = wrap12(f.s_e4 as i32 + a),
                _ => {}
            }
            Next::Go(next)
        }
        0x09 => {
            if gated(sub, fs[me].beast()) {
                return Next::Go(next);
            }
            let (a, b) = (ani.i16(pc + 2), ani.i16(pc + 4));
            let f = &mut fs[me];
            match sub & 0xF {
                0 => {
                    f.o_8a = a;
                    f.o_8c = b;
                }
                1 => {
                    f.o_96 = a;
                    f.o_98 = b;
                }
                2 => {
                    f.o_90 = a;
                    f.o_92 = b;
                }
                3 => f.o_8e = a,
                4 => {
                    f.o_96 = f.o_74;
                    let d = (a as i32).min(f.o_76 as i32);
                    f.o_98 = if b != 0 { ((d << 7) / b as i32) as i16 } else { 0 };
                }
                5 => {
                    f.o_96 = wrap12(f.o_70 as i32 + a as i32);
                    f.o_98 = b;
                }
                6 => f.o_96 = a,
                7 => f.o_92 = a,
                8 => f.o_98 = f.o_98.wrapping_add(a),
                9 => {
                    let v = f.o_98.wrapping_add(a);
                    f.o_98 = v.clamp(-0x500, 0x1000);
                }
                10 => f.o_98 = ((f.o_98 as i32 * a as i32) >> 7) as i16,
                _ => {}
            }
            Next::Go(next)
        }
        0x0C => {
            // animation timing (blend time into the current key, jump to a key)
            let a = ani.i16(pc + 2);
            let step = f.step;
            let be = f.o_be;
            let (c2, b2) = (f.o_c2, f.o_b2);
            let an = &mut f.anim;
            let set_blend = |an: &mut playback::Anim, n: i16| {
                if n < 2 {
                    an.c8 = step;
                    an.left = step;
                } else {
                    an.c8 = n.wrapping_mul(16);
                    an.left = n.wrapping_mul(16);
                }
            };
            match sub as i8 {
                0 => {
                    if a < 2 {
                        an.c2 = an.c2.wrapping_add(step.wrapping_sub(an.left));
                        an.c8 = step;
                        an.left = step;
                    } else {
                        let n = a.wrapping_mul(16);
                        an.c2 = an.c2.wrapping_add(n.wrapping_sub(an.left));
                        an.c8 = n;
                        an.left = n;
                    }
                }
                1 => {
                    an.key = a;
                    let e = an.base + 4 + 4 * a.max(0) as usize;
                    let b = &playback::act_of(an.src, &ctx.acts).bytes;
                    let dur = b.get(e + 2..e + 4).map(|s| i16::from_le_bytes([s[0], s[1]])).unwrap_or(0);
                    if dur < 2 {
                        an.c2 = an.c2.wrapping_add(step.wrapping_sub(an.next_dur));
                        an.next_dur = step;
                    } else {
                        let n = dur.wrapping_mul(16);
                        an.c2 = an.c2.wrapping_add(n.wrapping_sub(an.next_dur));
                        an.next_dur = n;
                    }
                    an.key += 1;
                    playback::key_setup(f, &ctx.acts);
                    return Next::Go(next);
                }
                2 => set_blend(an, be),
                3 => {
                    an.next_dur = an.next_dur.wrapping_add(a.wrapping_mul(16));
                    an.c2 = an.c2.wrapping_add(a.wrapping_mul(16));
                }
                4 => {
                    // blend time from +0xB2 (C division)
                    let mut n = b2 / 512 + a;
                    if n >= 16 {
                        n = 15;
                    }
                    set_blend(an, n);
                }
                5 => an.next_dur = an.next_dur.wrapping_add(c2.wrapping_mul(16)), // stretched by the hit stun
                6 => an.next_dur = an.next_dur.wrapping_add(be.wrapping_mul(16)),
                7 => {
                    an.left = an.left.wrapping_add(a.wrapping_mul(16));
                    an.c2 = an.c2.wrapping_add(a.wrapping_mul(16));
                    an.c8 = an.c8.wrapping_add(a.wrapping_mul(16));
                }
                _ => trace(f, pc, op, sub),
            }
            Next::Go(next)
        }
        0x11 => {
            // arm the automatic turn toward the opponent
            let arg = ani.i16(pc + 2) as i32;
            if s8 == 0 {
                let a3 = if f.flags[F5C] & 0x40 != 0 { arg } else { 0x400 };
                if (wrap12(f.o_74 as i32 + f.s_d2 as i32) as i32).abs() >= a3 {
                    return Next::Go(next);
                }
                let hi = ((a3 as i64 * 0xB40B_40B5u32 as i32 as i64) >> 32) as i32;
                f.s_10c = a3 as i16;
                f.s_10a = (((hi + a3) >> 7) - (a3 >> 31)) as i16;
            } else {
                f.s_10a = s8 as i16;
                f.s_10c = arg as i16;
            }
            f.flags[F5C] &= !0x800_0000;
            if f.s_10a != 0 {
                f.flags[F5C] |= 0x2_0000;
            } else {
                f.flags[F5C] &= !0x2_0000;
            }
            Next::Go(next)
        }
        0x0D | 0x0E | 0x0F | 0x10 | 0x23 => {
            let map = ani.bone_map(f.beast() as usize, f.anim.mirror);
            let bytes = &ani.bytes[pc..(pc + size).min(ani.bytes.len())];
            let mut b = [0u8; 8];
            b[..bytes.len()].copy_from_slice(bytes);
            let spheres = f.hit_spheres.len();
            crate::fight::hit::op_attack(f, op, sub, &b, map);
            if op == 0x0D && f.hit_spheres.len() > spheres {
                // the automatic claw / kick trail of a new hit sphere (S.101..S.104 from op 0x23)
                let node = f.hit_spheres[spheres].node as i32;
                let step = (f.step as i32).max(1);
                let [mode, d, l, fd] = f.s_101.map(|v| v as i32);
                let delay = if d < 0 { f.o_ba as i32 / step - 3 } else { d * 16 / step };
                let life = if l < 0 { f.o_b8 as i32 / step } else { l * 16 / step };
                let fade = if fd < 0 { 4 } else { fd * 16 / step };
                if crate::base::trace::flag("TRAILDBG") && !crate::base::trace::is_muted() {
                    eprintln!("op0D node {node} mode {mode} form {:x} life {life} delay {delay} fade {fade}", f.form);
                }
                let on = match mode {
                    -1 => f.form & 1 != 0 && matches!(node, 13 | 16),
                    2 => matches!(node, 7 | 10 | 13 | 16),
                    _ => false,
                };
                if on {
                    let c = |v: i32| v.clamp(-128, 127) as i8 as u8;
                    ctx.effects.push([0xF0, node as u8, c(life), c(delay), c(fade), 0]);
                }
            }
            Next::Go(next)
        }
        0x12 => {
            // form byte (bit 0 = beast model, 0x10 = transformed); the model switch follows it
            f.form = sub;
            Next::Go(next)
        }
        0x13 => {
            // hittable state (+0x27; -1 = cannot be hit)
            f.o_27 = sub as i8;
            Next::Go(next)
        }
        0x14 => {
            // fighter timers (frames → 1/16)
            let v = ani.i16(pc + 2).wrapping_mul(16);
            match sub {
                0 if f.o_c0 >= 0 => f.o_c0 = v,
                1 => f.o_d0 = v,
                2 => f.o_d2 = v,
                3 => f.o_cc = v,
                4 => f.o_d4 = v, // throw-escape window
                _ => {}
            }
            Next::Go(next)
        }
        0x19 => {
            let arg = ani.i16(pc + 2);
            throw_damage(fs, me, opp, sub, arg, ctx.need, ctx.gates, ctx.sounds, ctx.effects);
            Next::Go(next)
        }
        0x1C => {
            // bake the root offset into the base position
            physics::bake_root(f, ctx.trig);
            Next::Go(next)
        }
        0x1F => {
            // launch / fall parameters (+0xB2, +0xB4, +0xC4, +0x98)
            let a = ani.i16(pc + 2);
            let t = if sub & 0x40 != 0 { opp } else { me };
            if gated(sub, fs[t].beast()) {
                return Next::Go(next);
            }
            let f = &mut fs[me];
            match sub & 0xF {
                0 => f.o_b2 = ((f.o_b2 as i32 * a as i32) >> 7) as i16,
                1 => f.o_c4 = f.yaw.wrapping_add(a),
                2 => f.o_c4 = f.yaw.wrapping_add(f.o_70).wrapping_add(a),
                3 => {
                    f.o_b4 = a;
                    f.o_b2 = if f.o_98 < 0 { f.o_b2.wrapping_sub(f.o_98) } else { f.o_b2.wrapping_add(f.o_98) };
                    f.o_98 = 0;
                }
                4 => f.o_b4 = a,
                5 => f.o_b2 = ((f.o_72 as i32 * a as i32) >> 7) as i16,
                6 if a != 0 => f.o_b2 = (((f.o_72 as i32) << 7) / a as i32) as i16,
                _ => {}
            }
            Next::Go(next)
        }
        // 0x16 / 0x17 / 0x18: effect commands, run by the match
        0x16..=0x18 => {
            let mut b = [0u8; 6];
            for (k, v) in b.iter_mut().enumerate().take(size) {
                *v = ani.u8(pc + k);
            }
            ctx.effects.push(b);
            Next::Go(next)
        }
        // 0x1D: scale matrix of a skeleton node (MODEL TYPE shaping, 0x80 = 1.0)
        0x1D => {
            if !gated(sub, f.beast()) {
                let node = ani.i16(pc + 2) as usize;
                let s = ani.i16(pc + 4) as f32 / 128.0;
                let v = match sub & 0xF {
                    0 => glam::Vec3::splat(s),
                    1 => glam::Vec3::new(s, 1.0, s),
                    2 => glam::Vec3::new(s, 1.0, 1.0),
                    3 => glam::Vec3::new(1.0, s, 1.0),
                    4 => glam::Vec3::new(1.0, 1.0, s),
                    _ => glam::Vec3::ONE,
                };
                if crate::base::trace::flag("SHAPEDBG") && !crate::base::trace::is_muted() {
                    eprintln!("op1D P{} node {node} scale {v:?}", f.player + 1);
                }
                f.node_scale.retain(|(n, _)| *n != node);
                f.node_scale.push((node, v));
            }
            Next::Go(next)
        }
        0x1E => {
            // hide flags of the hand objects; 0B / 0E / 0F / 10 name the other hand when mirrored
            let v = ani.u16(pc + 2);
            let mir = f.anim.mirror;
            let h = &mut f.hand_hide[(f.form & 1) as usize];
            let slot = |obj: u8| match obj {
                11 => Some(0),
                14 => Some(1),
                15 => Some(2),
                16 => Some(3),
                _ => None,
            };
            let obj = match (sub, mir) {
                (0x0B, true) => 14,
                (0x0E, true) => 11,
                (0x0F, true) => 16,
                (0x10, true) => 15,
                (o, _) => o,
            };
            if sub == 0x80 {
                *h = [v; 4];
            } else if let Some(k) = slot(obj) {
                h[k] = v;
            }
            Next::Go(next)
        }
        0x26 | 0x27 => {
            let t = if sub & 0x40 != 0 { opp } else { me };
            if gated(sub, fs[t].beast()) {
                return Next::Go(next);
            }
            let id = ani.i16(pc + 2);
            let r = if op == 0x26 && matches!(sub & 0xF, 5 | 8) { rand(ctx.rand, &ctx.trig.random) } else { 0 };
            sound_op(fs, t, if t == me { opp } else { me }, op, sub & 0xF, id, if op == 0x27 { ani.i16(pc + 4) } else { 0 }, r, ctx.sounds);
            Next::Go(next)
        }
        0x2E => {
            // camera command
            let rd = |k: usize| if pc + 2 + 2 * k + 2 <= size + pc { ani.i16(pc + 2 + 2 * k) } else { 0 };
            ctx.cam.script(ctx.trig, s8, [rd(0), rd(1), rd(2)], (fs[me].player & 1) as usize);
            Next::Go(next)
        }
        0x24 => {
            // wall test → res (0: a present wall is touched, 1: any wall or edge contact)
            let t = if sub & 0x40 != 0 { opp } else { me };
            if gated(sub, fs[t].beast()) {
                return Next::Go(pc + 4);
            }
            let (e, f5c, hy) = (fs[t].flags[F6C], fs[t].flags[F5C], fs[t].hips[1]);
            let mut res = 0;
            if hy >= -0x280 && f5c & 0x100_0000 != 0 {
                res = match sub & 0xF {
                    0 => (0..4).any(|i| e & (1 << i) != 0 && ctx.walls[i]) as i32,
                    1 => 1,
                    _ => 0,
                };
            }
            fs[me].res = res;
            Next::Go(next)
        }
        0x29 => {
            if rand(ctx.rand, &ctx.trig.random) < sub as i32 {
                Next::Go(next)
            } else {
                Next::Go(ani.u16(pc + 2) as usize)
            }
        }
        0x2A => {
            f.halted = true;
            Next::Stop
        }
        0x0B | 0x15 | 0x1A => Next::Go(next), // no-ops (0x1A unused)
        0x1B => {
            // camera shake from F.24
            if sub == 0 {
                ctx.cam.shake[0] = f.o_24 / 16;
            } else if sub == 1 && !ctx.effects_off {
                ctx.cam.shake[1] = f.o_24;
            }
            Next::Go(next)
        }
        0x20 => {
            // forced throw (hit-throw follow-up move n + 0x160)
            let n = ani.i16(pc + 2);
            f.cand_attack.flags = 0x8000;
            f.cand_attack.mv = n.wrapping_add(0xD0);
            f.cand_attack.prio = 0xFFF;
            Next::Go(next)
        }
        0x21 | 0x22 | 0x2C | 0x25 => {
            // ops with an opponent bit: the gate tests T, the result goes to the own state
            let t = if sub & 0x40 != 0 { opp } else { me };
            if gated(sub, fs[t].beast()) {
                return Next::Go(next);
            }
            let tf = &fs[t];
            let res = match op {
                0x21 => {
                    // pad tests / input resets (bit 7: nothing)
                    let mask = ani.u16(pc + 2);
                    if sub & 0x80 != 0 {
                        None
                    } else {
                        match sub & 0xF {
                            1 => Some((tf.held & mask) as i32),
                            2 => Some((tf.pressed & mask) as i32),
                            3 => Some((tf.recent & mask) as i32),
                            4 => Some((tf.long & mask) as i32),
                            5 => Some((tf.dbl & mask) as i32),
                            6 => {
                                crate::fight::input::reset_chain(&mut fs[t]);
                                None
                            }
                            7 => {
                                crate::fight::input::reset_generic(&mut fs[t]);
                                None
                            }
                            8 => {
                                crate::fight::input::reset_attack(&mut fs[t]);
                                None
                            }
                            _ => None,
                        }
                    }
                }
                0x22 => {
                    // value − arg
                    let arg = ani.i16(pc + 2) as i32;
                    match sub & 0xF {
                        0 => Some(tf.m_t[1] - arg),
                        1 => Some(tf.o_90 as i32 - arg),
                        2 => Some(tf.move_req as i32 - arg),
                        3 => Some(tf.anim.id as i16 as i32 - arg),
                        4 => Some(ctx.round as i32 - arg),
                        5 => Some(ctx.stage as i32 - arg),
                        6 => Some(tf.o_98 as i32 - arg),
                        7 => Some(tf.anim.mirror as i32),
                        8 => Some(tf.o_d2 as i32 - arg),
                        _ => None,
                    }
                }
                0x2C => {
                    // attack phase / lying on the floor
                    let f58 = tf.flags[F58];
                    let atk = (f58 as i32) < 0;
                    match sub & 0xF {
                        0 => Some((atk && tf.o_ba != 0) as i32),
                        1 => Some((atk && tf.o_ba == 0 && tf.o_b8 != 0) as i32),
                        2 => Some((atk && tf.o_bc > 0) as i32),
                        3 => Some((f58 & 0x800 == 0 && f58 & 0x0803_0000 != 0) as i32),
                        _ => None,
                    }
                }
                _ => {
                    // swing envelope of the own extra node `idx` (sub 2; 0 / 1 descriptor flags unused)
                    let idx = ani.u8(pc + 2) as usize;
                    if sub & 0xF == 2
                        && let Some(sw) = idx.checked_sub(19).and_then(|k| fs[me].swing.get_mut(k))
                    {
                        sw.rise = ani.u8(pc + 3) as i8 as i16;
                        sw.hold = ani.u8(pc + 4) as i16;
                        sw.fall = ani.u8(pc + 5) as i8 as i16;
                        sw.target = ((ani.i16(pc + 6) as i32) << 12).wrapping_div(360) as i16;
                    }
                    None
                }
            };
            if let Some(r) = res {
                fs[me].res = r;
            }
            Next::Go(next)
        }
        0x28 => {
            // like op 0x11 but turns when the angle is OUTSIDE the range, back to the opponent
            let arg = ani.i16(pc + 2) as i32;
            if s8 == 0 {
                let a3 = if f.flags[F5C] & 0x40 != 0 { arg } else { 0x400 };
                if (wrap12(f.o_74 as i32 + f.s_d2 as i32) as i32).abs() <= a3 {
                    return Next::Go(next);
                }
                let hi = ((a3 as i64 * 0xB40B_40B5u32 as i32 as i64) >> 32) as i32;
                f.s_10c = a3 as i16;
                f.s_10a = (((hi + a3) >> 7) - (a3 >> 31)) as i16;
            } else {
                f.s_10a = s8 as i16;
                f.s_10c = arg as i16;
            }
            f.flags[F5C] |= 0x800_0000;
            if f.s_10a != 0 {
                f.flags[F5C] |= 0x2_0000;
            } else {
                f.flags[F5C] &= !0x2_0000;
            }
            Next::Go(next)
        }
        0x2B => {
            // stage darkening (sub 0) / this side's palette fade (sub 1; 2 only when none runs)
            let (a, b) = (ani.i16(pc + 2), ani.i16(pc + 4) as i32);
            match sub {
                0 => {
                    // no visible effect in a fight: the Beast Drive darkening overwrites mode / step every frame before the fade step
                    // steps them (Beast Drive +16, else −8), in the game as here
                    ctx.stage_fade[1] = 0;
                    ctx.stage_fade[3] = if a == 1 { 8 } else { -4 };
                }
                1 | 2 => {
                    let p = &mut ctx.fades[crate::fight::side::Slot::of_player(f.player)];
                    if sub == 1 || p[3] == 0 {
                        p[0] = a;
                        p[1] = 0;
                        p[3] = if b != 0 { ((b.abs() + 0xFF) / b) as i16 } else { -1 };
                    }
                }
                _ => {}
            }
            Next::Go(next)
        }
        0x2D => {
            // forced revert to human form; sub bit 1 = no sound, bit 0 = no burst
            if sub & 2 == 0 {
                ctx.sounds.push(revert_sound(ctx.gates));
            }
            if sub & 1 == 0 {
                ctx.effects.push([0x17, 0x0F, 0, 0, 0, 0]);
            }
            f.gauge = 0;
            f.o_d8 = 0;
            f.form = 0;
            f.flags[F6C] &= !0x8000;
            f.o_ae = f.o_ac;
            crate::fight::input::reset_chain(f);
            crate::fight::input::reset_generic(f);
            crate::fight::input::reset_attack(f);
            Next::Go(next)
        }
        0x2F => {
            // RON's counter F.DF
            let v = ani.i16(pc + 2);
            match sub {
                0 => f.o_df = v as u8,
                1 => f.o_df = f.o_df.wrapping_add(v as u8),
                2 => f.res = f.o_df as i32 - v as i32,
                _ => {}
            }
            Next::Go(next)
        }
        _ => {
            trace(f, pc, op, sub);
            Next::Go(next)
        }
    }
}

fn trace(f: &mut Fighter, pc: usize, op: u8, sub: u8) {
    if f.trace.len() < 64 {
        f.trace.push(format!("{pc:04X} op{op:02X}.{sub:02X} not ported"));
    }
}

/// Bits 4-5 of the sub-op: 1 = human form only, 2 = beast form only.
fn gated(sub: u8, beast: bool) -> bool {
    let g = (sub >> 4) & 3;
    (g == 1 && beast) || (g == 2 && !beast)
}
