//! The imported game's hits: whether an attack can hit, the sphere test, the direction bits, the damage multiplier, the
//! hit apply (guard, damage, combo scaling, stun, push, launch, the F.60 hit result) and the pair's hit step.
//!
//! State: none of its own: updates the serialised `Br1Fighter`s
//!
//! The main game's pipeline (`fight/hit.rs`) with the imported game's rules: no hit level (the strength class comes from
//! the damage), three counter multipliers, the combo scaling to 1/5, the guard gate F.54 0x40002000 (the automatic
//! guard), guard stun from the damage, the chip of 0x10000000 attacks, the launch kinds. The effects (sounds,
//! sparks, the camera's shake) go out as `Request`s in call order.

use super::object::Br1Fighter;
use super::script::{Call, CamWord, Ctx, Request, div};
use crate::fight::physics::{Trig, wrap12};
use crate::formats::act_ani::Sphere;

/// The camera's shake word the hit class writes.
pub const SHAKE: CamWord = CamWord::Horizontal;

/// Whether `att`'s active attack can hit `def` (by the attack's height against the defender's stance).
/// Clears the attacker's F.54 0x14 first.
pub fn can_hit(fs: &mut [Br1Fighter; 2], att: usize) -> bool {
    fs[att].f.f54 &= !0x14;
    let (a, d) = (&fs[att], &fs[1 - att]);
    if a.f.attack == -1 || d.f.hittable == -1 || d.f.f58 & 4 != 0 || d.f.f54 & 0x8008_0000 != 0 {
        return false;
    }
    let (t, s) = (a.f.f5c, d.f.f50);
    if t & 0x40 != 0 {
        s & 0x4000 == 0
    } else if t & 0x80 != 0 {
        s & 0x20 == 0
    } else if t & 0x100 != 0 {
        s & 0x30 == 0
    } else {
        true
    }
}

/// The side of the defender the attack comes from (1 front, 2 back, 4 / 8 the sides) and the knockback
/// class (0x10 weak, 0x20 strong). `opt` = the option byte G.98.
pub fn direction_bits(fs: &[Br1Fighter; 2], att: usize, opt: u8) -> u32 {
    let (a, d) = (&fs[att], &fs[1 - att]);
    let da = if opt != 0 && !((d.f.f50 as i32) < 0 && d.f.recovery > 0) { wrap12(d.anim.yaw_base as i32 + d.anim.turn as i32) as i32 } else { d.f.yaw as i32 };
    let base = a.f.yaw as i32 + if a.f.knock_strength == 0 { a.f.opp_angle } else { a.f.knock_angle } as i32;
    let v = wrap12(da - base - 0x800) as i32;
    let mut r = if v >= 0 { 4 } else { 8 };
    if v.abs() < 0x271 {
        r |= 1;
    } else if v.abs() >= 0x4E4 {
        r |= 2;
    }
    let k = (a.f.knock_strength as i32) << 4;
    if k >= 0x600 {
        r |= 0x20;
    } else if k < 0x201 {
        r |= 0x10;
    }
    r
}

/// The damage multiplier (×128): counters (0x99 startup, 0xA6 active, 0x73 recovery), a low on a crouching
/// defender ×2, from behind / against a recovery or a dash ×166 / 153 / 160 / 102 (/128). 0x80 within a combo.
pub fn multiplier(fs: &[Br1Fighter; 2], att: usize, dir: u32) -> i32 {
    let (a, d) = (&fs[att], &fs[1 - att]);
    if d.f.f54 & 0x2000_0000 != 0 {
        return 0x80;
    }
    let mut m = 0x80;
    let s = d.f.f50;
    if (s as i32) < 0 {
        m = if d.f.startup != 0 {
            0x99
        } else if d.f.active == 0 {
            0x73
        } else {
            0xA6
        };
    }
    if s & 0x20 != 0 && a.f.f5c & 0x40 != 0 {
        return m * 2;
    }
    let k = if dir & 2 != 0 || s & 8 != 0 {
        166
    } else if s & 0x1000 != 0 && d.f.f54 & 0x4_0000 != 0 || s & 0x200 != 0 {
        153
    } else if s == 0x402 {
        160
    } else if s == 0x404 {
        102
    } else {
        return m;
    };
    m * k / 128
}

/// Two spheres overlap; the contact point = `pb` + the half difference (halfwords).
fn pair_test(ra: i32, rb: i32, pa: [i32; 3], pb: [i32; 3]) -> Option<[i16; 3]> {
    let r = ra + rb;
    let d: [i32; 3] = std::array::from_fn(|i| pa[i].wrapping_sub(pb[i]));
    if d.iter().any(|v| v.wrapping_abs() >= r) {
        return None;
    }
    let rr = r.wrapping_mul(r);
    let mut s = 0i32;
    for v in d {
        s = s.wrapping_add(v.wrapping_mul(v));
        if s >= rr {
            return None;
        }
    }
    Some(std::array::from_fn(|i| (pb[i] as i16).wrapping_add((d[i] / 2) as i16)))
}

/// The attacker's hit spheres (F.14) against the defender's hurt spheres (its form's table 2 / 3): kind
/// 1 with the contact point, 4 for a proximity entry (radius 1, |F.6E| < 0x300: the defender's node 0), 0 none.
pub fn sphere_test(fs: &[Br1Fighter; 2], att: usize, hurt: &[Sphere], opt: u8) -> (i32, [i16; 3]) {
    let (a, d) = (&fs[att], &fs[1 - att]);
    let lying = d.f.f50 & 0x3_0000 != 0 && d.f.f54 & 0x8000_0080 != 0x80;
    let low_ok = a.f.f5c & 0x1000 != 0;
    let mut n = d.f.hit_spheres as i8 as i32;
    if opt == 0 && d.f.f50 & 0x200_0000 != 0 {
        n -= 2;
    }
    let node = |f: &Br1Fighter, k: i32| f.form().md.get(k as usize).map_or([0; 3], |m| m.wt);
    for e in a.spheres.iter() {
        if e.flag == -1 {
            break;
        }
        if e.flag == -2 || lying && !low_ok {
            continue;
        }
        let pa = node(a, e.node as i8 as i32);
        if e.radius as i16 == 1 {
            if d.f.f54 & 0x4_0080 != 0 || (a.f.opp_dist as i32).abs() >= 0x300 {
                continue;
            }
            return (4, node(d, 0).map(|v| v as i16));
        }
        for h in hurt.iter().take(n.max(0) as usize) {
            if let Some(p) = pair_test(e.radius as i16 as i32, h.radius as i32, pa, node(d, h.node as i8 as i32)) {
                return (1, p);
            }
        }
    }
    (0, [0; 3])
}

/// The strength class bit from the damage and the camera's shake amplitude.
fn class(s0: i32) -> (u32, i16) {
    if s0 >= 0x1E {
        (0x200, 1500)
    } else if s0 >= 0x16 {
        (0x100, 1000)
    } else if s0 >= 0x10 {
        (0x80, 750)
    } else {
        (0x40, 500)
    }
}

/// Launch A (CF ≥ 0x60, by the vertical part) or B (by the horizontal part); `special` =
/// the 0x800 launch's rule for a defender in state 0x58 / 0x59.
fn launch_ab(d: &mut Br1Fighter, cfb: u8, s0: i32, t: &Trig, opt: u8, special: bool) {
    let cf = (cfb as i32) << 4;
    if cfb >= 0x60 {
        d.f.vy = div(t.cos(cf) * s0, 256) as i16;
        let v1 = div(-t.sin(cf), 16) - 0x40;
        let v0 = ((d.f.vy as i32 / 192) as i16 as i32).abs();
        d.f.launch_speed = (div(v1, v0) << 7) as i16;
        if special && opt != 0 && (d.s.state_req as u16).wrapping_sub(0x58) < 2 && d.f.vy < 0 {
            d.f.launch_speed = d.f.launch_speed.wrapping_mul(6);
            d.f.vy /= 3;
        }
    } else {
        d.f.launch_speed = div(-t.sin(cf) * s0, 256) as i16;
        let v1 = div(t.cos(cf), 16) - 0x40;
        let v0 = ((d.f.launch_speed as i32 / 192) as i16 as i32).abs();
        d.f.vy = (div(v1, v0) << 7) as i16;
    }
}

/// `(a, b)` = (attacker, defender) mutably.
fn pair(fs: &mut [Br1Fighter; 2], att: usize) -> (&mut Br1Fighter, &mut Br1Fighter) {
    let [p, q] = fs;
    if att == 0 { (p, q) } else { (q, p) }
}

/// The hit or the guard. Returns 3 hit, 2 guarded (kinds 2 / 3, the
/// effect-only helpers, 1 / 2).
#[allow(clippy::too_many_arguments)]
pub fn apply(fs: &mut [Br1Fighter; 2], att: usize, dir: u32, mult: i32, pos: [i16; 3], kind: i32, ctx: &mut Ctx, opt: u8) -> i32 {
    let me = att as u8;
    let res = ctx.res[att];
    let t = &res.trig;
    let (a, d) = pair(fs, att);
    let mut s1 = dir;
    a.f.attack = -1;
    let mut s3 = a.f.f5c;
    if kind == 4 {
        let v = a.f.attack_damage as i32 * (0x300 - a.f.opp_dist as i32);
        a.f.attack_damage = div(v, 512) as u8;
        if a.f.attack_damage >= 0x14 {
            s3 |= 0x100_0000;
        }
    }
    let mut s0 = div(a.f.attack_damage as i32 * mult, 128);
    let fx = |ctx: &mut Ctx, call: Call, args: [i32; 4]| ctx.out.push((me, Request::Fx { call, pos, args }));
    if kind == 2 {
        fx(ctx, Call::GuardSparks, [0, 0, s0, 0]);
        return 1;
    }
    if kind == 3 {
        fx(ctx, Call::HitSparks, [0, 0, s0, a.s.trail_mode as i32]);
        return 2;
    }
    if a.f.knock_strength == 0 {
        d.f.knock_dir = wrap12(a.f.yaw as i32 + a.f.opp_angle as i32);
        a.f.knock_strength = 0x40;
    } else {
        d.f.knock_dir = wrap12(a.f.yaw as i32 + a.f.knock_angle as i32);
    }
    s1 &= 0xFF8F_FFFF;
    if s3 & 0x40 != 0 {
        s1 |= 0x10_0000;
    } else if s3 & 0x80 != 0 {
        s1 |= 0x20_0000;
    } else if s3 & 0x100 != 0 {
        s1 |= 0x40_0000;
    }
    let (mut s5, mut s7) = (0u32, 0u32);
    let s6;
    // ---- the guard (F.54 0x2000 the automatic guard, 0x40000000 the air-guard window) ----
    let d54 = d.f.f54;
    if d54 & 0x4000_2000 != 0 {
        let d50 = d.f.f50;
        let air = d54 & 0x80 != 0;
        let air_guard = || s3 & 0x2_0000 == 0 && d54 & 0x4000_0000 == 0;
        // the standing guard: Some(true) guarded, Some(false) crushed, None not guarded
        let ground_guard = || {
            if d50 & 1 == 0 {
                None
            } else if s3 & 0x2_0000 != 0 {
                Some(false)
            } else if s3 & 0x4000 != 0 {
                None
            } else {
                Some(true)
            }
        };
        let mut res = None;
        if s3 & 0x40 != 0 {
            if air {
                res = air_guard().then_some(true);
            } else if d50 & 0x20 == 0 {
                if d50 & 0x10 != 0 {
                    if s3 & 0x2_0000 != 0 {
                        s1 |= 0x80_0400;
                    } else if s3 & 0x4000 == 0 {
                        s5 = 0x4200_0000;
                    }
                }
                if s3 & 0x80 != 0 {
                    res = ground_guard();
                }
            }
        } else if s3 & 0x180 != 0 {
            if air {
                res = air_guard().then_some(true);
            } else if d50 & 0x20 == 0 {
                res = ground_guard();
            }
        }
        match res {
            Some(true) => s5 = 0x4200_0000,
            Some(false) => s1 |= 0x80_0400,
            None => {}
        }
        if s1 & 0x80_0000 != 0 {
            s5 |= 0x2400_0000;
        }
    }
    let (ce, cf) = (a.f.attack_damage as i32, a.f.knock_strength);
    if s5 == 0 {
        // ---- a hit ----
        s6 = 3;
        a.f.combo = if d.f.f54 & 0x2000_0000 != 0 { a.f.combo.saturating_add(1) } else { 1 };
        s0 = match a.f.combo {
            0 | 1 => s0,
            2 => s0 * 9 / 10,
            3 => s0 * 8 / 10,
            4 => s0 * 7 / 10,
            5 => s0 * 6 / 10,
            6 => s0 / 2,
            7 => s0 * 4 / 10,
            8 => s0 * 3 / 10,
            _ => s0 / 5,
        };
        s5 = 0x2400_0000;
        let k = d.factor();
        if d.f.down_timer >= 0 {
            let d50 = d.f.f50;
            if d50 & 0x3_0000 != 0 && d50 & 0x800_0000 == 0 {
                d.f.down_timer = d.f.down_timer.wrapping_add(((ce * k) >> 3) as i16);
                if d.f.down_timer >= 0x12C1 {
                    d.f.down_timer = 0x12C0;
                }
            } else {
                d.f.down_timer = ((((s0 * k) << 4) >> 7) - 0x190) as i16;
                if d.f.down_timer < 0 {
                    d.f.down_timer = 0;
                }
            }
        }
        s1 &= !0x3C0;
        d.f.queued_damage = s0 as i16;
        d.f.hit_stun = 0xF0;
        d.f.f54 &= 0xF7FF_FFFF;
        d.f.push_speed = div(s0 << 11, 10) as i16;
        let (c, shake) = class(s0);
        s1 |= c;
        ctx.out.push((me, Request::Cam { word: SHAKE, value: shake }));
        let (d50, d54) = (d.f.f50, d.f.f54);
        if d50 & 0x800 != 0 || d54 & 0x4_0000 != 0 {
            d.f.hit_stun = 0x140;
            s1 |= if s3 & 0x8_0000 != 0 {
                0xC00
            } else if s3 & 0x400_0000 != 0 {
                0x4_0400
            } else if s3 & 0x20_0000 != 0 {
                0x2400
            } else {
                0x4400
            };
        } else {
            d.f.vy = 0;
            d.f.gravity = 0;
            let st = d.s.state_req;
            if s3 & 0x10_0000 != 0 {
                if st == 0x5C {
                    s1 |= 0x4400;
                } else if d50 & 0x10 != 0 && mult > 0x80 {
                    s1 |= 0x1400;
                }
            }
            if s3 & 0x200_0000 != 0 {
                if st == 0x53 || st == 0x4F {
                    s1 |= 0x4400;
                } else if d54 & 0x10_0000 == 0 && d50 & 1 != 0 && mult > 0x80 {
                    s1 |= 0x2_0400;
                }
            }
            if s3 & 0x8_0000 != 0 {
                s1 |= 0xC00;
            } else if s3 & 0x20_0000 != 0 {
                s1 |= 0x2400;
            } else if s3 & 0x40_0000 != 0 {
                s1 |= 0x4400;
            } else if s3 & 0x80_0000 != 0 && mult > 0x80 {
                s1 |= 0x8400;
            } else if s3 & 0x100_0000 != 0 {
                s1 |= 0x1_0400;
            } else if s3 & 0x1000 != 0 {
                if d50 & 0x3_0000 != 0 && d54 & 0x80 == 0 {
                    s1 |= 0x200_0400;
                }
            } else if s3 & 0x400_0000 != 0 {
                s1 |= 0x4_0400;
            }
        }
        if s3 & 0x8_0000 != 0 {
            s1 |= 0xC00;
        } else if s3 & 0x20_0000 != 0 {
            s1 |= 0x2400;
        } else if s3 & 0x40_0000 != 0 {
            s1 |= 0x4400;
        }
        if s1 & 0x400 == 0 && s0 >= 0x27 {
            s1 |= 0x4400;
        }
        // the gauge: the human attacker gains, the defender gains (human) or loses (beast)
        if a.f.form & 0x10 == 0 {
            a.f.gauge_queued = a.f.gauge_queued.wrapping_add(((ce >> 1) * 60) as i16);
        }
        let be = d.f.queued_damage as i32;
        if d.f.form & 0x10 == 0 {
            d.f.gauge_queued = d.f.gauge_queued.wrapping_add(((be / 3) as i16 as i32 * 60) as i16);
        } else {
            d.f.gauge_queued = d.f.gauge_queued.wrapping_sub((be * 120) as i16);
        }
        ctx.out.push((me, Request::Call { call: Call::HitLight, args: [3, s0, 0, 0, 0, 0] }));
        s7 |= 0xC;
        let ang = wrap12(d.f.knock_dir as i32 - 0x400) as i32;
        fx(ctx, Call::HitSparks, [ang, (cf as i32) << 4, s0 | 0x1000, a.s.trail_mode as i32]);
    } else {
        // ---- guarded ----
        s6 = 2;
        a.f.combo = if d.f.f54 & 0x4000_0000 != 0 { a.f.combo.saturating_add(1) } else { 1 };
        d.f.push_speed = (s0 << 8) as i16;
        if opt != 0 {
            if s3 & 0x1000_0000 != 0 {
                d.f.queued_damage = (-s0 / 4) as i16;
            }
            if a.f.rave != 0 {
                d.f.queued_damage = (-s0 / 8) as i16;
            }
        } else if s3 & 0x1000_0000 != 0 {
            d.f.queued_damage = (-s0 / 8) as i16;
        }
        d.f.guard_stun = ((ce >> 1) - a.f.combo as i32 + 1) as i16;
        if d.f.guard_stun < 6 {
            d.f.guard_stun = 6;
        }
        d.f.hit_stun = 0;
        let g = ((ce >> 3) * 60) as i16;
        d.f.gauge_queued = if d.f.form & 0x10 == 0 { d.f.gauge_queued.wrapping_add(g) } else { d.f.gauge_queued.wrapping_sub(g) };
        if d.f.f50 & 0x800 != 0 {
            d.f.vy = div(-0x2A0 * 1500, d.f.weight as i32) as i16;
            d.f.gravity = 0xE0;
            if a.f.form & 1 != 0 {
                d.f.vy = div(d.f.vy as i32 * 3, 2) as i16;
            }
        }
        s1 &= !0x3C0;
        let (c, shake) = class(s0);
        s1 |= c;
        ctx.out.push((me, Request::Cam { word: SHAKE, value: shake }));
        if opt != 0 {
            d.f.knock_dir = wrap12(a.f.yaw as i32 + a.f.opp_angle as i32);
        }
        if s1 & 0x80_0000 != 0 {
            ctx.out.push((me, Request::Call { call: Call::HitLight, args: [0, s0, 0, 0, 0, 0] }));
            fx(ctx, Call::Task, [0x10C, 0xC8, 0xF, 1]);
        } else {
            ctx.out.push((me, Request::Call { call: Call::HitLight, args: [2, s0, 0, 0, 0, 0] }));
            let ang = wrap12(d.f.knock_dir as i32 + 0x400) as i32;
            fx(ctx, Call::GuardSparks, [ang, (cf as i32) << 4, s0 | 0x1000, 0]);
        }
        s7 |= 0x30;
    }
    // ---- the push and the launch ----
    if s3 & 0x800_0000 != 0 {
        d.f.push_speed = d.f.push_speed.wrapping_mul(4);
    }
    if s1 & 0x2000 != 0 {
        s1 |= 0x100_0000;
    }
    if d.f.push_speed != 0 {
        d.f.push_decay = 0x70;
    }
    let beast = a.f.form & 1 != 0;
    let cfi = (cf as i32) << 4;
    let w = d.f.weight as i32;
    let up = |s3: u32| if s3 & 0x2000_0000 != 0 || !beast { 0xC0 } else { 0x140 };
    let mut scale = false;
    if s1 & 0x800 != 0 {
        (d.f.push_decay, d.f.gravity, d.f.push_speed) = (0x78, 0xC0, 0);
        s0 += up(s3);
        launch_ab(d, cf, s0, t, opt, true);
        scale = true;
    } else if s1 & 0x1_0000 != 0 {
        d.f.push_decay = 0x78;
        d.f.push_speed = ((s0 + 0x180) * 8) as i16;
        d.f.gravity = 0xC0;
        let v = s0 + if s3 & 0x2000_0000 != 0 || !beast { 0xC0 } else { 0x100 };
        d.f.vy = (-(v * 8)) as i16;
        d.f.launch_speed = 0;
        d.f.vy = div(d.f.vy as i32 * 800, w) as i16;
    } else if s1 & 0x100_0000 != 0 {
        (d.f.push_decay, d.f.gravity, d.f.push_speed) = (0x78, 0xC0, 0);
        s1 &= !0x2000;
        s0 += 0x100;
        d.f.vy = (div(t.cos(cfi) * s0, 256) - 0x180) as i16;
        let v1 = div(-t.sin(cfi) * s0, 256) as i16 as i32;
        d.f.launch_speed = (v1 - if beast { 0x1800 } else { 0x800 }) as i16;
        scale = true;
    } else if s1 & 0x4_0000 != 0 {
        (d.f.push_decay, d.f.push_speed, d.f.gravity) = (0x78, 0, 0xC0);
        s0 += 0x100;
        d.f.vy = div(t.cos(cfi) * s0, 256) as i16;
        d.f.launch_speed = div(-t.sin(cfi) * s0, 256) as i16;
        d.f.vy = div(d.f.vy as i32 * 800, w) as i16;
    } else if s1 & 0x4000 != 0 {
        (d.f.push_decay, d.f.gravity, d.f.push_speed) = (0x78, 0xC0, 0);
        if s3 & 0x250_0000 != 0 {
            s0 += up(s3);
            launch_ab(d, cf, s0, t, opt, false);
        } else {
            s0 += 0xE0;
            if beast {
                d.f.vy = (div(t.cos(cfi) * s0, 512) - 0x300) as i16;
                d.f.launch_speed = div(-t.sin(cfi) * s0, 768) as i16;
            } else {
                d.f.vy = (div(t.cos(cfi) * s0, 768) - 0x180) as i16;
                d.f.launch_speed = div(-t.sin(cfi) * s0, 512) as i16;
            }
        }
        scale = true;
    }
    if scale {
        d.f.vy = div(d.f.vy as i32 * 800, w) as i16;
    }
    if d.f.f4c & 0x20 != 0 {
        d.f.push_speed /= 2;
        d.f.launch_speed /= 2;
    }
    if d.f.push_speed != 0 {
        if d.f.push_speed < 0 {
            d.f.push_speed = 0;
        }
        if d.f.f64 & 0xF0 != 0 {
            // against a wall: the attacker is pushed back instead
            a.f.knock_dir = wrap12(a.f.yaw as i32 + a.f.centre_angle as i32);
            a.f.push_decay = d.f.push_decay;
            a.f.push_speed = div(d.f.push_speed as i32 * 2, 3) as i16;
            d.f.push_speed /= 2;
            a.f.push_speed = div(a.f.push_speed as i32 * 500, a.f.weight as i32) as i16;
        }
        d.f.push_speed = div(d.f.push_speed as i32 * 500, w) as i16;
        if d.f.push_speed >= 0x6001 {
            d.f.push_speed = 0x6000;
        }
    }
    if s3 & 0x2000_0000 != 0 && s5 & 0x400_0000 != 0 {
        s1 |= 0x8_0000;
        d.f.push_speed = 0;
    }
    d.f.f60 = s1;
    d.f.f54 = (d.f.f54 & 0xF9FF_FFFF) | s5;
    a.f.f54 |= s7;
    s6
}

/// Both attacks tested (can hit, direction, multiplier, the damage, the spheres), the smaller
/// damage of a trade dropped, then P1's hit applied, then P2's.
pub fn hits(fs: &mut [Br1Fighter; 2], ctx: &mut Ctx, opt: u8) {
    let mut cand = [None, None];
    for att in 0..2 {
        if !can_hit(fs, att) {
            continue;
        }
        let dir = direction_bits(fs, att, opt);
        let mult = multiplier(fs, att, dir);
        let p = fs[att].f.attack_damage as i32 * mult;
        let dmg = (if p < 0 { p + 0x7F } else { p }) >> 7;
        let d = &fs[1 - att];
        let hurt = &ctx.res[1 - att].anm.spheres[2 + (d.cur as usize & 1)];
        let (kind, pos) = sphere_test(fs, att, hurt, opt);
        cand[att] = Some((kind, dir, mult, dmg, pos));
    }
    if let [Some(a), Some(b)] = &mut cand
        && a.0 != 0
        && b.0 != 0
    {
        if a.3 < b.3 {
            a.0 = 0;
        } else if b.3 < a.3 {
            b.0 = 0;
        }
    }
    for (att, c) in cand.into_iter().enumerate() {
        if let Some((kind, dir, mult, _, pos)) = c
            && kind != 0
        {
            apply(fs, att, dir, mult, pos, kind, ctx, opt);
        }
    }
}
