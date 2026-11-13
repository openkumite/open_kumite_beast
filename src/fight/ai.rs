//! The CPU opponent: the AI driver and its action VM.
//!
//! State: serialised: part of `GameState` (field changes are `STATE_VERSION` changes)
//!
//! CPU opponent: the AI routine (a state machine that reacts to the opponent and
//! picks an "action") and the action VM that runs the action scripts of the fighter's ANI
//! (ANI header word 5). The routine writes a logical pad word (after the facing swap) that the input
//! code uses instead of the controller.
//!
//! Fields are named after the AI block (fighter +0x3C) offsets they come from.

use crate::fight::fighter::{F5C, F6C, F58, F60, F64, F68, Fighter};
use crate::formats::act_ani::Ani;
use serde::{Deserialize, Serialize};

/// Offsets of the CPU level tables inside `Trig::cpu` (the static tables' CPU block).
mod tbl {
    pub const WAIT: usize = 0x00; // (min, max) idle frames per level
    pub const REACT: usize = 0x40; // ai +0x1C
    pub const GUARD: usize = 0x50; // ai +0x20
    pub const RECOVER: usize = 0x60;
    pub const DASH: usize = 0x70;
    pub const DASH_B: usize = 0x80;
    pub const SIDE: usize = 0x90;
}

/// The shared random table: a copy of the game's 256 random bytes, shuffled.
pub fn shuffle(r: &mut [u8; 256], src: &[u8], seed: u32) {
    for i in 0..256usize {
        let j = src[(seed as usize + i) & 0xFF] as usize;
        r.swap(i, j);
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Ai {
    #[serde(with = "crate::base::serde_util::big_array")]
    pub rnd: [u8; 256], // +0 (pointer to the shared table)
    pub idx: u8,         // +4
    pub pad: u16,        // +6 output
    pub level: u16,      // +8 (state +0xA0)
    pub flags: u16,      // +0xA bit 0 → pad 0x800, bit 1 guard in GPad, bit 2 wake-up kick
    tab5: usize,         // +0xC
    dist: usize,         // +0x14 distance bands (u16, ascending)
    recs: usize,         // +0x18 actions: u32 opponent mask, u16 script per band ×4
    pub pc: usize,       // +0x24
    pub state: u16,      // +0x28
    pub t2a: i16,        // +0x2A
    pub t2c: i16,        // +0x2C
    pub action: i32,     // +0x30
    pub band: i32,       // +0x34
    pub res: i32,        // +0x38 result of Chk / Cmp
    pub regs: [i16; 16], // +0x3C (r0 doubles as the wait counter)
    stack: [usize; 8],   // +0x4C
    depth: u16,          // +0x6C
}

fn u32at(b: &[u8], o: usize) -> u32 {
    b.get(o..o + 4).map(|s| u32::from_le_bytes([s[0], s[1], s[2], s[3]])).unwrap_or(0)
}

impl Ai {
    /// The AI block's init (mode = (fighter +1 & 0x7F) − 1, or 0). Modes without a case (the demo's 0x10 / 0x11 among
    /// them) keep state 0, the normal AI: the routine never reads the mode again.
    pub fn new(ani: &Ani, rnd: &[u8; 256], player: u8, level: u16, mode: i32) -> Self {
        Self::new_at(ani, rnd, player.wrapping_mul(128), level, mode)
    }

    /// `new` with the random table's start index `idx` (P1 0, P2 128; the team battle's P3 / P4, bank 1: 64 / 192).
    pub fn new_at(ani: &Ani, rnd: &[u8; 256], idx: u8, level: u16, mode: i32) -> Self {
        let b = &ani.bytes;
        let tab5 = u32at(b, 20) as usize;
        let mut ai = Self {
            rnd: *rnd,
            idx,
            pad: 0,
            level,
            flags: 0,
            tab5,
            dist: u32at(b, tab5 + 4) as usize,
            recs: u32at(b, tab5 + 8) as usize,
            pc: 0,
            state: 0,
            t2a: 0,
            t2c: 0,
            action: 0,
            band: 0,
            res: 0,
            regs: [0; 16],
            stack: [0; 8],
            depth: 0,
        };
        match mode {
            0 => ai.state = 0,
            1 | 3 => ai.state = 9,
            2 => {
                ai.state = 8;
                ai.pc = u32at(b, tab5 + 0x14) as usize;
            }
            m if m & 0x40 != 0 => ai.state = m as u16,
            _ => {}
        }
        ai
    }

    /// A decoded state's AI block is the one `new` built for `ani`: its table offsets, a script position and return
    /// stack within the ANI bytes, the stack at most 8 deep.
    pub fn valid(&self, ani: &Ani) -> bool {
        let b = &ani.bytes;
        let at = |p: usize| p <= b.len();
        self.tab5 == u32at(b, 20) as usize
            && self.dist == u32at(b, self.tab5 + 4) as usize
            && self.recs == u32at(b, self.tab5 + 8) as usize
            && at(self.pc)
            && (self.depth as usize) <= self.stack.len()
            && self.stack.iter().all(|&p| at(p))
    }

    /// The PRACTICE CPU. Action = state & 7 (0 standing, 1 blocks after taking
    /// damage, 2 crouching, 3 jumping, 4 dodging), state bit 3 = COM. GUARD, bit 4 = GUARD AND ESCAPE; bit 5
    /// (DEFENSIVE FALL) is read by nothing. Random draws only where the game draws them.
    fn practice(&mut self, f: &Fighter, opp: &Fighter) {
        let (f58, f5c, o58, o64) = (f.flags[F58], f.flags[F5C], opp.flags[F58], opp.flags[crate::fight::fighter::F64]);
        let attacking = (o58 as i32) < 0;

        let sv = match (self.state & 0x47) as i32 - 0x40 {
            4 => {
                // DODGING: GUARD and ESCAPE forced on; 1/4 per frame to jump over a slow low attack
                self.state |= 0x18;
                if attacking && opp.o_76 < 0x400 {
                    let q = if opp.step != 0 {
                        opp.o_ba as i32 / opp.step as i32
                    } else if opp.o_ba >= 0 {
                        -1
                    } else {
                        1
                    };
                    if q >= 12 && opp.o_bc <= 0 && o64 & 0x40 != 0 && self.r() & 3 == 0 {
                        self.pad = 0x1000;
                        return;
                    }
                }
                if f5c & 0x4000_0000 != 0 && f.o_cc < 0x61 && self.r() < 0x15 {
                    self.pad = 0x2000;
                    return;
                }
                false
            }
            a @ 0..=3 => {
                match a {
                    1 => {
                        // BLOCKS AFTER TAKING DAMAGE: guard on once stunned, off 4 frames after the attack
                        if attacking {
                            self.t2a = 4;
                            if f5c & 0x6000_0000 != 0 {
                                self.state |= 8;
                            }
                        } else if self.t2a > 0 {
                            self.t2a -= 1;
                        } else {
                            self.state &= 0xFFF7;
                        }
                    }
                    2 => self.pad = 0x4000, // CROUCHING
                    3 => self.pad = 0x1000, // JUMPING
                    _ => {}                 // STANDING
                }
                // GUARD AND ESCAPE: Forward in the first 6 frames of guard stun, 21/256 per frame
                if self.state & 0x10 != 0 && f5c & 0x4000_0000 != 0 && f.o_cc < 0x61 && self.r() < 0x15 {
                    self.pad = 0x2000;
                    return;
                }
                if self.state & 8 == 0 {
                    self.flags &= 0xFFFE;
                    return;
                }
                true
            }
            _ => return, // actions 5..7: no input
        };
        // GUARD
        self.flags |= 1;
        if f.o_74.unsigned_abs() >= 0x201 {
            self.pad = 0xC000; // not facing the opponent
            return;
        }
        if !attacking {
            return;
        }
        if f5c & 0x0200_0000 != 0 {
            if self.pad == 0 && f58 & 0x10 != 0 {
                self.pad = 0x4000;
            }
            return;
        }
        if f5c & 0x4000_0000 != 0 {
            if f58 & 0x10 != 0 {
                self.pad = 0x4000;
            }
            return;
        }
        if opp.o_76 >= 0x400 {
            return;
        }
        let low = o64 & 0x40 != 0 || (sv && self.state & 0x47 == 0x42) || (o64 & 0x180 == 0x100 && (!sv || f58 & 0x10 != 0));
        let crush = o64 & 0x2_0000 != 0;
        self.pad = match (low, crush) {
            (true, true) => 0x4400,
            (true, false) => 0x4000,
            (false, true) => 0x0400,
            (false, false) => 0,
        };
    }

    fn r(&mut self) -> u32 {
        let v = self.rnd[self.idx as usize];
        self.idx = self.idx.wrapping_add(1);
        v as u32
    }

    /// Reaction delay: (0x100 − level) × (three random bytes) / 0x2000.
    fn reaction(&mut self) -> i16 {
        let s = (self.r() + self.r() + self.r()) as i32;
        let v = (0x100 - self.level as i32) * s;
        ((if v < 0 { v + 0x1FFF } else { v }) >> 13) as i16
    }

    fn crouch_keep(&mut self, f: &Fighter) {
        if f.flags[F58] & 0x10 != 0 {
            self.pad = 0x4000;
        }
    }

    /// One frame; returns the logical pad.
    pub fn think(&mut self, f: &Fighter, opp: &Fighter, ani: &Ani, trig: &crate::fight::physics::Trig, timer: i16) -> u16 {
        let cpu = &trig.cpu[..];
        #[derive(Clone, Copy)]
        enum L {
            S0,
            S1,
            Opp,
            Matched,
            S2,
            S3Enter,
            S3,
            Choose,
            S4,
            S5,
            S6,
            S6Roll,
            S6Step,
            S7,
            Vm,
            End,
        }
        let t1 = (self.level >> 4) as usize & 15;
        let cpu_t = |o: usize| cpu.get(o + t1).copied().unwrap_or(0) as u32;
        let b = &ani.bytes;
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
            8 => {
                if f.flags[F60] & 0x8000 != 0 {
                    L::End
                } else {
                    L::Vm
                }
            }
            9 => L::End, // "Demo": no input
            _ => {
                // states >= 10: the PRACTICE dummy, 0x40 | M.D4
                self.practice(f, opp);
                L::End
            }
        };
        let (f58, o58) = (f.flags[F58], opp.flags[F58]);
        loop {
            lab = match lab {
                L::S0 => {
                    let lo = u16::from_le_bytes([cpu[tbl::WAIT + 4 * t1], cpu[tbl::WAIT + 4 * t1 + 1]]) as i32;
                    let hi = u16::from_le_bytes([cpu[tbl::WAIT + 4 * t1 + 2], cpu[tbl::WAIT + 4 * t1 + 3]]) as i32;
                    let v = (hi - lo + 1) * self.r() as i32;
                    self.action = -1;
                    self.state = 1;
                    self.flags |= 1;
                    self.t2a = (lo + ((if v < 0 { v + 0xFF } else { v }) >> 8)) as i16;
                    self.t2c = self.t2a / 2;
                    L::S1
                }
                L::S1 => {
                    if f58 & 0xC000_0000 != 0 {
                        L::End
                    } else if f58 & 0x1000_0000 != 0 {
                        // being hit / guarding: maybe answer the opponent's next attack
                        if (o58 as i32) < 0 && opp.o_ba >= 0 && opp.o_ba <= opp.step && self.r() < cpu_t(tbl::REACT) {
                            L::S3Enter
                        } else {
                            self.crouch_keep(f);
                            L::End
                        }
                    } else if f58 & 0x8000 != 0 {
                        self.action = 0;
                        L::Choose
                    } else {
                        let a = (f.o_74 as i32).abs();
                        if a >= 0x6AB {
                            self.action = 1;
                            L::Choose
                        } else if a >= 0x201 {
                            self.action = 2;
                            L::Choose
                        } else {
                            let expired = if self.t2a != 0 {
                                self.t2a -= 1;
                                self.t2a == 0
                            } else {
                                true
                            };
                            if !expired {
                                L::Opp
                            } else if f.flags[F60] & 0x8000 != 0 {
                                L::End
                            } else {
                                self.action = 3;
                                L::Choose
                            }
                        }
                    }
                }
                L::Opp => {
                    if (o58 as i32) >= 0 {
                        // react to the opponent's state: first action from 6 whose mask matches
                        self.action = 6;
                        let mut found = false;
                        loop {
                            let mask = u32at(b, self.recs + 12 * self.action as usize);
                            if mask == 0 {
                                break;
                            }
                            if o58 & mask == mask {
                                found = true;
                                break;
                            }
                            self.action += 1;
                        }
                        if found {
                            L::Matched
                        } else {
                            self.crouch_keep(f);
                            self.action = -1;
                            L::End
                        }
                    } else if opp.o_bc > 0 {
                        self.action = 5;
                        L::Choose
                    } else if self.r() < cpu_t(tbl::REACT) {
                        if self.r() < cpu_t(tbl::GUARD) {
                            L::S3Enter
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
                        self.crouch_keep(f);
                        self.action = -1;
                        self.state = 1;
                        L::End
                    } else {
                        L::Choose
                    }
                }
                L::S2 => {
                    // watching the opponent's attack
                    if o58 & 0x8000 != 0 || opp.o_b8 == 0 {
                        self.state = 0;
                    }
                    if f58 & 0x800 != 0 {
                        self.flags |= 1;
                        L::End
                    } else {
                        self.flags &= !1;
                        if opp.o_76 >= 0x401 || opp.flags[F64] & 0x80 == 0 || opp.step == 0 {
                            L::End
                        } else {
                            let q = opp.o_ba as i32 / opp.step as i32;
                            if q == 5 {
                                if self.r() < 0x40 {
                                    self.pad = 0x4000;
                                }
                            } else if q < 5 {
                                self.crouch_keep(f);
                            }
                            L::End
                        }
                    }
                }
                L::S3Enter => {
                    self.state = 3;
                    L::S3
                }
                L::S3 => {
                    // guard / evade
                    let v1 = f.flags[F5C];
                    let check_crouch = if v1 & 0x200_0000 != 0 {
                        if f.o_cc < 0x61 {
                            let t = if v1 & 0x80 != 0 { tbl::DASH_B } else { tbl::DASH };
                            if self.r() < cpu_t(t) {
                                self.pad = 0x2000;
                            }
                        }
                        true
                    } else if v1 & 0x4000_0000 != 0 {
                        true
                    } else {
                        let a = opp.flags[F64];
                        self.pad = if opp.o_76 < 0x400 && a & 0x40 != 0 {
                            if a & 0x2_0000 != 0 { 0x4400 } else { 0x4000 }
                        } else if a & 0x2_0000 != 0 {
                            0x400
                        } else {
                            0
                        };
                        false
                    };
                    if check_crouch && self.pad == 0 {
                        self.crouch_keep(f);
                    }
                    if opp.o_b8 == 0 {
                        self.state = 0;
                    }
                    L::End
                }
                L::Choose => {
                    // distance band, then the action's script for that band
                    self.band = 0;
                    // past the end of the ANI bytes the band reads 0xFFFF, which ends the scan
                    let band = |k: i32| b.get(self.dist + 2 * k as usize..).and_then(|s| s.get(..2)).map_or(0xFFFF, |s| u16::from_le_bytes([s[0], s[1]]));
                    while (band(self.band) as i32) < f.o_76 as i32 {
                        self.band += 1;
                    }
                    let o = self.recs + 12 * self.action.max(0) as usize + 4 + 2 * self.band as usize;
                    self.pc = ani.u16(o) as usize;
                    self.t2a = self.reaction();
                    self.depth = 0;
                    self.state = 4;
                    L::S4
                }
                L::S4 => {
                    if f58 & 0x8000 != 0 && f58 & 0x3_0000 != 0 {
                        self.state = 5;
                        if self.r() < 0x20 {
                            self.flags |= 4;
                        } else {
                            self.flags &= !4;
                        }
                        L::S5
                    } else if f.o_d2 != 0 {
                        self.state = 6;
                        self.t2a = 0;
                        L::End
                    } else if self.t2a != 0 {
                        self.t2a -= 1;
                        L::End
                    } else {
                        L::Vm
                    }
                }
                L::S5 => {
                    // knocked down: wake-up choice
                    if f58 & 0x200_0000 != 0 && f.flags[F68] & 0x80_0000 != 0 {
                        self.state = 7;
                        self.t2a = 0;
                        L::End
                    } else if f58 & 0x3_0000 == 0 {
                        self.action = 4;
                        L::Choose
                    } else if f58 & 0x4_0000 != 0 || f.o_c0 < 0 {
                        L::End
                    } else if f.o_c0 == 0 {
                        let r = self.r();
                        self.pad = match r {
                            0..=0x2F => 0x2000,
                            0x30..=0x5F => 0x8000,
                            0x60..=0x8F => 0x1000,
                            0x90..=0xBF => 0x4000,
                            _ => 0,
                        };
                        L::End
                    } else {
                        let r = self.r();
                        if (r as i32) < (self.level as i32 + 0x20) >> 1 {
                            self.pad = if self.flags & 4 != 0 {
                                0x40
                            } else if r & 1 == 0 {
                                0x4000
                            } else {
                                0x8000
                            };
                        }
                        L::End
                    }
                }
                L::S6 => {
                    // side step / stagger
                    if f.o_d2 == 0 {
                        self.pad = 0x400;
                        self.state = 0;
                        L::End
                    } else {
                        match self.t2a {
                            0 => {
                                self.t2a = if self.r() < 0x40 { -1 } else { 1 };
                                L::S6Roll
                            }
                            1 => L::S6Roll,
                            -1 => L::S6Step,
                            _ => L::End,
                        }
                    }
                }
                L::S6Roll => {
                    if cpu_t(tbl::SIDE) < self.r() {
                        L::End
                    } else {
                        L::S6Step
                    }
                }
                L::S6Step => {
                    self.pad = if f.o_d2 & 0x10 != 0 { 0x8400 } else { 0x4400 };
                    L::End
                }
                L::S7 => {
                    if f58 & 0x200_0000 == 0 {
                        self.state = 0;
                        L::End
                    } else {
                        if f.flags[F5C] & 0x400_0000 != 0 {
                            self.t2a = 0;
                        }
                        if self.t2a != 0 || f.o_86 < f.o_cc {
                            L::End
                        } else {
                            if self.r() < cpu_t(tbl::RECOVER) && ((o58 as i32) >= 0 || (opp.o_86 as i32) * 5 < opp.o_ba as i32) {
                                self.pad = trig.recover_pads[(self.r() & 7) as usize];
                                self.state = 0;
                            }
                            self.t2a = -1;
                            L::End
                        }
                    }
                }
                L::Vm => {
                    self.run(f, opp, ani, cpu, timer);
                    L::End
                }
                L::End => break,
            };
        }
        if self.pad & 0x400 != 0 {
            self.pad |= 8;
        }
        if self.flags & 1 != 0 {
            self.pad |= 0x800;
        }
        self.pad
    }

    /// Up to 50 ops until one yields.
    fn run(&mut self, f: &Fighter, opp: &Fighter, ani: &Ani, cpu: &[u8], timer: i16) {
        for _ in 0..50 {
            if !self.op(f, opp, ani, cpu, timer) {
                break;
            }
        }
    }

    fn hw(&self, ani: &Ani, o: usize) -> u16 {
        if self.pc + o + 2 <= ani.bytes.len() { ani.u16(self.pc + o) } else { 0 }
    }

    /// Weighted pick of op 1/2 sub 10: returns (target, next pc).
    fn weighted(&mut self, ani: &Ani) -> (u16, usize) {
        let s = self.pc;
        let n = ani.u8(s + 2) as usize;
        let w: Vec<u32> = (0..n).map(|k| ani.u8(s + 3 + k) as u32).collect();
        let total: u32 = w.iter().sum();
        let mut a = ((self.r() * total) >> 8) as i32 - w.first().copied().unwrap_or(0) as i32;
        let mut k = 0;
        while a >= 0 && k + 1 < n {
            k += 1;
            a -= w[k] as i32;
        }
        let t = ani.u16(s + ((n + 4) & 0x1FE) + 2 * k);
        (t, s + ((3 * n + 4) & 0x7FE))
    }

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

    /// Value sources of Move (sub of src 15) and Add.
    fn source(&self, f: &Fighter, timer: i16, k: u8) -> i16 {
        match k {
            1 => f.o_74,
            2 => f.o_76,
            3 => self.level as i16,
            4 => timer & 0x7FFF,
            _ => 0,
        }
    }

    /// One op; false = stop for this frame.
    fn op(&mut self, f: &Fighter, opp: &Fighter, ani: &Ani, cpu: &[u8], timer: i16) -> bool {
        let s = self.pc;
        if s + 2 > ani.bytes.len() {
            self.state = 0;
            return false;
        }
        let op = ani.u8(s);
        let sub = ani.u8(s + 1);
        let hp = |x: &Fighter| x.o_b0.wrapping_sub(x.o_ac);
        match op {
            1 | 2 => {
                // Bra / Bsr
                let push = op == 2;
                let (target, next) = match sub {
                    0 => (self.hw(ani, 2), s + 4),
                    1 => {
                        let t = if self.res > 0 {
                            self.hw(ani, 2)
                        } else {
                            self.res = 0;
                            self.hw(ani, 6)
                        };
                        (t, s + 8)
                    }
                    10 => self.weighted(ani),
                    11 => (if self.res != 0 { self.hw(ani, 2) } else { self.hw(ani, 4) }, s + 6),
                    2..=9 => (if self.cond(sub) { self.hw(ani, 2) } else { 0 }, s + 4),
                    _ => (0, s + 4),
                };
                if target == 0 {
                    self.pc = next;
                } else if push {
                    if (self.depth as usize) < 8 {
                        self.stack[self.depth as usize] = next;
                        self.depth += 1;
                        self.pc = target as usize;
                    } else {
                        self.pc = next;
                    }
                } else {
                    self.pc = target as usize;
                }
                true
            }
            3 => {
                // Ret
                if sub <= 1 || self.cond(sub) {
                    if self.depth == 0 {
                        self.state = 0;
                        return false;
                    }
                    self.depth -= 1;
                    self.pc = self.stack[self.depth as usize];
                } else {
                    self.pc = s + 2;
                }
                true
            }
            4 => {
                self.check(f, opp, ani, sub);
                true
            }
            5 => {
                // Move
                let (src, dst) = (sub & 15, (sub >> 4) as usize);
                let v = match src {
                    7 => {
                        self.pc = s + 4;
                        self.hw(ani, 2) as i16
                    }
                    1..=5 => {
                        self.pc = s + 2;
                        self.regs[src as usize]
                    }
                    15 => {
                        let k = ani.u8(s + 2);
                        if (1..=12).contains(&k) {
                            self.pc = s + 4;
                        }
                        let pct = |a: i16, b: i16, d: i16| if d != 0 { ((a as i32 - b as i32) * 100 / d as i32) as i16 } else { 0 };
                        match k {
                            1..=4 => self.source(f, timer, k),
                            5 => f.gauge / 64,
                            6 => opp.gauge / 64,
                            7 => pct(f.o_b0, f.o_ac, f.o_b0),
                            8 => pct(opp.o_b0, opp.o_ac, f.o_b0),
                            9 => (f.hips[1] - f.o_94 as i32) as i16,
                            10 => (opp.hips[1] - opp.o_94 as i32) as i16,
                            11 => pct(f.o_ac, f.o_ae, f.o_b0),
                            12 => pct(opp.o_ac, opp.o_ae, f.o_b0),
                            _ => k as i16,
                        }
                    }
                    _ => src as i16,
                };
                self.regs[dst] = v;
                true
            }
            6 => {
                // Cmp: lhs (high nibble) − rhs (low nibble)
                let (l, rr) = ((sub >> 4) as usize, sub & 15);
                let mut a3 = s;
                let rhs = match rr {
                    1..=5 => self.regs[rr as usize] as i32,
                    12 => opp.gauge as i32,
                    14 => hp(opp) as i32,
                    15 => {
                        a3 += 2;
                        ani.i16(s + 2) as i32
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
                        ani.i16(a3 + 2) as i32
                    }
                    _ => l as i32,
                };
                self.res = lhs - rhs;
                true
            }
            7 => {
                // Add
                let (src, dst) = (sub & 15, (sub >> 4) as usize);
                let v = match src {
                    1..=5 => self.regs[src as usize],
                    7..=10 => self.source(f, timer, src - 6),
                    11 => f.gauge,
                    12 => opp.gauge,
                    13 => hp(f),
                    14 => hp(opp),
                    15 => ani.i16(s + 2),
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
                        ani.i16(s + 2)
                    }
                    _ => src as i16,
                };
                let d = self.regs[dst];
                self.regs[dst] = match op {
                    8 => d.wrapping_sub(v),
                    9 => d.wrapping_mul(v),
                    _ => {
                        if v != 0 {
                            d.wrapping_div(v)
                        } else {
                            d
                        }
                    }
                };
                true
            }
            11 => self.wait(f, opp, ani, sub),
            12 => {
                // SFlg
                let v = self.hw(ani, 2);
                if sub == 1 {
                    self.flags &= v
                } else {
                    self.flags |= v
                }
                self.pc = s + 4;
                true
            }
            16 => {
                // Pad: hold hw2 for byte1 frames
                if self.regs[0] == 0 {
                    self.regs[0] = sub as i16;
                }
                self.regs[0] -= 1;
                self.pad = self.hw(ani, 2);
                if self.regs[0] == 0 {
                    self.pc = s + 4;
                }
                false
            }
            17 => {
                self.guarded_pad(f, opp, ani, cpu, sub);
                false
            }
            18 => {
                // camera command of demo scripts (same interpreter as move-script op 0x2E; not applied): size only
                self.pc = s + match sub as i8 {
                    0 | 5..=9 => 4,
                    1..=4 => 8,
                    _ => 2,
                };
                true
            }
            _ => {
                // End (0, 13–15)
                self.state = 0;
                false
            }
        }
    }

    /// Op 4 (Chk): tests of the fighter (even subs) or the opponent (odd subs) → `res`.
    fn check(&mut self, f: &Fighter, opp: &Fighter, ani: &Ani, sub: u8) {
        let s = self.pc;
        let x = if sub & 1 == 0 { f } else { opp };
        let atk = (x.flags[F58] as i32) < 0;
        let mask = || ani.u16(s + 2) as u32 | (ani.u16(s + 4) as u32) << 16;
        let b = |c: bool| c as i32;
        let (v, size) = match sub {
            0 | 1 => ((x.flags[F58] & 0x8000) as i32, 2),
            2 | 3 => ((x.flags[F58] & 0x1000_0000) as i32, 2),
            4 | 5 => ((x.flags[F58] & 0x800) as i32, 2),
            6 | 7 => (b(x.flags[F58] & 0x800 == 0), 2),
            8 | 9 => ((x.flags[F58] & 0x8000_0000) as i32, 2),
            10 | 11 => (b(atk && x.o_ba != 0), 2),
            12 | 13 => (b(atk && x.o_ba == 0 && x.o_b8 != 0), 2),
            14 | 15 => (b(atk && x.o_bc > 0), 2),
            16 | 17 => ((x.form & 1) as i32, 2),
            18 | 19 => (b(atk && x.flags[F6C] & 0x100_0000 != 0), 2),
            20 | 21 => (b(atk && x.flags[F6C] & 0x200_0000 != 0), 2),
            22 | 23 => ((x.flags[F58] & 1) as i32, 2),
            24 | 25 => ((x.flags[F58] & 0x10) as i32, 2),
            26 | 27 => ((x.flags[F58] & 0x20) as i32, 2),
            28 | 29 => (b(!atk || x.o_bc > 0), 2),
            30 | 31 => (b(x.form & 1 == 0 && x.gauge >= 0x1E00), 2),
            32 | 33 => ((x.flags[F64] & mask()) as i32, 6),
            34 | 35 => ((x.flags[F5C] & mask()) as i32, 6),
            36 | 37 => ((x.flags[F58] & mask()) as i32, 6),
            38 | 39 => ((x.flags[F68] & mask()) as i32, 6),
            40 => (b((f.o_74 as i32).abs() >= 0x6AB), 2),
            41 => ((f.flags[F6C] & mask()) as i32, 6),
            42 => ((opp.flags[F6C] & mask()) as i32, 6),
            43 | 44 => {
                let y = if sub == 43 { f } else { opp };
                let ok = (y.flags[F58] as i32) < 0 && y.o_ba > 0 && y.flags[F5C] & 0x4_0080 == 0 && y.flags[F64] & 0x4000_2000 == 0x4000_2000;
                (b(ok), 2)
            }
            _ => return,
        };
        self.res = v;
        self.pc = s + size;
    }

    /// Op 11: waits. Returns false while waiting (and for the plain timed waits also when done).
    fn wait(&mut self, f: &Fighter, opp: &Fighter, ani: &Ani, sub: u8) -> bool {
        let s = self.pc;
        let atk = |x: &Fighter| (x.flags[F58] as i32) < 0;
        let cont = |me: &mut Self| {
            me.pc = s + 2;
            true
        };
        match sub {
            0 | 8 => {
                if self.regs[0] == 0 {
                    self.regs[0] = if sub == 0 { self.hw(ani, 2) as i16 } else { ((self.hw(ani, 2) as u32 * self.r()) >> 8) as i16 + 1 };
                }
                self.regs[0] -= 1;
                if self.regs[0] <= 0 {
                    self.pc = s + 4;
                    self.regs[0] = 0;
                }
                false
            }
            1 | 11 => {
                if self.regs[0] == 0 {
                    self.regs[0] = self.reaction();
                }
                self.regs[0] = self.regs[0].wrapping_sub(1);
                if self.regs[0] > 0 {
                    if sub == 11 {
                        self.pad = self.hw(ani, 2);
                    }
                    false
                } else {
                    self.pc = s + if sub == 1 { 2 } else { 4 };
                    self.regs[0] = 0;
                    true
                }
            }
            2 | 3 => {
                let x = if sub == 2 { f } else { opp };
                if !atk(x) || (x.o_ba == 0 && x.o_b8 != 0) { cont(self) } else { false }
            }
            4 | 5 => {
                let x = if sub == 4 { f } else { opp };
                if !atk(x) || x.o_bc > 0 { cont(self) } else { false }
            }
            6 | 7 => {
                let x = if sub == 6 { f } else { opp };
                if !atk(x) { cont(self) } else { false }
            }
            9 | 10 => {
                let x = if sub == 9 { f } else { opp };
                if !atk(x) || x.flags[F6C] & 0x100_0000 == 0 { cont(self) } else { false }
            }
            _ => false,
        }
    }

    /// Op 17: hold a pad for byte1 frames, but let go of forward / crouch when the
    /// opponent attacks (guard), and leave the action when hit.
    fn guarded_pad(&mut self, f: &Fighter, opp: &Fighter, ani: &Ani, cpu: &[u8], sub: u8) {
        let s = self.pc;
        let lv = (self.level >> 4) as u32 & 15;
        if f.flags[F58] & 0x1000_0000 != 0 || f.flags[F5C] & 0x200_0000 != 0 {
            if f.flags[F5C] & 0x200_0080 == 0x200_0000 && f.o_cc < 0x61 && self.r() < (self.level >> 3) as u32 {
                self.pad = 0x2000;
            }
            self.state = 3;
            return;
        }
        if self.regs[0] == 0 {
            self.regs[0] = sub as i16;
            self.flags &= !2;
        }
        self.pad = self.hw(ani, 2);
        if (opp.flags[F58] as i32) < 0 && opp.o_ba <= opp.step && opp.o_ba >= 0 {
            let t = cpu.get(tbl::REACT + lv as usize).copied().unwrap_or(0);
            if self.r() < t as u32 {
                self.flags |= 2;
            }
        }
        if self.flags & 2 != 0 {
            if opp.o_b8 == 0 {
                self.flags &= !2;
            } else {
                let a0 = self.pad;
                self.pad = a0 & 0xDFFF;
                let a = opp.flags[F64];
                if a & 0x80 != 0 {
                    self.pad = a0 & 0x9FFF;
                } else if a & 0x40 != 0 {
                    self.pad |= 0x4000;
                }
            }
        }
        self.regs[0] -= 1;
        if self.regs[0] == 0 {
            self.pc = s + 4;
        }
    }
}
