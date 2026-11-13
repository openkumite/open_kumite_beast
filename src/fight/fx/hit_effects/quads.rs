//! The hit effects' draw-list build: recorded draws as PS1 quads through one ordering table.
//!
//! State: none of its own: steps the serialised `Fx`
//!
//! The draw-list build: the frame's recorded draws and the effect VM's billboards as PS1 quads through one
//! ordering table, projected with the current camera (`Gte`).

use super::{Draw, Fx, Gte, Prim, rcos, rsin};
use crate::output::psx_quad::PsxQuad;

impl Fx {
    // ------------------------------------------------------------------ drawing

    /// The recorded draws as PS1 quads for the current camera, in ordering-table order (back to front).
    /// The PS1 quads of this frame: the native effect draws plus the effect VM's billboards (`extra`), sorted
    /// through one ordering table.
    pub fn quads(&self, gte: &Gte, extra: &[crate::fight::fx::script_vm::Billboard], models: &[crate::fight::fx::effect_models::ModelPrim]) -> Vec<PsxQuad> {
        let mut out: Vec<(i32, usize, PsxQuad)> = Vec::new();
        let mut put = |slot: i32, n: usize, prim: &Prim, xy: [[f32; 2]; 4]| {
            let z = gte.slot_depth(slot);
            let pos = xy.map(|p| gte.clip(p, z));
            out.push((
                slot,
                n,
                PsxQuad {
                    pos,
                    uv: prim.uv.map(|p| [p[0] as f32, p[1] as f32]),
                    rgb: [prim.rgb; 4],
                    tpage: prim.tpage,
                    clut: prim.clut,
                    textured: true,
                    semi: prim.semi,
                    sheet: None,
                    clip: crate::output::layout::Clip::No,
                },
            ));
        };
        let culled = |otz: i32| ((otz - 4) as u32) >= 0x80B;
        for (n, d) in &self.draws {
            match d {
                Draw::Square { pos, size, prim, sorted } => {
                    let Some(p) = gte.rtps(*pos) else { continue };
                    if *sorted && culled(p.otz) {
                        continue;
                    }
                    let s0 = (*size as f32 * gte.h / (p.otz.max(1) as f32 * 4.0)).trunc();
                    let a1 = (s0 / 2.0).trunc();
                    let (x0, y0) = (p.sx - a1, p.sy - a1);
                    put(if *sorted { p.otz } else { 3 }, *n, prim, [[x0, y0], [x0 + s0, y0], [x0, y0 + s0], [x0 + s0, y0 + s0]]);
                }
                Draw::Rot { pos, size, angle, prim } => {
                    let Some(p) = gte.rtps(*pos) else { continue };
                    if culled(p.otz) {
                        continue;
                    }
                    let s0 = (*size as f32 * gte.h / (p.otz as f32 * 4.0)).trunc() as i32;
                    let a = *angle;
                    let (c0, s0a) = (((s0 * rcos(a)) >> 12) as f32, ((s0 * rsin(a)) >> 12) as f32);
                    let (c1, s1) = (((s0 * rcos(a + 0x400)) >> 12) as f32, ((s0 * rsin(a + 0x400)) >> 12) as f32);
                    let xy = [[p.sx + c0, p.sy + s0a], [p.sx + c1, p.sy + s1], [p.sx - c1, p.sy - s1], [p.sx - c0, p.sy - s0a]];
                    put(3, *n, prim, xy);
                }
                Draw::Floor { pos, size, prim } => {
                    let Some(p) = gte.rtps(*pos) else { continue };
                    if culled(p.otz) {
                        continue;
                    }
                    let s0 = (*size as f32 * gte.h / (p.otz as f32 * 4.0)).trunc() as i32;
                    let a3 = (s0 * rsin(gte.pitch + 0xA0)) >> 12;
                    let (a1, a2) = ((s0 / 2) as f32, (a3 / 2) as f32);
                    let (x0, y0) = (p.sx - a1, p.sy - a2);
                    let (w, h) = (s0 as f32, a3 as f32);
                    put(p.otz, *n, prim, [[x0, y0], [x0 + w, y0], [x0, y0 + h], [x0 + w, y0 + h]]);
                }
                Draw::Wall { pos, p2, prim } => {
                    let (Some(p), Some(q)) = (gte.rtps(*pos), gte.rtps(*p2)) else { continue };
                    let (w, h) = ((p.sx - q.sx).trunc(), (p.sy - q.sy).trunc());
                    if w >= 0.0 || culled(p.otz) {
                        continue;
                    }
                    let (hw, hh) = ((w / 2.0).trunc(), (h / 2.0).trunc());
                    let (x0, y0) = (p.sx - hw, p.sy - hh);
                    put(p.otz, *n, prim, [[x0, y0], [x0 + w, y0], [x0, y0 + h], [x0 + w, y0 + h]]);
                }
                Draw::Shard { pos, life, prim } => {
                    let Some(p) = gte.rtps(*pos) else { continue };
                    if ((p.otz - 10) as u32) >= 0x7F6 {
                        continue;
                    }
                    let (h, q) = ((life / 8) as f32, (life / 4) as f32);
                    let (x0, y0) = (p.sx - h, p.sy - h);
                    put(p.otz, *n, prim, [[x0, y0], [x0 + q, y0], [x0, y0 + q], [x0 + q, y0 + q]]);
                }
                Draw::Ring { pos, rad, wid, ang, step, prims } => {
                    let Some(p) = gte.rtps(*pos) else { continue };
                    let otz = p.otz.max(1) as f32;
                    let mut w = (*wid as f32 * gte.h / otz).trunc() as i32;
                    let r = (*rad as f32 * gte.h / otz).trunc() as i32;
                    let r3 = r / 3;
                    let mut a = *ang;
                    // interlaced geometry (square pixels): y uses >>12 like x
                    let mut q = [[0f32; 6]; 4];
                    for k in 0..4 {
                        q[k] = [
                            p.sx + ((r * rcos(a)) >> 12) as f32,
                            p.sy + ((r * rsin(a)) >> 12) as f32,
                            p.sx + (((r + w) * rcos(a)) >> 12) as f32,
                            p.sy + (((r + w) * rsin(a)) >> 12) as f32,
                            p.sx + ((r3 * rcos(a)) >> 12) as f32,
                            p.sy + ((r3 * rsin(a)) >> 12) as f32,
                        ];
                        w -= w / 8;
                        a += *step;
                    }
                    for k in 0..3 {
                        let (i0, i1) = (q[k], q[k + 1]);
                        put(3, *n, &prims[2 * k], [[i1[2], i1[3]], [i1[0], i1[1]], [i0[2], i0[3]], [i0[0], i0[1]]]);
                        put(3, *n, &prims[2 * k + 1], [[i1[0], i1[1]], [i1[4], i1[5]], [i0[0], i0[1]], [i0[4], i0[5]]]);
                    }
                }
            }
        }
        // VM billboards (op05.09 / 0A / 0B): square of (w << 9) / otz pixels (240-line units, doubled
        // here for the 480-line space), culled outside 0x11..=0x81F, linked at OT[otz] (or the fixed otz of 0B)
        for (k, b) in extra.iter().enumerate() {
            let Some(p) = gte.rtps(b.pos) else { continue };
            let otz = b.fixed_otz.unwrap_or(p.otz);
            if b.fixed_otz.is_none() && !(0x11..=0x81F).contains(&otz) {
                continue;
            }
            let size = (((b.w << 9) / otz.max(1)) * 2) as f32;
            let half = (size / 2.0).trunc();
            let (x0, y0) = (p.sx - half, p.sy - half);
            let prim = Prim { uv: b.uv, rgb: b.rgb, clut: b.clut, tpage: b.tpage, semi: b.semi };
            put(otz, 1_000_000 + k, &prim, [[x0, y0], [x0 + size, y0], [x0, y0 + size], [x0 + size, y0 + size]]);
        }
        // 3-D effect models (list 13): already projected, at their sorted or fixed OT slot
        for (k, mp) in models.iter().enumerate() {
            let z = gte.slot_depth(mp.otz);
            let pos = mp.xy.map(|p| gte.clip(p, z));
            out.push((
                mp.otz,
                2_000_000 + k,
                PsxQuad {
                    pos,
                    uv: mp.uv.map(|p| [p[0] as f32, p[1] as f32]),
                    rgb: mp.rgb,
                    tpage: mp.tpage,
                    clut: mp.clut,
                    textured: mp.textured,
                    semi: mp.semi,
                    sheet: None,
                    clip: crate::output::layout::Clip::No,
                },
            ));
        }
        // ordering table: far slots first; within a slot the prim linked last is drawn first
        out.sort_by(|a, b| b.0.cmp(&a.0).then(b.1.cmp(&a.1)));
        out.into_iter().map(|(_, _, q)| q).collect()
    }
}
