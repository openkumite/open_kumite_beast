//! The effect VM's data ops that `exec` hands on (op00, op01, op02, op04, op05, op0F).
//!
//! State: none of its own: steps the serialised `Vm`
//!
//! The data ops `exec` hands on: op00 (an object field with an immediate), op01 (an array element with another
//! array's element), op02 (an array element with an immediate), op04 (vectors and matrices, MulMatrix0),
//! op05 (sprite blocks and billboards) and op0F (2-D screen sprites, unused).

use super::{Billboard, Dtor, Kind, Mem, Task, Vm, rcos, rsin};
use crate::base::libc_rand::rand;

impl Vm {
    pub(super) fn op00(&mut self, h: u32, a: u32, pc: u32, sub: u8) {
        let imm16 = self.mem.s16(pc + 4) as i32;
        let imm32 = self.mem.u32(pc + 4);
        let imm8 = self.mem.u8(pc + 4);
        let mem = &mut self.mem;
        let mut cond = None;
        match sub {
            0x00 => mem.w8(a, mem.u8(a).wrapping_add(imm8)),
            0x01 => mem.w16(a, mem.u16(a).wrapping_add(imm16 as u16)),
            0x02 => mem.w32(a, mem.u32(a).wrapping_add(imm32)),
            0x03 => mem.w8(a, imm8),
            0x04 => mem.w16(a, imm16 as u16),
            0x05 => mem.w32(a, imm32),
            0x06 => mem.w8(a, 0),
            0x07 => mem.w16(a, 0),
            0x08 => mem.w32(a, 0),
            0x09 => cond = Some(mem.u8(a) as i32),
            0x0A => cond = Some(mem.s16(a) as i32),
            0x0B => cond = Some(mem.s32(a)),
            0x0C => cond = Some(mem.u8(a) as i32 - imm16),
            0x0D => cond = Some(mem.s16(a) as i32 - imm16),
            0x0E => cond = Some(mem.s32(a).wrapping_sub(imm32 as i32)),
            0x0F => mem.w8(a, (mem.u8(a) as i32).wrapping_mul(imm16) as u8),
            0x10 => mem.w16(a, mem.u16(a).wrapping_mul(imm16 as u16)),
            0x11 => mem.w32(a, mem.u32(a).wrapping_mul(imm32)),
            0x12 => mem.w8(a, imm8.wrapping_sub(mem.u8(a))),
            0x13 => mem.w16(a, (imm16 as u16).wrapping_sub(mem.u16(a))),
            0x14 => mem.w32(a, imm32.wrapping_sub(mem.u32(a))),
            0x15 => {
                let v = mem.u8(a) & imm8;
                mem.w8(a, v);
                cond = Some(v as i32);
            }
            0x16 => {
                let v = mem.u16(a) & imm16 as u16;
                mem.w16(a, v);
                cond = Some(v as i16 as i32);
            }
            0x17 => {
                let v = mem.u32(a) & imm32;
                mem.w32(a, v);
                cond = Some(v as i32);
            }
            0x18 => {
                let v = mem.u8(a) | imm8;
                mem.w8(a, v);
                cond = Some(v as i32);
            }
            0x19 => {
                let v = mem.u16(a) | imm16 as u16;
                mem.w16(a, v);
                cond = Some(v as i16 as i32);
            }
            0x1A => {
                let v = mem.u32(a) | imm32;
                mem.w32(a, v);
                cond = Some(v as i32);
            }
            0x1B => mem.w8(a, mem.u8(a) >> (imm16 & 31)),
            0x1C | 0x1F => mem.w16(a, (mem.s16(a) >> (imm16 & 15)) as u16),
            0x1D | 0x20 => mem.w32(a, (mem.s32(a) >> (imm16 & 31)) as u32),
            0x1E => mem.w8(a, ((mem.u8(a) as i8) >> (imm16 & 7)) as u8),
            0x21 | 0x24 => mem.w8(a, mem.u8(a).wrapping_shl(imm16 as u32)),
            0x22 | 0x25 => mem.w16(a, mem.u16(a).wrapping_shl(imm16 as u32)),
            0x23 | 0x26 => mem.w32(a, mem.u32(a).wrapping_shl(imm16 as u32)),
            0x27 => {
                let r = rand(&mut self.seed);
                self.mem.w8(a, (r / 129) as u8);
            }
            0x28 => {
                let r = rand(&mut self.seed);
                self.mem.w16(a, r as u16);
            }
            0x29 => {
                let r = rand(&mut self.seed);
                self.mem.w32(a, r as u32);
            }
            _ => self.unsupported_op(0, sub),
        }
        if let Some(c) = cond {
            self.set_cond(h, c);
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn op01(&mut self, h: u32, p: u32, i: i32, q: u32, j: i32, n: i32, sub: u8) {
        let e = |base: u32, k: i32, sz: i32| (base as i32).wrapping_add(k * sz) as u32;
        let mem = &mut self.mem;
        let mut cond = None;
        match sub {
            0x00 => mem.w8(e(p, i, 1), mem.u8(e(q, j, 1))),
            0x01 => mem.w16(e(p, i, 2), mem.u16(e(q, j, 2))),
            0x02 => mem.w32(e(p, i, 4), mem.u32(e(q, j, 4))),
            0x03 => mem.w8(e(p, i, 1), mem.u8(e(p, i, 1)).wrapping_add(mem.u8(e(q, j, 1)))),
            0x04 => mem.w16(e(p, i, 2), mem.u16(e(p, i, 2)).wrapping_add(mem.u16(e(q, j, 2)))),
            0x05 => mem.w32(e(p, i, 4), mem.u32(e(p, i, 4)).wrapping_add(mem.u32(e(q, j, 4)))),
            0x06 => mem.w8(e(p, i, 1), mem.u8(e(p, i, 1)).wrapping_sub(mem.u8(e(q, j, 1)))),
            0x07 => mem.w16(e(p, i, 2), mem.u16(e(p, i, 2)).wrapping_sub(mem.u16(e(q, j, 2)))),
            0x08 => mem.w32(e(p, i, 4), mem.u32(e(p, i, 4)).wrapping_sub(mem.u32(e(q, j, 4)))),
            0x09 => mem.w8(e(p, i, 1), mem.u8(e(p, i, 1)).wrapping_mul(mem.u8(e(q, j, 1)))),
            0x0A => mem.w16(e(p, i, 2), mem.u16(e(p, i, 2)).wrapping_mul(mem.u16(e(q, j, 2)))),
            0x0B => mem.w32(e(p, i, 4), mem.u32(e(p, i, 4)).wrapping_mul(mem.u32(e(q, j, 4)))),
            0x0C => cond = Some(mem.u8(e(p, i, 1)) as i32 - mem.u8(e(q, j, 1)) as i32),
            0x0D => cond = Some(mem.s16(e(p, i, 2)) as i32 - mem.s16(e(q, j, 2)) as i32),
            0x0E => cond = Some(mem.s32(e(p, i, 4)).wrapping_sub(mem.s32(e(q, j, 4)))),
            0x0F => mem.w8(e(p, i, 1), mem.u8(e(p, i, 1)) & mem.u8(e(q, j, 1))),
            0x10 => mem.w16(e(p, i, 2), mem.u16(e(p, i, 2)) & mem.u16(e(q, j, 2))),
            0x11 => mem.w32(e(p, i, 4), mem.u32(e(p, i, 4)) & mem.u32(e(q, j, 4))),
            0x12 => mem.w8(e(p, i, 1), mem.u8(e(p, i, 1)) | mem.u8(e(q, j, 1))),
            0x13 => mem.w16(e(p, i, 2), mem.u16(e(p, i, 2)) | mem.u16(e(q, j, 2))),
            0x14 => mem.w32(e(p, i, 4), mem.u32(e(p, i, 4)) | mem.u32(e(q, j, 4))),
            0x15 => mem.w32(e(p, i, 4), (q as i32).wrapping_add(j) as u32),
            0x16 => {
                for k in 0..n.max(0) {
                    let v = mem.u32(e(q, j + k, 4));
                    mem.w32(e(p, i + k, 4), v);
                }
            }
            0x17 => {
                for k in 0..n.max(0) {
                    let v = mem.u32(e(p, i + k, 4)).wrapping_add(mem.u32(e(q, j + k, 4)));
                    mem.w32(e(p, i + k, 4), v);
                }
            }
            0x18 => {
                if let Some(v) = mem.u8(e(p, i, 1)).checked_div(mem.u8(e(q, j, 1))) {
                    mem.w8(e(p, i, 1), v);
                }
            }
            0x19 => {
                let d = mem.s16(e(q, j, 2));
                if d != 0 {
                    mem.w16(e(p, i, 2), mem.s16(e(p, i, 2)).wrapping_div(d) as u16);
                }
            }
            0x1A => {
                let d = mem.s32(e(q, j, 4));
                if d != 0 {
                    mem.w32(e(p, i, 4), mem.s32(e(p, i, 4)).wrapping_div(d) as u32);
                }
            }
            _ => self.unsupported_op(1, sub),
        }
        if let Some(c) = cond {
            self.set_cond(h, c);
        }
    }

    pub(super) fn op02(&mut self, h: u32, p: u32, pc: u32, sub: u8) {
        let i = self.mem.s16(pc + 4) as i32;
        let imm16 = self.mem.s16(pc + 6) as i32;
        let imm8 = imm16 as u8;
        let imm32 = self.mem.u32(pc + 8);
        let e = |k: i32, sz: i32| (p as i32).wrapping_add(k * sz) as u32;
        let mem = &mut self.mem;
        let mut cond = None;
        match sub {
            0x00 => mem.w8(e(i, 1), imm8),
            0x01 => mem.w16(e(i, 2), imm16 as u16),
            0x02 => mem.w32(e(i, 4), imm32),
            0x03 => mem.w8(e(i, 1), mem.u8(e(i, 1)).wrapping_add(imm8)),
            0x04 => mem.w16(e(i, 2), mem.u16(e(i, 2)).wrapping_add(imm16 as u16)),
            0x05 => mem.w32(e(i, 4), mem.u32(e(i, 4)).wrapping_add(imm32)),
            0x06 => mem.w8(e(i, 1), imm8.wrapping_sub(mem.u8(e(i, 1)))),
            0x07 => mem.w16(e(i, 2), (imm16 as u16).wrapping_sub(mem.u16(e(i, 2)))),
            0x08 => mem.w32(e(i, 4), imm32.wrapping_sub(mem.u32(e(i, 4)))),
            0x09 => mem.w8(e(i, 1), (mem.u8(e(i, 1)) as i32).wrapping_mul(imm16) as u8),
            0x0A => mem.w16(e(i, 2), mem.u16(e(i, 2)).wrapping_mul(imm16 as u16)),
            0x0B => mem.w32(e(i, 4), mem.u32(e(i, 4)).wrapping_mul(imm32)),
            0x0C => mem.w8(e(i, 1), 0),
            0x0D => mem.w16(e(i, 2), 0),
            0x0E => mem.w32(e(i, 4), 0),
            0x0F => cond = Some(mem.u8(e(i, 1)) as i32),
            0x10 => cond = Some(mem.s16(e(i, 2)) as i32),
            0x11 => cond = Some(mem.s32(e(i, 4))),
            0x12 => cond = Some(mem.u8(e(i, 1)) as i32 - imm16),
            0x13 => cond = Some(mem.s16(e(i, 2)) as i32 - imm16),
            0x14 => cond = Some(mem.s32(e(i, 4)).wrapping_sub(imm32 as i32)),
            0x15 => mem.w8(e(i, 1), mem.u8(e(i, 1)) & imm8),
            0x16 => mem.w16(e(i, 2), mem.u16(e(i, 2)) & imm16 as u16),
            0x17 => mem.w32(e(i, 4), mem.u32(e(i, 4)) & imm32),
            0x18 => mem.w8(e(i, 1), mem.u8(e(i, 1)) | imm8),
            0x19 => mem.w16(e(i, 2), mem.u16(e(i, 2)) | imm16 as u16),
            0x1A => mem.w32(e(i, 4), mem.u32(e(i, 4)) | imm32),
            0x1B => {
                for k in 0..imm16.max(0) {
                    mem.w32(e(i + k, 4), 0);
                }
            }
            _ => self.unsupported_op(2, sub),
        }
        if let Some(c) = cond {
            self.set_cond(h, c);
        }
    }

    pub(super) fn op04(&mut self, o: u32, a: u32, pc: u32, sub: u8) {
        let mp = self.mem.u32(a);
        let h4 = self.mem.s16(pc + 4) as i32;
        let h6 = self.mem.s16(pc + 6) as i32;
        let at = |off: i32| (o as i32).wrapping_add(off) as u32;
        let v = at(h4);
        let vs = |vm: &Vm, k: u32| vm.mem.s16(v + 2 * k) as i32;
        match sub {
            0x00 => self.write_m3(mp, &Self::euler(vs(self, 0), vs(self, 1), vs(self, 2))),
            0x01 => self.write_m3(mp, &Self::rot_x(vs(self, 0))),
            0x02 => self.write_m3(mp, &Self::rot_y(vs(self, 0))),
            0x03 => self.write_m3(mp, &Self::rot_z(vs(self, 0))),
            0x04 => self.write_m3(mp, &Self::rot2(vs(self, 0), vs(self, 1))),
            0x05 => self.write_m3(mp, &Self::rot_x(h4)),
            0x06 => self.write_m3(mp, &Self::rot_y(h4)),
            0x07 => self.write_m3(mp, &Self::rot_z(h4)),
            0x08 => self.write_m3(mp, &Self::rot2(h4, h6)),
            0x09 | 0x0A => {
                // MulMatrix0 / with translation: out = M × B
                let b = self.mem.u32(at(h4));
                let out = self.mem.u32(at(h6));
                let (ma, mb) = (self.read_m3(mp), self.read_m3(b));
                let r = Self::mul(&ma, &mb);
                if sub == 0x0A {
                    let tb = [0, 1, 2].map(|k| self.mem.s32(b + 0x14 + 4 * k));
                    let ta = [0, 1, 2].map(|k| self.mem.s32(mp + 0x14 + 4 * k));
                    for k in 0..3 {
                        let t = ta[k] + ((ma[k][0] * tb[0] + ma[k][1] * tb[1] + ma[k][2] * tb[2]) >> 12);
                        self.mem.w32(out + 0x14 + 4 * k as u32, t as u32);
                    }
                }
                self.write_m3(out, &r);
            }
            0x0B => self.mem.w32(a, rsin(h4) as u32),
            0x0C => self.mem.w32(a, rcos(h4) as u32),
            0x0D..=0x0F => {
                let t = self.mem.u32(at(h4));
                match sub {
                    0x0D => {
                        let ang = self.mem.s16((t as i32 + 2 * h6) as u32) as i32;
                        self.mem.w32(a, rsin(ang) as u32);
                    }
                    0x0E => {
                        let ang = self.mem.s16((t as i32 + 2 * h6) as u32) as i32;
                        self.mem.w32(a, rcos(ang) as u32);
                    }
                    _ => {
                        let y = self.mem.s32((t as i32 + 4 * h6) as u32) as f64;
                        let x = self.mem.s32((t as i32 + 4 * (h6 + 2)) as u32) as f64;
                        // the platform's atan2 before simulation version 3, then libm's
                        let ang = (crate::base::portable::atan2_f64(y, x, self.portable) * 4096.0 / std::f64::consts::TAU).round() as i32 & 0xFFF;
                        self.mem.w32(a, ang as u32);
                    }
                }
            }
            _ => self.unsupported_op(4, sub),
        }
    }

    // ------------------------------------------------------------------ sprites

    /// Write a field of both FT4 of the sprite block at `s`.
    fn ft4_both(&mut self, s: u32, off: u32, f: impl Fn(&mut Mem, u32)) {
        f(&mut self.mem, s + off);
        f(&mut self.mem, s + 0x28 + off);
    }

    pub(super) fn set_uv_rect(&mut self, s: u32, rect: u32) {
        let (u, v, w, hh) = (self.mem.u8(rect), self.mem.u8(rect + 1), self.mem.u8(rect + 2), self.mem.u8(rect + 3));
        for (k, (du, dv)) in [(0, 0), (w, 0), (0, hh), (w, hh)].iter().enumerate() {
            let off = [0x0C, 0x14, 0x1C, 0x24][k];
            let (uu, vv) = (u.wrapping_add(*du), v.wrapping_add(*dv));
            self.ft4_both(s, off, |m, a| {
                m.w8(a, uu);
                m.w8(a + 1, vv);
            });
        }
    }

    pub(super) fn op05(&mut self, h: u32, base: u32, o: u32, s: u32, pc: u32, sub: u8) {
        let h4 = self.mem.s16(pc + 4) as i32;
        let h6 = self.mem.s16(pc + 6) as i32;
        let at = |off: i32| (o as i32).wrapping_add(off) as u32;
        let sc = |off: i32| (base as i32).wrapping_add(off) as u32;
        match sub {
            0x00 => self.ft4_both(s, 0x16, |m, a| m.w16(a, h4 as u16)),
            0x01 => self.ft4_both(s, 0x0E, |m, a| m.w16(a, h4 as u16)),
            0x02 => self.set_uv_rect(s, sc(h4)),
            0x03 => {
                let (w, hh) = (self.mem.u16(at(h4)), self.mem.u16(at(h6)));
                self.mem.w16(s + 0x50, w);
                self.mem.w16(s + 0x52, hh);
            }
            0x04 => {
                self.mem.w16(s + 0x50, h4 as u16);
                self.mem.w16(s + 0x52, h6 as u16);
            }
            0x05 => {
                self.mem.w32(s + 0x54, sc(h4));
                self.mem.w32(s + 0x5C, sc(h6));
                self.mem.w32(s + 0x58, sc(h6).wrapping_sub(4));
                self.mem.w16(s + 0x60, 0);
                self.mem.w16(s + 0x62, 0);
            }
            0x06 => {
                self.mem.w32(s + 0x68, sc(h4));
                self.mem.w32(s + 0x6C, sc(h4));
                self.mem.w16(s + 0x70, 0);
            }
            0x07 => {
                self.ft4_both(s, 3, |m, a| {
                    m.w8(a, 9);
                    m.w8(a + 1, 0x80);
                    m.w8(a + 2, 0x80);
                    m.w8(a + 3, 0x80);
                    m.w8(a + 4, 0x2E);
                });
                self.mem.w16(s + 0x50, h4 as u16);
                self.mem.w16(s + 0x52, h6 as u16);
            }
            0x08 => {
                // 16-byte def: tpage, clut, w, h (u16), u, v, uw, vh (u8), r, g, b, semi (u8)
                let d = sc(h4);
                let (tp, cl, w, hh) = (self.mem.u16(d), self.mem.u16(d + 2), self.mem.u16(d + 4), self.mem.u16(d + 6));
                let (r, g, b, semi) = (self.mem.u8(d + 12), self.mem.u8(d + 13), self.mem.u8(d + 14), self.mem.u8(d + 15));
                let code = if semi != 0 { 0x2E } else { 0x2C };
                self.ft4_both(s, 3, |m, a| {
                    m.w8(a, 9);
                    m.w8(a + 1, r);
                    m.w8(a + 2, g);
                    m.w8(a + 3, b);
                    m.w8(a + 4, code);
                    m.w16(a + 0x0B, cl);
                    m.w16(a + 0x13, tp);
                });
                self.mem.w16(s + 0x50, w);
                self.mem.w16(s + 0x52, hh);
                self.set_uv_rect(s, d + 8);
            }
            0x09..=0x0B => {
                let p = at(h4);
                let pos = if sub == 0x09 { [0, 1, 2].map(|k| self.mem.s16(p + 2 * k) as i32) } else { [0, 1, 2].map(|k| self.mem.s16(p + 4 * k + 2) as i32) };
                let ft = s + 0x28 * self.buf as u32;
                let rd = |m: &Mem, k: u32| [m.u8(ft + k), m.u8(ft + k + 1)];
                let code = self.mem.u8(ft + 7);
                self.draws.push(Billboard {
                    pos,
                    w: self.mem.s16(s + 0x50) as i32,
                    fixed_otz: (sub == 0x0B).then_some(h6),
                    uv: [rd(&self.mem, 0x0C), rd(&self.mem, 0x14), rd(&self.mem, 0x1C), rd(&self.mem, 0x24)],
                    rgb: [self.mem.u8(ft + 4), self.mem.u8(ft + 5), self.mem.u8(ft + 6)],
                    tpage: self.mem.u16(ft + 0x16),
                    // a script of the team battle's groups 5 / 6 (fighters 2 / 3's `<CHR>EFT`) draws from VRAM bank 1
                    // (engine addition: CLUT bit 15, which the GPU does not read)
                    clut: self.mem.u16(ft + 0x0E) | Self::bank_of(base).clut_bit(),
                    semi: code & 2 != 0,
                });
            }
            0x0C => {
                let c = self.uv_step(s);
                self.set_cond(h, c);
            }
            0x0D => {} // CLUT animation: broken in the original (works on a stack copy)
            0x0E => {
                let tab = sc(h4);
                let seq = sc(h6);
                let t = self.insert(Task {
                    kind: Kind::Colour { ft4: [s, s + 0x28], seq, tab, cur: seq.wrapping_sub(4), timer: 0, loops: 0, hold: 0, dur: 0, dir: 0 },
                    slot: 0,
                    dtor: Dtor::None,
                    kill: false,
                });
                let slot = at(self.mem.s16(pc + 8) as i32);
                self.mem.w32(slot, t);
            }
            _ => self.unsupported_op(5, sub),
        }
    }

    pub(super) fn op0f(&mut self, base: u32, s: u32, pc: u32, sub: u8) {
        // 2-D screen sprites: no script uses them; recorded as unsupported if ever met
        let _ = (base, s, pc);
        self.unsupported_op(0x0F, sub);
    }
}
