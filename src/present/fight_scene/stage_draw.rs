//! The stage's draw: the stage objects, the ordering-table stage, the backdrop, the shadow floor.
//!
//! State: presentation (reads the state, never writes it)
//!
//! The stage's draw: the stage alone for the attract ranking (`draw_stage`), the per-stage routine's
//! objects and CLUT rows (`stage_anim`), the ordering-table stage (`stage_ot`, `ot_units`), the semi-transparent
//! polygons, the Beast Drive line grids (M.9F), the backdrop and the shadow floor.

use super::FightView;
use crate::fight::fight_data::Loaded;
use crate::fight::scene::SceneRef;
use crate::formats::gltf_model;
use crate::output::layout::Split;
use crate::render::{GpuModel, Instance, Renderer};
use glam::{Mat4, Vec3};

/// Floor height (game units, y down) under the point (x, z) of stage `stage`: its record of the table
/// `stage.shadow_floor` (8 bytes: shape, height off the ring, half extents x / z; stages ST0..ST9): the ground test gives 0
/// over the ring (|x| <= hx and |z| <= hz, game units) and the lower floor's height outside it.
pub(super) fn shadow_floor(tables: &crate::formats::tables::Tables, stage: usize, x: f32, z: f32) -> f32 {
    let t = tables.table("stage.shadow_floor");
    let o = 8 * stage.min(9);
    let (h, hx, hz) = (t.i16(o + 2), t.i16(o + 4), t.i16(o + 6));
    if x.abs() <= hx as f32 && z.abs() <= hz as f32 { 0.0 } else { h as f32 }
}

impl FightView {
    /// The stage alone under a free camera with `quads` on top (the attract ranking's background):
    /// stage model and backdrop and the walls; no fighters, effects or HUD.
    pub fn draw_stage(
        &self,
        s: &SceneRef,
        data: &Loaded,
        r: &mut Renderer,
        cam: &crate::fight::camera::Camera,
        n: u64,
        quads: &[crate::render::PsxQuad],
        capture: bool,
    ) -> Option<(u32, u32, Vec<u8>)> {
        let to_game = Mat4::from_scale(Vec3::new(300.0, -300.0, -300.0));
        let vp = cam.projection(r.viewport()) * cam.view(&data.trig) * to_game;
        let dim = s.darkening[2] as f32 / 255.0;
        r.set_instance_sub(&self.stage_inst, vp, super::stage_xf(data), dim);
        let mut items = if data.stage_ot { Vec::new() } else { vec![(&self.stage_gpu, &self.stage_inst)] };
        let view = crate::fight::stage::anim::View::Frame(n);
        items.extend(self.stage_anim(data, r, &view, cam.pos, vp, dim));
        items.extend(self.semi(data, r, vp, dim));
        items.extend(self.stage_ot(s, data, r, &view, cam, cam.ang[1], &[], cam.view(&data.trig) * to_game, vp, dim));
        let w = &s.stage.walls;
        r.walls_hidden = (0..4u8).filter(|&k| !w.model.contains(&(k as i16))).fold(0, |m, k| m | 1 << k);
        if s.custom.flags & 0x40 != 0 {
            r.walls_hidden = 0xF;
        }
        if data.stage_no == 4 {
            r.walls_hidden = 0; // ST4's routine draws objects 0..3 (ceiling borders, no wall table) every frame
        }
        r.afterimage = 0;
        r.backdrop = Some(backdrop(data, cam, r.viewport(), s.darkening[2] as i32));
        // a capped 3D view: the ambient fill beside it, copied after the ranking text and its fade (a fade over the
        // darker fill would reach black first); those window-space quads mapped into the band's viewport
        let layout = *r.viewport();
        r.ambient = layout.scene_capped().then(|| Split { at: quads.len(), rect: layout.scene_rect(), scene: true });
        let mut band = Vec::new();
        let quads = if layout.scene_capped() {
            band.extend_from_slice(quads);
            crate::output::layout::to_scene(&mut band, &layout);
            &band[..]
        } else {
            quads
        };
        let k = 1.0 - dim as f64;
        // RENDER QUALITY (engine addition): the stage scaled, the ranking text at the window's resolution
        r.scaled_at = Some(0);
        r.frame_capture(&items, quads, &[], [0.05 * k, 0.05 * k, 0.08 * k], capture)
    }

    /// The stage draw's per-stage routine under `view`: the CLUT rows of the paletted textures
    /// and the stage objects drawn this frame with their matrices (`stage_anim`); `cam` = the camera pivot.
    pub(super) fn stage_anim(
        &self,
        data: &Loaded,
        r: &Renderer,
        view: &crate::fight::stage::anim::View,
        cam: [i16; 3],
        vp: Mat4,
        dim: f32,
    ) -> Vec<(&GpuModel, &Instance)> {
        use crate::fight::stage::anim::{PAL_X, PAL_Y};
        for &(mi, ii, clut) in &self.pal_tex {
            let gm = if mi == 0 { &self.stage_gpu } else { &self.debris_gpu[mi - 1] };
            if let Some((x, y)) = view.clut_source(data.stage_no, clut) {
                r.set_palette_row(gm, ii, [x - PAL_X, y - PAL_Y]);
            }
        }
        let mut out = Vec::new();
        for (k, gm, inst) in &self.obj_gpu {
            if let Some(xf) = crate::fight::stage::anim::object_xf(data.stage_no, *k, view, cam, &data.trig) {
                r.set_instance_sub(inst, vp, super::team_scale(data) * xf, dim);
                out.push((gm, inst));
            }
        }
        out
    }

    /// The stage of a `Loaded::stage_ot` export as the PS1 ordering table draws it: the far ring walls
    /// (0x80F), the units the stage routine sorts (`stage::anim::sorts`) and the wall `debris` pieces (0x81A, with their
    /// matrices) by OT slot, the farthest first, and inside a slot in reverse link order (the sorter links each
    /// packet at the head of its entry, so the GPU draws the last one first). Slot 0x80F comes after the fighter
    /// shadows (0x810). `yaw`: the camera yaw the walls' far test sees; `anim`: the stage routine's phase; `view`: the
    /// camera's view matrix in game units (the GTE's, for the floor tiles' depth).
    #[allow(clippy::too_many_arguments)]
    pub(super) fn stage_ot(
        &self,
        s: &SceneRef,
        data: &Loaded,
        r: &Renderer,
        anim: &crate::fight::stage::anim::View,
        cam: &crate::fight::camera::Camera,
        yaw: i16,
        debris: &[(usize, Mat4)],
        view: Mat4,
        vp: Mat4,
        dim: f32,
    ) -> Vec<(&GpuModel, &Instance)> {
        if !data.stage_ot {
            return Vec::new();
        }
        // (slot, object / FLOOR or debris piece, matrix)
        let mut links: Vec<(u16, Result<u8, usize>, Mat4)> = Vec::new();
        // each side whose wall stands (hp > 0, a model) and is on the far side of the camera
        // (`drive_grid::far`), shifted by its shake, at M.8 + 0x203C; WALL DISPLAY OFF draws none
        let w = &s.stage.walls;
        for k in (0..4).filter(|_| s.custom.flags & 0x40 == 0) {
            if w.hp[k] <= 0 || w.model[k] < 0 || !crate::present::drive_grid::far(k, yaw) {
                continue;
            }
            let off = if k < 2 { Vec3::new(w.offset[k] as f32, 0.0, 0.0) } else { Vec3::new(0.0, 0.0, -w.offset[k] as f32) };
            // (the team battle's walls, T7: the shake in game units, before the stage's x / z scale)
            let off = off * data.team.as_ref().map_or(1.0, |t| 100.0 / t.stage_scale.max(1) as f32);
            links.push((0x80F, Ok(w.model[k] as u8), super::team_scale(data) * Mat4::from_translation(off / 300.0)));
        }
        // the stage routine (by M.98, after the walls); W+0xC = the stage's object count, here derived from
        // the export (its highest object unit + 1), not read from the game's stage data
        let count = self.ot_units.iter().filter(|(u, ..)| *u != crate::formats::gltf_model::FLOOR).map(|(u, ..)| u + 1).max().unwrap_or(0);
        let slots = data.tables.table("stage.object_slots");
        let empty = |k: u8| k < 23 && slots.u8(23 * data.stage_no + k as usize) == 0xFF;
        // the team battle's stage is scaled in x / z (`team_scale`, after the routine's matrices): the objects that
        // follow the camera (the sky, the skyline) get its place in the unscaled stage, so that scaled they sit where
        // a match's do relative to the camera
        let cam_pos = match &data.team {
            Some(t) => {
                let k = |v: i16| (v as i32 * 100 / t.stage_scale.max(1) as i32) as i16;
                [k(cam.pos[0]), cam.pos[1], k(cam.pos[2])]
            }
            None => cam.pos,
        };
        for (slot, u, xf) in crate::fight::stage::anim::sorts(data.stage_no, anim, count, &empty, cam_pos, &data.trig) {
            links.push((slot, Ok(u), super::team_scale(data) * xf));
        }
        links.extend(debris.iter().map(|&(k, xf)| (0x81A, Err(k), xf)));
        // the polygons the sorter's drivers and the GPU drop under this camera (`stage_cull`)
        if let Some(c) = &self.cull {
            let drawn: Vec<(u8, Mat4)> = links.iter().filter_map(|&(_, u, xf)| Some((u.ok()?, xf))).collect();
            c.cull(r, &self.stage_gpu, &drawn, view, &super::stage_cull::Screen::new(cam, r.viewport()));
        }
        links.reverse();
        links.sort_by_key(|l| std::cmp::Reverse(l.0));
        let mut out = Vec::new();
        for (slot, u, xf) in links {
            match u {
                Ok(u) => {
                    let Some((_, range, inst)) = self.ot_units.iter().find(|(k, ..)| *k == u) else { continue };
                    if u == crate::formats::gltf_model::FLOOR {
                        // the tiles' CLUT rows by their view depth SZ (`view`: game units, the GTE's view)
                        let depth = (view * xf).row(2).to_array();
                        r.set_instance_floor(inst, vp, xf, dim, depth, data.stage_no >= 9);
                    } else {
                        r.set_instance_sub(inst, vp, xf, dim);
                    }
                    inst.set_ot(Some(*range), slot < 0x810);
                    out.push((&self.stage_gpu, inst));
                }
                Err(k) => {
                    let inst = &self.debris_inst[k];
                    r.set_instance(inst, vp, xf, &[]);
                    inst.set_ot(None, false);
                    out.push((&self.debris_gpu[k], inst));
                }
            }
        }
        out
    }

    /// The stage's semi-transparent polygons, after the rest of the stage (before the fighters).
    pub(super) fn semi(&self, data: &Loaded, r: &Renderer, vp: Mat4, dim: f32) -> Option<(&GpuModel, &Instance)> {
        let (gm, inst) = self.semi_gpu.as_ref()?;
        r.set_instance_semi(inst, vp * super::team_scale(data), dim);
        Some((gm, inst))
    }

    /// The stage draw with M.9F set: each present wall (hp > 0) shifted by its shake offset, behind the fighters
    /// (ot 0x80F) or, near the camera, over them (ot 6); then the floor (ot 0x81D) with the offset of the last
    /// wall drawn (none drawn: the plain view translation).
    pub(super) fn drive_grid_prims(&self, s: &SceneRef, gte: &crate::fight::fx::hit_effects::Gte, out: &mut Vec<crate::fight::fx::effect_models::ModelPrim>) {
        let Some(g) = &self.drive_grid else { return };
        let w = &s.stage.walls;
        let yaw = s.cam.ang[1];
        let mut off = [0; 3];
        for k in 0..4 {
            if w.hp[k] <= 0 {
                continue;
            }
            if s.custom.flags & 0x40 != 0 {
                continue; // WALL DISPLAY OFF (assumed to be the W+0x20 bit-0 test): skips before the wall draw
            }
            off = if k < 2 { [w.offset[k], 0, 0] } else { [0, 0, w.offset[k]] };
            let far = crate::present::drive_grid::far(k, yaw);
            crate::present::drive_grid::prims(&g[[0, 1, 3, 2][k]], off, gte, if far { 0x80F } else { 6 }, out);
        }
        crate::present::drive_grid::prims(&g[4], off, gte, 0x81D, out);
    }
}

/// The units of a stage in ordering-table order (`gltf_model::Prim::ot`): unit and its range of draws (prims), in model
/// order. An export with primitives of no unit, or a unit in more than one run, is warned about (engine addition):
/// those primitives, or the unit's later runs, are not drawn (`stage_ot` takes a unit's first run).
pub(super) fn ot_units(m: &gltf_model::ModelData) -> Vec<(u8, (u32, u32))> {
    let mut out: Vec<(u8, (u32, u32))> = Vec::new();
    for (i, p) in m.prims.iter().enumerate() {
        let Some(u) = p.ot else { continue };
        match out.last_mut() {
            Some((v, range)) if *v == u && range.1 == i as u32 => range.1 += 1,
            _ => out.push((u, (i as u32, i as u32 + 1))),
        }
    }
    let untagged = m.prims.iter().filter(|p| p.ot.is_none()).count();
    if untagged > 0 {
        log::warn!("stage OT export: {untagged} primitives without a unit (`_ot_<unit>` node), not drawn");
    }
    for (k, (u, _)) in out.iter().enumerate() {
        if out[..k].iter().any(|(v, _)| v == u) {
            log::warn!("stage OT export: unit {u} in more than one run of primitives, only the first is drawn");
        }
    }
    out
}

/// The stage's backdrop under camera `cam` at darkening level `level`.
pub(super) fn backdrop(
    data: &Loaded,
    cam: &crate::fight::camera::Camera,
    layout: &crate::output::layout::Viewport,
    level: i32,
) -> crate::present::backdrop::Backdrop {
    let gte = crate::fight::fx::hit_effects::Gte::new(cam.view(&data.trig), cam.projection(layout), layout.fov3d(), cam.ang[0] as i32);
    // display y (0 = the screen's centre) → frame-buffer row
    crate::present::backdrop::backdrop(&data.tables, &data.trig, data.stage_no, cam.ang[1], level, |p| gte.rtps(p).map(|q| q.sy + 240.0))
}
