//! Drawing a frame: the fight or the front end, the screenshots, the surface size.
//!
//! Drawing a frame (`draw`): the fight, or the front end over the kept demo stage, the screenshots; the window's
//! surface size (`surface_size`, `clamp_to_monitor`) and the fight on screen (`game`).

#[cfg(not(target_arch = "wasm32"))]
use super::capture;
use super::overlay::front_prims;
use super::{App, debug_env};
use crate::output;
#[cfg(not(target_arch = "wasm32"))]
use crate::platform::support;
use crate::present::fight_scene;
use winit::event_loop::ActiveEventLoop;

/// A logical window size no larger than the monitor's (WINDOW SIZE presets; engine addition).
#[cfg(not(target_arch = "wasm32"))]
pub(super) fn clamp_to_monitor(m: Option<winit::monitor::MonitorHandle>, w: u32, h: u32) -> winit::dpi::LogicalSize<u32> {
    match m {
        Some(m) => {
            let s: winit::dpi::LogicalSize<u32> = m.size().to_logical(m.scale_factor());
            winit::dpi::LogicalSize::new(w.min(s.width.max(1)), h.min(s.height.max(1)))
        }
        None => winit::dpi::LogicalSize::new(w, h),
    }
}

impl App {
    /// OKB_WINDOW=WxH (physical): the drawn surface keeps exactly that size even when the OS clamps the window to
    /// the screen (the compositor scales it), so shots of windows wider than the display stay reproducible.
    pub(super) fn surface_size(&self) -> Option<(u32, u32)> {
        match self.env.window {
            Some(debug_env::WindowSize::Physical(w, h)) => Some((w, h)),
            _ => None,
        }
    }

    /// The fight on screen: the state's, its data and its view.
    pub(super) fn game(&self) -> Option<fight_scene::Game<'_>> {
        fight_scene::Game::of(self.store.fight.as_deref(), self.state.fight.as_ref(), self.view.as_ref())
    }

    /// Draw the state: the fight, or the front end (over the kept demo stage while it wants one).
    pub(super) fn draw(&mut self, el: &ActiveEventLoop) {
        // Android: the touch controls, drawn last over the whole window
        #[cfg(target_os = "android")]
        self.touch_screen();
        let overlay = self.overlay_prims();
        // netplay before its session: the connect screen
        if self.net_ui.before_session() {
            self.draw_net_screen(&overlay);
            return;
        }
        // debug OKB_UISHOTS: the list's / playback's frames saved as `okb_ui_<n>.png`
        #[cfg(not(target_arch = "wasm32"))]
        if self.env.uishots.contains(&self.ui_frame) && self.ui_shot_done != self.ui_frame {
            self.ui_shot_done = self.ui_frame;
            self.shot_req = true;
            if let Some(t) = &mut self.tools {
                t.shot_name = Some(format!("okb_ui_{:05}.png", self.ui_frame));
            } else {
                self.ui_shot_name = Some(format!("okb_ui_{:05}.png", self.ui_frame));
            }
        }
        // debug OKB_NETSHOTS: a netplay session's frames saved as `okb_net_<n>.png`
        #[cfg(not(target_arch = "wasm32"))]
        if self.net.is_some() && self.env.netshots.contains(&self.state.frame) && self.net_ui.shot_done != Some(self.state.frame) {
            self.net_ui.shot_done = Some(self.state.frame);
            self.shot_req = true;
            self.ui_shot_name = Some(format!("okb_net_{:05}.png", self.state.frame));
        }
        let shot = std::mem::take(&mut self.shot_req);
        let hide_hud = self.player.as_ref().is_some_and(|pb| !pb.ctl.hud);
        if let Some(v) = &mut self.view {
            v.overlay = overlay.clone();
            v.hide_hud = hide_hud;
        }
        let game = fight_scene::Game::of(self.store.fight.as_deref(), self.state.fight.as_ref(), self.view.as_ref());
        #[cfg(not(target_arch = "wasm32"))]
        if shot {
            let path = self
                .tools
                .as_mut()
                .and_then(|t| t.shot_name.take())
                .or_else(|| self.ui_shot_name.take())
                .unwrap_or_else(|| format!("okb_shot_{}.png", support::stamp()));
            let front = self.state.front.as_ref().map(|f| front_prims(&self.browser, &self.net_ui.menu, f));
            let ui_shot = path.starts_with("okb_ui_") || path.starts_with("okb_net_");
            let team = fight_scene::TeamGame::of(
                self.store.fight.as_deref(),
                self.state.team.as_deref(),
                self.view.as_ref(),
                self.state.team_config.as_ref().map(|c| &c.menu),
            );
            if let Some(path) = capture::screenshot(path, self.renderer.as_mut(), game, team, front, &overlay) {
                log::info!("debug: screenshot {path}");
                // a scripted one leaves the next frames as they are
                if !ui_shot {
                    self.toast = Some((format!("screenshot {path}"), 180));
                }
            }
            return;
        }
        #[cfg(target_arch = "wasm32")]
        if shot {
            self.toast = Some(("screenshots: use the browser (desktop build saves PNGs)".into(), 180));
        }
        let Some(r) = &mut self.renderer else { return };
        #[cfg(target_arch = "wasm32")]
        let _ = el;
        // the team battle (engine addition; started by OKB_TEAM until its menus come)
        if let Some(t) = fight_scene::TeamGame::of(
            self.store.fight.as_deref(),
            self.state.team.as_deref(),
            self.view.as_ref(),
            self.state.team_config.as_ref().map(|c| &c.menu),
        ) {
            #[cfg(not(target_arch = "wasm32"))]
            capture::team(&self.env, &mut self.shot_done, t, r, el);
            #[cfg(target_arch = "wasm32")]
            t.draw_capture(r, false);
            if t.team.frame % 15 == 0
                && let Some(w) = &self.window
            {
                w.set_title(&t.status());
            }
            return;
        }
        if let Some(g) = game {
            #[cfg(not(target_arch = "wasm32"))]
            capture::fight(&self.env, &mut self.shot_done, g, r, el);
            #[cfg(target_arch = "wasm32")]
            g.draw(r);
            if g.fight.m.frame % 15 == 0
                && let Some(w) = &self.window
            {
                w.set_title(&g.status());
            }
            return;
        }
        let Some(f) = &self.state.front else { return };
        r.afterimage = 0;
        let backdrop = fight_scene::Game::of(self.store.backdrop.as_deref(), self.state.backdrop.as_ref(), self.backdrop_view.as_ref());
        // the kept demo stage's VRAM (fonts included) stays up while the ranking shows it
        if let (None, Some(data)) = (&self.state.backdrop, &self.store.front)
            && self.vram_view.sync(f, data, &self.store.files)
        {
            #[cfg(not(target_arch = "wasm32"))]
            capture::vram_dump(&self.env, &self.vram_view.vram.words);
            r.upload_vram(&self.vram_view.vram.words);
        }
        let mut prims = front_prims(&self.browser, &self.net_ui.menu, f);
        #[cfg(target_os = "android")]
        if let Some(m) = &self.droid.menu {
            prims.clear();
            m.draw(output::font::fonts(), &mut prims);
        }
        prims.extend(overlay);
        r.ambient = output::layout::ambient_split(&prims, r.viewport());
        let quads = output::layout::to_quads(&prims, r.viewport());
        #[cfg(not(target_arch = "wasm32"))]
        capture::front(&self.env, &mut self.shot_done, f, backdrop, r, &quads, el);
        #[cfg(target_arch = "wasm32")]
        match (backdrop, f.backdrop_cam()) {
            (Some(g), Some(cam)) => g.draw_stage(r, cam, f.frame_no() as u64, &quads, false),
            _ => r.frame_capture(&[], &quads, &[], [0.0, 0.0, 0.0], false),
        };
    }
}
