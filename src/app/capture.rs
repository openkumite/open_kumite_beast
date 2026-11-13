//! Desktop capture hooks (engine additions): the F7 screenshot, OKB_SHOTS / OKB_FESHOTS frame dumps, OKB_VRAMDUMP.

use super::debug_env::DebugEnv;
use crate::present::fight_scene;
use crate::{frontend, output, render};
use winit::event_loop::ActiveEventLoop;

/// F7: the frame drawn now (overlay included) saved as `path` (`okb_shot_<time>.png`; a OKB_DEBUGKEYS or
/// OKB_UISHOTS shot's name): the fight, the team battle (T6; its view carries the overlay as a match's), or `front` =
/// the front end's primitives (or the replay list's); the path saved, if any.
pub(super) fn screenshot(
    path: String,
    r: Option<&mut render::Renderer>,
    g: Option<fight_scene::Game>,
    team: Option<fight_scene::TeamGame>,
    front: Option<Vec<output::prim::Prim>>,
    overlay: &[output::prim::Prim],
) -> Option<String> {
    let saved = match (r, g, team, front) {
        (Some(r), Some(g), _, _) => g.draw_capture(r, true),
        (Some(r), None, Some(t), _) => t.draw_capture(r, true),
        (Some(r), None, None, Some(mut p)) => {
            p.extend(overlay.iter().copied());
            r.ambient = output::layout::ambient_split(&p, r.viewport());
            let quads = output::layout::to_quads(&p, r.viewport());
            r.frame_capture(&[], &quads, &[], [0.0; 3], true)
        }
        _ => None,
    };
    let (w, h, px) = saved?;
    let _ = image::save_buffer(&path, &px, w, h, image::ExtendedColorType::Rgba8);
    Some(path)
}

/// Draw the fight; OKB_SHOTS=n,m,…: save those frames as `okb_frame_<n>.png`, with a OKB_SCRIPT (OKB_NETSCRIPT) quit 5
/// frames after the last.
pub(super) fn fight(env: &DebugEnv, shot_done: &mut u64, g: fight_scene::Game, r: &mut render::Renderer, el: &ActiveEventLoop) {
    let want = env.shots.contains(&g.fight.m.frame) && *shot_done != g.fight.m.frame;
    if let Some((w, h, px)) = g.draw_capture(r, want) {
        let path = format!("okb_frame_{:05}.png", g.fight.m.frame);
        let _ = image::save_buffer(&path, &px, w, h, image::ExtendedColorType::Rgba8);
        *shot_done = g.fight.m.frame;
        log::info!("saved {path}");
    }
    if env.shots.iter().max().is_some_and(|&m| g.fight.m.frame > m + 5) && (!env.script.is_empty() || !env.netscript.is_empty()) {
        el.exit();
    }
}

/// Draw the team battle (engine addition, the debug start `OKB_TEAM`): OKB_SHOTS as `fight`, by the battle's frame.
pub(super) fn team(env: &DebugEnv, shot_done: &mut u64, g: fight_scene::TeamGame, r: &mut render::Renderer, el: &ActiveEventLoop) {
    let frame = g.team.frame;
    let want = env.shots.contains(&frame) && *shot_done != frame;
    if let Some((w, h, px)) = g.draw_capture(r, want) {
        let path = format!("okb_frame_{frame:05}.png");
        let _ = image::save_buffer(&path, &px, w, h, image::ExtendedColorType::Rgba8);
        *shot_done = frame;
        log::info!("saved {path}");
    }
    if env.shots.iter().max().is_some_and(|&m| frame > m + 5) && !env.script.is_empty() {
        el.exit();
    }
}

/// Draw the front end (over the kept demo stage when it wants one); OKB_FESHOTS=n,m,…: save the first frame drawn at or
/// after each listed frame (counted from the start) as `okb_fe_<n>.png`, with OKB_FEEXIT quit 5 frames after the last.
pub(super) fn front(
    env: &DebugEnv,
    shot_done: &mut u64,
    f: &frontend::Front,
    backdrop: Option<fight_scene::Game>,
    r: &mut render::Renderer,
    quads: &[render::PsxQuad],
    el: &ActiveEventLoop,
) {
    let shots = &env.feshots;
    let n = f.frame_no();
    // the first frame drawn at or after each listed frame
    let next = next_front(env, *shot_done);
    let want = next.is_some_and(|s| n >= s);
    let shot = match (backdrop, f.backdrop_cam()) {
        (Some(g), Some(cam)) => g.draw_stage(r, cam, n as u64, quads, want),
        _ => r.frame_capture(&[], quads, &[], [0.0, 0.0, 0.0], want),
    };
    if let Some((w, h, px)) = shot {
        let path = format!("okb_fe_{n:05}.png");
        let _ = image::save_buffer(&path, &px, w, h, image::ExtendedColorType::Rgba8);
        *shot_done = next.unwrap_or(n) as u64 + (1 << 40);
        log::info!("saved {path}");
    }
    if shots.iter().max().is_some_and(|&m| n > m + 5) && env.feexit {
        el.exit();
    }
}

/// The OKB_FESHOTS frame saved next (`front`; OKB_FEFAST runs up to it).
pub(super) fn next_front(env: &DebugEnv, shot_done: u64) -> Option<u32> {
    env.feshots.iter().copied().filter(|&s| shot_done == u64::MAX || s as u64 + (1 << 40) > shot_done).min()
}

/// OKB_VRAMDUMP=path: the front end's VRAM written there on every upload.
pub(super) fn vram_dump(env: &DebugEnv, words: &[u16]) {
    if let Some(path) = &env.vramdump {
        let _ = std::fs::write(path, bytemuck::cast_slice::<u16, u8>(words));
    }
}
