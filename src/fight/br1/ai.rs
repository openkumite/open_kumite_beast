//! The imported game's CPU opponent: the AI block's init, the driver and its action VM.
//!
//! State: serialised (`Br1Ai`, in `bridge::Br1Match`)
//!
//! The main game's AI (`fight::ai`) is its direct descendant: the same driver states 0..9, the same random-table
//! mechanism and reaction formula, the same VM. The imported game's differences are kept as they are: its level tables
//! (WAIT / REACT / GUARD in the static tables, no dash / side-step / recovery-chance tables), its guard-stun push out of state 3
//! and op 17 (`level >> 3`; never for Uriko in beast form), its stagger escape (state 6) and recovery timing by the
//! hips' height (state 7), its practice / watch states 10..21, raw HP in the value sources, the beast gauge's 0xF00,
//! jumps of Bra / Bsr subs ≥ 12, Chk subs ≥ 41 and Wait subs ≥ 11 taking sub 0's path, the "may guard" pad bit
//! 0x400. The block's pointers are offsets: `pc` and the stack into the fighter's ANM (absolute pointers in the
//! game, less the ANM's base), the tables by their place in the static tables. Its debug flag (+0x6E) and the copies of F.50
//! / F.64 (+0x70 / +0x74: only state 4 reads +0x70, which then equals F.50) are not kept. The registers r8.. and the
//! return stack overlap in the game's block (+0x4C); no script of the nine fighters names a register above r5, so they
//! are kept apart here.

use super::object::{Block, Br1Fighter};
use crate::formats::tables::Tables;
use serde::{Deserialize, Serialize};

/// The level tables of the init: WAIT (u16 min, max idle frames per level / 16), REACT (chance / 256 to
/// answer an attack), GUARD (chance / 256 to guard rather than counter-attack), RECOVER the pads of a recovery.
const WAIT: &str = "ai.wait";
const REACT: &str = "ai.react";
const GUARD: &str = "ai.guard";
const RECOVER: &str = "ai.recover";
/// The random bytes (`Table::boot`, `Table::shuffle`).
const RANDOM: &str = "math.random_bytes";

/// The AI's random table (both AI blocks point at it).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Table(#[serde(with = "crate::base::serde_util::big_array")] pub [u8; 256]);

impl Default for Table {
    fn default() -> Self {
        Self([0; 256])
    }
}

impl Table {
    /// The table at the boot: the 256 random bytes (`math.random_bytes`) copied, then shuffled with seed 0
    /// them; every round start shuffles it again with a seed (`shuffle`).
    pub fn boot(tables: &Tables) -> Self {
        let r = tables.table(RANDOM);
        let mut t = Self(std::array::from_fn(|k| r.u8(k)));
        t.shuffle(tables, 0);
        t
    }

    /// Each byte swapped with the one the random bytes from `seed` on name.
    pub fn shuffle(&mut self, tables: &Tables, seed: u32) {
        let r = tables.table(RANDOM);
        for i in 0..256u32 {
            let j = r.u8((seed.wrapping_add(i) & 0xFF) as usize) as usize;
            self.0.swap(i as usize, j);
        }
    }
}

/// The R3000's division: a zero divisor gives −1 (dividend ≥ 0) or 1 (the code has no check).
fn div(a: i32, b: i32) -> i32 {
    if b == 0 { if a >= 0 { -1 } else { 1 } } else { a.wrapping_div(b) }
}

/// The AI block A = `*(F.34)` (P1, P2, 0x78 bytes): the fields the driver and the VM keep.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Br1Ai {
    /// +0x04: the random index (player · 128 at the init, + 1 a draw)
    pub idx: u8,
    /// +0x06: the pad of this frame
    pub pad: u16,
    /// +0x08: the level 0..0xFF (G.9C at the init)
    pub level: u16,
    /// +0x0A: 1 → pad 0x400 (may guard), 2 the guard reaction of op 17, 4 the wake-up kick
    pub flags: u16,
    /// +0x24: the VM's position (ANM offset)
    pub pc: u32,
    /// +0x28: the driver's state 0..21
    pub state: i16,
    /// +0x2A / +0x2C: timers
    pub t2a: i16,
    pub t2c: i16,
    /// +0x30: the action (−1 none), +0x34 its distance band, +0x38 the result of Chk / Cmp
    pub action: i32,
    pub band: i32,
    pub res: i32,
    /// +0x3C: the VM's registers (r0 the ops' wait counter)
    pub regs: [i16; 16],
    /// +0x4C: the return stack (ANM offsets), +0x6C its depth
    pub stack: [u32; 8],
    pub depth: u16,
}

/// What one frame of the AI reads besides its block: its fighter, the opponent's block (an imported core's, or a
/// main-game fighter's view, `bridge::shadow`) and hips height (MD + 0x178), the fighter's ANM and the static tables, the
/// random table and the round timer G.90.
pub struct Ctx<'a> {
    pub me: &'a Br1Fighter,
    pub opp: &'a Block,
    pub opp_hips: i32,
    pub anm: &'a [u8],
    pub tables: &'a Tables,
    pub rnd: &'a Table,
    pub timer: i16,
}

impl Ctx<'_> {
    fn u8(&self, o: u32) -> u8 {
        self.anm.get(o as usize).copied().unwrap_or(0)
    }
    fn u16(&self, o: u32) -> u16 {
        u16::from_le_bytes([self.u8(o), self.u8(o.wrapping_add(1))])
    }
    fn i16(&self, o: u32) -> i16 {
        self.u16(o) as i16
    }
    fn u32(&self, o: u32) -> u32 {
        self.u16(o) as u32 | (self.u16(o.wrapping_add(2)) as u32) << 16
    }
    /// The AI section's word `k` (ANM header +0x18).
    fn word(&self, k: u32) -> u32 {
        self.u32(self.u32(0x18).wrapping_add(4 * k))
    }
}

impl Br1Ai {
    /// The block's start for a round (the rest of it is kept from the last round).
    /// `mode` = (F.01 & 0x7F) − 1 for a CPU fighter, else 0; modes 0..15 by the mode table, any other: state 0.
    pub fn init(&mut self, anm: &[u8], player: u8, level: u16, mode: i32) {
        let c = |k: u32| {
            let o = u32::from_le_bytes(anm.get(0x18..0x1C).and_then(|s| s.try_into().ok()).unwrap_or([0; 4])).wrapping_add(4 * k) as usize;
            anm.get(o..o + 4).map_or(0, |s| u32::from_le_bytes([s[0], s[1], s[2], s[3]]))
        };
        self.idx = player << 7;
        self.pad = 0;
        self.state = 0;
        self.t2a = 0;
        self.flags = 0;
        self.depth = 0;
        self.level = level;
        self.regs[..7].fill(0);
        let (state, flag, pc) = match mode {
            1 | 3 => (9, false, None),
            2 => (8, false, Some(5)),
            4 => (10, true, None),
            5 => (11, false, None),
            6 => (12, true, None),
            7 => (13, false, None),
            8 => (14, true, None),
            9 => (15, true, Some(6)),
            10 => (16, true, Some(7)),
            11 => (17, true, Some(8)),
            12 => (18, true, Some(9)),
            13 => {
                self.res = 0;
                (19, false, None)
            }
            14 => (20, true, None),
            15 => (21, false, None),
            _ => (0, false, None),
        };
        self.state = state;
        if flag {
            self.flags |= 1;
        }
        if let Some(k) = pc {
            self.pc = c(k);
        }
    }

    /// A decoded block the driver can run: a state 0..21, the return stack at most 8 deep and a level 0..0xFF (the
    /// tables' rows are `level >> 4`). `pc` and the stack's entries are not bounded (they are words of the ANM, any
    /// value; an op outside the ANM sets state 0).
    pub fn valid(&self) -> bool {
        (0..=21).contains(&self.state) && (self.depth as usize) <= self.stack.len() && self.level <= 0xFF
    }

    /// One draw of the random table (`table[A+4++]`).
    fn r(&mut self, c: &Ctx) -> u32 {
        let v = c.rnd.0[self.idx as usize];
        self.idx = self.idx.wrapping_add(1);
        v as u32
    }

    /// The reaction delay: (0x100 − level) · (three draws) / 0x2000, rounded toward 0.
    fn reaction(&mut self, c: &Ctx) -> i16 {
        let s = (self.r(c) + self.r(c) + self.r(c)) as i32;
        let v = (0x100 - self.level as i32).wrapping_mul(s);
        ((if v < 0 { v + 0x1FFF } else { v }) >> 13) as i16
    }

    /// The level's row of a byte table (REACT / GUARD: `level >> 4`; a level past 0xFF reads on into what follows,
    /// `Tables::abs_u8`).
    fn row(&self, c: &Ctx, t: &str) -> u32 {
        c.tables.abs_u8(c.tables.table(t).origin() + (self.level as u32 >> 4)) as u32
    }

    /// In guard stun (F.54 & 0x2000080 == 0x2000000) a push forward out of it while F.C2 < 0x61,
    /// with the chance level / 8; never for Uriko in her beast form (chr 8, F.5 0x11).
    fn guard_push(&mut self, c: &Ctx) {
        let f = &c.me.f;
        if f.f54 & 0x200_0080 == 0x200_0000 && !(f.chr == 8 && f.form == 0x11) && f.hit_stun < 0x61 && self.r(c) < (self.level >> 3) as u32 {
            self.pad = 0x2000;
        }
    }

    /// The hips' fall timing of states 7 and 11: a share of the height by a draw.
    fn hips_share(&mut self, c: &Ctx, hips: i32) -> i16 {
        let v = hips.wrapping_mul(self.r(c) as i32);
        ((if v < 0 { v + 0xFF } else { v }) >> 8) as i16
    }

    fn recover_pad(&mut self, c: &Ctx) -> u16 {
        let k = self.r(c) & 7;
        c.tables.table(RECOVER).u16(2 * k as usize)
    }

    /// One frame; returns the pad (`A+6`).
    pub fn think(&mut self, c: &Ctx) -> u16 {
        #[derive(Clone, Copy)]
        enum L {
            S0,
            S1,
            Opp,
            Matched,
            S2,
            S3,
            Choose,
            S4,
            S5,
            S6,
            S7,
            /// The VM unless F.58 & 0x8000
            S8,
            Vm,
            S10,
            S11,
            S13,
            S14,
            S16,
            S20,
            S21,
            End,
            /// Out without the pad's 0x400
            Out,
        }
        let (f, o) = (&c.me.f, c.opp);
        let lv = self.level as u32 >> 4;
        let angle = (f.opp_angle as i32).abs();
        self.pad = 0;
        let mut lab = match self.state {
            0 => L::S0,
            1 => L::S1,
            2 => L::S2,
            3 => L::S3,
            4 => L::S4,
            5 => L::S5,
            6 => L::S6,
            7 => L::S7,
            8 | 15 => L::S8,
            10 => L::S10,
            11 => L::S11,
            // jump
            12 => {
                self.pad = 0x1000;
                L::End
            }
            13 => L::S13,
            14 => L::S14,
            16..=18 => L::S16,
            // crouch
            19 => {
                self.pad = 0x4000;
                L::End
            }
            20 => L::S20,
            21 => L::S21,
            _ => L::End,
        };
        loop {
            lab = match lab {
                L::S0 => {
                    let w = c.tables.table(WAIT).origin() + 4 * lv;
                    let (lo, hi) = (c.tables.abs_u16(w) as i32, c.tables.abs_u16(w + 2) as i32);
                    let v = (hi - lo + 1).wrapping_mul(self.r(c) as i32);
                    self.action = -1;
                    self.state = 1;
                    self.flags |= 1;
                    self.t2a = (lo + ((if v < 0 { v + 0xFF } else { v }) >> 8)) as i16;
                    self.t2c = self.t2a / 2;
                    L::S1
                }
                L::S1 => {
                    let f50 = f.f50;
                    if f50 & 0xC000_0000 != 0 {
                        L::End
                    } else if f50 & 0x1000_0000 != 0 {
                        // hit or guarding: answer the opponent's next attack in its first step
                        if (o.f50 as i32) >= 0 || o.step < o.startup || o.startup < 0 {
                            L::End
                        } else if self.r(c) < self.row(c, REACT) {
                            self.state = 3;
                            L::S3
                        } else {
                            L::End
                        }
                    } else if f50 & 0x8000 != 0 {
                        self.action = 0;
                        L::Choose
                    } else if angle >= 0x6AB {
                        self.action = 1;
                        L::Choose
                    } else if angle >= 0x201 {
                        self.action = 2;
                        L::Choose
                    } else {
                        let running = self.t2a != 0 && {
                            self.t2a = self.t2a.wrapping_sub(1);
                            self.t2a != 0
                        };
                        if running {
                            L::Opp
                        } else if f.f58 & 0x8000 != 0 {
                            L::End
                        } else {
                            self.action = 3;
                            L::Choose
                        }
                    }
                }
                L::Opp => {
                    let o50 = o.f50;
                    if (o50 as i32) >= 0 {
                        // the first action from 6 whose opponent mask F.50 holds (the list ends at mask 0)
                        let recs = c.word(2);
                        self.action = 6;
                        loop {
                            let mask = c.u32(recs.wrapping_add((self.action as u32).wrapping_mul(12)));
                            if mask == 0 {
                                break L::End;
                            }
                            if o50 & mask == mask {
                                break L::Matched;
                            }
                            self.action += 1;
                        }
                    } else if o.recovery > 0 {
                        self.action = 5;
                        L::Choose
                    } else if self.r(c) < self.row(c, REACT) {
                        if self.r(c) < self.row(c, GUARD) {
                            self.state = 3;
                            L::S3
                        } else {
                            self.action = 4;
                            L::Choose
                        }
                    } else {
                        self.state = 2;
                        L::S2
                    }
                }
                L::Matched => {
                    if self.t2c < self.t2a {
                        self.action = -1;
                        self.state = 1;
                        L::End
                    } else {
                        L::Choose
                    }
                }
                L::S2 => {
                    // watching the opponent's attack
                    if o.f50 & 0x8000 != 0 || o.active == 0 {
                        self.state = 0;
                        L::End
                    } else {
                        self.flags &= 0xFFFE;
                        if o.opp_dist < 0x401 && o.f5c & 0x80 != 0 {
                            let q = div(o.startup as i32, o.step as i32);
                            if q == 5 {
                                if self.r(c) < 0x40 {
                                    self.pad = 0x4000;
                                }
                            } else if q < 5 && f.f50 & 0x10 != 0 {
                                self.pad = 0x4000;
                            }
                        }
                        L::End
                    }
                }
                L::S3 => {
                    // guard / evade
                    if f.f54 & 0x200_0080 == 0x200_0000 {
                        self.guard_push(c);
                    } else {
                        self.pad = if o.opp_dist < 0x400 && o.f5c & 0x40 != 0 { 0x4000 } else { 0 };
                    }
                    if o.active == 0 {
                        self.state = 0;
                    }
                    L::End
                }
                L::Choose => {
                    // the distance band, then the action's script for it
                    let dist = c.word(1);
                    self.band = 0;
                    let mut odd = false;
                    while (c.u16(dist.wrapping_add(2 * self.band as u32)) as i32) < f.opp_dist as i32 {
                        // past the ANM's end (an odd file; the game would read on in RAM): state 0
                        if dist.wrapping_add(2 * self.band as u32) as usize >= c.anm.len() {
                            odd = true;
                            break;
                        }
                        self.band += 1;
                    }
                    if odd {
                        self.state = 0;
                        L::End
                    } else {
                        let rec = c.word(2).wrapping_add(self.action.wrapping_mul(12) as u32);
                        self.pc = c.u16(rec.wrapping_add(4 + 2 * self.band as u32)) as u32;
                        self.t2a = self.reaction(c);
                        self.depth = 0;
                        self.state = 4;
                        L::S4
                    }
                }
                L::S4 => {
                    // state 4 (+0x70, the frame's copy of F.50, is F.50: its second test never holds)
                    let f50 = f.f50;
                    if f50 & 0x8000 != 0 && f50 & 0x3_0000 != 0 {
                        self.state = 5;
                        if self.r(c) < 0x20 {
                            self.flags |= 4;
                        } else {
                            self.flags &= 0xFFFB;
                        }
                        L::S5
                    } else if f50 & 0x8000 != 0 && f.f60 & 0x82_1000 != 0 {
                        self.state = 6;
                        L::End
                    } else if self.t2a != 0 {
                        self.t2a = self.t2a.wrapping_sub(1);
                        L::End
                    } else {
                        L::Vm
                    }
                }
                L::S5 => {
                    // down; the wake-up
                    let f50 = f.f50;
                    if f50 & 0x200_0000 != 0 {
                        self.state = 7;
                        self.t2a = 0;
                        L::End
                    } else if f50 & 0x3_0000 == 0 {
                        self.action = 4;
                        L::Choose
                    } else if f50 & 0x4_0000 != 0 || f.down_timer < 0 {
                        L::End
                    } else if f.down_timer == 0 {
                        self.pad = match self.r(c) {
                            0..=0x2F => 0x2000,
                            0x30..=0x5F => 0x8000,
                            0x60..=0x8F => 0x1000,
                            0x90..=0xBF => 0x4000,
                            _ => 0,
                        };
                        L::End
                    } else {
                        let r = self.r(c);
                        if (r as i32) < (self.level as i32 + 0x20) >> 1 {
                            self.pad = if self.flags & 4 != 0 {
                                0x40
                            } else if r & 1 != 0 {
                                0x8000
                            } else {
                                0x4000
                            };
                        }
                        L::End
                    }
                }
                L::S6 => {
                    // staggered
                    if f.f50 & 0x8000 == 0 {
                        self.state = 0;
                    } else {
                        let r = self.r(c);
                        if r as i32 <= (self.level as i32 + 0x20) >> 2 {
                            self.pad = if r & 1 != 0 { 0x8000 } else { 0x4000 };
                        }
                    }
                    L::End
                }
                L::S7 => {
                    // in the air after a hit; the recovery when the hips have come down far enough
                    if f.f50 & 0x200_0000 == 0 {
                        self.state = 0;
                        L::End
                    } else {
                        if f.f54 & 0x400_0000 != 0 {
                            self.t2a = 0;
                        }
                        if f.hit_stun == 0 && f.vy > 0 {
                            let hips = c.me.hips_y();
                            if self.t2a == 0 {
                                self.t2a = self.hips_share(c, hips);
                                if self.t2a >= -0xFF {
                                    self.t2a = 1;
                                }
                            }
                            if (self.t2a as i32) < hips {
                                self.pad = self.recover_pad(c);
                                self.state = 0;
                            }
                        }
                        L::End
                    }
                }
                L::S8 => {
                    if f.f58 & 0x8000 != 0 {
                        L::End
                    } else {
                        L::Vm
                    }
                }
                L::Vm => {
                    self.run(c);
                    L::End
                }
                L::S13 => {
                    // guard after a hit, 4 frames past the attack
                    if (o.f50 as i32) < 0 {
                        self.t2a = 4;
                        if f.f54 & 0x6000_0000 != 0 {
                            self.flags |= 1;
                        }
                        if self.flags & 1 == 0 { L::Out } else { L::S10 }
                    } else if self.t2a > 0 {
                        self.t2a -= 1;
                        L::End
                    } else {
                        self.flags &= 0xFFFE;
                        L::End
                    }
                }
                L::S10 => {
                    // guard (turned toward the opponent; a low guard unless the attack is high / mid)
                    if angle >= 0x201 {
                        self.pad = 0xC000;
                    } else if (o.f50 as i32) < 0 && div(o.startup as i32, o.step as i32) < 5 && o.recovery <= 0 {
                        self.pad = if o.f5c & 0x180 != 0 { 0 } else { 0x4000 };
                    }
                    L::End
                }
                L::S21 => {
                    if f.f50 & 0x1000 != 0 {
                        self.pad = self.recover_pad(c) & 0xFF7F;
                        L::End
                    } else {
                        L::S11
                    }
                }
                L::S11 => {
                    // recover in the air
                    if f.f54 & 0x400_0000 != 0 {
                        self.t2a = 0;
                    }
                    if f.f50 & 0x600_0000 != 0 && f.hit_stun == 0 && f.vy > 0 {
                        let hips = c.me.hips_y();
                        if self.t2a == 0 {
                            self.t2a = self.hips_share(c, hips);
                        }
                        if (self.t2a as i32) < hips {
                            self.pad = self.recover_pad(c);
                        }
                    }
                    L::End
                }
                L::S14 => {
                    // dodge
                    if f.f54 & 0x200_0000 != 0 && f.hit_stun < 0x61 && self.r(c) & 1 == 0 {
                        self.t2a = 0;
                        self.pad = 0x2000;
                    } else if angle >= 0x201 {
                        self.pad = 0xC000;
                    } else if f.f54 & 0x2000_0000 != 0 && f.f60 & 0x82_1000 != 0 {
                        self.pad = if self.r(c) & 1 == 0 { 0x4000 } else { 0x8000 };
                    } else if (o.f50 as i32) < 0 {
                        let q = div(o.startup as i32, o.step as i32);
                        if q >= 12 && o.recovery <= 0 && o.f5c & 0x40 != 0 && self.r(c) & 3 == 0 {
                            self.pad = 0x1000;
                            self.res = 0x1000;
                            self.t2a = 2;
                        }
                        if q == 4 && o.recovery <= 0 {
                            self.t2a = 0x11;
                            let low = o.f5c & 0x80 == 0 && (o.f5c & 0x100 == 0 || self.r(c) & 1 == 0);
                            self.pad = if low { 0x4000 } else { 0 };
                            self.res = self.pad as i32;
                        }
                        if self.t2a > 0 {
                            self.t2a -= 1;
                            self.pad = self.res as u16;
                        }
                    }
                    L::End
                }
                L::S16 => {
                    // turn to the opponent, then the script
                    if (f.f50 as i32) >= 0 && angle >= 0x201 {
                        self.pad = 0xC000;
                        L::End
                    } else {
                        L::S8
                    }
                }
                L::S20 => {
                    // crouch, stand up for 2 frames, crouch on
                    self.t2a = self.t2a.wrapping_add(1);
                    let v = self.t2a;
                    if (3..5).contains(&v) {
                        self.pad = 0;
                    } else if v < 0x11 || f.f50 & 0x20 != 0 || c.me.anim.id == 0x18 {
                        self.pad = 0x4000;
                    } else {
                        self.t2a = 0;
                    }
                    L::End
                }
                L::End => {
                    if self.flags & 1 != 0 {
                        self.pad |= 0x400;
                    }
                    break;
                }
                L::Out => break,
            };
        }
        self.pad
    }

    /// Up to 50 ops until one yields.
    fn run(&mut self, c: &Ctx) {
        for _ in 0..50 {
            if !self.op(c) {
                break;
            }
        }
    }

    /// The Bra / Bsr / Ret conditions on the result (subs 2..9; any other sub: always).
    fn cond(&self, sub: u8) -> bool {
        let r = self.res;
        match sub {
            2 | 5 => r != 0,
            3 | 4 => r == 0,
            6 => r > 0,
            7 => r >= 0,
            8 => r < 0,
            9 => r <= 0,
            _ => true,
        }
    }

    /// The target slot of Bra / Bsr subs 1, 10 and 11 (0 = none) and the op's end.
    fn pick(&mut self, c: &Ctx, sub: u8) -> (u32, u32) {
        let s = self.pc;
        match sub {
            1 => {
                let t = if self.res > 0 {
                    c.u16(s + 2)
                } else {
                    self.res = 0;
                    c.u16(s + 6)
                };
                (t as u32, s + 8)
            }
            10 => {
                // the weighted pick (a run past the n weights cannot happen while they sum to > 0)
                let n = c.u8(s + 2) as u32;
                let w = |k: u32| if k < n { c.u8(s + 3 + k) as i32 } else { 0 };
                let total: i32 = (0..n).map(w).sum();
                let mut a = ((self.r(c) as i32 * total) >> 8) - w(0);
                let mut k = 0;
                while a >= 0 && k + 1 < n {
                    k += 1;
                    a -= w(k);
                }
                (c.u16(s + ((n + 4) & 0x1FE) + 2 * k) as u32, s + ((3 * n + 4) & 0x7FE))
            }
            _ => (if self.res != 0 { c.u16(s + 2) } else { c.u16(s + 4) } as u32, s + 6),
        }
    }

    fn push(&mut self, ret: u32, target: u32) {
        if (self.depth as usize) < self.stack.len() {
            self.stack[self.depth as usize] = ret;
            self.depth += 1;
            self.pc = target;
        } else {
            self.pc = ret;
        }
    }

    /// One op (`pc` at it); false = stop for this frame.
    fn op(&mut self, c: &Ctx) -> bool {
        let s = self.pc;
        if s as u64 + 2 > c.anm.len() as u64 {
            self.state = 0;
            return false;
        }
        let (op, sub) = (c.u8(s), c.u8(s + 1));
        let (f, o) = (&c.me.f, c.opp);
        match op {
            1 => {
                // Bra: subs 0 and ≥ 12 jump, 2..9 by the result (a target 0 is the ANM's start)
                match sub {
                    1 | 10 | 11 => {
                        let (t, next) = self.pick(c, sub);
                        self.pc = if t != 0 { t } else { next };
                    }
                    _ => self.pc = if self.cond(sub) { c.u16(s + 2) as u32 } else { s + 4 },
                }
                true
            }
            2 => {
                // Bsr
                match sub {
                    1 | 10 | 11 => {
                        let (t, next) = self.pick(c, sub);
                        if t == 0 {
                            self.pc = next;
                        } else {
                            self.push(next, t);
                        }
                    }
                    _ if self.cond(sub) => self.push(s + 4, c.u16(s + 2) as u32),
                    _ => self.pc = s + 4,
                }
                true
            }
            3 => {
                // Ret
                if !self.cond(sub) {
                    self.pc = s + 2;
                    return true;
                }
                let Some(&ret) = self.depth.checked_sub(1).and_then(|d| self.stack.get(d as usize)) else {
                    self.state = 0;
                    return false;
                };
                self.depth -= 1;
                self.pc = ret;
                true
            }
            4 => {
                self.check(c, sub);
                true
            }
            5 => {
                // Move: dst = high nibble; src 1..5 a register, 7 an immediate, 15 a value source; others
                // the src itself without advancing
                let (src, dst) = (sub & 15, (sub >> 4) as usize);
                let v = match src {
                    7 => {
                        self.pc = s + 4;
                        c.i16(s + 2)
                    }
                    1..=5 => {
                        self.pc = s + 2;
                        self.regs[src as usize]
                    }
                    15 => {
                        let k = c.u8(s + 2);
                        if (1..=10).contains(&k) {
                            self.pc = s + 4;
                        }
                        let hp = |x: &Block| x.max_hp.wrapping_sub(x.damage);
                        match k {
                            1 => f.opp_angle,
                            2 => f.opp_dist,
                            3 => self.level as i16,
                            4 => c.timer & 0x7FFF,
                            5 => f.gauge / 64,
                            6 => o.gauge / 64,
                            7 => hp(f),
                            8 => hp(o),
                            9 => c.me.hips_y().wrapping_sub(f.floor as i32) as i16,
                            10 => c.opp_hips.wrapping_sub(o.floor as i32) as i16,
                            _ => k as i16,
                        }
                    }
                    _ => src as i16,
                };
                self.regs[dst] = v;
                true
            }
            6 => {
                // Cmp: res = lhs (high nibble) − rhs (low nibble)
                let (l, rr) = ((sub >> 4) as usize, sub & 15);
                let mut a3 = s;
                let rhs = match rr {
                    1..=5 => self.regs[rr as usize] as i32,
                    12 => o.gauge as i32,
                    14 => o.max_hp as i32 - o.damage as i32,
                    15 => {
                        a3 += 2;
                        c.i16(s + 2) as i32
                    }
                    _ => rr as i32,
                };
                let lhs = match l {
                    1..=5 => {
                        self.pc = a3 + 2;
                        self.regs[l] as i32
                    }
                    15 => {
                        self.pc = a3 + 4;
                        c.i16(a3 + 2) as i32
                    }
                    _ => l as i32,
                };
                self.res = lhs.wrapping_sub(rhs);
                true
            }
            7 => {
                // Add
                let (src, dst) = (sub & 15, (sub >> 4) as usize);
                let hp = |x: &Block| x.max_hp.wrapping_sub(x.damage);
                let v = match src {
                    1..=5 => self.regs[src as usize],
                    7 => f.opp_angle,
                    8 => f.opp_dist,
                    9 => self.level as i16,
                    10 => c.timer & 0x7FFF,
                    11 => f.gauge,
                    12 => o.gauge,
                    13 => hp(f),
                    14 => hp(o),
                    15 => c.i16(s + 2),
                    _ => src as i16,
                };
                match src {
                    1..=5 | 7..=14 => self.pc = s + 2,
                    15 => self.pc = s + 4,
                    _ => {}
                }
                self.regs[dst] = self.regs[dst].wrapping_add(v);
                true
            }
            8..=10 => {
                // Sub / Mul / Div
                let (src, dst) = (sub & 15, (sub >> 4) as usize);
                let v = match src {
                    1..=5 => {
                        self.pc = s + 2;
                        self.regs[src as usize]
                    }
                    15 => {
                        self.pc = s + 4;
                        c.i16(s + 2)
                    }
                    _ => src as i16,
                };
                let d = self.regs[dst];
                self.regs[dst] = match op {
                    8 => d.wrapping_sub(v),
                    9 => d.wrapping_mul(v),
                    _ => div(d as i32, v as i32) as i16,
                };
                true
            }
            11 => self.wait(c, sub),
            12 => {
                // SFlg
                let v = c.u16(s + 2);
                self.flags = if sub == 1 { self.flags & v } else { self.flags | v };
                self.pc = s + 4;
                true
            }
            16 | 18 => {
                // Pad: hold hw2 for byte1 frames
                if self.regs[0] == 0 {
                    self.regs[0] = sub as i16;
                }
                self.regs[0] = self.regs[0].wrapping_sub(1);
                self.pad = c.u16(s + 2);
                if self.regs[0] == 0 {
                    self.pc = s + 4;
                }
                false
            }
            17 | 19 => {
                self.guarded_pad(c, sub);
                false
            }
            _ => {
                // End (0, 13..15; the table has no entry past 19)
                self.state = 0;
                false
            }
        }
    }

    /// Chk: a test of the fighter (even subs) or the opponent (odd subs) → `res`; subs ≥ 41 are sub 0.
    fn check(&mut self, c: &Ctx, sub: u8) {
        let s = self.pc;
        let x = if sub & 1 == 0 || sub >= 41 { &c.me.f } else { c.opp };
        let atk = (x.f50 as i32) < 0;
        let mask = c.u16(s + 2) as u32 | (c.u16(s + 4) as u32) << 16;
        let b = |v: bool| v as i32;
        let (v, size) = match sub {
            2 | 3 => ((x.f50 & 0x1000_0000) as i32, 2),
            4 | 5 => ((x.f50 & 0x800) as i32, 2),
            6 | 7 => (b(x.f50 & 0x800 == 0), 2),
            8 | 9 => ((x.f50 & 0x8000_0000) as i32, 2),
            10 | 11 => (b(atk && x.startup != 0), 2),
            12 | 13 => (b(atk && x.startup == 0 && x.active != 0), 2),
            14 | 15 => (b(atk && x.recovery > 0), 2),
            16 | 17 => ((x.form & 1) as i32, 2),
            18 | 19 => (b(atk && x.f64 & 0x100_0000 != 0), 2),
            20 | 21 => (b(atk && x.f64 & 0x200_0000 != 0), 2),
            22 | 23 => ((x.f50 & 1) as i32, 2),
            24 | 25 => ((x.f50 & 0x10) as i32, 2),
            26 | 27 => ((x.f50 & 0x20) as i32, 2),
            28 | 29 => (b(!atk || x.recovery > 0), 2),
            30 | 31 => (b(x.form & 1 == 0 && x.gauge >= 0xF00), 2),
            32 | 33 => ((x.f5c & mask) as i32, 6),
            34 | 35 => ((x.f54 & mask) as i32, 6),
            36 | 37 => ((x.f50 & mask) as i32, 6),
            38 | 39 => ((x.f60 & mask) as i32, 6),
            40 => (b((c.me.f.opp_angle as i32).abs() >= 0x6AB), 2),
            _ => ((x.f50 & 0x8000) as i32, 2),
        };
        self.res = v;
        self.pc = s + size;
    }

    /// Wait; false while waiting (and for the counted waits also when done). Subs ≥ 11 are sub 0.
    fn wait(&mut self, c: &Ctx, sub: u8) -> bool {
        let s = self.pc;
        let (me, o) = (&c.me.f, c.opp);
        let atk = |x: &Block| (x.f50 as i32) < 0;
        let go = |a: &mut Self| {
            a.pc = s + 2;
            true
        };
        match sub {
            1 => {
                if self.regs[0] == 0 {
                    self.regs[0] = self.reaction(c);
                }
                self.regs[0] = self.regs[0].wrapping_sub(1);
                if self.regs[0] > 0 {
                    false
                } else {
                    self.pc = s + 2;
                    self.regs[0] = 0;
                    true
                }
            }
            2 | 3 => {
                let x = if sub == 2 { me } else { o };
                if !atk(x) || (x.startup == 0 && x.active != 0) { go(self) } else { false }
            }
            4 | 5 => {
                let x = if sub == 4 { me } else { o };
                if !atk(x) || x.recovery > 0 { go(self) } else { false }
            }
            6 | 7 => {
                let x = if sub == 6 { me } else { o };
                if !atk(x) { go(self) } else { false }
            }
            9 | 10 => {
                let x = if sub == 9 { me } else { o };
                if !atk(x) || x.f64 & 0x100_0000 == 0 { go(self) } else { false }
            }
            _ => {
                // 0 (and ≥ 11) timed, 8 random
                if self.regs[0] == 0 {
                    self.regs[0] = if sub == 8 { ((c.u16(s + 2) as i32 * self.r(c) as i32) >> 8) as i16 + 1 } else { c.u16(s + 2) as i16 };
                }
                self.regs[0] = self.regs[0].wrapping_sub(1);
                if self.regs[0] <= 0 {
                    self.pc = s + 4;
                    self.regs[0] = 0;
                }
                false
            }
        }
    }

    /// GPad: hold a pad for byte1 frames, but let go of forward / guard low or high when the opponent
    /// attacks (REACT's chance per attack), and go to guarding (state 3) when hit or in guard stun.
    fn guarded_pad(&mut self, c: &Ctx, sub: u8) {
        let s = self.pc;
        let (f, o) = (&c.me.f, c.opp);
        if f.f50 & 0x1000_0000 != 0 || f.f54 & 0x200_0000 != 0 {
            self.guard_push(c);
            self.state = 3;
            return;
        }
        if self.regs[0] == 0 {
            self.regs[0] = sub as i16;
            self.flags &= 0xFFFD;
        }
        self.pad = c.u16(s + 2);
        if (o.f50 as i32) < 0 && o.step >= o.startup && o.startup >= 0 && self.r(c) < self.row(c, REACT) {
            self.flags |= 2;
        }
        if self.flags & 2 != 0 {
            if o.active == 0 {
                self.flags &= 0xFFFD;
            } else {
                let a0 = self.pad;
                self.pad = a0 & 0xDFFF;
                if o.f5c & 0x80 != 0 {
                    self.pad = a0 & 0x9FFF;
                } else if o.f5c & 0x40 != 0 {
                    self.pad |= 0x4000;
                }
            }
        }
        self.regs[0] = self.regs[0].wrapping_sub(1);
        if self.regs[0] == 0 {
            self.pc = s + 4;
        }
    }
}
