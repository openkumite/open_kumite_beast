//! Move decision: which move a fighter requests this frame (with taking the generic command and the beast revert).
//!
//! State: none of its own: updates the serialised `Fighter`
//!
//! Called once per frame per fighter (from the fighter's step) after input processing. Returns the move to
//! request (state +0x80): values >= 0x90 are attack moves (index = move - 0x90), bit 0x1000 marks a
//! chain follow-up, -1 = keep the current move.
//!
//! The code keeps the game's control flow; the labels `L` name its branch targets
//! by what they do. Locals keep the game's register roles:
//! `dir` = s6 (held directions), `dbl_dir` = a3, `btn` = s7 (recent buttons), `s2` = movement move,
//! `s4` = attack index.
//!
//! `decide` is one function, as the routine is one in the game; its sections (the `// ---- … ----` markers):
//! - the prologue: the locals in their registers, the +0xCE timer;
//! - pending requests (F6C): the 0x10_0000 request (knock-down moves 0x6B..0x6E), a throw this fighter started,
//!   being thrown, then whether a new hit was taken (→ `hit_reaction`, `reaction.rs`);
//! - no new hit: getting up / ukemi, airborne after a hit and the walls,
//!   air recovery, knocked down, otherwise: in an attack,
//!   jumping; it picks the first label;
//! - the shared tail: the loop over the labels `L` (`Neutral` … `Done`), the attack selection
//!   in `L::Attacks`.
//!
//! `world.rs` holds `World`, the HUD flash and the beast revert, `reaction.rs` the hit reaction and its helpers.

use crate::fight::fighter::*;
use crate::fight::input;
use crate::fight::side::Slot;

mod reaction;
mod world;

use reaction::{guard_flags, hit_reaction, take_generic};
use world::hud_flash;
pub use world::{World, revert_human};

#[derive(Clone, Copy, Debug)]
enum L {
    Neutral,
    Back,
    GuardHeld,
    Up,
    DblDir,
    Taunt,
    HeldMove,
    Candidates,
    AttackCancels,
    TransformCheck,
    Transform,
    Attacks,
    Generic,
    Done,
}

/// The decision. `opp` = opponent (fighter +0x2C), `opp_groups` = the ANI's per-opponent table the
/// state's +4 pointer selects (used for being thrown).
pub fn decide(f: &mut Fighter, opp: &Fighter, w: &mut World, opp_group: &[i16], trig: &crate::fight::physics::Trig) -> i32 {
    let dir = f.held & 0xF000; // s6
    let s0 = f.flags[F68];
    let dbl_dir = f.dbl & 0xF000; // a3
    let s3 = f.flags[F58];
    let fp = f.flags[F5C];
    let t1 = f.flags[F60];
    let f64 = f.flags[F64];
    let t0 = f.flags[F6C];
    let mask = if f.beast() { 0x4E8 } else { 0x4C8 };
    let btn = f.recent & mask; // s7
    let long = f.long & mask; // 0x14(sp)
    let mut s4: i32 = -1;
    let mut s2: i32 = -1;
    let p = Slot::of_player(f.player);

    if f.o_ce > 0 {
        f.o_ce -= f.step;
        if f.o_ce < 0 {
            f.o_ce = 0;
        }
    }

    // ---- pending requests (F6C) ----
    if t0 & 0x10_0000 != 0 {
        if f.pos[1] < 0x100 {
            f.pos[1] = 0;
        }
        f.o_90 = 0;
        if f.o_92 == 0 {
            f.o_92 = 0xA0;
        }
        f.o_b2 = -0x1400;
        f.o_b4 = 0x78;
        f.o_c4 = f.o_4e.wrapping_add(f.o_70);
        f.flags[F6C] &= 0xFFF7_FFFF;
        if t0 & 0x8_0000 != 0 {
            return f.s_9a as i32;
        }
        f.flags[F6C] &= 0xFFEF_FFFF;
        let fast = f.o_70.unsigned_abs() >= 0x400;
        s2 = if s3 & 0x2_0000 != 0 {
            if fast { 0x6E } else { 0x6D }
        } else if s3 & 0x1_0000 != 0 {
            if fast { 0x6F } else { 0x70 }
        } else if fast {
            0x6C
        } else {
            0x6B
        };
        return s2;
    }
    if t0 & 0x40_0000 != 0 {
        // a throw this fighter started (set by the throw resolution)
        f.flags[F6C] &= 0xFFBF_FFFF;
        s4 = f.cand_attack.mv as i32;
        f.cand_attack.mv = -1;
        f.cand_attack.prio = -1;
        input::reset_generic(f);
        input::reset_chain(f);
        return s4 + 0x90;
    }
    if t0 & 0x80_0000 != 0 {
        // being thrown
        f.o_d4 = 0;
        f.flags[F6C] &= 0xFF7F_FFFF;
        if t0 & 0x8000 != 0 {
            revert_human(f, w);
        } else {
            input::reset_generic(f);
            input::reset_chain(f);
        }
        w.p4b18[p][0] = 0;
        w.p4b18[p][1] = 0;
        w.p4b18[p][3] = -10;
        let i = f.s_2c as i32 - 0xD0;
        return opp_group.get(i.max(0) as usize).copied().unwrap_or(0) as i32 + 0x90;
    }

    let hit_taken: bool = if s3 & 0x4000_0000 != 0 {
        if t0 & 0x400_0000 != 0 {
            f.flags[F6C] &= 0xFBFF_FFFF;
            f.o_ce = f.step.wrapping_mul(38);
            input::reset_generic(f);
            input::reset_attack(f);
            return 0x7F;
        }
        if t0 & 0x8_0000 != 0 {
            f.flags[F6C] &= 0xFFF7_FFFF;
            return f.s_9a as i32;
        }
        if fp & 0x400_0000 == 0 {
            f.flags[F5C] &= 0xFBFF_FFFF;
            return s2;
        }
        true
    } else if s3 & 0x10_0000 != 0 {
        if f.o_d4 != 0 {
            f.o_d4 -= 0x10;
            if f.o_d4 < 0 {
                f.o_d4 = 0;
            }
        }
        if t0 & 0x400_0000 != 0 {
            f.flags[F6C] &= 0xFBFF_FFFF;
            f.o_ce = f.step.wrapping_mul(38);
            input::reset_generic(f);
            input::reset_attack(f);
            return 0x7E;
        }
        if t0 & 0x8_0000 != 0 {
            f.flags[F6C] &= 0xFFF7_FFFF;
            return f.s_9a as i32;
        }
        f.flags[F5C] &= 0xFBFF_FFFF;
        return s2;
    } else {
        fp & 0x400_0000 != 0
    };

    if hit_taken {
        return hit_reaction(f, opp, w, s0, s3, fp, t0, long, p, trig);
    }

    // ---- no new hit ----
    if t0 & 0x8_0000 != 0 {
        if s3 & 0x3000 == 0x3000 {
            guard_flags(f, dir, t1, fp);
            f.flags[F5C] |= 0x4000;
        }
        f.flags[F6C] &= 0xFFF7_FFFF;
        return f.s_9a as i32;
    }

    let mut lab;
    if s3 & 0x1000_0000 != 0 || fp & 0x200_0000 != 0 {
        if (s3 as i32) < 0 {
            f.flags[F5C] = (f.flags[F5C] & 0xFDFF_FFFB) | 1;
            lab = L::HeldMove;
        } else {
            // getting up / ukemi
            if fp & 0x200_0000 != 0 {
                f.move_req = 0x30;
                f.move_cur = -1;
                f.o_cc = 0;
                s2 = if s3 & 0x800 != 0 {
                    if fp & 4 != 0 { 0x34 } else { 0x33 }
                } else if fp & 4 != 0 {
                    if s3 & 0x10 != 0 {
                        0x38
                    } else if s0 & 0x40_0000 != 0 {
                        0x36
                    } else {
                        0x37
                    }
                } else if s3 & 0x10 != 0 {
                    0x32
                } else if s0 & 0x40_0000 != 0 {
                    0x30
                } else {
                    0x31
                };
            }
            f.flags[F5C] &= 0xFDFF_FFFB;
            f.o_cc = f.o_cc.wrapping_add(f.step);
            if f.o_cc < 0x61 && fp & 0x2004 == 0 && f.pressed & 0x2000 != 0 {
                f.flags[F5C] |= 1;
                return if fp & 0x80 == 0 { 0x35 } else { 0x3F };
            }
            if f.move_req == 0x35 || f.move_req == 0x3F {
                if t1 & 0x400_0000 == 0 {
                    lab = L::HeldMove;
                } else {
                    f.flags[F5C] |= 1;
                    return s2;
                }
            } else {
                if f.held & 0x800 != 0 {
                    if t1 & 0x100_0000 != 0 || fp & 0x1400 != 0x1000 || dir & 0x2000 != 0 {
                        return s2;
                    }
                    if f.held & 0x400 != 0 {
                        f.flags[F5C] |= 2;
                    }
                    f.flags[F5C] |= 1;
                }
                return s2;
            }
        }
    } else if s3 & 0x4_0000 != 0 || s3 & 0x40 != 0 {
        lab = L::HeldMove;
    } else if s3 & 0x600_0000 != 0 {
        // airborne after a hit / wall
        if s3 & 0x800 == 0 {
            return s2;
        }
        if f.o_90 > 0 && (f.o_94 as i32) < f.model_268.max(f.model_358) {
            // (below the stage's ceiling values)
            if s3 & w.match_addr == 0 {
                return 0x5A;
            }
            return f.s_9a as i32;
        }
        if f.o_cc > 0 {
            f.o_cc -= f.step;
            if f.o_cc < 0 {
                f.o_cc = 0;
            }
        }
        // walls and the outer boundary while flying after a hit
        let hips = f.hips;
        let back = f.o_96 == 0x800;
        if (fp as i32) < 0 {
            if let Some(r) = w.stage.outer {
                let (x, z) = (hips[0], hips[2]);
                let hit = if r.kind == 0 {
                    let (hx, hz) = (r.hx as i32 - 0x40, r.hz as i32 - 0x40);
                    (x > hx && f.vel[0] > 0) || (x < -hx && f.vel[0] < 0) || (z > hz && f.vel[2] > 0) || (z < -hz && f.vel[2] < 0)
                } else {
                    (x as i64 * x as i64 + z as i64 * z as i64) > (r.w4 as i64 - 0x1000)
                };
                if hit {
                    s2 = if hips[1] < f.o_94 as i32 - 0x100 { if back { 0x4B } else { 0x4C } } else { 0x8A };
                }
            }
        } else if s3 & 0x400_0000 != 0 {
            if fp & 0x100_0000 != 0 {
                // blown into a wall
                w.stage.walls.lock = 0;
                let pw = f.o_24 / 10;
                if f.o_00 & 0x80 != 0 && f.o_e5 >= w.c0_b4 {
                    // the match-deciding KO breaks the wall
                    for side in 0..4 {
                        if t0 & (1 << side) != 0 {
                            w.stage.walls.pow[side] = 10000;
                            w.stage.walls.st[side] = -1;
                        }
                    }
                } else if hips[1] < -0x100 {
                    for side in 0..4 {
                        if t0 & (1 << side) != 0 {
                            w.stage.walls.pow[side] = pw;
                            w.stage.walls.st[side] = -1;
                        }
                    }
                    s2 = if back { 0x61 } else { 0x62 };
                } else {
                    s2 = 0x8A;
                }
            }
        } else if !(f.o_b2 == 0 && f.o_98.abs() < 0x400) && fp & 0x100_0000 != 0 {
            s2 = if hips[1] < -0x100 { if back { 0x61 } else { 0x62 } } else { 0x8A };
        }
        // P+K in the air: air recovery (ukemi)
        if btn & 0xC0 == 0xC0 && f.o_cc == 0 {
            f.o_c0 = 0;
            if hips[1] - (f.o_94 as i32) < -0x98 {
                s2 = 0x3D;
                f.o_90 = if f.held & 0x1000 != 0 {
                    -0x500
                } else if f.held & 0x4000 != 0 {
                    0x280
                } else {
                    0
                };
                f.o_92 = 0xA0;
                f.o_98 = if f.held & 0x2000 != 0 {
                    0x200
                } else if f.held & 0x8000 != 0 {
                    -0x200
                } else {
                    0
                };
            } else {
                f.o_b4 = 0x78;
                s2 = 0x3E;
                f.o_b2 = f.o_b2.wrapping_add(f.o_98.abs());
                f.o_98 = 0;
            }
        }
        lab = L::HeldMove;
    } else if s3 & 0x3_0000 != 0 && t1 & 0x20 == 0 {
        // knocked down / lying
        if s3 & 0x800_0000 != 0 {
            if fp & 0x8_0080 == 0x8_0080 {
                let fast = f.o_70.unsigned_abs() >= 0x400;
                s2 = if s3 & 0x2_0000 != 0 {
                    if fast { 0x6E } else { 0x6D }
                } else if s3 & 0x1_0000 != 0 {
                    if fast { 0x6F } else { 0x70 }
                } else if fast {
                    0x6C
                } else {
                    0x6B
                };
                return s2;
            }
            // a knocked-down fighter flying outside the ring box stops at the outer area
            if (fp as i32) < 0
                && let Some(r) = w.stage.outer
            {
                let (x, z) = (f.pos[0] as i32, f.pos[2] as i32);
                if r.kind == 0 {
                    let (hx, hz) = (r.hx as i32 - 0x40, r.hz as i32 - 0x40);
                    if x > hx && f.vel[0] > 0 {
                        f.pos[0] = hx as i16;
                        f.o_b4 = 0;
                        f.o_b2 = 0;
                    } else if x < -hx && f.vel[0] < 0 {
                        f.pos[0] = -hx as i16;
                        f.o_b4 = 0;
                        f.o_b2 = 0;
                    }
                    if z > hz && f.vel[2] > 0 {
                        f.pos[2] = hz as i16;
                        f.o_b4 = 0;
                        f.o_b2 = 0;
                    } else if z < -hz && f.vel[2] < 0 {
                        f.pos[2] = -hz as i16;
                        f.o_b4 = 0;
                        f.o_b2 = 0;
                    }
                } else if r.w4.wrapping_sub(0x1000) < x.wrapping_mul(x).wrapping_add(z.wrapping_mul(z)) {
                    f.o_b4 = 0; // circle: only the outward push stops
                    f.o_b2 = 0;
                }
            }
            return s2;
        }
        // lying on the floor
        f.flags[F5C] |= 0x8000;
        if f.o_c0 > 0 {
            f.o_c0 -= f.step;
            if f.o_c0 < 0 {
                f.o_c0 = 0;
            }
            if f.pressed & 0xF0E0 != 0 && f.o_c0 != 0 {
                f.o_c0 -= f.step;
                if f.o_c0 < 0 {
                    f.o_c0 = 0;
                }
            }
            if f.o_c0 < 0x80 {
                if f.dbl & 0x40 != 0 {
                    f.flags[F6C] |= 0x1_0000;
                }
                lab = L::HeldMove;
            } else {
                lab = L::HeldMove;
            }
        } else if f.o_c0 != 0 {
            lab = L::HeldMove;
        } else {
            // get-up direction
            let mut v1 = dir;
            if f.held & 0x200 != 0 {
                v1 = dir & 0xA000;
                if dir & 0x1000 != 0 {
                    v1 |= 0x4000;
                }
                if dir & 0x4000 != 0 {
                    v1 |= 0x1000;
                }
            }
            if s3 & 0x2_0000 != 0 {
                if t0 & 0x1_0000 != 0 {
                    s4 = 0x1E;
                } else {
                    s2 = match v1 {
                        0x2000 => 0x73,
                        0x1000 => 0x76,
                        0x4000 => 0x75,
                        0x8000 => 0x74,
                        _ => 0x71,
                    };
                }
            } else if t0 & 0x1_0000 != 0 {
                s4 = 0x1F;
            } else {
                s2 = match v1 {
                    0x2000 => 0x79,
                    0x1000 => 0x7C,
                    0x4000 => 0x7B,
                    0x8000 => 0x7A,
                    _ => 0x77,
                };
            }
            lab = L::HeldMove;
        }
    } else {
        if s3 & 0x8000 != 0 || s3 & 0x8_0000 != 0 || s3 & 0x100_0000 != 0 || (s3 as i32) < 0 {
            lab = L::HeldMove;
        } else if s3 & 0x1000 != 0 {
            // in an attack
            if s3 & 0x40_0000 != 0 {
                lab = L::HeldMove;
            } else {
                let v1 = s3 & 0x2800;
                if v1 == 0x800 || v1 == 0x2800 {
                    if s3 & 0x4000 == 0 {
                        guard_flags(f, dir, t1, fp);
                        f.flags[F5C] |= 0x4000;
                    }
                    lab = L::HeldMove;
                } else if v1 == 0x2000 {
                    guard_flags(f, dir, t1, fp);
                    f.flags[F5C] |= 0x4000;
                    lab = L::HeldMove;
                } else {
                    lab = L::HeldMove;
                }
            }
        } else if s3 & 0x200 != 0 {
            if f.o_cc > 0 {
                f.o_cc -= f.step;
                if f.o_cc < 0 {
                    f.o_cc = 0;
                }
                lab = L::HeldMove;
            } else if f.o_cc != 0 {
                lab = L::HeldMove;
            } else {
                f.flags[F5C] |= 0x4000;
                lab = L::HeldMove;
            }
        } else if s3 & 0x400 != 0 {
            // jumping
            if s3 & 8 != 0 {
                if s3 & 2 != 0 {
                    match dir {
                        0x2000 => {}
                        0x1000 => s2 = 0x1A,
                        0x3000 => s2 = 0x1B,
                        0x9000 => s2 = 0x1C,
                        _ => s2 = 6,
                    }
                    if fp & 0x180_0000 != 0 {
                        s2 = 6;
                    }
                }
                f.flags[F5C] |= 0x4000;
            } else if dir != 0x2000 {
                f.s_98 = 6;
            }
            if f.pressed & 8 != 0 {
                s2 = 0x3C;
            }
            if f.move_req == 6 {
                guard_flags(f, dir, t1, fp);
            }
            lab = L::HeldMove;
        } else {
            lab = L::Neutral;
        }
    }

    // ---- shared tail ----
    loop {
        match lab {
            L::Neutral => {
                if s3 & 0x20 != 0 {
                    // crouching: stand up unless DOWN is held
                    if dir & 0x4000 == 0 {
                        s2 = 1;
                    }
                    lab = L::HeldMove;
                    continue;
                }
                if s3 & 0x20_0000 != 0 {
                    return s2;
                }
                if t1 & 1 != 0 {
                    lab = L::HeldMove;
                    continue;
                }
                s2 = 1;
                if f.o_ce > 0 {
                    return s2;
                }
                f.flags[F5C] |= 0x1000_0000;
                f.o_c0 = 0;
                if f.held & 0x800 != 0 && t1 & 0x100_0000 == 0 && fp & 0x1400 == 0x1000 && dir & 0x2000 == 0 {
                    if f.held & 0x400 != 0 {
                        f.flags[F5C] |= 2;
                    }
                    f.flags[F5C] |= 1;
                }
                f.flags[F5C] |= 0x4000;
                match dir {
                    0x6000 => s2 = 0xD,
                    0x2000 => s2 = 2,
                    0x4000 => s2 = 0xB,
                    0x8000 => {
                        if fp & 0x10_0000 != 0 {
                            f.move_cur = -1;
                        }
                        s2 = 3;
                    }
                    0xC000 => {
                        if fp & 0x10_0000 != 0 {
                            f.move_cur = -1;
                        }
                        s2 = 0xE;
                    }
                    _ => {}
                }
                lab = L::Back;
            }
            L::Back => {
                if f.held & 0x400 == 0 {
                    lab = L::Up;
                    continue;
                }
                // auto-guard while the opponent attacks within range
                if (opp.flags[F58] as i32) >= 0 || opp.o_ba as i32 >= f.step as i32 * 10 || opp.o_bc > 0 || f.o_76 >= 0x281 {
                    lab = L::GuardHeld;
                    continue;
                }
                s2 = if dir & 0x4000 != 0 {
                    0x3B
                } else if opp.flags[F64] & 0x100 != 0 {
                    0x3A
                } else {
                    0x39
                };
                if f.held & 8 != 0 {
                    return s2;
                }
                lab = L::Up;
            }
            L::GuardHeld => {
                if f.held & 8 != 0 {
                    f.move_cur = -1;
                    s2 = if dir & 0x4000 != 0 { 0x3B } else { 0x39 };
                    return s2;
                }
                lab = L::Up;
            }
            L::Up => {
                if dir & 0x1000 != 0 {
                    return 0x13;
                }
                lab = L::DblDir;
            }
            L::DblDir => {
                match dbl_dir {
                    0x4000 => s2 = 0x11,
                    0x2000 => {
                        s2 = 4;
                        f.s_98 = 5;
                    }
                    0x8000 => s2 = 7,
                    _ => {}
                }
                lab = L::Taunt;
            }
            L::Taunt => {
                if (w.c4_w8 >> 10) & 1 != 0 {
                    if f.pressed & 1 != 0 {
                        return 0x1E;
                    }
                    if f.pressed & 2 != 0 {
                        return 0x1F;
                    }
                }
                lab = L::HeldMove;
            }
            L::HeldMove => {
                if t1 & 0x1_0000 != 0 {
                    return if s4 >= 0 { s4 + 0x90 } else { s2 };
                }
                lab = L::Candidates;
            }
            L::Candidates => {
                if f.o_ce > 0 {
                    return s2;
                }
                let mut a1 = false;
                if (s3 as i32) < 0 && f.o_bc == f.step && (f64 & 0x8000_0000 != 0 || (w.c4_w8 >> 13) & 1 != 0) {
                    a1 = true;
                }
                if s3 & 0x4_0000 != 0 || s3 & 0x40 != 0 || s3 & 0x8000 != 0 || s3 & 0x8008_0000 == 0x8_0000 || s3 & 0x100_0000 != 0 {
                    lab = L::Done;
                    continue;
                }
                if (s3 as i32) >= 0 {
                    lab = L::AttackCancels;
                    continue;
                }
                // during a move's cancel window (F58 bit 31)
                if a1 {
                    if f.recent & 0xE8 == 0xE0 && f.form & 0x10 == 0 {
                        // all three attack buttons: transform
                        input::reset_generic(f);
                        input::reset_chain(f);
                        if f.gauge < 0x1E00 {
                            return 0xBD;
                        }
                        if f.o_01 != 0 {
                            return 0xBE;
                        }
                        hud_flash(w, p, 0x3C, 0xFF);
                        return 0xBE;
                    }
                    if f.cand_generic.mv >= 0 && f.cand_generic.flags & 0x20 != 0 {
                        s4 = take_generic(f, fp, s3);
                        if s4 >= 0 {
                            return s4;
                        }
                    }
                }
                // chain follow-ups
                if f.chain_pos != f.chain_len && f.o_bc > 0 {
                    let k = f.chain_pos as usize;
                    if f.o_bc >= f.chain_wait[k] {
                        let mv = f.chain_moves[k];
                        if f.chain_sel == f.chain_pos as i8 {
                            f.chain_sel = -1;
                        }
                        f.chain_pos = ((f.chain_pos as usize + 1) % 16) as u8;
                        f.move_cur = -1;
                        return mv as i32 + 0x1090;
                    }
                }
                // escape from a throw / guard-cancel
                if f.o_ba > 0 && fp & 0x4_0080 == 0 && f64 & 0x4000_2000 == 0x4000_2000 && f.pressed & 8 != 0 {
                    if t0 & 0x8000 != 0 {
                        revert_human(f, w);
                    } else {
                        input::reset_chain(f);
                        input::reset_generic(f);
                        input::reset_attack(f);
                    }
                    f.fx_cancel = 2; // the fighter's trails / delayed effects are cancelled
                    s2 = if s3 & 0x10 != 0 { 0xB } else { 1 };
                    w.p4b18[p][0] = 0;
                    w.p4b18[p][1] = 0;
                    w.p4b18[p][3] = -0x20;
                    f.o_ce = 0xA0;
                }
                lab = L::Done;
            }
            L::AttackCancels => {
                if s3 & 0x1000 == 0 {
                    lab = L::Attacks;
                    continue;
                }
                // in an attack: cancels (+0x30 block)
                if s3 & 0x40_0000 != 0 {
                    lab = L::Done;
                    continue;
                }
                let cf = f.cand_cancel.flags;
                if cf & 0x2_0000 != 0 {
                    if s3 & 0x2800 == 0 {
                        s4 = f.cand_cancel.mv as i32;
                        f.cand_cancel.mv = -1;
                    }
                    lab = L::TransformCheck;
                } else if cf & 0x4_0000 != 0 {
                    if s3 & 0x2800 == 0x2000 {
                        f.cand_cancel.mv = -1; // (this branch jumps past the load of s4)
                    }
                    lab = L::TransformCheck;
                } else if f.hips[1] < f.cand_cancel.param as i32 && s3 & 0x800 != 0 {
                    s4 = f.cand_cancel.mv as i32;
                    f.cand_cancel.mv = -1;
                    lab = L::TransformCheck;
                } else {
                    lab = L::TransformCheck;
                }
            }
            L::TransformCheck => {
                let v1 = s3 & 0x2800;
                if v1 == 0x800 || v1 == 0x2800 {
                    lab = L::Transform;
                } else {
                    lab = L::Done;
                }
            }
            L::Transform => {
                if f.recent & 0x20 == 0 || f.gauge < 0x1E00 || f.form & 0x10 != 0 {
                    lab = L::Done;
                    continue;
                }
                s4 = 0x2E;
                if f.o_01 == 0 {
                    hud_flash(w, p, 0x3C, 0xFF);
                }
                lab = L::Done;
            }
            L::Attacks => {
                // ---- attack selection ----
                if s3 & 0x400 != 0 {
                    if s3 & 8 != 0 {
                        if s3 & 2 == 0 || dir & 0x2000 == 0 {
                            lab = L::Generic;
                            continue;
                        }
                        if btn & 0x80 != 0 {
                            s4 = 7;
                        } else if btn & 0x40 != 0 {
                            s4 = 0x17;
                        } else if f.recent & 0x20 != 0 && (f.form & 0x10 != 0 || f.gauge >= 0x1E00) {
                            s4 = 0x27;
                        }
                    } else if s3 & 2 != 0 {
                        if btn & 0x80 != 0 {
                            s4 = 6;
                        } else if btn & 0x40 != 0 {
                            s4 = 0x16;
                        } else if btn & 0x20 != 0 {
                            s4 = 0x26;
                        }
                    }
                    lab = L::Done;
                    continue;
                }
                if s3 & 0x20 != 0 {
                    if btn & 0x80 != 0 {
                        s4 = 0xA;
                    } else if btn & 0x40 != 0 {
                        s4 = 0x1A;
                    } else if btn & 0x20 != 0 {
                        s4 = 0x2A;
                    }
                    lab = L::Done;
                    continue;
                }
                lab = L::Generic;
            }
            L::Generic => {
                if f.cand_generic.mv >= 0 {
                    s4 = take_generic(f, fp, s3);
                    if s4 >= 0 {
                        return s4;
                    }
                }
                let (p_, k_, b_) = (btn & 0x80 != 0, btn & 0x40 != 0, btn & 0x20 != 0);
                let pick = |a: i32, b: i32, c: i32| -> i32 {
                    if p_ {
                        a
                    } else if k_ {
                        b
                    } else if b_ {
                        c
                    } else {
                        -1
                    }
                };
                let chosen;
                if dir & 0x4000 != 0 {
                    chosen = if fp & 0x40_0000 != 0 {
                        pick(0xC, 0x1C, 0x2C)
                    } else if dir & 0x2000 != 0 {
                        pick(4, 0x14, 0x24)
                    } else if dir & 0x8000 != 0 {
                        pick(5, 0x15, 0x25)
                    } else {
                        pick(3, 0x13, 0x23)
                    };
                    if chosen >= 0 {
                        s4 = chosen;
                    }
                    // crouch dash variants
                    if fp & 0x800 != 0 && f.recent & 0x4000 != 0 {
                        if btn & 0x80 != 0 {
                            s4 = 0x30;
                        } else if btn & 0x40 != 0 {
                            s4 = 0x31;
                        } else if btn & 0x20 != 0 {
                            s4 = 0x32;
                        }
                    }
                } else {
                    chosen = if fp & 0x40_0000 != 0 {
                        pick(0xB, 0x1B, 0x2B)
                    } else if dir & 0x2000 != 0 {
                        pick(1, 0x11, 0x21)
                    } else if dir & 0x8000 != 0 {
                        pick(2, 0x12, 0x22)
                    } else {
                        pick(0, 0x10, 0x20)
                    };
                    if chosen >= 0 {
                        s4 = chosen;
                    }
                }
                // Circle in human form = transform
                if f.recent & 0x20 != 0 && f.form & 0x10 == 0 {
                    input::reset_generic(f);
                    input::reset_chain(f);
                    if f.gauge < 0x1E00 {
                        return 0xBD;
                    }
                    if f.o_01 != 0 {
                        return 0xBE;
                    }
                    hud_flash(w, p, 0x3C, 0xFF);
                    return 0xBE;
                }
                lab = L::Done;
            }
            L::Done => {
                if s4 < 0 {
                    return s2;
                }
                return s4 + 0x90;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The fighter is human again, its sound (the sound queue's a0, a1) and burst (op17 sub F) are queued.
    #[test]
    fn revert_queues_sound_and_burst() {
        let mut f = Fighter::new(0, 0);
        f.form = 0x11;
        f.gauge = -1;
        let mut w = World::new();
        revert_human(&mut f, &mut w);
        assert_eq!((f.form, f.gauge), (0, 0));
        assert_eq!(w.sounds, vec![(0x400, 7)]);
        assert_eq!(w.effects, vec![[0x17, 0x0F, 0, 0, 0, 0]]);
        assert_eq!(
            (
                crate::fight::move_script::revert_sound(crate::fight::version::Gates::of(3)),
                crate::fight::move_script::revert_sound(crate::fight::version::Gates::of(4))
            ),
            ((0x400, 7), (7, 0))
        );
    }
}
