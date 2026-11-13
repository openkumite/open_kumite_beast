//! The effect VM's animator steps: UV and colour animators.
//!
//! State: none of its own: steps the serialised `Vm`
//!
//! The animator steps: op05.0C's UV animation and the UV animator and colour animator tasks the scheduler runs
//! (`Vm::tick`).

use super::{Dtor, Kind, Task, Vm};

impl Vm {
    /// op05.0C UV animation step. Returns COND.
    pub(super) fn uv_step(&mut self, s: u32) -> i32 {
        let t = self.mem.s16(s + 0x60) - 1;
        self.mem.w16(s + 0x60, t as u16);
        if t > 0 {
            return 1;
        }
        let seq = self.mem.u32(s + 0x5C);
        let mut cur = self.mem.u32(s + 0x58).wrapping_add(4);
        for _ in 0..64 {
            let (a, b) = (self.mem.u16(cur), self.mem.u16(cur + 2));
            match a & 0xFF00 {
                0 => {
                    let rect = self.mem.u32(s + 0x54) + 4 * a as u32;
                    self.set_uv_rect(s, rect);
                    self.mem.w16(s + 0x60, b);
                    self.mem.w32(s + 0x58, cur);
                    return 1;
                }
                0xFE00 => cur = seq + 4 * b as u32,
                0xFF00 => {
                    let n = a & 0xFF;
                    if self.mem.u16(s + 0x62) < n {
                        cur += 4;
                    } else {
                        self.mem.w32(s + 0x58, cur.wrapping_sub(4));
                        return 0;
                    }
                }
                x if x & 0x8000 != 0 => {
                    let n = a & 0xFF;
                    let l = self.mem.u16(s + 0x62);
                    if n < l {
                        cur += 4;
                    } else {
                        self.mem.w16(s + 0x62, l + 1);
                        cur = seq + 4 * b as u32;
                    }
                }
                _ => return 1, // an endless loop in the original
            }
        }
        1
    }

    /// uv_anim: a task stepping `script` (4-byte entries {u16 idx|control, s16 frames}) on `model`.
    pub fn uv_anim(&mut self, model: u32, slot: u32, uvtab: u32, script: u32) -> u32 {
        let timer = self.mem.s16(script + 2).wrapping_add(1);
        self.insert(Task { kind: Kind::UvAnim { model, slot, uvtab, base: script, cur: script, timer, count: 0 }, slot: 0, dtor: Dtor::None, kill: false })
    }

    /// One step of the UV animator task (`uv_anim`).
    pub(super) fn uv_anim_step(&mut self, h: u32) {
        let Some(Task { kind: Kind::UvAnim { model, slot, uvtab, base, mut cur, mut timer, mut count }, .. }) = self.tasks.get(&h).cloned() else { return };
        if !self.model_alive(model) {
            self.kill(h); // the owner kills it with its model
            return;
        }
        if !self.paused {
            timer -= 1;
        }
        let mut end = false;
        if timer <= 0 {
            cur += 4;
            for _ in 0..64 {
                let a = self.mem.u16(cur);
                if a & 0xFF00 == 0 {
                    break;
                }
                let lo = (a & 0xFF) as i16;
                match a & 0xFF00 {
                    0xFE00 => cur = base + 4 * lo as u32,
                    0xFC00 => {
                        if lo < count {
                            cur += 4;
                        } else {
                            count += 1;
                            cur = (base as i32 + 4 * self.mem.s16(cur + 2) as i32) as u32;
                        }
                    }
                    0xFF00 => {
                        if count < lo {
                            cur += 4;
                        } else {
                            end = true;
                            break;
                        }
                    }
                    _ => break,
                }
            }
            if !end {
                let idx = self.mem.u16(cur) as u32;
                let (du, dv) = (self.mem.s16(uvtab + 4 * idx), self.mem.s16(uvtab + 4 * idx + 2));
                self.with_model(model, |mi| mi.uv_scroll(du as u8, dv as u8));
                timer = self.mem.s16(cur + 2);
            }
        }
        if end {
            if slot != 0 {
                self.mem.w32(slot, 0);
            }
            self.kill(h);
            return;
        }
        if let Some(Task { kind: Kind::UvAnim { cur: c, timer: t, count: n, .. }, .. }) = self.tasks.get_mut(&h) {
            (*c, *t, *n) = (cur, timer, count);
        }
    }

    /// The colour animator task (`rgb = from + delta·t`, t = timer / duration).
    pub(super) fn colour_step(&mut self, h: u32) {
        let Some(Task { kind: Kind::Colour { ft4, seq, tab, mut cur, mut timer, mut loops, mut hold, mut dur, mut dir }, .. }) = self.tasks.get(&h).cloned()
        else {
            return;
        };
        let mut killed = false;
        'step: {
            if hold != 0 {
                hold -= 1;
                break 'step;
            }
            if !self.paused {
                timer -= 1;
            }
            if timer > 0 {
                break 'step;
            }
            cur = cur.wrapping_add(4);
            for _ in 0..64 {
                let a = self.mem.u16(cur);
                if a & 0xFF00 == 0 {
                    break;
                }
                let n = (a & 0xFF) as i16;
                match a & 0xFF00 {
                    0xFD00 => cur = seq.wrapping_add(4 * self.mem.s16(cur + 2) as i32 as u32),
                    0xFE00 => {
                        hold = self.mem.s16(cur + 2);
                        cur += 4;
                    }
                    0xFC00 => {
                        if n < loops {
                            cur += 4;
                        } else {
                            loops += 1;
                            cur = seq.wrapping_add(4 * self.mem.s16(cur + 2) as i32 as u32);
                        }
                    }
                    0xFF00 => {
                        if n < loops {
                            cur += 4;
                        } else {
                            killed = true;
                            timer = 0;
                            cur = cur.wrapping_sub(4);
                            break 'step;
                        }
                    }
                    _ => {}
                }
            }
            dur = self.mem.s16(cur + 2);
            timer = dur;
            dir = if dur > 0 { 0 } else { 1 };
        }
        let idx = self.mem.s16(cur) as i32;
        let e = (tab as i32 + 12 * idx) as u32;
        let from = [0, 1, 2].map(|k| self.mem.s16(e + 2 * k) as i32);
        let delta = [0, 1, 2].map(|k| self.mem.s16(e + 6 + 2 * k) as i32);
        let t = if dur != 0 { ((timer as i32) << 12) / dur as i32 } else { 0 };
        let t = if dir == 0 { t } else { 0x1000 - t };
        let rgb = [0, 1, 2].map(|k| (from[k] + ((delta[k] * t) >> 12)) as u8);
        for p in ft4 {
            for k in 0..3 {
                self.mem.w8(p + 4 + k as u32, rgb[k]);
            }
        }
        if let Some(Task { kind: Kind::Colour { cur: c, timer: ti, loops: l, hold: ho, dur: d, dir: di, .. }, .. }) = self.tasks.get_mut(&h) {
            (*c, *ti, *l, *ho, *d, *di) = (cur, timer, loops, hold, dur, dir);
        }
        if killed {
            self.kill(h);
        }
    }
}
