//! Hits: can the attack hit, the sphere test, damage, hit and guard application.
//!
//! State: serialised: part of `GameState` (field changes are `STATE_VERSION` changes)
//!
//! Hits: whether the attack can hit, the sphere test, the direction bits,
//! the damage multiplier, the hit / guard apply, the trade rule of the pair step and the
//! health / gauge update of a fighter's step. Sphere centres come from the posed glTF skeleton (game units).
//!
//! Attack properties live in the attacker: F64 = attack type (0x40 low, 0x80 high, 0x100 mid, 0x1000 hits lying,
//! 0x4000 unblockable, 0x20000 guard crush, 0x10000 / 0x80000 / 0x100000 … reaction kinds), +0xDA damage,
//! +0xDC hit level, +0xDB / +0xC6 knockback, +0x26 attack state (-1 none, 0 active), hit spheres at +0x14.

use crate::fight::fighter::*;
use crate::fight::physics::wrap12;
use crate::formats::act_ani::Sphere;
use serde::{Deserialize, Serialize};

/// Attacker's hit sphere (fighter +0x14 list entry): skeleton node and radius.
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub struct HitSphere {
    pub node: u8,
    pub radius: i16,
}

pub fn can_hit(att: &Fighter, def: &Fighter) -> bool {
    if att.o_dc == 0 || att.o_26 == -1 || def.o_27 == -1 {
        return false;
    }
    if def.flags[F60] & 4 != 0 || def.flags[F5C] & 0x8008_0000 != 0 {
        return false;
    }
    let (a, d) = (att.flags[F64], def.flags[F58]);
    if a & 0x80 != 0 {
        d & 0x30 != 0x20
    } else if a & 0x40 != 0 {
        d & 0x4000 == 0
    } else if a & 0x100 != 0 {
        d & 0x30 == 0
    } else {
        true
    }
}

fn dist(a: [i32; 3], b: [i32; 3]) -> i32 {
    let d = [(a[0] - b[0]) as f64, (a[1] - b[1]) as f64, (a[2] - b[2]) as f64];
    (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt() as i32
}

/// Returns (kind, hit position): 0 = miss, 1 = sphere hit, 4 = proximity (radius-1 entries).
pub fn sphere_test(att: &Fighter, def: &Fighter, att_nodes: &[[i32; 3]], def_nodes: &[[i32; 3]], hurt: &[Sphere]) -> (i32, [i32; 3]) {
    let d58 = def.flags[F58];
    let lying = d58 & 0x4_0000 != 0 || (d58 & 0x3_0000 != 0 && d58 & 0x800_0000 == 0 && def.flags[F5C] & 0x8000_0080 != 0x80);
    if lying && att.flags[F64] & 0x1000 == 0 {
        return (0, [0; 3]);
    }
    for h in &att.hit_spheres {
        let Some(&c) = att_nodes.get(h.node as usize) else { continue };
        if h.radius == 1 {
            if def.flags[F5C] & 0x4_0080 == 0 && att.o_76.unsigned_abs() < 0x300 {
                return (4, def.m_t);
            }
            continue;
        }
        for s in hurt {
            let Some(&p) = def_nodes.get(s.node as usize) else { continue };
            if dist(c, p) < h.radius as i32 + s.radius as i32 {
                return (1, c);
            }
        }
    }
    (0, [0; 3])
}

/// Direction bits of the hit as seen by the defender (1 front, 2 back, 4/8 side).
pub fn direction_bits(att: &Fighter, def: &Fighter) -> u32 {
    let base = att.yaw as i32 + if att.o_db == 0 { att.o_74 as i32 } else { att.o_c6 as i32 };
    let v = wrap12(def.yaw as i32 - base - 0x800) as i32;
    let mut a = if v >= 0 { 4 } else { 8 };
    if v.abs() < 0x155 {
        a |= 1;
    } else if v.abs() >= 0x4E4 {
        a |= 2;
    }
    a
}

/// Damage multiplier (0x80 = 1.0).
pub fn multiplier(att: &Fighter, def: &mut Fighter) -> i32 {
    let mut m = 0x80;
    if (def.flags[F58] as i32) < 0 && (def.o_ba != 0 || def.o_b8 != 0) {
        m = 0x99; // counter hit
        def.flags[F68] |= 0x800_0000;
    }
    let d = def.flags[F58];
    if d & 0x20 != 0 && att.flags[F64] & 0x40 != 0 && d & 0x10 == 0 {
        return m * 2;
    }
    if def.flags[F68] & 2 != 0 || d & 8 != 0 || d == 0x402 {
        let v = m * 153;
        return (if v < 0 { v + 0x7F } else { v }) >> 7;
    }
    m
}

fn div_round0(v: i32, d: i32) -> i32 {
    if d == 0 { 0 } else { v / d }
}

fn scale7(v: i32, m: i32) -> i32 {
    let p = v * m;
    (if p < 0 { p + 0x7F } else { p }) >> 7
}

/// Combo damage scaling (the combo scaling table): ×1, ×1, ×0.9 … ×0.2, then ×0.1.
fn combo_scale(dd: u8, v: i32) -> i32 {
    match dd {
        0 | 1 => v,
        2 => v * 9 / 10,
        3 => v * 8 / 10,
        4 => v * 7 / 10,
        5 => v * 6 / 10,
        6 => v / 2,
        7 => v * 4 / 10,
        8 => v * 3 / 10,
        9 => v / 5,
        _ => v / 10,
    }
}

/// Result of applying a hit, for effects and sounds the caller plays.
#[derive(Debug, Clone, Copy)]
pub struct HitResult {
    pub guarded: bool,
    pub damage: i32,
    pub level: i32,
    pub launch: Option<u8>,
}

/// Apply the hit of `att` on `def` (kind from `sphere_test`, `mult` from `multiplier`).
pub fn apply(att: &mut Fighter, def: &mut Fighter, mult: i32, kind: i32) -> HitResult {
    att.o_26 = -1;
    let mut s0 = def.flags[F68];
    let mut s3 = att.flags[F64];
    if kind == 4 {
        let v = att.o_da as i32 * (0x300 - att.o_76 as i32);
        att.o_da = ((if v < 0 { v + 0x1FF } else { v }) >> 9) as u8;
        if att.o_da >= 0x14 {
            s3 |= 0x100_0000;
        }
    }
    let mut a1 = scale7(att.o_da as i32, mult); // damage
    let mut s2 = scale7(att.o_dc as i32, mult); // hit level
    if s3 & 0x40 != 0 {
        s0 |= 0x10_0000;
    } else if s3 & 0x100 != 0 {
        s0 |= 0x40_0000;
    } else if s3 & 0x80 != 0 {
        s0 |= 0x20_0000;
    }
    let mut s7: u32 = 0; // attacker F5C bits
    let mut s5: u32 = 0; // defender F5C bits
    // guard
    if s3 & 0x4000 == 0 && def.flags[F5C] & 1 != 0 {
        let (d5c, d58) = (def.flags[F5C], def.flags[F58]);
        let air = d5c & 0x80 != 0;
        let guard = if s3 & 0x40 != 0 {
            if air {
                d5c & 0x4000_0000 == 0
            } else if d58 & 0x10 != 0 {
                true
            } else if d58 & 0x20 != 0 {
                false
            } else {
                d58 & 1 != 0 && s3 & 0x80 != 0
            }
        } else if s3 & 0x80 != 0 || s3 & 0x100 != 0 {
            if air { d5c & 0x4000_0000 == 0 } else { d58 & 0x20 == 0 && d58 & 1 != 0 }
        } else {
            false
        };
        if guard {
            s5 = 0x4200_0000;
        }
        if d5c & 2 != 0 && s5 & 0x200_0000 != 0 {
            s5 |= 4;
        }
    }
    if s3 & 0x2_0000 != 0 && s5 & 4 == 0 {
        if s5 & 0x200_0000 != 0 {
            a1 /= 2;
        } else {
            s0 |= 0x80_0400;
        }
        s5 &= 0xBDFF_FFFF;
    }
    let s6;
    if s5 == 0 {
        // ---- hit ----
        att.o_dd = if def.flags[F5C] & 0x2000_0000 != 0 { att.o_dd.saturating_add(1) } else { 1 };
        s2 = combo_scale(att.o_dd, s2);
        a1 = combo_scale(att.o_dd, a1);
        def.o_b2 = (s2 * 96 + 0x800) as i16;
        s5 = 0x2400_0000;
        a1 = scale7(a1, if att.form & 0x10 != 0 { att.o_100 as i32 } else { att.o_fe as i32 });
        a1 = scale7(a1, if def.form & 0x10 != 0 { def.o_104 as i32 } else { def.o_102 as i32 });
        if def.flags[F5C] & 0x2000_0000 != 0 && def.flags[F58] & 0x100 != 0 {
            a1 /= 2;
        } else if def.flags[F58] & 0x800 == 0 && def.flags[F58] & 0x803_0000 != 0 && att.flags[F64] & 0x1000 == 0 {
            let v = a1 * 3;
            a1 = (if v < 0 { v + 3 } else { v }) >> 2;
        }
        s2 = s2.max(1);
        a1 = a1.max(1);
        s0 &= !0x340; // addiu -0x341 = !0x340
        def.o_c2 = (s2 + 6).max(0x10) as i16;
        def.o_c8 = a1 as i16;
        def.o_cc = (def.o_c2 - 5) * 16;
        if def.o_c2 >= 0x1B {
            s0 |= 0x200;
            def.o_c2 -= 0x1A;
        } else if def.o_c2 >= 0x16 {
            s0 |= 0x100;
            def.o_c2 -= 0x15;
        } else {
            s0 |= 0x40;
            def.o_c2 -= 0x10;
        }
        s0 |= reaction(def, s3, s0);
        def.o_c4 = wrap12(att.yaw as i32 + att.o_74 as i32);
        if s0 & 0x80_0400 == 0x80_0400 {
            if att.o_db == 0 {
                att.o_db = 0x40;
            } else {
                def.o_c4 = wrap12(att.yaw as i32 + att.o_c6 as i32);
            }
        }
        if def.o_c0 >= 0 {
            def.o_c0 = if s0 & 0x200_0000 != 0 { def.o_c0 / 2 } else { (def.o_c0 + (s2 * 16) as i16).min(0x12C0) };
        }
        if att.form & 0x10 == 0 && att.flags[F6C] & 0x8000 == 0 {
            att.o_d8 = att.o_d8.wrapping_add(((att.o_da >> 1) as i16).wrapping_mul(60));
        }
        if def.form & 0x10 == 0 {
            def.o_d8 = def.o_d8.wrapping_add((def.o_c8 / 3).wrapping_mul(60));
        } else {
            def.o_d8 = def.o_d8.wrapping_sub(def.o_c8.wrapping_mul(120));
        }
        s6 = 3;
        s7 |= 8;
    } else {
        // ---- guarded ----
        def.o_c4 = wrap12(att.yaw as i32 + att.o_74 as i32);
        if att.o_db == 0 {
            att.o_db = 0x40;
        }
        att.o_dd = if def.flags[F5C] & 0x4000_0000 != 0 { att.o_dd.saturating_add(1) } else { 1 };
        def.o_b2 = (s2 * 96 + 0x800) as i16;
        if att.form & 0x10 != 0 {
            def.o_c8 = (-a1 / 10) as i16;
        }
        let be = if s5 & 4 != 0 {
            let v = s2 * 3;
            (((if v < 0 { v + 3 } else { v }) >> 2) + 6).clamp(0xD, 0x78) - 0xC
        } else {
            (s2 / 2 + 5).clamp(0xA, 0x18) - 2
        };
        def.o_be = be as i16;
        def.o_cc = 0;
        let g = ((att.o_da >> 3) as i16).wrapping_mul(60);
        def.o_d8 = if def.form & 0x10 == 0 { def.o_d8.wrapping_add(g) } else { def.o_d8.wrapping_sub(g) };
        if def.flags[F58] & 0x800 != 0 {
            let mut vy = div_round0(-0x2A0 * 1000, def.o_24 as i32);
            if att.form & 0x10 != 0 {
                vy = vy * 3 / 2;
            }
            def.o_90 = vy as i16;
            def.o_92 = 0xE0;
        }
        s0 &= !0x340; // addiu -0x341 = !0x340
        s0 |= if s2 >= 0x1A {
            0x200
        } else if s2 >= 0x15 {
            0x100
        } else {
            0x40
        };
        s6 = 2;
        s7 |= 0x20;
    }
    // common tail
    if def.o_b2 != 0 {
        def.o_b4 = 0x68;
    }
    let launch_kind = launch_kind(s0);
    if def.flags[F54] & 0x20 != 0 {
        def.o_b2 /= 2;
        def.o_98 /= 2;
    }
    if def.o_b2 != 0 {
        if def.o_b2 < 0 {
            def.o_b2 = 0;
        }
        if def.flags[F6C] & 0xF0 != 0 {
            // pinned against a wall: the attacker is pushed back instead
            att.o_c4 = wrap12(att.yaw as i32 + att.o_70 as i32);
            att.o_b4 = def.o_b4;
            att.o_b2 = (def.o_b2 as i32 * 2 / 3) as i16;
            def.o_b2 /= 2;
            att.o_b2 = div_round0(att.o_b2 as i32 * 500, att.o_24 as i32) as i16;
        }
        def.o_b2 = div_round0(def.o_b2 as i32 * 500, def.o_24 as i32).min(0x6000) as i16;
    }
    if s3 & 0x2000_0000 != 0 && s5 & 0x400_0000 != 0 {
        s0 |= 0x8_0000;
        def.o_b2 = 0;
    }
    def.flags[F68] = s0;
    def.flags[F5C] = (def.flags[F5C] & 0xF9FF_FFFF) | s5;
    att.flags[F5C] |= s7;
    HitResult { guarded: s6 == 2, damage: a1, level: s2, launch: launch_kind }
}

/// Reaction flags from the attack type and the defender's state.
fn reaction(def: &mut Fighter, s3: u32, s0: u32) -> u32 {
    let d58 = def.flags[F58];
    let counter = s0 & 0x800_0000 != 0;
    let air = |s3: u32| -> u32 {
        if s3 & 0x8_0000 != 0 {
            if d58 & 0x80 != 0 { 0x80_4400 } else { 0x80_0C00 }
        } else if s3 & 0x400_0000 != 0 {
            0x84_0400
        } else if s3 & 0x100_0000 != 0 {
            0x81_0400
        } else if s3 & 0x20_0000 != 0 {
            0x80_2400
        } else {
            0x80_4400
        }
    };
    if d58 & 0x800 != 0 {
        return air(s3);
    }
    if d58 & 0x803_0000 != 0 {
        return 0x200_0400;
    }
    if def.flags[F5C] & 0x4_0000 != 0 {
        return air(s3);
    }
    def.o_90 = 0;
    def.o_92 = 0;
    if d58 & 0x100 != 0 {
        return if s3 & 0x8_0000 != 0 {
            0x80_0C00
        } else if s3 & 0x40_0000 != 0 {
            0x80_4400
        } else if s3 & 0x20_0000 != 0 {
            0x80_2400
        } else if s3 & 0x100_0000 != 0 {
            0x81_0400
        } else if s3 & 0x400_0000 != 0 {
            0x84_0400
        } else if s3 & 0x10_0000 != 0 {
            0x80_4400
        } else {
            0
        };
    }
    let soft = s3 & 0x1_0000 != 0; // only reacts strongly on a counter hit
    let big = s0 & 0x80_0000 != 0;
    if s3 & 0x8_0000 != 0 {
        if soft {
            if !counter {
                0
            } else if big {
                0xC00
            } else {
                0x80_4400
            }
        } else if counter || big {
            0xC00
        } else {
            0x80_4400
        }
    } else if s3 & 0x40_0000 != 0 {
        if soft {
            if counter { 0x4400 } else { 0 }
        } else if !counter {
            0x4400
        } else if big {
            0xC00
        } else {
            0x80_4400
        }
    } else if s3 & 0x20_0000 != 0 {
        if soft {
            if counter { 0x2400 } else { 0 }
        } else if counter {
            0x80_2400
        } else {
            0x2400
        }
    } else if s3 & 0x100_0000 != 0 {
        if soft {
            if counter { 0x1_0400 } else { 0 }
        } else if counter {
            0x81_0400
        } else {
            0x1_0400
        }
    } else if s3 & 0x400_0000 != 0 {
        if soft {
            if counter { 0x4_0400 } else { 0 }
        } else if counter {
            0x84_0400
        } else {
            0x4_0400
        }
    } else if s3 & 0x10_0000 != 0 {
        if soft && !counter { 0 } else { 0x1400 }
    } else if s3 & 0x80_0000 != 0 {
        if soft && !counter { 0 } else { 0x8400 }
    } else if s3 & 0x1000 != 0 {
        if d58 & 0x3_0000 != 0 && def.flags[F5C] & 0x80 == 0 { 0x200_0400 } else { 0 }
    } else {
        0
    }
}

/// Health and gauge of this frame: queued damage +0xC8, red bar +0xAE, KO.
/// `charge` = CHARGE SPEED (cb+3), `recover` = the beast recovery rate F8 = (RECOVER SPEED << 8) / ANI+0x2C + 1.
pub fn health(f: &mut Fighter, charge: i16, recover: u16) {
    // nothing below runs for a KO'd fighter (health −1), with F60 0x800000, or out of the fight —
    // later hits on the loser are counted (+0xC8) but never added
    if f.o_ac < 0 || f.flags[F60] & 0x80_0000 != 0 || f.o_00 != 1 {
        return;
    }
    // gauge (+0xD6) from the queued change +0xD8, passive gain, recovery
    if f.flags[F54] & 2 == 0 {
        if f.o_d8 < 0 {
            if f.gauge >= 0 {
                f.gauge = f.gauge.wrapping_add(f.o_d8);
                f.o_d8 = 0;
                if f.gauge < 0 {
                    f.gauge = -1;
                }
            }
        } else if f.o_d8 > 0 {
            if f.gauge >= 0 {
                f.gauge = f.gauge.wrapping_add(f.o_d8.wrapping_mul(charge));
                if f.gauge >= 0x3C00 {
                    f.gauge = 0x3BFF;
                }
            }
            f.o_d8 = 0;
        }
        // passive gain in human form: +CHARGE SPEED per frame
        if f.form & 0x10 == 0 {
            if f.gauge >= 0 && f.flags[F6C] & 0x8000 == 0 {
                f.gauge = f.gauge.wrapping_add(charge);
                if f.gauge >= 0x3C00 {
                    f.gauge = 0x3BFF;
                }
            } else if f.flags[F58] & 0x8000 == 0 {
                f.gauge = 0; // an emptied gauge restarts once out of hit-stun
            }
        }
        // red health regenerates in beast form, never below the permanent part
        if f.form & 0x10 != 0 && f.flags[F58] & 0x1000_8000 == 0 && f.o_ae < f.o_ac {
            f.o_fa = f.o_fa.wrapping_add(recover);
            if f.o_fa >= 0x100 {
                f.o_ac = (f.o_ac as i32 - (f.o_fa >> 8) as i32).max(f.o_ae as i32) as i16;
                f.o_fa &= 0xFF;
            }
        }
        // the blue bar follows the permanent damage
        if f.o_aa < f.o_ae {
            f.o_aa += 1;
        }
    }
    let dmg = f.o_c8 as i32;
    if f.flags[F54] & 2 != 0 {
        // PRACTICE (F54 bit 1): damage only goes to the blue bar +0xAA, health never drops, no KO
        if dmg != 0 {
            f.o_aa = (f.o_aa as i32 + dmg.abs()).min(f.o_b0 as i32) as i16;
            f.o_ca = dmg.abs() as i16;
            f.o_c8 = 0;
        }
        return;
    }
    if dmg < 0 {
        // guarded beast attack: chip damage that cannot KO
        f.o_ac = (f.o_ac as i32 - dmg) as i16;
        f.o_ae = (f.o_ae as i32 - dmg * 60 / 100) as i16;
        f.o_ae = f.o_ae.min(f.o_b0);
        f.o_ac = f.o_ac.min(f.o_b0);
        f.o_ca = (-dmg) as i16;
        f.o_c8 = 0;
    } else if dmg > 0 {
        f.o_ac = (f.o_ac as i32 + dmg) as i16;
        f.o_ae = (f.o_ae as i32 + dmg * 60 / 100) as i16;
        f.o_ae = f.o_ae.min(f.o_b0);
        f.o_ca = dmg as i16;
        f.o_c8 = 0;
    }
    if f.o_b0 < f.o_ac {
        // KO
        f.o_00 = 0x80;
        f.o_ac = -1;
        f.o_ae = -1;
        f.o_c0 = -1;
        f.flags[F5C] |= 0x20_0000;
        f.o_e5 = f.o_e5.wrapping_add(1);
        f.ko = true;
    }
}

/// Attack opcodes 10, 0D, 0E, 0F, 23.
pub fn op_attack(f: &mut Fighter, op: u8, sub: u8, b: &[u8], bone_map: &[u8]) {
    let gate = ((sub as i8) >> 4) as i32;
    if (gate == 1 && f.beast()) || (gate == 2 && !f.beast()) {
        return;
    }
    let (b2, b3) = (b[2], b[3]);
    match op {
        0x10 => {
            f.s_101 = [-1; 4];
            f.o_26 = -1;
            f.hit_spheres.clear();
            f.o_bc = if sub & 0xF != 0 { -1 } else { 0 };
            let st = b2 as i8;
            if st < 0 {
                f.o_ba = -1;
                f.fx_cancel = 2; // the move is cancelled
            } else {
                f.o_ba = (st as i16) * 16;
                if st == 0 {
                    f.o_26 = 0;
                }
            }
            let act = b3 as i8;
            f.o_b8 = if act < 0 { -1 } else { (act as i16) * 16 };
            f.s_116 = f.o_ba;
        }
        0x0D => {
            if f.hit_spheres.len() < 8 {
                let node = bone_map.get(b2 as usize).copied().unwrap_or(b2).wrapping_add(1);
                f.hit_spheres.push(HitSphere { node, radius: b3 as i16 });
            }
        }
        0x0E => {
            let (lo, hi) = (b2, b3 as i8 as i32 as u8);
            match sub & 0xF {
                0 => {
                    f.o_da = lo;
                    f.o_dc = hi;
                }
                1 | 2 => {
                    f.o_da = f.o_da.wrapping_add(lo);
                    f.o_dc = f.o_dc.wrapping_add(hi);
                }
                3 => {
                    let v = if f.flags[F6C] & 0x100 != 0 { f.gauge as i32 * 2 + 0x12BFB } else { f.gauge as i32 * 3 + 0x1A3F9 };
                    f.o_da = ((v * lo as i32) / 0x25800) as u8;
                    f.o_dc = hi;
                }
                _ => {}
            }
            f.s_3a = f.s_3a.wrapping_add(f.o_da as i16);
        }
        0x0F => {
            f.o_db = b2;
            let a = i16::from_le_bytes([b2, b3]) >> 4;
            f.o_c6 = if f.anim.mirror { -a } else { a };
        }
        0x23 => match sub & 0xF {
            k @ 0..=3 => f.s_101[k as usize] = b2 as i8,
            4 => {
                f.s_101 = [b2 as i8, b3 as i8, b[4] as i8, b[5] as i8];
            }
            _ => {}
        },
        _ => {}
    }
}

fn mass_scale(v: i32, mass: i16) -> i16 {
    (v * 4800 / (mass as i32 + 0xFA0)) as i16
}

fn sra_round(v: i32, s: u32) -> i32 {
    (if v < 0 { v + (1 << s) - 1 } else { v }) >> s
}

/// The reaction setups for knock-downs and launches: the `launch` kind of hit result `s0`
/// (F68), None for a reaction whose move moves the fighter itself.
pub fn launch_kind(s0: u32) -> Option<u8> {
    if s0 & 0x800 != 0 || s0 & 0x80_4000 == 0x80_4000 {
        Some(0)
    } else if s0 & 0x81_0000 == 0x81_0000 {
        Some(1)
    } else if s0 & 0x80_2000 == 0x80_2000 {
        Some(2)
    } else if s0 & 0x84_0000 == 0x84_0000 {
        Some(3)
    } else {
        None
    }
}

/// Launch / knock-down velocity setups, `kind` 0..3.
pub fn launch(att: &Fighter, def: &mut Fighter, kind: u8, t: &crate::fight::physics::Trig) {
    def.o_b4 = 0x78;
    def.o_b2 = 0;
    def.o_92 = 0xA0;
    let db = att.o_db as i32;
    match kind {
        0 => {
            // (96·db >> 3) + 0x200 = 12·db + 0x200, × (0x41 − dd) / 64
            let a = (sra_round(db * 96, 3) + 0x200) * (0x41 - att.o_dd as i32);
            let a = sra_round(a, 6);
            let mut vy = sra_round(t.cos(a) * 13 * 256, 12);
            let vx = sra_round(-t.sin(a) * 5 * 256, 12);
            if att.form & 0x10 != 0 && att.flags[F64] & 0x48_0000 != 0 {
                vy = sra_round(vy * 320, 8);
            }
            def.o_90 = mass_scale(vy, def.o_24);
            def.o_98 = mass_scale(vx, def.o_24);
        }
        1 => {
            let a = sra_round((db * 8 + 0x400) * (0x41 - att.o_dd as i32), 6);
            let vy = t.cos(a) / 2;
            def.o_98 = 0;
            def.o_90 = mass_scale(vy, def.o_24);
        }
        2 => {
            let a = sra_round(db * 16, 2) + 0x300;
            def.o_90 = t.cos(a) as i16;
            let mut vx = -t.sin(a) * 2;
            if att.form & 0x10 != 0 {
                vx -= 0x2000;
            }
            def.o_90 = mass_scale(def.o_90 as i32, def.o_24);
            def.o_98 = mass_scale(vx, def.o_24);
        }
        _ => {
            let a = sra_round(db * 16, 2);
            def.o_90 = mass_scale(t.cos(a), def.o_24);
            def.o_98 = mass_scale(-t.sin(a), def.o_24);
        }
    }
}
