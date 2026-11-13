//! Throws: the resolution at the start of the match frame and the per-fighter throw test.
//!
//! State: none of its own: steps the serialised `Match`
//!
//! Throws: the resolution at the start of the match frame (after both players' input) and
//! the per-fighter throw test. A throw is a command candidate in the attack block
//! (state +0x20) whose record flags have 0x80 (ground) or 0x100 (air); `param` is the reach.
//!
//! Results go to F6C: thrower 0x400000 (start the throw move, `cand_attack.mv + 0x90`), +0x200000 when
//! it misses; the victim gets 0x800000 and `s_2c` = the throw, and plays its reaction from the
//! per-opponent group table (see `decide`). 0x4000000 on both = throw escape.

use crate::fight::fighter::{F5C, F6C, F58, F60, F64, Fighter};

/// Can `a` throw `b` now? 0 = no attempt, 1 = the throw connects, -1 = it misses.
pub fn test(a: &Fighter, b: &Fighter) -> i32 {
    let c = &a.cand_attack;
    if c.mv < 0 || c.flags == 0 || a.o_ce > 0 {
        return 0;
    }
    if a.pressed & 8 == 0 && a.held & 8 != 0 {
        return 0;
    }
    if b.flags[F58] & 0x1000_8000 != 0 && b.flags[F5C] & 0x4_0000 == 0 && b.flags[F64] & 0x8000 == 0 {
        return 0;
    }
    if a.flags[F58] & 0x104_0000 != 0 || a.flags[F5C] & 0x40 != 0 {
        return 0;
    }
    if c.flags & 0x100 != 0 {
        // air throw: both high enough, height difference within reach, `a` rising
        if !(a.hips[1] < -0x17F && b.hips[1] < -0x17F) {
            return 0;
        }
        if (a.o_78 as i32).abs() > c.param as i32 {
            return 0;
        }
        if a.flags[F5C] & 0x80 == 0 || a.flags[F58] & 0x1000 == 0 || a.flags[F58] & 0x4000 != 0 {
            return 0;
        }
    }
    if c.flags & 0x80 != 0 && a.flags[F58] & 0x1800 != 0 {
        return 0;
    }
    if a.flags[F58] & 0xA000_0000 == 0xA000_0000 || a.flags[F5C] & 0x4000 == 0 {
        return 0;
    }
    // an attempt from here on
    if b.o_d0 != 0 || b.flags[F60] & 0x4000_0000 != 0 {
        return -1;
    }
    let b_air = b.flags[F5C] & 0x40_0000 != 0;
    let jump_attack = b_air && b.flags[F58] & 0x8008_0000 == 0x8008_0000;
    if (!b_air || jump_attack) && (b.flags[F58] as i32) < 0 && b.flags[F64] & 0x8000 == 0 && b.o_bc <= 0 {
        return -1; // the opponent is in an attack (before its recovery)
    }
    if a.o_76 > c.param {
        return -1;
    }
    if (a.o_74 as u16).wrapping_add(0xAA) >= 0x155 {
        return -1;
    }
    let ok_height = if c.flags & 0x1000 != 0 && !b_air { true } else { c.flags & 0x2000 != 0 && b_air };
    if !ok_height {
        return -1;
    }
    let b_down = b.flags[F5C] & 0x4_0000 != 0;
    if c.flags & 0x200 != 0 {
        if b_down {
            return -1;
        }
        if b.flags[F58] & 1 != 0 {
            return 1;
        }
    }
    if c.flags & 0x400 != 0 {
        if b_down {
            return -1;
        }
        if b.flags[F58] & 0x30 != 0 {
            return 1;
        }
    }
    if c.flags & 0x800 != 0 && b_down {
        return 1;
    }
    -1
}

/// Apply a pending side flip right away.
fn flip_now(f: &mut Fighter) {
    if f.anim.flip_next {
        f.anim.flip_next = false;
        f.anim.mirror = !f.anim.mirror;
        f.swap_hands();
    }
}

fn grab(fs: &mut [Fighter], a: usize, b: usize) {
    fs[a].cand_attack.flags = 0;
    fs[a].flags[F6C] |= 0x40_0000;
    fs[b].flags[F6C] |= 0x80_0000;
    fs[b].s_2c = fs[a].cand_attack.mv;
}

fn miss(f: &mut Fighter) {
    if f.cand_attack.flags & 0x4000 != 0 {
        f.flags[F6C] |= 0x60_0000;
    }
}

/// The throw resolution (the input of both players has already run).
pub fn resolve(fs: &mut [Fighter; 2]) {
    resolve_pair(fs, [0, 1]);
}

/// `resolve` of the fighters `p` = (P1's place, P2's place) of `fs` (the team battle runs a fighter and its target,
/// engine addition).
pub fn resolve_pair(fs: &mut [Fighter], p: [usize; 2]) {
    let [x, y] = p;
    if fs[x].flags[F60] & 0x1_0000 != 0 {
        return;
    }
    if fs[x].flags[F58] & 0x4010_0000 != 0 || fs[y].flags[F58] & 0x4010_0000 != 0 {
        escape(fs, p);
        return;
    }
    // forced throws (candidate flag 0x8000)
    for k in 0..2 {
        let (a, b) = (p[k], p[1 - k]);
        if fs[a].cand_attack.flags & 0x8000 != 0 {
            grab(fs, a, b);
            fs[b].flags[F60] |= 0x1000_0000;
            flip_now(&mut fs[x]);
            flip_now(&mut fs[y]);
            return;
        }
    }
    if fs[x].flags[F5C] & 0x600_0000 != 0 || fs[y].flags[F5C] & 0x600_0000 != 0 {
        return;
    }
    let r0 = test(&fs[x], &fs[y]);
    let r1 = test(&fs[y], &fs[x]);
    if r0 == 0 && r1 == 0 {
        return;
    }
    if r0 == r1 {
        // both throw at once (or both miss): both whiff
        miss(&mut fs[x]);
        miss(&mut fs[y]);
        fs[x].cand_attack.flags = 0;
        fs[y].cand_attack.flags = 0;
        return;
    }
    match r0 {
        -1 => {
            miss(&mut fs[x]);
            fs[x].cand_attack.flags = 0;
        }
        1 => {
            grab(fs, x, y);
            flip_now(&mut fs[x]);
            flip_now(&mut fs[y]);
        }
        _ => {}
    }
    match r1 {
        -1 => {
            miss(&mut fs[y]);
            fs[x].cand_attack.flags = 0;
            fs[y].cand_attack.flags = 0;
        }
        1 => {
            grab(fs, y, x);
            flip_now(&mut fs[x]);
            flip_now(&mut fs[y]);
        }
        _ => {}
    }
}

/// The thrown fighter presses Square+Cross (0xC0) within its escape window (+0xD4).
fn escape(fs: &mut [Fighter], p: [usize; 2]) {
    for k in 0..2 {
        let (a, b) = (p[k], p[1 - k]);
        if fs[a].flags[F58] & 0x4000_0000 != 0
            && fs[b].flags[F58] & 0x10_0000 != 0
            && fs[b].o_d4 > 0
            && fs[b].recent & 0xC0 == 0xC0
            && fs[b].pressed & 0xC0 != 0
        {
            fs[a].flags[F6C] |= 0x400_0000;
            fs[b].flags[F6C] |= 0x400_0000;
            return;
        }
    }
}
