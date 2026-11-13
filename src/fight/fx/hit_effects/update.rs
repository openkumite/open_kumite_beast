//! One hit-effect task's frame (`update`).
//!
//! State: none of its own: steps the serialised `Fx`
//!
//! One task's frame (`update`, called by `tick` in list order): UV animations, timelines,
//! sprites, shards, ring parents and children
//! and the flash. One match over `Kind`, as one function.

use super::{Draw, Fx, Kind, LIST_RING, Prim, RingChild, RingRec, Task, rcos, rsin};
use crate::base::libc_rand::rand;
use crate::formats::tables::Tables;

impl Fx {
    /// Returns true when the task kills itself.
    pub(super) fn update(&mut self, tables: &Tables, task: &mut Task) -> bool {
        let frozen = self.frozen;
        let buf = self.buf;
        match &mut task.kind {
            Kind::UvAnim(a) => {
                if !frozen {
                    a.timer -= 1;
                }
                if a.timer > 0 {
                    return false;
                }
                a.cur += 1;
                loop {
                    let key = a.track + 4 * a.cur as u32;
                    let fr = tables.abs_u16(key);
                    if fr & 0xFF00 == 0 {
                        break;
                    }
                    let n = (fr & 0xFF) as i16;
                    match fr & 0xFF00 {
                        0xFE00 => a.cur = n as i32,
                        0xFC00 => {
                            if n < a.loop_n {
                                a.cur += 1;
                            } else {
                                a.loop_n += 1;
                                a.cur = tables.abs_i16(key + 2) as i32;
                            }
                        }
                        0xFF00 => {
                            if a.loop_n < n {
                                a.cur += 1;
                            } else {
                                let sid = a.sprite;
                                if let Some(s) = self.sprite_mut(sid) {
                                    s.alive = false;
                                }
                                return true;
                            }
                        }
                        _ => return true,
                    }
                }
                let key = a.track + 4 * a.cur as u32;
                let f = tables.abs_u16(key) as u32;
                let uv = a.uvtab + 4 * f;
                let (u, v, w, h) = (tables.abs_u8(uv), tables.abs_u8(uv + 1), tables.abs_u8(uv + 2), tables.abs_u8(uv + 3));
                let uvs = [[u, v], [u.wrapping_add(w), v], [u, v.wrapping_add(h)], [u.wrapping_add(w), v.wrapping_add(h)]];
                a.timer = tables.abs_i16(key + 2);
                let sid = a.sprite;
                if let Some(s) = self.sprite_mut(sid) {
                    for p in &mut s.prims {
                        p.uv = uvs;
                    }
                }
                false
            }
            Kind::Timeline(t) => {
                if frozen {
                    return false;
                }
                loop {
                    let (f, count, idx) = (tables.abs_i16(t.cur), tables.abs_i16(t.cur + 2), tables.abs_i16(t.cur + 4));
                    match f {
                        0x1000 => {
                            if t.loop_n < count {
                                t.cur -= 6;
                                t.frame = tables.abs_i16(t.cur);
                                t.loop_n += 1;
                            } else {
                                t.cur += 6;
                                t.loop_n = 0;
                            }
                        }
                        0x2000 => {
                            if t.loop_n < count {
                                t.cur = t.start + 6 * idx as u32;
                                t.frame = tables.abs_i16(t.cur);
                                t.loop_n += 1;
                            } else {
                                t.cur += 6;
                                t.loop_n = 0;
                            }
                        }
                        0x3000 => {
                            t.cur = t.start + 6 * count as u32;
                            t.loop_n = 0;
                            t.frame = tables.abs_i16(t.cur);
                        }
                        0x4000 => return true,
                        _ => break,
                    }
                }
                let mut spawns = 0;
                while t.frame == tables.abs_i16(t.cur) {
                    spawns += tables.abs_i16(t.cur + 2).max(0);
                    t.cur += 6;
                }
                t.frame += 1;
                for _ in 0..spawns {
                    self.emit(tables, t); // normal size: the size-mode word is 0
                }
                false
            }
            Kind::Flash(f) => {
                let c = [0, 1, 2].map(|k| (f.base[k] + ((f.t * f.delta[k]) >> 12)).clamp(-0x8000, 0x7FFF));
                self.flash = Some(c);
                if f.t == 0 {
                    self.flash = None;
                    self.flash_task = None;
                    return true;
                }
                if !frozen {
                    f.frame += 1;
                    if f.frame >= 11 {
                        f.t = (f.t - 0x88).max(0);
                    }
                }
                false
            }
            Kind::Sprite(s) => {
                match s.kind {
                    6 | 7 => {
                        if !frozen {
                            let t = if s.kind == 6 { &mut s.ex[0] } else { &mut s.ex[4] };
                            *t -= 1;
                            if *t <= 0 {
                                return true;
                            }
                            for c in &mut s.prims[buf].rgb {
                                *c = c.saturating_sub(8);
                            }
                        }
                    }
                    _ => {
                        if !s.alive {
                            return true;
                        }
                    }
                }
                if let Some((side, bone)) = s.follow {
                    // kind 3: the joint's position (low halves) re-read every frame
                    s.pos = self.bone(side, bone).map(|v| v as i16);
                }
                let pos = s.pos.map(|v| v as i32);
                let prim = s.prims[buf];
                match s.kind {
                    0 | 3 => self.draw(Draw::Square { pos, size: s.size, prim, sorted: false }),
                    5 => {
                        if !frozen {
                            for k in 0..3 {
                                s.pos[k] = s.pos[k].wrapping_add(s.ex[k]);
                                s.ex[k] -= s.ex[k] / 8;
                            }
                            s.ex[1] += 1;
                        }
                        let pos = s.pos.map(|v| v as i32);
                        self.draw(Draw::Square { pos, size: s.size, prim, sorted: true });
                    }
                    1 => {
                        if !frozen {
                            s.ex[0] = s.ex[0].wrapping_add(s.ex[1]);
                            s.ex[1] -= 0x32;
                            if s.ex[1] < 0 {
                                s.ex[1] = 0;
                            }
                            s.size += 4;
                            for c in &mut s.prims[buf].rgb {
                                *c = c.wrapping_sub(0x18);
                            }
                        }
                        let prim = s.prims[buf];
                        self.draw(Draw::Rot { pos, size: s.size, angle: s.ex[0] as i32, prim });
                    }
                    4 => self.draw(Draw::Square { pos, size: s.size, prim, sorted: false }),
                    8 | 15 | 18 => {
                        if !frozen {
                            if s.kind == 18 {
                                for k in 0..3 {
                                    s.pos[k] = s.pos[k].wrapping_add(s.ex[k]);
                                }
                                s.size += 16;
                            }
                            if s.kind != 15 {
                                s.ex[7] += 1;
                                let idx = ((s.ex[7] - 1) / 5).min(3) as usize;
                                let c = tables.table(super::sprites::HIT_SPRITES).u16(0x3CC + 2 * idx);
                                for p in &mut s.prims {
                                    p.clut = c;
                                }
                            }
                        }
                        let pos = s.pos.map(|v| v as i32);
                        self.draw(Draw::Square { pos, size: s.size, prim: s.prims[buf], sorted: true });
                    }
                    10..=13 => {
                        if !frozen {
                            for k in 0..3 {
                                s.pos[k] = s.pos[k].wrapping_add(s.ex[k]);
                            }
                            s.ex[1] += 2;
                            if s.size >= 0x33 {
                                s.size -= 1;
                            }
                            if s.pos[1] >= 0 {
                                s.ex[1] = -(s.ex[1] / 2 + s.ex[1] / 4);
                            }
                        }
                        let pos = s.pos.map(|v| v as i32);
                        self.draw(Draw::Square { pos, size: s.size, prim, sorted: true });
                    }
                    14 => {
                        if !frozen {
                            s.ex[3] += 1;
                            if s.ex[3] == 3 {
                                s.ex[3] = 0;
                                let p = s.pos;
                                self.sprite_spawn(tables, p, 15, 0, 0, 0);
                            }
                            for k in 0..3 {
                                s.pos[k] = s.pos[k].wrapping_add(s.ex[k]);
                                s.ex[k] -= s.ex[k] / 4;
                            }
                        }
                        let pos = s.pos.map(|v| v as i32);
                        self.draw(Draw::Square { pos, size: s.size, prim, sorted: true });
                    }
                    6 => self.draw(Draw::Floor { pos, size: s.size, prim }),
                    7 => self.draw(Draw::Wall { pos, p2: s.p2.map(|v| v as i32), prim }),
                    _ => {}
                }
                false
            }
            Kind::Shards(d) => {
                if !frozen {
                    d.frame += 1;
                    if d.emitted < d.n {
                        let mut i = d.emitted;
                        while i < d.emitted + 2 {
                            if d.emitted < d.n {
                                let idx = i as usize;
                                let (clut, u0) = if d.alt { (0x7918, 0x60) } else { (0x7818, 0x40) };
                                d.prims[idx] =
                                    Prim { uv: [[u0, 0x80], [u0 + 8, 0x80], [u0, 0x88], [u0 + 8, 0x88]], rgb: [0x7F; 3], clut, tpage: 0x3F, semi: true };
                                let pa = d.pitch - 0x200 + (rand(&mut self.seed) & 0x3FF);
                                let ya = d.yaw - 0x200 + (rand(&mut self.seed) & 0x3FF);
                                let sc = rand(&mut self.seed) & 0xBFF;
                                d.pos[idx] = d.origin;
                                let mut v = [(rcos(ya) >> 8) as i16, (rsin(pa) >> 8) as i16, (rsin(ya) >> 8) as i16];
                                for c in &mut v {
                                    *c = (*c as i32 * d.power / 16) as i16;
                                    *c = ((*c as i32 * sc) >> 12) as i16;
                                }
                                d.vel[idx] = v;
                                d.life[idx] = 0x3C;
                                d.emitted += 1;
                            }
                            i += 1;
                        }
                    }
                }
                let groups = (d.emitted / 3) as usize;
                let mut out = Vec::new();
                for p in 0..groups * 3 {
                    if d.life[p] == 0 {
                        continue;
                    }
                    let before = d.pos[p].map(|v| v as i32);
                    if !frozen {
                        d.life[p] -= 1;
                        for k in 0..3 {
                            d.pos[p][k] = d.pos[p][k].wrapping_add(d.vel[p][k]);
                        }
                    }
                    out.push(Draw::Shard { pos: before, life: d.life[p] as i32, prim: d.prims[p] });
                }
                let done = d.frame >= 0x38;
                for o in out {
                    self.draw(o);
                }
                done
            }
            Kind::RingParent(p) => {
                if !frozen {
                    let rd = |o: usize| self.rings.get(o..o + 2).map(|s| i16::from_le_bytes([s[0], s[1]])).unwrap_or(-1);
                    if rd(p.sched) >= 0 {
                        let mut children = Vec::new();
                        if p.frame == rd(p.sched) {
                            loop {
                                let n = rd(p.sched + 2) as usize;
                                let ro = p.recs + 18 * (n.max(1) - 1);
                                let v: Vec<i16> = (0..9).map(|k| rd(ro + 2 * k)).collect();
                                let r =
                                    RingRec { life: v[0], rad: v[1], d_rad: v[2], wid: v[3], d_wid: v[4], ang: v[5], d_ang: v[6], step: v[7], d_step: v[8] };
                                if r.life >= p.life {
                                    p.life = r.life + 1;
                                }
                                children.push(r);
                                p.sched += 4;
                                if p.frame != rd(p.sched) {
                                    break;
                                }
                            }
                        }
                        p.frame += 1;
                        let clut = if p.beast { 0x7A5A } else { 0x7A9A };
                        let pos = p.pos;
                        let uvs = tables.table(crate::fight::fx::move_effects::MOVE_EFFECTS);
                        for r in children {
                            let prims = core::array::from_fn(|q| {
                                let a = 8 * q;
                                Prim { uv: [0, 1, 2, 3].map(|c| [uvs.u8(a + 2 * c), uvs.u8(a + 2 * c + 1)]), rgb: [0xE0; 3], clut, tpage: 0x39, semi: true }
                            });
                            self.spawn(LIST_RING, Kind::RingChild(RingChild { pos, r, prims }));
                        }
                    } else {
                        p.life -= 1;
                        if p.life < 0 {
                            return true;
                        }
                    }
                }
                false
            }
            Kind::RingChild(c) => {
                if !frozen {
                    c.r.life -= 1;
                    if c.r.life < 0 {
                        return true;
                    }
                    c.r.rad = c.r.rad.wrapping_add(c.r.d_rad);
                    c.r.ang = c.r.ang.wrapping_add(c.r.d_ang);
                    c.r.step = c.r.step.wrapping_add(c.r.d_step);
                    c.r.wid = c.r.wid.wrapping_add(c.r.d_wid);
                }
                let d = Draw::Ring {
                    pos: c.pos.map(|v| v as i32),
                    rad: c.r.rad as i32,
                    wid: c.r.wid as i32,
                    ang: c.r.ang as i32,
                    step: c.r.step as i32,
                    prims: c.prims,
                };
                self.draw(d);
                false
            }
        }
    }
}
