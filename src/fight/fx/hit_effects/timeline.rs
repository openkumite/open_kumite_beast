//! Emitters: the spawn timelines and the emitter handler.
//!
//! State: none of its own: steps the serialised `Fx`
//!
//! Emitters: emitter_spawn, a spawn timeline task (run by `update`), and the emitter
//! handler that spawns its sprites by mode.

use super::{Fx, Kind, LIST_ANIM, Timeline, atan_xz};
use crate::base::libc_rand::rand;
use crate::formats::tables::Tables;

impl Fx {
    /// emitter_spawn: sprites of `kind` from `pos` over a timeline, spawn `mode`;
    /// `follow` = the joint the source follows when mode bit 7 is set. Stale caller fields are 0.
    pub(crate) fn emitter_spawn(&mut self, pos: [i16; 3], follow: Option<(usize, usize)>, kind: i32, mode: i32, timeline: u32, e: [i16; 4]) {
        let follow = if mode & 0x80 != 0 { follow } else { None };
        self.spawn(LIST_ANIM, Kind::Timeline(Timeline { pos, kind, mode, follow, e, start: timeline, cur: timeline, frame: 0, loop_n: 0 }));
    }

    /// One spawn of an emitter.
    pub(super) fn emit(&mut self, tables: &Tables, t: &mut Timeline) {
        if let Some((side, bone)) = t.follow {
            t.pos = self.bone(side, bone).map(|v| v as i16);
        }
        let cur = t.pos;
        let e = &mut t.e;
        match t.mode & 0x7F {
            0 => self.sprite_spawn(tables, cur, t.kind as i16, e[0] as i32, e[1] as i32, 0x20_0000 | e[2] as i32),
            1 => {
                let x = cur[0] as i32 - 0x80 + (rand(&mut self.seed) & 0xFF);
                let y = if cur[1] > 0 { cur[1] as i32 } else { -(rand(&mut self.seed) & 0x7F) };
                let z = cur[2] as i32 - 0x80 + (rand(&mut self.seed) & 0xFF);
                self.sprite_spawn(tables, [x as i16, y as i16, z as i16], t.kind as i16, 0, 0, 0);
            }
            2 => {
                let k = 10 + (rand(&mut self.seed) & 3);
                self.sprite_spawn(tables, cur, k as i16, e[0] as i32, e[1] as i32, t.kind);
            }
            3 => {
                e[2] = ((e[2] as i32 + 0x378) & 0xBFF) as i16;
                self.sprite_spawn(tables, cur, 14, e[0] as i32, e[1] as i32, 0x1000);
                e[0] = e[0].wrapping_add(0x2AA);
                e[1] = e[1].wrapping_add(0x555);
            }
            14 => {
                e[2] = e[2].wrapping_add(1);
                e[3] = e[3].wrapping_add(0x80);
                let a = e[3] as i32 + ((e[2] as i32 & 3) << 10);
                let (f, o) = (self.bone(e[0] as usize & 3, 3), self.bone(e[1] as usize & 3, 3));
                let dir = atan_xz(o[0] - f[0], o[2] - f[2]);
                self.sprite_spawn(tables, cur, 18, a, dir - 0x400, 0);
            }
            31 => self.sprite_spawn(tables, cur, t.kind as i16, 0, 0, 0),
            _ => {}
        }
    }
}
