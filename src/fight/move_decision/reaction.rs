//! The parts `decide` hands on: the hit reaction, the guard flags, the generic command.
//!
//! State: none of its own: updates the serialised `Fighter`
//!
//! The parts `decide` hands on: the hit reaction (entered when a new hit lands), the
//! guard flags (a pattern the game repeats in three places) and taking the generic command.

use super::{World, revert_human};
use crate::fight::fighter::*;
use crate::fight::input;
use crate::fight::side::Slot;

/// Take the generic command candidate (+0x18) if the current state allows it.
pub(super) fn take_generic(f: &mut Fighter, f5c: u32, f58: u32) -> i32 {
    let fl = f.cand_generic.flags;
    if fl & 0x10_0000 != 0 {
        f.flags[F6C] |= 0x100;
    }
    let ok = if f5c & 0x800 != 0 && fl & 0x10 != 0 {
        true
    } else if f58 & 0x800 != 0 {
        fl & 4 != 0 || fl & 8 != 0
    } else if f58 & 0x10 != 0 {
        fl & 2 != 0
    } else {
        fl & 1 != 0
    };
    if !ok {
        f.flags[F6C] &= !0x100;
        return -1;
    }
    let mv = f.cand_generic.mv;
    f.cand_generic.mv = -1;
    input::reset_generic(f); // also zeroes +0x1C again
    input::reset_chain(f);
    mv as i32 + 0x90
}

/// Guard flags while BACK + guard bit are held (the repeated pattern).
pub(super) fn guard_flags(f: &mut Fighter, dir: u16, t1: u32, fp: u32) {
    if f.held & 0x800 != 0 && t1 & 0x100_0000 == 0 && fp & 0x1400 == 0x1000 && dir & 0x2000 == 0 {
        if f.held & 0x400 != 0 {
            f.flags[F5C] |= 2;
        }
        f.flags[F5C] |= 1;
    }
}

/// Choose the hit-reaction move from the hit event bits (F68).
#[allow(clippy::too_many_arguments)]
pub(super) fn hit_reaction(
    f: &mut Fighter,
    opp: &Fighter,
    w: &mut World,
    mut s0: u32,
    s3: u32,
    fp: u32,
    t0: u32,
    long: u16,
    p: Slot,
    trig: &crate::fight::physics::Trig,
) -> i32 {
    let mut s2: i32 = -1;
    f.flags[F5C] &= 0xFBFF_FFFF;
    f.flags[F6C] &= 0xFFF7_EFFF;
    f.move_cur = -1;
    f.fx_cancel = 2; // the fighter's trails / delayed effects are cancelled
    if t0 & 0x8000 != 0 {
        revert_human(f, w);
    } else {
        input::reset_generic(f);
    }
    w.p4b18[p][0] = 0;
    w.p4b18[p][1] = 0;
    w.p4b18[p][3] = -0x100;
    if s0 & 0x200_0000 != 0 {
        if s3 & 0x2_0000 != 0 {
            return 0x69;
        }
        if s3 & 0x1_0000 != 0 {
            return 0x6A;
        }
    }
    if f.o_00 & 0x80 != 0 {
        f.flags[F5C] &= 0xFFDF_FFFF;
        // the KO launch, with the opponent as the attacker
        let kind = if s0 & 0x4800 != 0 {
            Some(0)
        } else if s0 & 0x200_0000 != 0 {
            Some(1)
        } else if s0 & 0x2000 != 0 {
            Some(2)
        } else if s0 & 0x4_0000 != 0 {
            Some(3)
        } else {
            None
        };
        if let Some(k) = kind {
            s0 |= 0x80_0000;
            f.flags[F68] = s0;
            crate::fight::hit::launch(opp, f, k, trig);
        } else {
            f.flags[F68] = (f.flags[F68] & !0x1000) | 0x80_4400;
            crate::fight::hit::launch(opp, f, 0, trig);
            if !f.beast() {
                return 0x5B;
            }
            if !(f.o_e5 < w.c0_b4) || f.gauge < 0 {
                revert_human(f, w);
            }
            return 0x5B;
        }
    }
    let strong = s0 & 0x200_0000 != 0 || (s0 & 0x80_0000 != 0 && s0 & 0x5_6800 != 0) || s0 & 0x2000 != 0;
    if strong && f.beast() && f.gauge < 0 {
        revert_human(f, w);
    }
    let lo = s0 & 3; // hit direction: 1 front, 2 from behind
    if lo == 2 && long & 0x400 == 0 {
        s2 = 0x45;
    } else if s3 & 0x30 != 0 {
        if s0 & 0x10_0000 != 0 {
            s2 = 0x44;
        } else if s0 & 0x60_0000 != 0 {
            s2 = 0x43;
        }
    } else if s0 & 0x10_0000 != 0 {
        s2 = 0x42;
    } else if s0 & 0x20_0000 != 0 {
        s2 = 0x41;
    } else if s0 & 0x40_0000 != 0 {
        s2 = 0x40;
    }
    if s0 & 0x800 != 0 {
        let k = s0 & 0xC;
        if fp & 0x10_0000 != 0 {
            if k == 4 {
                s2 = 0x58;
            } else if k == 8 {
                s2 = 0x59;
            }
        } else if k == 4 {
            s2 = 0x59;
        } else if k == 8 {
            s2 = 0x58;
        }
    }
    if s0 & 0x1000 != 0 {
        s2 = if lo == 2 { 0x53 } else { 0x4F };
    }
    if s0 & 0x2000 != 0 {
        if s0 & 0x80_0000 != 0 {
            s2 = if lo == 2 { 0x51 } else { 0x50 };
        } else {
            match s0 & 0xC {
                4 => s2 = 0x84,
                8 => s2 = 0x85,
                _ => {}
            }
            match lo {
                1 => s2 = 0x88,
                2 => s2 = 0x89,
                _ => {}
            }
        }
    }
    if s0 & 0x4000 != 0 {
        s2 = 0x5B;
        if s0 & 0x80_0000 == 0 {
            s2 = if lo == 2 { 0x82 } else { 0x80 };
        }
    }
    if s0 & 0x8000 != 0 {
        match s0 & 0xC {
            4 => s2 = 0x5F,
            8 => s2 = 0x5E,
            _ => {}
        }
        if fp & 0x10_0000 != 0 {
            s2 = 0x53;
        }
    }
    if s0 & 0x1_0000 != 0 {
        if s0 & 0x80_0000 != 0 {
            match s0 & 0xC {
                4 => s2 = 0x55,
                8 => s2 = 0x56,
                _ => {}
            }
            if lo == 1 {
                s2 = 0x60;
            }
        } else {
            match s0 & 0xC {
                4 => s2 = 0x87,
                8 => s2 = 0x86,
                _ => {}
            }
            match lo {
                1 => s2 = 0x8C,
                2 => s2 = 0x8D,
                _ => {}
            }
        }
    }
    if s0 & 0x4_0000 != 0 {
        s2 = if s0 & 0x80_0000 != 0 {
            if lo == 2 { 0x57 } else { 0x54 }
        } else if lo == 2 {
            0x83
        } else {
            0x81
        };
    }
    s2
}
