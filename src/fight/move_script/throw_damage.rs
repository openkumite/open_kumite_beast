//! Move-script op 19: throw damage and the beast revert it starts.
//!
//! State: none of its own: updates the serialised `Fighter`
//!
//! Op 19: damage from a throw or a follow-up, and the beast revert a deciding KO or an empty gauge
//! starts.

use super::revert_sound;
use crate::fight::fighter::{F6C, Fighter};

/// Op 19: damage from a throw or a follow-up. Sub 2 = the opponent's attack value (+0xDA) × `arg`
/// with both fighters' attack / defence factors; 0/1 = 3/10 or 1/10 of the last damage; 3 = `arg` points.
/// `opp` is the opponent (the thrower of a victim's script).
#[allow(clippy::too_many_arguments)]
pub(super) fn throw_damage(
    fs: &mut [Fighter],
    me: usize,
    opp: usize,
    sub: u8,
    arg: i16,
    need: u8,
    g: crate::fight::version::Gates,
    sounds: &mut Vec<(i16, i16)>,
    effects: &mut Vec<[u8; 6]>,
) {
    let (f, o) = crate::fight::fighter::pair_mut(fs, me, opp);
    let o = &*o;
    let mut gauge_opp = 0i16;
    match sub {
        0 | 1 => {
            if sub == 0 {
                f.o_cc = f.o_cc.wrapping_add(0x20);
            }
            if (f.o_ac as i32 + f.o_c8 as i32) <= f.o_b0 as i32 {
                if f.o_c8 != 0 {
                    f.o_ca = f.o_c8;
                }
                let d = if sub == 0 { (f.o_ca as i32 * 3) / 10 } else { f.o_ca as i32 / 10 };
                if f.o_c0 >= 0 {
                    f.o_c0 = f.o_c0.wrapping_add((d * 16) as i16);
                }
                f.o_c8 = f.o_c8.wrapping_add(d as i16);
                if f.o_b0 < f.o_ac.wrapping_add(f.o_c8) {
                    f.o_c8 = f.o_b0.wrapping_sub(f.o_ac);
                }
            }
        }
        2 => {
            let atk = if o.form & 0x10 != 0 { o.o_100 } else { o.o_fe } as i32;
            let mut d = o.o_da as i32 * arg as i32 * atk;
            d = (if d < 0 { d + 0x7F } else { d }) >> 7;
            let def = if f.form & 0x10 != 0 { f.o_104 } else { f.o_102 } as i32;
            d *= def;
            d = (if d < 0 { d + 0x7F } else { d }) >> 7;
            d += f.o_106 as i32;
            f.o_106 = (d & 0x7F) as u8;
            let d = d >> 7;
            if f.o_c0 >= 0 {
                f.o_c0 = (d * 16) as i16;
            }
            f.o_cc = 0;
            f.o_c8 = f.o_c8.wrapping_add(d as i16);
            if o.form & 0x10 == 0 && f.flags[F6C] & 0x8000 == 0 {
                gauge_opp = ((d / 2) * 60) as i16;
            }
            if f.form & 0x10 == 0 {
                f.o_d8 = f.o_d8.wrapping_add(((d / 3) * 60) as i16);
            } else {
                f.o_d8 = f.o_d8.wrapping_sub((d * 15 * 8) as i16);
            }
        }
        3 => {
            if f.o_c0 >= 0 {
                f.o_c0 = arg.wrapping_mul(16);
            }
            f.o_cc = 0;
            f.o_c8 = f.o_c8.wrapping_add(arg);
        }
        _ => {}
    }
    // a beast turns back when its gauge ran out or when this KO decides the match
    let decides = f.o_00 & 0x80 != 0 && f.o_e5 >= need;
    if f.form & 1 != 0 && (decides || f.gauge < 0) {
        sounds.push(revert_sound(g));
        effects.push([0x17, 0x0F, 0, 0, 0, 0]); // the burst = op17 sub F
        f.gauge = 0;
        f.o_d8 = 0;
        f.form = 0;
        f.o_ae = f.o_ac;
        crate::fight::input::reset_chain(f);
        crate::fight::input::reset_generic(f);
        crate::fight::input::reset_attack(f);
    }
    if gauge_opp != 0 {
        let o = &mut fs[opp];
        o.o_d8 = o.o_d8.wrapping_add(gauge_opp);
    }
}
