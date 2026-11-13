//! Billboard sprites: their spawn, the bone-following sprite and the debris spark's destructor.
//!
//! State: none of its own: steps the serialised `Fx`
//!
//! Billboard sprites: sprite_spawn with its kinds, the bone-following lightning sprite
//! and the debris spark's destructor. Their per-frame update is `update.rs`'s `Kind::Sprite` arm.

use super::{Fx, Kind, LIST_ANIM, LIST_SPRITE, Prim, Sprite, UvAnim, atan_xz, rcos, rsin};
use crate::base::libc_rand::rand;
use crate::formats::tables::Tables;

/// The hit sprites' data (`fx.hit_sprites`): per sprite kind its size (+0), CLUT (+0x28) and texture page (+0x5C), the
/// dust chips' CLUT selector per stage (+0x50), the UV animations' key tracks and rectangle tables, the blood drops'
/// timelines (+0x32C, +0x340) and the flame puff's CLUT cycle (+0x3CC). Its parts link to each other by address (the
/// table's origin + offset, kept in the tasks: `Tables::abs_*` follows them).
pub(super) const HIT_SPRITES: &str = "fx.hit_sprites";
const CLUT: usize = 0x28;
const CHIP_SEL: usize = 0x50;
const TPAGE: usize = 0x5C;

impl Fx {
    /// sprite_spawn kind 3: a lightning sprite following BONEPOS(`bone`) of fighter `side`;
    /// `clut` != 0 selects CLUT row 480 + clut.
    pub(crate) fn sprite_follow(&mut self, tables: &Tables, side: usize, bone: usize, clut: i32) {
        let (hs, k) = (tables.table(HIT_SPRITES), 3);
        let mut base = Prim { uv: [[0; 2]; 4], rgb: [0xFF; 3], clut: hs.u16(CLUT + 2 * k), tpage: hs.u16(TPAGE + 2 * k), semi: true };
        if clut != 0 {
            base.clut = ((((clut + 0x1E0) & 0x1FF) << 6) | 0x1A) as u16;
        }
        let size = 0x60 + (rand(&mut self.seed) & 0x7F);
        let pos = self.bone(side, bone).map(|v| v as i16);
        let (uvtab, track) = (hs.origin() + 0x1CC, hs.origin() + 0x1A8);
        let uv = uvtab + 4 * tables.abs_u16(track) as u32;
        let (u, v, w, h) = (tables.abs_u8(uv), tables.abs_u8(uv + 1), tables.abs_u8(uv + 2), tables.abs_u8(uv + 3));
        base.uv = [[u, v], [u.wrapping_add(w), v], [u, v.wrapping_add(h)], [u.wrapping_add(w), v.wrapping_add(h)]];
        let s = Sprite { kind: 3, prims: [base; 2], alive: true, size, pos, ex: [0; 8], p2: [0; 3], follow: Some((side, bone)) };
        let id = self.spawn(LIST_SPRITE, Kind::Sprite(s));
        self.spawn(LIST_ANIM, Kind::UvAnim(UvAnim { sprite: id, uvtab, track, cur: -1, timer: 0, loop_n: 0 }));
    }

    /// sprite_spawn(pos, kind, p2, p3, p4) for kinds 0, 1, 5, 6, 7.
    pub(crate) fn sprite_spawn(&mut self, tables: &Tables, pos: [i16; 3], kind: i16, p2: i32, p3: i32, p4: i32) {
        let (hs, k) = (tables.table(HIT_SPRITES), kind as u16 as usize);
        let at = |off: u32| hs.origin() + off;
        let size = hs.i16(2 * k) as i32;
        let base = Prim { uv: [[0; 2]; 4], rgb: [0x7F; 3], clut: hs.u16(CLUT + 2 * k), tpage: hs.u16(TPAGE + 2 * k), semi: true };
        let mut s = Sprite { kind, prims: [base; 2], alive: true, size, pos, ex: [0; 8], p2: [0; 3], follow: None };
        let anim = match kind {
            0 => {
                s.size = if p2 < 10 {
                    0xA4
                } else if p2 < 15 {
                    0xC8
                } else {
                    0x104
                };
                Some((at(0x274), at(0x230)))
            }
            1 => {
                s.ex[0] = rand(&mut self.seed) as i16;
                s.ex[1] = 0x29A;
                for p in &mut s.prims {
                    p.rgb = [0xC0; 3];
                }
                Some((at(0x2D0), at(0x2B4)))
            }
            5 => {
                s.size = p4 >> 15;
                let spd = ((p4 & 0xFFFF) * ((rand(&mut self.seed) & 0x7FF) + 0x400)) >> 12;
                let yaw = p2 - 0x100 + (rand(&mut self.seed) & 0x1FF);
                let pitch = p3 - 0x100 + (rand(&mut self.seed) & 0x1FF);
                s.ex[0] = ((rcos(yaw) * spd) >> 11) as i16;
                s.ex[1] = ((rsin(pitch) * spd) >> 11) as i16;
                s.ex[2] = ((rsin(yaw) * spd) >> 11) as i16;
                for p in &mut s.prims {
                    p.rgb = [0xA0, 0xFF, 0x64];
                }
                Some((at(0x30C), at(0x2E8)))
            }
            4 => Some((at(0x210), at(0x1EC))),
            8 | 18 => {
                // flame puff: CLUT cycled by a u16 animation (age in ex[7]); kind 18 drifts
                if kind == 18 {
                    s.ex[0] = (rcos(p3) >> 7) as i16;
                    s.ex[1] = (rsin(p2) >> 9) as i16;
                    s.ex[2] = (rsin(p3) >> 7) as i16;
                }
                Some((at(0x388), at(0x354)))
            }
            10..=13 => {
                // dust chips: opaque, stage palette, thrown in a fan around `p2` / `p3`
                let mh = (rand(&mut self.seed) & 0xC63) + 0x200;
                let mv = (rand(&mut self.seed) & 0xC63) + 0x200;
                let yaw = p2 - 0x400 + (rand(&mut self.seed) & 0x7FF);
                let pitch = p3 - 0x400 + (rand(&mut self.seed) & 0x7FF);
                s.ex[0] = (((rcos(yaw) >> 7) * mh) / 4096) as i16;
                s.ex[2] = (((rsin(yaw) >> 7) * mh) / 4096) as i16;
                s.ex[1] = if p4 == 0x10 { ((rand(&mut self.seed) & 0x3F) + 0x20) as i16 } else { (((rsin(pitch) >> 7) * mv) / 4096) as i16 };
                let sel = hs.u8(CHIP_SEL + self.stage.min(11)) as usize;
                let clut = hs.u16(CLUT + 2 * (10 + sel));
                for p in &mut s.prims {
                    p.semi = false;
                    p.clut = clut;
                }
                Some((hs.u32(0x12C + 4 * (k - 10)), at(0x84)))
            }
            14 => {
                s.ex[0] = (((rcos(p2) >> 5) * p4) / 4096) as i16;
                s.ex[1] = (((rsin(p3) >> 5) * p4) / 4096) as i16;
                s.ex[2] = (((rsin(p2) >> 5) * p4) / 4096) as i16;
                let _ = rand(&mut self.seed); // ex.bit, never read
                s.ex[3] = 0;
                Some((at(0x184), at(0x170)))
            }
            15 => Some((at(0x184), at(0x194))),
            6 | 7 => {
                for p in &mut s.prims {
                    p.rgb = [0xA0, 0xFF, 0x64];
                    p.uv = [[0xE0, 0xB0], [0xEF, 0xB0], [0xE0, 0xBF], [0xEF, 0xBF]];
                }
                if kind == 6 {
                    s.ex[0] = 0x20;
                } else {
                    s.ex[4] = 0x20;
                    s.p2 = s.pos;
                    let sz = s.size as i16;
                    match p2 {
                        0 => s.p2[0] -= sz,
                        1 => s.p2[2] -= sz,
                        2 => s.p2[0] += sz,
                        3 => s.p2[2] += sz,
                        _ => {}
                    }
                    s.p2[1] += sz;
                }
                None
            }
            _ => return,
        };
        if let Some((uvtab, track)) = anim {
            // the game draws stale heap UVs on a sprite's first frame when its animation starts later;
            // key 0's rectangle stands in
            let uv = uvtab + 4 * tables.abs_u16(track) as u32;
            let (u, v, w, h) = (tables.abs_u8(uv), tables.abs_u8(uv + 1), tables.abs_u8(uv + 2), tables.abs_u8(uv + 3));
            for p in &mut s.prims {
                p.uv = [[u, v], [u.wrapping_add(w), v], [u, v.wrapping_add(h)], [u.wrapping_add(w), v.wrapping_add(h)]];
            }
        }
        let id = self.spawn(LIST_SPRITE, Kind::Sprite(s));
        if let Some((uvtab, track)) = anim {
            self.spawn(LIST_ANIM, Kind::UvAnim(UvAnim { sprite: id, uvtab, track, cur: -1, timer: 0, loop_n: 0 }));
        }
    }

    /// A kind-5 debris spark may leave a puff on the floor or the wall it lands at.
    pub(super) fn sprite_dtor(&mut self, tables: &Tables, s: &Sprite) {
        if s.kind != 5 {
            return;
        }
        if rand(&mut self.seed) & 1 != 0 {
            return;
        }
        let (hx, hz) = self.arena.half;
        let mut pos = s.pos;
        if (pos[0] as i32).abs() <= hx - 200 && (pos[2] as i32).abs() <= hz - 200 {
            pos[1] = 0;
            self.sprite_spawn(tables, pos, 6, 0, 0, 0);
            return;
        }
        let side = ((atan_xz(pos[0] as i32, pos[2] as i32) + 0x200) & 0xFFF) >> 10;
        match side {
            0 => pos[2] = -hz as i16,
            1 => pos[0] = hx as i16,
            2 => pos[2] = hz as i16,
            _ => pos[0] = -hx as i16,
        }
        self.sprite_spawn(tables, pos, 7, side, 0, 0);
    }
}
