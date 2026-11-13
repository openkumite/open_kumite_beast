//! The team battle on screen (engine addition): its world, the player markers and target rings, its HUD.
//!
//! State: presentation (reads the state, never writes it)
//!
//! `FightView::draw_team` draws a `TeamFight`: the world as a match's (`draw_world` over `TeamFight::scene`: the
//! stage scaled with the ring, four fighters, the effects, bank 1's in VRAM bank 1), a ring on the floor under each
//! human player's target in that player's colour (with the effects: hidden by what stands in front of it), a "P1" ..
//! "P4" marker over each fighter still in the round, the `TeamHud` and the round message.

use super::FightView;
use super::scene::World;
use crate::fight::fight_data::Loaded;
use crate::fight::team::{N, TeamFight, hud::colour};
use crate::output::font::pal;
use crate::output::gte::Gte;
use crate::output::layout::Viewport;
use crate::output::prim::{Prim, f4};
use crate::render::{PsxQuad, Renderer};

/// A marker's colour for font palette `p` (`fight::team::hud::colour`).
fn rgb(p: u16) -> [u8; 3] {
    match p {
        pal::RED => [255, 56, 40],
        pal::BLUE => [56, 128, 255],
        pal::GREEN => [48, 232, 72],
        _ => [255, 224, 32],
    }
}

/// The target ring's radius and width (game units).
const RING_R: i32 = 260;
const RING_W: i32 = 40;
/// The marker's height over the fighter's head (game units).
const MARK_UP: i32 = 0x30;

/// The quads of a floor ring of `RING_R` around `c` (game units, y down) in colour `col`: additive, each segment
/// at its own depth (`Gte::slot_depth` of its OT slot, as an effect quad).
fn ring(gte: &Gte, c: [i32; 3], col: [u8; 3], out: &mut Vec<PsxQuad>) {
    const SEGS: i32 = 24;
    let at = |a: i32, r: i32| {
        let p = [c[0] + ((r * crate::base::portable::rsin(a + 0x400)) >> 12), c[1], c[2] + ((r * crate::base::portable::rsin(a)) >> 12)];
        gte.rtps(p)
    };
    for k in 0..SEGS {
        let (a0, a1) = (k * 4096 / SEGS, (k + 1) * 4096 / SEGS);
        let (Some(p0), Some(p1), Some(q0), Some(q1)) = (at(a0, RING_R), at(a1, RING_R), at(a0, RING_R + RING_W), at(a1, RING_R + RING_W)) else {
            continue;
        };
        let z = gte.slot_depth(p0.otz.max(p1.otz));
        out.push(PsxQuad {
            pos: [[p0.sx, p0.sy], [p1.sx, p1.sy], [q0.sx, q0.sy], [q1.sx, q1.sy]].map(|v| gte.clip(v, z)),
            uv: [[0.0; 2]; 4],
            rgb: [col; 4],
            // B + F (tpage bits 5-6 = 1)
            tpage: 0x20,
            clut: 0,
            textured: false,
            semi: true,
            sheet: None,
            clip: crate::output::layout::Clip::No,
        });
    }
}

/// A point of the 3D view's clip space (`Gte::clip`) in clip space of the drawn area (`layout::from_scene`): where the
/// HUD's quads go.
fn view_clip(p: [f32; 4], vp: &Viewport) -> [f32; 2] {
    crate::output::layout::from_scene([p[0], p[1]], vp)
}

impl FightView {
    /// A team battle's frame (`TeamFight`), as `draw_capture` draws a match's; `config` the pause's open CONTROLLER
    /// CONFIG. screen (T6).
    pub fn draw_team(
        &self,
        t: &TeamFight,
        data: &Loaded,
        config: Option<&crate::frontend::menus::Menu>,
        r: &mut Renderer,
        capture: bool,
    ) -> Option<(u32, u32, Vec<u8>)> {
        let layout = *r.viewport();
        let s = t.scene();
        let World { items, mut fx, dim, gte } = self.draw_world(&s, data, r, &layout);
        let show = !t.round.hud_hidden && !self.hide_hud;
        // the human players' targets: a ring on the floor in the player's colour, drawn with the effects
        if show {
            for k in (0..N).filter(|&k| !t.cpu[k] && !t.out(k)) {
                let g = t.target[k];
                if t.out(g) {
                    continue;
                }
                let m = t.fighters[g].m_t;
                let c = [m[0], t.world.stage.ring.floor(m[0], m[2]), m[2]];
                ring(&gte, c, rgb(colour(t.mode, k)), &mut fx);
            }
        }
        if let Some(a) = &mut r.ambient {
            a.at = fx.len();
        }
        r.scaled_at = Some(fx.len());
        // "P1" .. "P4" over each fighter in the round, with a pointer under it
        if show {
            let font = crate::output::font::fonts();
            for k in (0..N).filter(|&k| !t.out(k)) {
                let f = &t.fighters[k];
                let Some(p) = gte.rtps([f.head[0], f.head[1] - MARK_UP, f.head[2]]) else { continue };
                let [cx, cy] = view_clip(gte.clip([p.sx, p.sy], 0.0), &layout);
                // the marker built over (256, 240) of HUD space, its pointer's tip 4 lines above, then moved to the fighter
                let mut ps: Vec<Prim> = Vec::new();
                let label = format!("P{}", k + 1);
                font.small_text(&mut ps, &label, 256 - font.width(&font.small, &label) / 2, 240 - 34, colour(t.mode, k));
                let mut tip = f4(250, 228, 12, 8, rgb(colour(t.mode, k)));
                tip.xy = [[250.0, 228.0], [262.0, 228.0], [256.0, 236.0], [256.0, 236.0]];
                ps.push(tip);
                for mut q in crate::output::layout::to_quads(&ps, &layout) {
                    for v in &mut q.pos {
                        v[0] += cx;
                        v[1] += cy;
                    }
                    fx.push(q);
                }
            }
        }
        // the HUD and the round message (as a match's)
        let hud_layout = layout.with_hud(crate::output::layout::hud_fit(&t.hud.prims, &layout, r.hud_scale));
        if show {
            fx.extend(crate::output::layout::to_quads(&t.hud.prims, &hud_layout));
        }
        if !self.hide_hud {
            fx.extend(crate::output::layout::to_quads(&crate::fight::hud::round_message(t.round.message), &layout));
        }
        fx.extend(crate::output::layout::to_quads(&s.fxn.sprites, &layout));
        // the pause menu (T4: the match's, in front of everything)
        let held = t.raw_pads.iter().fold(0, |a, p| a | p);
        // (T6) CONTROLLER CONFIG. greyed without a human on pad 1 or 2 (`TeamFight::pause_step`), its screen over the pause
        let greyed = t.config_greyed(data.gates());
        fx.extend(crate::output::layout::to_quads(&t.pause.draw(crate::output::font::fonts(), false, greyed, held), &layout));
        if let Some(menu) = config {
            let mut p = Vec::new();
            menu.draw(crate::output::font::fonts(), &mut p);
            fx.extend(crate::output::layout::to_quads(&p, &layout));
        }
        fx.extend(crate::output::layout::to_quads(&self.overlay, &layout));
        let k = 1.0 - dim as f64;
        r.frame_capture(&items, &fx, &[], [0.05 * k, 0.05 * k, 0.08 * k], capture)
    }
}
