//! The imported game's throw resolution after both inputs, and its throw test.
//!
//! State: none of its own: updates the serialised `Br1Fighter`s
//!
//! The main game's resolution without the throw escape and the side flips (the imported game's throws cannot be escaped:
//! a user decision for the import, and the imported game's rule). The victim's state comes from the decision (F.64
//! 0x800000, `S.34` = the thrower's move).

use super::object::Br1Fighter;
use super::script::{Call, Ctx, Request};

/// Whether `a`'s attack candidate (`S.28`) grabs `b`: 0 no attempt, 1 connects, −1 misses.
pub fn test(fs: &[Br1Fighter; 2], a: usize, ctx: &mut Ctx) -> i32 {
    let (x, y) = (&fs[a], &fs[1 - a]);
    let s = &x.s;
    if s.att_mv < 0 || s.att_flags == 0 || x.f.recover_timer > 0 {
        return 0;
    }
    // the victim in hit-stun / a throw: only while F.C8 runs or in the air
    if y.f.f50 & 0x1000_8000 != 0 && y.f.fc8 == 0 && y.f.f54 & 0x4_0000 == 0 {
        return 0;
    }
    if x.f.f50 & 0x0104_0000 != 0 || x.f.f54 & 0x40 != 0 {
        return 0;
    }
    if s.att_flags & 0x100 != 0 {
        // the air throw: both hips above −0x17F, the height difference within the reach, airborne, in a jump
        if x.hips_y() >= -0x17F || y.hips_y() >= -0x17F {
            return 0;
        }
        if (s.att_param as i32) < (x.f.opp_dy as i32).abs() || x.f.f54 & 0x80 == 0 {
            return 0;
        }
        if x.f.f50 & 0x1000 == 0 || x.f.f50 & 0x4000 != 0 {
            return 0;
        }
    }
    if s.att_flags & 0x80 != 0 && x.f.f50 & 0x1800 != 0 {
        return 0;
    }
    let f50 = x.f.f50;
    if f50 & 0xA000_0000 == 0xA000_0000 {
        return 0;
    }
    if (f50 as i32) < 0 && x.f.move_time < 0x40 && x.f.f5c & 0x0804_3800 == 0 {
        // a throw out of an early attack, the attack's effects cancelled (as op 0x0A)
        ctx.out.push((a as u8, Request::Call { call: Call::MoveCancel, args: [x.f.player as i32, 0, 0, 0, 0, 0] }));
    } else if x.f.f54 & 0x4000 == 0 {
        return 0;
    }
    if y.f.no_throw != 0 || y.f.f58 & 0x4000_0000 != 0 {
        return -1;
    }
    // an attacking victim (before its recovery) escapes; a 0x2000 throw tests that only on an attack with F.50 0x80000
    let skip = s.att_flags & 0x2000 != 0 && y.f.f50 & 0x8008_0000 != 0x8008_0000;
    if !skip && (y.f.f50 as i32) < 0 && y.f.f5c & 0x8000 == 0 && y.f.recovery <= 0 {
        return -1;
    }
    if (s.att_param as i32) < x.f.opp_dist as i32 || (x.f.opp_angle as u16).wrapping_add(0xAA) >= 0x155 {
        return -1;
    }
    let fl = s.att_flags;
    // 0x1000 from the front, 0x2000 from behind (the victim's F.54 0x400000: the thrower behind it)
    let opp_back = y.f.f54 & 0x40_0000 != 0;
    if !(fl & 0x1000 != 0 && !opp_back) && !(fl & 0x2000 != 0 && opp_back) {
        return -1;
    }
    if fl & 0x200 != 0 {
        if y.f.f54 & 0x4_0000 != 0 {
            return -1;
        }
        if y.f.f50 & 1 != 0 {
            return 1;
        }
    }
    if fl & 0x400 != 0 {
        if y.f.f54 & 0x4_0000 != 0 {
            return -1;
        }
        if y.f.f50 & 0x30 != 0 {
            return 1;
        }
    }
    if fl & 0x800 != 0 && y.f.f54 & 0x4_0000 != 0 {
        return 1;
    }
    -1
}

/// The grab: the thrower's F.64 0x400000, the victim's 0x800000 and `S.34` = the thrower's move.
fn grab(fs: &mut [Br1Fighter; 2], a: usize) {
    fs[a].f.f64 |= 0x40_0000;
    fs[1 - a].f.f64 |= 0x80_0000;
    fs[1 - a].s.thrown_by = fs[a].s.att_mv;
}

/// A miss: a throw-type candidate (`S.28 & 0x4000`) whiffs (F.64 0x600000).
fn whiff(f: &mut Br1Fighter) {
    if f.s.att_flags & 0x4000 != 0 {
        f.f.f64 |= 0x60_0000;
    }
}

/// The throw resolution (after both inputs).
pub fn resolve(fs: &mut [Br1Fighter; 2], ctx: &mut Ctx) {
    if fs[0].f.f58 & 0x1_0000 != 0 || (fs[0].f.f50 | fs[1].f.f50) & 0x4010_0000 != 0 {
        return;
    }
    // a forced throw (op 0x20), P1 first
    for a in 0..2 {
        if fs[a].s.att_flags & 0x8000 != 0 {
            fs[a].s.att_flags = 0;
            fs[a].f.f64 |= 0x40_0000;
            fs[1 - a].f.f64 |= 0x80_0000;
            fs[1 - a].s.thrown_by = fs[a].s.att_mv;
            fs[1 - a].f.f58 |= 0x1000_0000;
            return;
        }
    }
    if (fs[0].f.f54 | fs[1].f.f54) & 0x0600_0000 != 0 {
        return;
    }
    let r1 = test(fs, 0, ctx);
    let r2 = test(fs, 1, ctx);
    if r1 | r2 == 0 {
        return;
    }
    if r1 == r2 {
        whiff(&mut fs[0]);
        whiff(&mut fs[1]);
        fs[0].s.att_flags = 0;
        fs[1].s.att_flags = 0;
        return;
    }
    for (a, r) in [(0, r1), (1, r2)] {
        match r {
            -1 => {
                whiff(&mut fs[a]);
                fs[a].s.att_flags = 0;
            }
            1 => {
                grab(fs, a);
                fs[a].s.att_flags = 0;
            }
            _ => {}
        }
    }
}
