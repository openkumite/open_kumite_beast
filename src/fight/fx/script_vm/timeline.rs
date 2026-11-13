//! op0E.00, the timeline sequencer: keyed frames that run subroutines, loops and jumps.
//!
//! State: none of its own: steps the serialised `Vm`

use super::{Exit, Vm};

impl Vm {
    /// op0E.00 timeline sequencer.
    pub(super) fn timeline(&mut self, h: u32, base: u32, o: u32, pc: u32, f: i32, depth: u32) -> Option<Exit> {
        let at = |off: i32| (o as i32).wrapping_add(off) as u32;
        let t = at(f);
        let evf = at(self.mem.s16(pc + 4) as i32);
        let tab = self.mem.u32(at(self.mem.s16(pc + 6) as i32));
        let mut e = self.mem.u32(evf);
        for _ in 0..256 {
            let key = self.mem.s16(e) as i32;
            let n = self.mem.s16(e + 2) as i32;
            let frame = self.mem.s16(t) as i32;
            let lp = self.mem.s16(t + 2) as i32;
            match key {
                0x1000 => {
                    if lp < n {
                        e = e.wrapping_sub(6);
                        self.mem.w16(t, self.mem.u16(e));
                        self.mem.w16(t + 2, (lp + 1) as u16);
                    } else {
                        e += 4;
                        self.mem.w16(t + 2, 0);
                    }
                }
                0x2000 => {
                    if lp < n {
                        e = (base as i32 + self.mem.s16(e + 4) as i32) as u32;
                        self.mem.w16(t, self.mem.u16(e));
                        self.mem.w16(t + 2, (lp + 1) as u16);
                    } else {
                        e += 6;
                        self.mem.w16(t + 2, 0);
                    }
                }
                0x3000 => {
                    e = (base as i32 + n) as u32;
                    self.mem.w16(t, self.mem.u16(e));
                    self.mem.w16(t + 2, 0);
                }
                0x4000 => {
                    self.mem.w32(evf, e);
                    self.set_cond(h, 0);
                    return None;
                }
                _ if frame == key => {
                    let idx = self.mem.s16(e + 4) as i32;
                    let code = (base as i32 + self.mem.s16((tab as i32 + 2 * idx) as u32) as i32) as u32;
                    for _ in 0..n.max(0) {
                        self.set_pc(h, code);
                        if self.run(h, depth + 1) == Exit::Destroy && !self.alive(h) {
                            self.mem.w32(evf, e + 6);
                            return Some(Exit::Destroy);
                        }
                    }
                    e += 6;
                }
                _ => break,
            }
        }
        self.mem.w32(evf, e);
        self.mem.w16(t, (self.mem.s16(t) + 1) as u16);
        let next = self.mem.s16(e) as i32;
        self.set_cond(h, next);
        None
    }
}
