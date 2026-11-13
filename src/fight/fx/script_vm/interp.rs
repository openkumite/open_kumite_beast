//! The effect VM's interpreter: `run`, `exec`, `op_size`.
//!
//! State: none of its own: steps the serialised `Vm`
//!
//! The interpreter: `run` (vm_run, with the control ops 07.xx: jumps, CALL, YIELD, loops, END,
//! DESTROY), `exec` (one non-control instruction; op03 (models) and op06-op0D inline, the others in `ops.rs`,
//! op0E in `timeline.rs`) and the instruction sizes `op_size`.

use super::{Dtor, Exit, Kind, ModelSlot, Task, Vm};

impl Vm {
    /// vm_run. `depth` > 0 for nested runs (CALL, timeline subroutines).
    pub(super) fn run(&mut self, h: u32, depth: u32) -> Exit {
        let Some(Kind::Script { base, obj: o, pc: saved, .. }) = self.tasks.get(&h).map(|t| t.kind.clone()) else { return Exit::End };
        let mut pc = saved;
        if self.paused && depth == 0 {
            let p5 = self.mem.u32(base + 0x14);
            if p5 == 0 || saved == base + self.mem.u32(base) {
                return Exit::End;
            }
            pc = base + p5;
        }
        loop {
            if self.budget == 0 {
                log::warn!("fxvm: script {base:#x} did not yield, stopped");
                self.kill(h);
                return Exit::Destroy;
            }
            self.budget -= 1;
            if !self.tasks.contains_key(&h) {
                return Exit::Destroy;
            }
            let w = self.mem.u32(pc);
            let (op, sub, f) = ((w & 0xFF) as u8, ((w >> 8) & 0xFF) as u8, (w >> 16) as i16 as i32);
            let hh = |vm: &Vm, k: u32| vm.mem.s16(pc + k) as i32;
            let at = |off: i32| (o as i32).wrapping_add(off) as u32;
            let tgt = |off: i32| (base as i32).wrapping_add(off) as u32;
            if op == 7 {
                let cond = self.get_cond(h);
                let jump = match sub {
                    0 => Some(true),
                    1 => Some(cond == 0),
                    2 => Some(cond != 0),
                    5 => Some(cond > 0),
                    6 => Some(cond >= 0),
                    7 => Some(cond < 0),
                    8 => Some(cond <= 0),
                    _ => None,
                };
                if let Some(j) = jump {
                    pc = if j { tgt(f) } else { pc + 4 };
                    continue;
                }
                match sub {
                    3 => {
                        // CALL: nested run from F with the shared ctx.pc
                        self.set_pc(h, tgt(f));
                        let e = self.run(h, depth + 1);
                        if e == Exit::Destroy && !self.alive(h) {
                            return Exit::Destroy;
                        }
                        pc += 4;
                    }
                    4 => {
                        self.set_pc(h, tgt(f));
                        return Exit::Yield;
                    }
                    9 => {
                        let v = hh(self, 4);
                        self.mem.w32(at(f), v as u32);
                        pc += 8;
                    }
                    0xA => {
                        let v = self.mem.s32(at(f)).wrapping_add(1);
                        self.mem.w32(at(f), v as u32);
                        pc = if v < hh(self, 4) { tgt(hh(self, 6)) } else { pc + 8 };
                    }
                    0xB => return Exit::End,
                    0xC => {
                        self.kill(h);
                        return Exit::Destroy;
                    }
                    _ => {
                        self.unsupported_op(op, sub);
                        return Exit::End;
                    }
                }
                continue;
            }
            let size = op_size(op, sub);
            if size == 0 {
                self.unsupported_op(op, sub);
                self.kill(h);
                return Exit::Destroy;
            }
            let next = pc + size;
            match self.exec(h, base, o, pc, op, sub, f, depth) {
                Some(e) => return e,
                None => pc = next,
            }
        }
    }

    pub(super) fn unsupported_op(&mut self, op: u8, sub: u8) {
        if !self.unsupported.contains(&(op, sub)) {
            log::warn!("fxvm: op{op:02X}.{sub:02X} not supported");
            self.unsupported.push((op, sub));
        }
    }

    /// One non-control instruction. Returns Some(exit) when the task ended inside (timeline subroutine kill).
    #[allow(clippy::too_many_arguments)]
    fn exec(&mut self, h: u32, base: u32, o: u32, pc: u32, op: u8, sub: u8, f: i32, depth: u32) -> Option<Exit> {
        let m = |vm: &Vm, k: u32| vm.mem.s16(pc + k) as i32;
        let at = |off: i32| (o as i32).wrapping_add(off) as u32;
        let a = at(f);
        match op {
            0x00 => self.op00(h, a, pc, sub),
            0x01 => {
                let p = self.mem.u32(a);
                let i = m(self, 4);
                let q = self.mem.u32(at(m(self, 6)));
                let j = m(self, 8);
                self.op01(h, p, i, q, j, m(self, 10), sub);
            }
            0x02 => {
                let p = self.mem.u32(a);
                self.op02(h, p, pc, sub);
            }
            0x03 => {
                let hm = self.mem.u32(a);
                let (h4, h6, h8) = (m(self, 4), m(self, 6), m(self, 8));
                let var32 = self.mem.s32(at(h4));
                let var16 = self.mem.s16(at(h4)) as i32;
                let buf = self.buf;
                match sub {
                    0 | 1 => {
                        let slot = if sub == 0 { a } else { at(self.mem.s32(a)) };
                        let hm = self.mem.u32(slot);
                        if hm != 0 {
                            self.free_model(hm);
                            self.mem.w32(slot, 0);
                        }
                    }
                    0x19 => {
                        // UV animator on the model: uvtab = base+h6, script = base+h8; handle → O+h4 or COND
                        if self.model_alive(hm) {
                            let (uvtab, script) = ((base as i32 + h6) as u32, (base as i32 + h8) as u32);
                            let slot = if h4 != 0 { at(h4) } else { 0 };
                            let t = self.uv_anim(hm, slot, uvtab, script);
                            if h4 != 0 {
                                self.mem.w32(at(h4), t);
                            } else {
                                self.set_cond(h, t as i32);
                            }
                        }
                    }
                    0x18 => {
                        // model rotation from three s16 angles
                        if self.models.iter().any(|(k, _)| *k == hm) {
                            let v = at(h4);
                            let e = Self::euler(self.mem.s16(v) as i32, self.mem.s16(v + 2) as i32, self.mem.s16(v + 4) as i32);
                            let ma = self.mem.u32(hm);
                            self.write_m3(ma, &e);
                        }
                    }
                    _ => {
                        let mut cond = None;
                        if let Some(mi) = self.model(hm) {
                            match sub {
                                2 => cond = Some(mi.play_a(h4)),
                                3 => cond = Some(mi.play_a(var32)),
                                4 => cond = Some(mi.play_b(h4)),
                                5 => cond = Some(mi.play_b(var32)),
                                6 | 7 => cond = Some(mi.play_default_b()),
                                9 => mi.set_time_b(var16, buf),
                                0x0A => mi.track_ctl(false, h4, 0, 0, 0),
                                0x0B => mi.track_ctl(false, h4, 1, 0, 0),
                                0x0C => mi.track_ctl(false, h4, 2, h6, h8),
                                0x0D => mi.track_ctl(true, h4, 0, 0, 0),
                                0x0E => mi.track_ctl(true, h4, 1, 0, 0),
                                0x0F => mi.track_ctl(true, h4, 2, h6, h8),
                                0x10 => mi.set_depth(h4),
                                0x11 => mi.clear_depth(),
                                0x12 => mi.show(true),
                                0x13 => mi.show(false),
                                0x14 => mi.show(var16 != 0),
                                0x15 => mi.set_clut(h4 as u16),
                                0x16 => mi.uv_scroll(h4 as u8, h6 as u8),
                                _ => {} // 08 set_time_A, 17 attach helper: unused
                            }
                        }
                        if let Some(c) = cond {
                            self.set_cond(h, c);
                        }
                    }
                }
            }
            0x04 => self.op04(o, a, pc, sub),
            0x05 => self.op05(h, base, o, a, pc, sub),
            0x06 => {
                let p = self.mem.u32(a);
                let dp = match self.tasks.get(&h).map(|t| &t.kind) {
                    Some(Kind::Script { dp, .. }) => *dp,
                    _ => 0,
                };
                let i = m(self, 4);
                match sub {
                    0 => {
                        let v = self.mem.u8((dp as i32 + m(self, 6)) as u32);
                        self.mem.w8((p as i32 + i) as u32, v);
                    }
                    1 => {
                        let v = self.mem.u16((dp as i32 + m(self, 6)) as u32);
                        self.mem.w16((p as i32 + 2 * i) as u32, v);
                    }
                    2 => {
                        let v = self.mem.u32((dp as i32 + m(self, 6)) as u32);
                        self.mem.w32((p as i32 + 4 * i) as u32, v);
                    }
                    3..=5 => {
                        let q = self.mem.u32(at(m(self, 6)));
                        let j = m(self, 8);
                        match sub {
                            3 => {
                                let k = self.mem.u8((q as i32 + j) as u32) as i32;
                                let v = self.mem.u8((dp as i32 + k) as u32);
                                self.mem.w8((p as i32 + i) as u32, v);
                            }
                            4 => {
                                let k = self.mem.s16((q as i32 + 2 * j) as u32) as i32;
                                let v = self.mem.u16((dp as i32 + k) as u32);
                                self.mem.w16((p as i32 + 2 * i) as u32, v);
                            }
                            _ => {
                                let k = self.mem.s32((q as i32 + 4 * j) as u32);
                                let v = self.mem.u32((dp as i32 + k) as u32);
                                self.mem.w32((p as i32 + 4 * i) as u32, v);
                            }
                        }
                    }
                    6 => {
                        let off = m(self, 6);
                        for k in 0..m(self, 8).max(0) {
                            let v = self.mem.u32((dp as i32 + off + 4 * k) as u32);
                            self.mem.w32((p as i32 + 4 * (i + k)) as u32, v);
                        }
                    }
                    _ => self.unsupported_op(op, sub),
                }
            }
            0x08 => match sub {
                0 => {
                    if let Some(Task { kind: Kind::Script { dp, .. }, .. }) = self.tasks.get_mut(&h) {
                        *dp = (base as i32 + f) as u32;
                    }
                }
                1 => self.mem.w32(a, (base as i32 + m(self, 4)) as u32),
                2 => self.mem.w32(a, (o as i32 + m(self, 4)) as u32),
                _ => self.unsupported_op(op, sub),
            },
            0x09 => {
                // args: g = F, heap = h4, bank = h6, idx = h8, slot = O + h10
                let bank = Self::own_group(base, if sub == 3 { self.mem.s32(at(m(self, 6))) } else { m(self, 6) } as usize);
                let idx = m(self, 8) as usize;
                let slot = at(m(self, 10));
                match sub {
                    0 => {
                        if self.mem.u32(slot) == 0 {
                            let t = self.spawn(bank, idx, Some(slot), &[]);
                            self.mem.w32(slot, t);
                        }
                    }
                    1 => {
                        self.spawn(bank, idx, None, &[]);
                    }
                    2 | 3 => {
                        let t = self.spawn(bank, idx, None, &[]);
                        self.mem.w32(slot, t);
                    }
                    _ => self.unsupported_op(op, sub),
                }
            }
            0x0A => {
                let target = self.mem.u32(a);
                let t = self.obj_of(target);
                match sub {
                    0 => {
                        let v = self.mem.u32((o as i32 + m(self, 6) + 4 * m(self, 8)) as u32);
                        if t != 0 {
                            self.mem.w32((t as i32 + 4 * m(self, 4)) as u32, v);
                        }
                    }
                    1..=4 => {
                        let r = self.mem.u32(at(m(self, 4)));
                        let k = m(self, 6);
                        for n in 0..sub as i32 {
                            let v = self.mem.u32((r as i32 + 4 * (k + n)) as u32);
                            if t != 0 {
                                self.mem.w32(t + 4 + 4 * n as u32, v);
                            }
                        }
                    }
                    5 => {
                        let v = self.mem.u32(pc + 8);
                        if t != 0 {
                            self.mem.w32((t as i32 + 4 * m(self, 4)) as u32, v);
                        }
                    }
                    6 => {
                        if target != 0 {
                            self.kill(target);
                        }
                    }
                    _ => self.unsupported_op(op, sub),
                }
            }
            0x0B => {
                // fork: shares base and O, own pc / COND / DP
                let start = (base as i32 + m(self, 6)) as u32;
                let kind = Kind::Script { base, obj: o, pc: start, cond: 0, dp: 0, owns_obj: false };
                match sub {
                    0 => {
                        let slot = at(m(self, 8));
                        if self.mem.u32(slot) == 0 {
                            let t = self.insert(Task { kind, slot, dtor: Dtor::Slot, kill: false });
                            self.mem.w32(slot, t);
                        }
                    }
                    1 => {
                        self.insert(Task { kind, slot: 0, dtor: Dtor::None, kill: false });
                    }
                    _ => self.unsupported_op(op, sub),
                }
            }
            0x0C => self.unsupported_op(op, sub), // native calls: no script uses them
            0x0D => {
                // model_create into the slot A; COND = 1 only when this op created one
                let (bank, idx) = match sub {
                    0 => (m(self, 4), m(self, 6)),
                    1 => (m(self, 4), self.mem.s32(at(m(self, 6)))),
                    _ => (self.mem.s32(at(m(self, 4))), m(self, 6)),
                };
                self.set_cond(h, 0);
                if self.mem.u32(a) == 0 {
                    let bank = Self::own_group(base, bank as usize);
                    let rec = self.model_groups.get(bank).and_then(|g| g.get(idx as usize)).cloned().flatten();
                    if let Some(rec) = rec {
                        // handle: +0 → the model MATRIX (identity) at +0x20
                        let hm = self.alloc_obj(0x40);
                        self.mem.w32(hm, hm + 0x20);
                        for k in 0..3 {
                            self.mem.w16(hm + 0x20 + 8 * k, 0x1000);
                        }
                        let rec_at = (bank, idx as usize);
                        self.models.push((hm, ModelSlot { inst: crate::fight::fx::effect_models::Instance::new(rec, m(self, 8)), slot: a, rec: rec_at }));
                        self.mem.w32(a, hm);
                        self.set_cond(h, 1);
                    }
                }
            }
            0x0E => return self.timeline(h, base, o, pc, f, depth),
            0x0F => self.op0f(base, a, pc, sub),
            0x10 => log::debug!("fxvm: debug op10.{sub:02X} at {pc:#x}"),
            _ => self.unsupported_op(op, sub),
        }
        None
    }
}

/// Instruction sizes; 0 = not a valid sub.
fn op_size(op: u8, sub: u8) -> u32 {
    let s = sub as usize;
    match op {
        0x00 => match s {
            0..=5 => 8,
            6..=0xB => 4,
            0xC..=0x26 => 8,
            0x27..=0x29 => 4,
            _ => 0,
        },
        0x01 => {
            if s <= 0x1A {
                12
            } else {
                0
            }
        }
        0x02 => match s {
            2 | 5 | 8 | 0xB | 0x14 | 0x17 | 0x19 | 0x1A => 12,
            0..=0x1B => 8,
            _ => 0,
        },
        0x03 => match s {
            0 | 1 | 6 | 7 | 0x11 | 0x12 | 0x13 => 4,
            0x19 => 12,
            2..=0x18 => 8,
            _ => 0,
        },
        0x04 => {
            if s <= 0x0F {
                8
            } else {
                0
            }
        }
        0x05 => match s {
            0x0C | 0x0D => 4,
            0x0E => 12,
            0..=0x0B => 8,
            _ => 0,
        },
        0x06 => match s {
            0..=2 => 8,
            3..=6 => 12,
            _ => 0,
        },
        0x08 => match s {
            0 => 4,
            1 | 2 => 8,
            _ => 0,
        },
        0x09 => {
            if s <= 3 {
                12
            } else {
                0
            }
        }
        0x0A => match s {
            0 | 5 => 12,
            1..=4 => 8,
            6 => 4,
            _ => 0,
        },
        0x0B => match s {
            0 => 12,
            1 => 8,
            _ => 0,
        },
        0x0C => match s {
            0 | 1 => 8,
            2 | 3 => 12,
            4 => 16,
            _ => 0,
        },
        0x0D => {
            if s <= 2 {
                12
            } else {
                0
            }
        }
        0x0E => {
            if s == 0 {
                8
            } else {
                0
            }
        }
        0x0F => match s {
            0 | 2 | 3 | 4 => 8,
            1 => 12,
            _ => 0,
        },
        0x10 => match s {
            0 | 1 => 8,
            2 => 4,
            _ => 0,
        },
        _ => 0,
    }
}
