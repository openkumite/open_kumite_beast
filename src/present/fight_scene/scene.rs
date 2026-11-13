//! A fight's frame (`FightView::draw_capture`): camera, stage, fighters, effects, HUD.
//!
//! State: presentation (reads the state, never writes it)
//!
//! A fight's frame (`FightView::draw_capture`): the game's camera (and the debug orbit), the stage, the
//! fighters (pose, light set, fade, shadow) and the wall debris, the effects through
//! the GTE, then the HUD, the status labels, the attract demo's overlay and the pause menu.

use super::FightView;
#[cfg(not(target_arch = "wasm32"))]
use super::debug_hud::old_hud;
use super::stage_draw::{backdrop, shadow_floor};
use crate::fight::fight_data::{Loaded, clip_for_vs};
use crate::fight::scene::SceneRef;
use crate::formats::gltf_model;
use crate::output::gte::Gte;
use crate::output::layout::{Split, Viewport};
use crate::render::{GpuModel, Instance, Lighting, PsxQuad, Renderer};
use crate::session::fight::Fight;
use glam::{Mat4, Vec3};

/// What `FightView::draw_world` drew: the models in drawing order, the effect quads (the HUD goes after them), the
/// stage darkening level (0..1) and the effects' projection.
pub(super) struct World<'v> {
    pub items: Vec<(&'v GpuModel, &'v Instance)>,
    pub fx: Vec<PsxQuad>,
    pub dim: f32,
    pub gte: Gte,
}

impl FightView {
    pub fn draw_capture(&self, f: &Fight, data: &Loaded, r: &mut Renderer, capture: bool) -> Option<(u32, u32, Vec<u8>)> {
        let layout = *r.viewport();
        let (p0, p1) = (f.m.world_pos(0), f.m.world_pos(1));
        if std::env::var("CAMDBG").is_ok() && f.m.frame.is_multiple_of(60) {
            eprintln!(
                "frame {} p0 {p0:?} p1 {p1:?} right {:?} moves {:#x} {:#x} pad {:04x} held {:04x} yaw {}",
                f.m.frame, f.m.right, f.m.fighters[0].move_cur, f.m.fighters[1].move_cur, f.m.pads[0], f.m.fighters[0].held, f.m.fighters[0].yaw
            );
        }
        let s = f.m.scene(data);
        let World { items, mut fx, dim, .. } = self.draw_world(&s, data, r, &layout);
        #[cfg(not(target_arch = "wasm32"))]
        let hud = if crate::base::env::var("OLDHUD").is_ok() { old_hud(&f.m, layout.view_rect()[2] as f32, layout.view_rect()[3] as f32) } else { Vec::new() };
        #[cfg(target_arch = "wasm32")]
        let hud: Vec<([f32; 4], [f32; 4])> = Vec::new();
        // HUD SIZE (engine addition): the HUD's anchored groups scaled as far as they fit
        let hud_layout = layout.with_hud(crate::output::layout::hud_fit(&f.hud.prims, &layout, r.hud_scale));
        // the game's HUD and round messages (ot[0x80E] / ot[4] / ot[0]: in front of the scene)
        if !f.m.round.hud_hidden && !self.hide_hud {
            fx.extend(crate::output::layout::to_quads(&f.hud.prims, &hud_layout));
        }
        if !self.hide_hud {
            fx.extend(crate::output::layout::to_quads(&f.hud.message(f.m.round.message), &layout));
        }
        // the status labels (before the pause check: drawn while paused, not gated on the HUD)
        let dc = self.dc.get().wrapping_add(1);
        self.dc.set(dc);
        let cpu = [0, 1].map(|i| f.m.cpu[i]);
        let cont = matches!(f.m.round.state, 0xF | 0x10).then_some(f.m.round.loser);
        let labels = crate::fight::hud::status_labels(
            crate::output::font::fonts(),
            &f.labels,
            cpu,
            f.m.round.state,
            f.m.round.hud_hidden,
            f.m.replay.mode == 3,
            f.m.play_frames,
            dc,
            cont,
            f.m.frame,
        );
        if !self.hide_hud {
            fx.extend(crate::output::layout::to_quads(&labels, &hud_layout));
        }
        if f.demo {
            // the exit fade over the frozen fight, the prompt while fighting (48 of 128
            // frames), DEMONSTRATION on top
            let mut p = Vec::new();
            match f.demo_fade {
                Some(k) => p.push(crate::output::prim::f4(0, 0, 512, 480, [(8 * (k + 1)).min(255) as u8; 3]).blend(crate::output::sprite::Blend::Sub)),
                None => {
                    if (dc >> 4) & 3 != 0 && dc & 0x40 != 0 {
                        crate::output::font::fonts().small_text(
                            &mut p,
                            crate::assets::text::get("press_start_or_x_button"),
                            187,
                            360,
                            crate::output::font::pal::YELLOW,
                        );
                    }
                }
            }
            let s0 = p.len();
            crate::output::font::fonts().small_text(&mut p, crate::assets::text::get("demonstration"), 32, 32, crate::output::font::pal::ORANGE);
            crate::output::layout::tag(&mut p[s0..], crate::fight::hud::side_anchor(0, crate::output::layout::Ax::Start));
            fx.extend(crate::output::layout::to_quads(&p, &layout));
        }
        // the lens flare's 2-D glows (ot[0])
        fx.extend(crate::output::layout::to_quads(&s.fxn.sprites, &layout));
        // the pause menu (ot[0], in front of everything)
        let watch = f.m.cpu[0] && f.m.cpu[1];
        let held = f.m.pads[0] | f.m.pads[1];
        fx.extend(crate::output::layout::to_quads(&f.m.pause.draw(crate::output::font::fonts(), f.m.practice, watch, held), &layout));
        if let Some(menu) = &f.config {
            let mut p = Vec::new();
            menu.draw(crate::output::font::fonts(), &mut p);
            fx.extend(crate::output::layout::to_quads(&p, &layout));
        }
        fx.extend(crate::output::layout::to_quads(&self.overlay, &layout));
        if let (Some(a), Some(_)) = (&mut r.ambient, f.demo_fade) {
            // the demo's exit fade covers the HUD too: the fill is copied after it, the HUD drawn in the band's viewport
            crate::output::layout::to_scene(&mut fx[a.at..], &layout);
            a.at = fx.len();
        }
        let k = 1.0 - dim as f64;
        r.frame_capture(&items, &fx, &hud, [0.05 * k, 0.05 * k, 0.08 * k], capture)
    }

    /// The world of a frame (a match's or the team battle's, `s`): the camera, the stage, the fighters with their
    /// shadows, the debris and walls, the effects; the renderer's split for the HUD after them set.
    pub(super) fn draw_world<'v>(&'v self, s: &SceneRef, data: &Loaded, r: &mut Renderer, layout: &Viewport) -> World<'v> {
        let layout = *layout;
        // the game's camera: game units, y down; glTF space = (x, −y, −z) / 300
        let to_game = Mat4::from_scale(Vec3::new(300.0, -300.0, -300.0));
        let (mut view_game, proj_game) = (s.cam.view(&data.trig), s.cam.projection(&layout));
        if let (Some((_, fx)), Some(b)) = (&self.br1_fx, s.br1) {
            // The imported game's camera shake (engine addition, the import S8: its words in the view's translation)
            let [x, y] = fx.borrow_mut().shake(&b, s.frame);
            view_game = Mat4::from_translation(Vec3::new(x as f32, y as f32, 0.0)) * view_game;
        }
        if self.cam_yaw != 0.0 || self.cam_dist != 0.0 {
            // debug orbit / zoom (Q / E / Z / X)
            view_game = Mat4::from_translation(Vec3::new(0.0, 0.0, self.cam_dist * 300.0)) * view_game * Mat4::from_rotation_y(self.cam_yaw);
        }

        let vp = proj_game * view_game * to_game;
        let cam_pitch = s.cam.ang[0] as i32;
        // the stage darkening (set every frame by the Beast Drive darkening — Beast Drive +16, else −8 — and stepped by the fade step):
        // the backdrop colours and the clear colour are base − L,
        // and a subtract tile of (L − 64)·256/192 covers the stage; the stage model (geometry and backdrop in one)
        // gets base − L. At L = 255 (M.9F) the stage draw draws the line grids instead of the stage (below).
        let dim = s.darkening[2] as f32 / 255.0;
        r.set_instance_sub(&self.stage_inst, vp, super::stage_xf(data), dim);
        // AFTERIMAGE MODE (M.9E): the stage draw skips the walls and the whole stage and the frame skips the backdrop,
        // so only fighters and effects land on the never-cleared, slowly darkened screen (trails)
        let afterimage = s.m9e != 0;
        let m9f = s.darkening[2] == 255;
        // the backdrop (M.9E skips it and the frame flip's ClearImage): the horizon of the game's camera,
        // not of the debug orbit. At L = 255 (M.9F, not drawn) both colours are 0 already.
        // (the extra stage without its map, `Loaded::arena`: the stand-in's backdrop over an empty stage model)
        r.backdrop = (!afterimage && self.arena.is_none() && data.br1_stage.is_none()).then(|| backdrop(data, s.cam, &layout, s.darkening[2] as i32));
        // The imported game's effects (engine addition, the import S8): stepped to this frame; their background flash
        // colours the backdrop
        let gte = crate::fight::fx::hit_effects::Gte::new(view_game, proj_game, layout.fov3d(), cam_pitch);
        let br1_flash = match (&self.br1_fx, s.br1) {
            (Some((t, fx)), Some(b)) => {
                let mut fx = fx.borrow_mut();
                fx.advance(t, &b, s.frame, &gte, s.cam.ang[1] as i32);
                fx.flash()
            }
            _ => None,
        };
        let br1_l3 = self.br1_fx.as_ref().and_then(|(_, fx)| fx.borrow().light3());
        let br1_back = self.br1_fx.as_ref().map_or([[None; 3]; crate::present::br1_fx::SIDES], |(_, fx)| std::array::from_fn(|p| fx.borrow().back(p)));
        if let Some((bg, _, _)) = self.br1_bg.as_ref().filter(|_| !afterimage) {
            // an imported stage's screen clear colour (the draw environment's, from its band
            // record +0x14), seen wherever neither the scene nor the background's strip and bands cover; darkened by
            // the main game's level L as `backdrop` darkens its colours (each byte − L, below 0 → 0)
            let l = s.darkening[2] as i32;
            let c = bg.clear().map(|v| (v as i32 - l).max(0) as u8);
            r.backdrop = Some(crate::output::backdrop::Backdrop { top: c, bottom: c, rows: 480 });
        }
        if let Some(c) = br1_flash.filter(|_| !afterimage) {
            r.backdrop = Some(crate::output::backdrop::Backdrop { top: c, bottom: c, rows: 480 });
        }
        let mut items: Vec<(&GpuModel, &Instance)> = match &self.arena {
            // an extra stage (engine addition): its sky and map instead of a main-game stage model and backdrop
            Some(a) if !(afterimage || m9f) => {
                // (T6) the team battle's zoomed-out camera sees its courtyard through what stands in front of it
                let ring = &s.stage.ring;
                let clip = crate::fight::stage::extra::by_stage(data.stage_id)
                    .filter(|_| data.team.is_some())
                    .and_then(|e| super::arena::courtyard_clip(e, view_game, [ring.hx, ring.hz], data.gates()));
                a.items(r, view_game, proj_game, vp, dim, s.frame as f32 / 60.0, clip)
            }
            // (the extra stage without its map: its empty stage model is not drawn)
            _ if afterimage || m9f || data.stage_ot || crate::fight::stage::extra::by_stage(data.stage_id).is_some() => Vec::new(),
            _ => vec![(&self.stage_gpu, &self.stage_inst)],
        };
        // an imported stage's background (engine addition, the import S8: the strip's draw from the main game's camera)
        if let Some((bg, gpu, inst)) = self.br1_bg.as_ref().filter(|_| !(afterimage || m9f) && br1_flash.is_none()) {
            // the floor's vanishing line: the camera's forward direction laid flat, at infinity
            let fwd = view_game.inverse().transform_vector3(Vec3::Z);
            let clip = proj_game * view_game * glam::Vec4::new(fwd.x, 0.0, fwd.z, 0.0);
            let horizon = if clip.w > 1e-6 { 120.0 - clip.y / clip.w * 120.0 } else { 120.0 };
            let t3 = bg.bottom(horizon, s.cam.pos[1] as i32);
            if bg.shown(t3) {
                let yaw = s.cam.ang[1].wrapping_add((-self.cam_yaw * 4096.0 / std::f32::consts::TAU).round() as i16);
                r.set_instance_sub(inst, bg.transform(layout.aspect3d(), yaw, t3), Mat4::IDENTITY, dim);
                items.insert(0, (gpu, inst));
            }
        }
        // the per-stage routine at the end of the stage draw: skipped with it
        let view = s.stage_view;
        let objs = self.stage_anim(data, r, &view, s.cam.pos, vp, dim);
        // wall debris pieces (the debris update links them at the fixed slot 0x81A)
        let ang = |a: i32| a as f32 * std::f32::consts::TAU / 4096.0;
        let debris: Vec<(usize, Mat4)> = (s.stage.debris.iter().enumerate())
            .filter(|(k, p)| p.alive != 0 && *k < self.debris_gpu.len().min(self.debris_inst.len()))
            // The imported game's debris (simulation v18) blinks while it settles: drawn at 0x10 or on odd counts
            .filter(|(_, p)| !(data.br1_stage.is_some() && data.gates().br1_debris) || p.alive >= 0x10 || p.alive & 1 != 0)
            .map(|(k, p)| {
                let xf = Mat4::from_translation(Vec3::new(p.pos[0] as f32, -p.pos[1] as f32, -p.pos[2] as f32) / 300.0)
                    * Mat4::from_rotation_y(ang(p.rot[1]))
                    * Mat4::from_rotation_x(-ang(p.rot[0]))
                    * super::stage_xf(data);
                (k, xf)
            })
            .collect();
        let ot_stage = data.stage_ot && !(afterimage || m9f);
        if !(afterimage || m9f) {
            items.extend(objs);
            items.extend(self.semi(data, r, vp, dim));
            // the debug orbit (OKB_CAMYAW, Q / E) turns the world by −D: the walls' far test sees the yaw it shows
            let yaw = s.cam.ang[1].wrapping_add((-self.cam_yaw * 4096.0 / std::f32::consts::TAU).round() as i16);
            items.extend(self.stage_ot(s, data, r, &view, s.cam, yaw, &debris, view_game * to_game, vp, dim));
        }
        for i in 0..s.fighters.len() {
            let fi = &s.fighters[i];
            if fi.flags[crate::fight::fighter::F60] & 2 != 0 {
                continue; // F60 bit 1: not drawn
            }
            let form = fi.beast() as usize;
            let m = &data.fighters[i].models[form];
            // an imported fighter (engine addition, the import S8): posed from its core's world, in world space
            if let (Some(side), Some(core)) = (&self.br1[i], s.br1.filter(|b| b.side[i]).map(|b| &b.fs[i])) {
                let form = core.cur as usize & 1;
                let m = &data.fighters[i].models[form];
                let (joints, mask) = {
                    let mut ch = side.chains.borrow_mut();
                    let c = &mut ch[form];
                    let key = (s.frame, s.round_index);
                    match c.last {
                        Some((fr, r)) if r == key.1 as u8 && fr <= key.0 && key.0 - fr <= 4 => (fr..key.0).for_each(|_| c.step(core)),
                        _ => {
                            *c = crate::present::br1_fighter::Chains::new(data.br1_res(i).expect("an imported side"), form, side.chr);
                            c.step(core);
                        }
                    }
                    c.last = Some((key.0, key.1 as u8));
                    crate::present::br1_fighter::joints(m, &side.rig[form], core, c)
                };
                let l = super::lights(data, s, i, br1_l3, br1_back);
                let c = |v: [i32; 3]| v.map(|x| x as f32 / 4096.0);
                let lighting = Lighting { dir: l.dir.map(|v| c([v[0], -v[1], -v[2]])), colour: l.colour.map(c), back: c(l.back) };
                let (level, bkout) = s.body(i);
                r.set_instance_lit(&self.fighter_inst[i], vp, Mat4::IDENTITY, Some(&lighting), &joints, [mask, mask], level);
                if !bkout {
                    items.push((&self.fighter_gpu[i][form], &self.fighter_inst[i]));
                }
                // the shadow (after the model, none on Uriko's stage G.94 8): the model flattened by the
                // light set's shadow matrix onto the floor the ground test gives with the stage's shadow
                // record (0 inside its box, its drop outside; the game asks per object, here at node
                // 0); on a main-game stage that stage's floor
                let floor_br1 = data.br1_stage.map(|s| (s.g94, s.shadow));
                if dim < 1.0 && floor_br1.is_none_or(|(g, _)| g != 8) {
                    let p = crate::present::br1_fighter::root(core) * 300.0;
                    let floor = match floor_br1 {
                        Some((_, rec)) => -(crate::fight::br1::physics::ground(p.x as i32, -p.z as i32, rec, 0) as f32) / 300.0,
                        None if data.gates().extra_bounds && data.stage_no >= 9 => -(s.stage.ring.floor(p.x as i32, -p.z as i32) as f32) / 300.0,
                        None => -shadow_floor(&data.tables, data.stage_no, p.x, -p.z) / 300.0,
                    };
                    let shear = [-l.shadow[0] as f32 / 4096.0, l.shadow[1] as f32 / 4096.0];
                    // the model's shadow lists (ok_br1_converter's shadow proxy; an older export: the whole model) in
                    // the stage's shadow colour, the light record's +0x10 (R, G, B) written into the lists at stage
                    // load (none on a main-game stage: black)
                    let gpu = &self.fighter_gpu[i][form];
                    let colour = data.br1_light.map_or([0.0; 3], |rec| [0, 1, 2].map(|k| rec[0x10 + k] as f32 / 255.0));
                    r.set_instance_shadow_br1(&self.shadow_inst[i], vp, Mat4::IDENTITY, &joints, floor, shear, [mask, mask], !gpu.has_shadow_proxy(), colour);
                    items.push((&self.fighter_gpu[i][form], &self.shadow_inst[i]));
                }
                continue;
            }
            let joints = if fi.pose_set {
                let rsqrt = data.tables.rsqrt_fn();
                let l = crate::fight::anim::pose::locals(fi, &data.fighters[i].skels[form], &data.trig, &rsqrt);
                gltf_model::pose_locals_scaled(m, &l, &fi.shape_render())
            } else {
                let clip = clip_for_vs(m, &fi.anim, data.clip_opp(s.opp[i]));
                let t = crate::fight::anim::playback::glb_time(fi, &data.acts(i, s.opp[i]));
                gltf_model::pose_scaled(m, clip, t, fi.anim.mirror, &fi.shape_render())
            };
            let o = fi.draw_off;
            let xf = Mat4::from_translation(Vec3::new(o[0], -o[1], -o[2]) / 300.0) * s.transform(i);
            // the light set: the stage's light record and hooks, the colour flash's back
            // colour, the hit flash's light #3; directions from game space (x, −y, −z)
            let l = super::lights(data, s, i, br1_l3, br1_back);
            let c = |v: [i32; 3]| v.map(|x| x as f32 / 4096.0);
            let lighting = Lighting { dir: l.dir.map(|v| c([v[0], -v[1], -v[2]])), colour: l.colour.map(c), back: c(l.back) };
            // hand objects: fists 11 / 14 and open hands 15 / 16 on joints 13 (right) / 16 (left)
            let h = fi.hand_hide[form];
            let bit = |hidden: u16, j: u32| if hidden != 0 { 1u32 << j } else { 0 };
            let hide = [bit(h[0], 13) | bit(h[1], 16), bit(h[2], 13) | bit(h[3], 16)];
            // the side's fade (before the model; op 0x2B sub 1 / 2, decide): level P.4 subtracted from the
            // fighter's palettes (a semi-transparent subtract TILE over its 32×16 CLUT block in VRAM, restored from
            // the copy below it first), so the body darkens and its colours drop out to transparent; with mode 0 at
            // level 255 (F6C 0x400 BKOUT) the body is not drawn but its shadow is
            let (level, bkout) = s.body(i);
            r.set_instance_lit(&self.fighter_inst[i], vp, xf, Some(&lighting), &joints, hide, level);
            if !bkout {
                items.push((&self.fighter_gpu[i][form], &self.fighter_inst[i]));
            }
            // the shadow (after the model; M.9F = 255 stops it with the stage)
            if dim < 1.0 {
                let p = xf.transform_point3(Vec3::ZERO) * 300.0;
                // (the extra stages' engine-made bounds from simulation v7, Q3DM17's / Q3DM6's and the team battle's
                // enlarged rings: the ground of the ring the fight uses)
                let floor = if (data.gates().extra_bounds && data.stage_no >= 9)
                    || crate::fight::stage::extra::by_stage(data.stage_id).is_some()
                    || data.team.is_some()
                {
                    -(s.stage.ring.floor(p.x as i32, -p.z as i32) as f32) / 300.0
                } else {
                    -shadow_floor(&data.tables, data.stage_no, p.x, -p.z) / 300.0
                };
                // the shadow matrix: x += m01 · y, z += m21 · y in game space (y down) = −m01 / +m21 per unit of
                // height in glTF space
                let shear = [-l.shadow[0] as f32 / 4096.0, l.shadow[1] as f32 / 4096.0];
                r.set_instance_shadow(&self.shadow_inst[i], vp, xf, &joints, floor, shear, hide);
                items.push((&self.fighter_gpu[i][form], &self.shadow_inst[i]));
            }
        }
        // wall debris pieces drawn with the stage objects (in OT order by `stage_ot`)
        for &(k, xf) in debris.iter().filter(|_| !ot_stage) {
            let (gm, inst) = (&self.debris_gpu[k], &self.debris_inst[k]);
            r.set_instance_sub(inst, vp, xf, 0.0);
            items.push((gm, inst));
        }
        // Uriko's stage's walls: wall k standing (its counter G.50 + 2k not 0)
        // draws object 5 + its model, from G.4A 0x4B object 9 + it; from simulation v18 a shattered wall (counter 0) is
        // not drawn and one coming back (1..15) drops into place from (counter − 16) · 64 above (the
        // translation's y); before v18 the walls stand (and the counter G.4A, v17, switches the set)
        // (WALL DISPLAY OFF hides them as the other walls)
        if !(afterimage || m9f) && self.br1_walls.len() == 8 && s.custom.flags & 0x40 == 0 {
            let (w, c) = (&s.stage.walls, &s.stage.br1);
            let v18 = data.gates().br1_debris;
            let set = if c.g4a < 0x4B { 0 } else { 4 };
            // (one instance per model: a model two sides name, never in the stage records' 0 1 3 2, draws once)
            let mut drawn = 0u8;
            for side in 0..4 {
                // (until the routine's first frame stands them, G.50.. hold 0: standing too)
                let n = if v18 && c.init { c.g50[side] } else { -1 };
                let Some(k) = usize::try_from(w.model[side]).ok().map(|k| k & 3) else { continue };
                if n == 0 || drawn & 1 << k != 0 {
                    continue;
                }
                drawn |= 1 << k;
                let (gm, inst) = &self.br1_walls[set + k];
                let drop = if (1..16).contains(&n) { (16 - n as i32) as f32 * 64.0 / 300.0 } else { 0.0 };
                r.set_instance_sub(inst, vp, Mat4::from_translation(Vec3::new(0.0, drop, 0.0)) * super::stage_xf(data), dim);
                items.push((gm, inst));
            }
        }
        // an imported stage's props (in its stage draw), stepped to this frame, drawn with the stage
        if let Some((props, gpu, inst)) = &self.br1_props {
            let mut p = props.borrow_mut();
            p.advance(s.frame, s.round_index as u8, s.round_state);
            if !(afterimage || m9f) {
                let pivot = s.cam.pos.map(|v| v as i32);
                for ((j, part), inst) in p.parts(gpu.len(), pivot, s.cam.zom[2] as i32).into_iter().zip(inst) {
                    r.set_instance_sub(inst, vp, crate::present::br1_props::model_matrix(&part, super::stage_xf(data)), dim);
                    items.push((&gpu[j], inst));
                }
            }
        }
        // ring walls: a wall model is drawn when some side still uses it
        let w = &s.stage.walls;
        r.walls_hidden = (0..4u8).filter(|&k| !w.model.contains(&(k as i16))).fold(0, |m, k| m | 1 << k);
        // M.9E: AFTERIMAGE MODE (1) or the special KO (2 white, 1 dark): the image is kept; walls are
        // not drawn
        r.afterimage = s.m9e;
        if r.afterimage != 0 {
            r.walls_hidden = 0xF;
        }
        if s.custom.flags & 0x40 != 0 {
            r.walls_hidden = 0xF; // WALL DISPLAY OFF: drawing only, the walls still collide and break
        }
        if data.stage_no == 4 && r.afterimage == 0 {
            r.walls_hidden = 0; // ST4's routine draws objects 0..3 (ceiling borders, no wall table) every frame
        }
        let mut models = s.vm.model_prims(&|p| gte.rtps(p).map(|q| (q.sx, q.sy, q.otz)));
        models.extend(crate::fight::fx::native_tasks::prims(s.fxn, &gte));
        models.extend(crate::fight::fx::move_effects::prims(&s.fxn.mv, &gte, s.vm.buf));
        if m9f && !afterimage {
            // linked after the streak task: drawn before the streaks in slot 0x80F
            self.drive_grid_prims(s, &gte, &mut models);
        }
        let mut fx = s.fx.quads(&gte, &s.vm.draws, &models);
        if let Some((t, b1)) = &self.br1_fx {
            let mut q = b1.borrow().quads(t, &gte);
            q.append(&mut fx);
            fx = q;
        }
        #[cfg(not(target_arch = "wasm32"))]
        if crate::base::env::var("VRAMDBG").is_ok() {
            // debug: the hit-effect sheet (tpage 0x3F: 4-bit page at (960,256)… and 0x39 (576,256)) on screen
            for (k, (tp, clut)) in [(0x3Fu16, 0x7958u16), (0x39, 0x7A5A), (0x3E, 0x7818)].iter().enumerate() {
                let x0 = -0.95 + 0.65 * k as f32;
                let (x1, y0, y1) = (x0 + 0.6, 0.9, 0.1);
                fx.push(crate::render::PsxQuad {
                    pos: [[x0, y0, 0.0, 1.0], [x1, y0, 0.0, 1.0], [x0, y1, 0.0, 1.0], [x1, y1, 0.0, 1.0]],
                    uv: [[0.0, 0.0], [255.9, 0.0], [0.0, 255.9], [255.9, 255.9]],
                    rgb: [[128, 128, 128]; 4],
                    tpage: *tp,
                    clut: *clut,
                    textured: true,
                    semi: false,
                    sheet: None,
                    clip: crate::output::layout::Clip::No,
                });
            }
        }
        // a capped 3D view (engine addition): the scene and its effects in the band (the fighters' fades are their own
        // palettes), the ambient fill beside it copied from there, the HUD over the whole window
        r.ambient = layout.scene_capped().then(|| Split { at: fx.len(), rect: layout.scene_rect(), scene: true });
        // RENDER QUALITY (engine addition): the 2D layer from here on is drawn at the window's resolution
        r.scaled_at = Some(fx.len());
        World { items, fx, dim, gte }
    }
}
