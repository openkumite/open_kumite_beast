//! The winit event handler: startup, load completions, the frame loop and the window's events.
//!
//! The winit side of the app (`ApplicationHandler`): the window and the startup's load (`resumed`), the loads
//! completing (`user_event`), the page going (`exiting`, `suspended`), the frame loop's wake-ups (`about_to_wait`)
//! and the window's events (`window_event`: keys, focus, resizes, and the redraw that runs the 60 Hz steps and
//! draws the frame).

#[cfg(not(target_arch = "wasm32"))]
use super::debug_env;
use super::devices::{key_bit, key_bit2};
#[cfg(not(target_arch = "wasm32"))]
use super::draw::clamp_to_monitor;
#[cfg(not(target_arch = "wasm32"))]
use super::stepping::next_slot;
use super::{App, FRAME, UserEvent, netplay, replays, requests};
use crate::fight::fight_data;
use crate::frontend::{self, options};
use crate::platform::support;
use crate::present::fight_scene;
use crate::session::{Cheat, FrameInput};
use crate::{now, render};
use std::sync::Arc;
use winit::application::ApplicationHandler;
use winit::event::{ElementState, KeyEvent, WindowEvent};
use winit::event_loop::ActiveEventLoop;
use winit::keyboard::{KeyCode, PhysicalKey};
#[cfg(not(target_arch = "wasm32"))]
use winit::window::Fullscreen;
use winit::window::{Window, WindowId};

impl ApplicationHandler<UserEvent> for App {
    fn resumed(&mut self, el: &ActiveEventLoop) {
        if self.window.is_some() {
            // Android: a new native window after the background
            #[cfg(target_os = "android")]
            self.android_resumed();
            return;
        }
        #[allow(unused_mut)]
        let mut attrs = Window::default_attributes().with_title("OpenKumiteBeast").with_inner_size(winit::dpi::LogicalSize::new(1280, 720));
        #[cfg(not(target_arch = "wasm32"))]
        if let Some((x, y)) = self.env.pos {
            attrs = attrs.with_position(winit::dpi::LogicalPosition::new(x, y));
        }
        #[cfg(not(target_arch = "wasm32"))]
        match self.env.window {
            Some(debug_env::WindowSize::Physical(w, h)) => attrs = attrs.with_inner_size(winit::dpi::PhysicalSize::new(w, h)),
            Some(debug_env::WindowSize::Logical(w, h)) => attrs = attrs.with_inner_size(winit::dpi::LogicalSize::new(w, h)),
            // GRAPHICS → WINDOW SIZE / WINDOW (engine addition; the default is the 1280×720 above)
            None => {
                let p = requests::presented(self.prefs);
                let (w, h) = p.window_size();
                attrs = attrs.with_inner_size(clamp_to_monitor(el.primary_monitor(), w, h));
                if p.fullscreen {
                    attrs = attrs.with_fullscreen(Some(Fullscreen::Borderless(None)));
                }
            }
        }
        #[cfg(target_arch = "wasm32")]
        {
            use winit::platform::web::WindowAttributesExtWebSys;
            let w = web_sys::window().unwrap();
            let (iw, ih) = (w.inner_width().unwrap().as_f64().unwrap_or(1280.0), w.inner_height().unwrap().as_f64().unwrap_or(720.0));
            attrs = attrs.with_append(true).with_inner_size(winit::dpi::LogicalSize::new(iw, ih));
        }
        let window = Arc::new(el.create_window(attrs).expect("window"));
        self.window = Some(window);
        // no usable game data: the setup screen first
        if let Some(why) = self.cfg.as_mut().and_then(|c| c.setup.take()) {
            self.enter_setup(why, super::setup::Act::Quit);
            return;
        }
        // the browser's own game data (OPFS, not `?assets=`): checked first, asynchronously (`DataChecked`)
        #[cfg(target_arch = "wasm32")]
        if let Some(dir) = self.cfg.as_ref().filter(|c| c.own_data).map(|c| c.assets.opfs_dir().map(str::to_string)) {
            let proxy = self.proxy.clone();
            wasm_bindgen_futures::spawn_local(async move {
                crate::platform::game_data::web::forget_old().await;
                let t = crate::perf_now();
                let why = match crate::assets::manifest::check_opfs(dir.as_deref()).await {
                    Ok(v) => {
                        log::info!("game data {dir:?}: {v:?} (checked in {:.0} ms)", (crate::perf_now() - t) * 1000.0);
                        v.reason()
                    }
                    Err(e) => Some(e),
                };
                let _ = proxy.send_event(UserEvent::DataChecked(why));
            });
            return;
        }
        self.start_up();
    }

    fn user_event(&mut self, el: &ActiveEventLoop, ev: UserEvent) {
        #[cfg(target_os = "android")]
        let _ = el;
        match ev {
            UserEvent::Ready(b) => {
                let (r, front, loaded) = *b;
                if let Some(mut r) = r {
                    if let Some((w, h)) = self.surface_size() {
                        r.resize(w, h);
                    }
                    r.set_fit(self.prefs.aspect.fit());
                    r.set_vsync(requests::presented(self.prefs).vsync);
                    r.render_scale = self.prefs.render_scale();
                    r.hud_scale = self.prefs.hud_scale();
                    self.renderer = Some(r);
                }
                if let (Some(loaded), Some(r)) = (loaded, &mut self.renderer) {
                    self.view = Some(fight_scene::FightView::new(r, &loaded, &mut self.audio));
                    self.store.fight = Some(Arc::new(loaded));
                }
                if let Some(data) = front {
                    #[cfg(not(target_arch = "wasm32"))]
                    if !self.env.mute {
                        self.audio.start();
                    }
                    self.audio.load_front_banks(&data);
                    self.store.front = Some(Arc::new(data));
                }
                // netplay: the session starts once connected too (`net_start`; the agreed direct fight's match: now)
                if self.net_ui.before_session() && !(self.net_ui.loading() && self.store.fight.is_some()) {
                    self.net_start();
                    if let Some(w) = &self.window {
                        w.request_redraw();
                    }
                    return;
                }
                self.begin_session();
            }
            // loading is an input: the next step applies it (`FrameInput::LOADED` / `FETCHED`)
            UserEvent::Loaded(_, epoch, _) | UserEvent::Fetched(_, epoch, _) if epoch != self.restore_gen => {
                log::info!("debug: a load / fetch asked for before a state was put back: dropped");
            }
            UserEvent::Loaded(_, _, n) if n != self.load_seq => log::info!("a match load replaced by a later one: dropped"),
            UserEvent::Fetched(_, _, n) if n != self.fetch_seq => log::info!("a fetch replaced by a later one: dropped"),
            UserEvent::Loaded(b, ..) => {
                // a load finishing while a fight runs would swap its data: nothing asks for one then
                if self.state.fight.is_some() || self.state.team.is_some() {
                    log::warn!("a match loaded during a fight: ignored");
                    return;
                }
                self.store.fight = Some(Arc::from(b));
                self.flags |= FrameInput::LOADED;
            }
            UserEvent::Music { m, looped, secs, epoch } => {
                if epoch == self.restore_gen {
                    self.audio.play_music_from(m, looped, secs);
                }
            }
            UserEvent::Fetched(files, ..) => {
                self.audio.log(|| format!("fetched {}", files.len()));
                self.store.files.extend(files.into_iter().map(|(n, b)| (n, b.into())));
                self.flags |= FrameInput::FETCHED;
                self.last = None; // no catch-up after a blocking load
            }
            // Android: the activity's dialog says why and ends the app (`android_failed`)
            #[cfg(target_os = "android")]
            UserEvent::Failed(e) => self.android_failed(&e),
            #[cfg(not(target_os = "android"))]
            UserEvent::Failed(e) => {
                log::error!("startup failed: {e}");
                el.exit();
            }
            UserEvent::ReplayData(l, epoch) => {
                if epoch == self.restore_gen && self.player.is_some() {
                    self.store.fight = Some(Arc::from(l));
                    self.start_session();
                }
            }
            UserEvent::ReplayFailed(_, epoch) if epoch != self.restore_gen || self.player.is_none() => {}
            UserEvent::ReplayFailed(e, _) => {
                log::warn!("replay: {e}");
                self.toast = Some((format!("cannot play: {e}"), 300));
                let held = self.pads();
                self.end_playback(held);
            }
            UserEvent::Imported(files) => self.imported(files),
            UserEvent::SetupReady(r) => self.setup_ready(*r),
            UserEvent::DataChecked(Some(why)) => self.enter_setup(why, super::setup::Act::Quit),
            UserEvent::DataChecked(None) => self.start_up(),
            UserEvent::Br1(br1) => self.br1_found(br1),
            UserEvent::Q3(q3) => self.q3_found(q3),
            UserEvent::Content(id) => self.content_found(id),
            UserEvent::DataFailed(e) => {
                log::error!("startup failed: {e}");
                self.enter_setup(format!("The game data could not be read: {e}"), super::setup::Act::Quit);
            }
            UserEvent::KeepAlive => {
                self.net_keepalive();
            }
        }
    }

    /// The page goes (the browser): the other player is told.
    fn exiting(&mut self, _el: &ActiveEventLoop) {
        #[cfg(target_arch = "wasm32")]
        self.net_close();
        // the setup screen's copy stopped and waited for (no `.part` left; macOS Cmd+Q comes here)
        self.setup_stop();
    }

    /// The page goes into the back-forward cache (the browser): the session ends as if the page closed. Android: the
    /// native window went (the app in the background).
    fn suspended(&mut self, _el: &ActiveEventLoop) {
        #[cfg(target_arch = "wasm32")]
        self.net_close();
        #[cfg(target_os = "android")]
        self.android_suspended();
    }

    /// Keeps the frame loop alive: a redraw request lost while the window was hidden would otherwise stop the game
    /// until something else (a resize) asked for one. Redraw requests coalesce, and the vsync'd present paces them.
    /// FRAME CAP (engine addition) or a skipped frame: the next redraw waits until its 1/60 s slot.
    fn about_to_wait(&mut self, _el: &ActiveEventLoop) {
        // Android: nothing to draw while the game data is being copied or the app is not in front
        #[cfg(target_os = "android")]
        if self.android_idle(_el) {
            return;
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            // netplay with the window occluded: the link kept alive on a timer (redraws may not come; one is asked
            // for at each keep-alive, not on every wake-up)
            if let Some((t, ran)) = self.net_keepalive() {
                let wait = std::time::Duration::from_secs_f64((t - crate::perf_now()).max(0.0));
                _el.set_control_flow(winit::event_loop::ControlFlow::WaitUntil(std::time::Instant::now() + wait));
                if let (true, Some(w)) = (ran, &self.window) {
                    w.request_redraw();
                }
                return;
            }
            if requests::presented(self.prefs).frame_cap || self.skipped {
                let t = crate::perf_now();
                if t < self.next_due {
                    let wait = std::time::Duration::from_secs_f64(self.next_due - t);
                    _el.set_control_flow(winit::event_loop::ControlFlow::WaitUntil(std::time::Instant::now() + wait));
                    return;
                }
            }
            // cap off or the slot reached: never leave a stale WaitUntil from the capped mode (an expired deadline
            // makes the loop spin)
            _el.set_control_flow(winit::event_loop::ControlFlow::Wait);
        }
        if let Some(w) = &self.window {
            w.request_redraw();
        }
    }

    fn window_event(&mut self, el: &ActiveEventLoop, _id: WindowId, ev: WindowEvent) {
        // the setup screen: its own keys, pointer, drops and redraws (no session meanwhile)
        if self.setup.is_some() {
            match ev {
                WindowEvent::KeyboardInput { event, .. } => return self.setup_key(&event, el),
                WindowEvent::CursorMoved { position: p, .. } => return self.setup_pointer([p.x as f32, p.y as f32]),
                WindowEvent::MouseInput { state: ElementState::Pressed, button: winit::event::MouseButton::Left, .. } => return self.setup_click(None, el),
                WindowEvent::Touch(t) => {
                    use winit::event::TouchPhase;
                    let phase = match t.phase {
                        TouchPhase::Started => 0,
                        TouchPhase::Moved => 1,
                        TouchPhase::Ended => 2,
                        TouchPhase::Cancelled => 3,
                    };
                    return self.setup_touch(phase, t.id as i32, [t.location.x as f32, t.location.y as f32], el);
                }
                WindowEvent::DroppedFile(p) => return self.setup_dropped(p, el),
                WindowEvent::RedrawRequested => return self.setup_redraw(el),
                WindowEvent::CloseRequested => return self.setup_quit(el),
                _ => {}
            }
        }
        match ev {
            WindowEvent::CloseRequested => el.exit(),
            WindowEvent::Resized(s) => {
                let (w, h) = self.surface_size().unwrap_or((s.width, s.height));
                if let Some(r) = &mut self.renderer {
                    r.resize(w, h);
                }
                self.sync_fullscreen();
            }
            WindowEvent::ModifiersChanged(m) => {
                self.mods = m.state();
                // Shift pressed or let go while Backspace is held: the rewind's speed follows
                if let Some(t) = &mut self.tools
                    && t.rewinding.is_some()
                {
                    t.rewinding = Some(if self.mods.shift_key() { 2 } else { 1 });
                }
            }
            // focus lost: the key releases go to the other application, so nothing may stay held
            WindowEvent::Focused(false) => {
                self.keys = 0;
                self.keys2 = 0;
                self.latch = [0; 2];
                self.end_rewind();
                // Android: out of front (the shade, a dialog, the recents): no steps, no sound
                #[cfg(target_os = "android")]
                self.android_focus(false);
            }
            // back on screen / in focus: restart the frame clock (no catch-up burst) and the redraw chain, which an
            // occluded window may have dropped
            WindowEvent::Occluded(true) => self.net_ui.occluded = true,
            WindowEvent::Focused(true) | WindowEvent::Occluded(false) => {
                if matches!(ev, WindowEvent::Occluded(false)) {
                    self.net_ui.occluded = false;
                }
                #[cfg(target_os = "android")]
                if matches!(ev, WindowEvent::Focused(true)) {
                    self.android_focus(true);
                }
                self.last = None;
                if let Some(w) = &self.window {
                    w.request_redraw();
                }
            }
            WindowEvent::KeyboardInput { event: KeyEvent { physical_key: PhysicalKey::Code(k), state: ElementState::Released, .. }, .. } => {
                if self.tools.is_some() && self.tool_key(k, false, false, el) {
                    return;
                }
                self.keys &= !key_bit(k);
                self.keys2 &= !key_bit2(k);
            }
            WindowEvent::KeyboardInput { event: KeyEvent { physical_key: PhysicalKey::Code(k), state: ElementState::Pressed, repeat, .. }, .. } => {
                self.audio.start(); // browsers allow audio only after a user gesture
                // Alt+Enter: WINDOW / FULLSCREEN (engine addition; the key is not passed to the game)
                if matches!(k, KeyCode::Enter | KeyCode::NumpadEnter) && self.mods.alt_key() {
                    if !repeat {
                        let fs = !self.prefs.fullscreen;
                        self.apply_prefs(options::Prefs { fullscreen: fs, ..self.prefs });
                    }
                    return;
                }
                // the debug tools' keys (P, ., F12, Shift+F1..F4, Backspace) on every screen
                if self.tools.is_some() && self.tool_key(k, true, repeat, el) {
                    return;
                }
                self.keys |= key_bit(k);
                self.keys2 |= key_bit2(k);
                self.latch[0] |= key_bit(k);
                self.latch[1] |= key_bit2(k);

                // Esc quits, as EXIT GAME does; not in the browser, where a page cannot close itself (the stopped
                // event loop would leave a frozen frame) and Esc is the browser's own key (it leaves fullscreen)
                #[cfg(not(target_arch = "wasm32"))]
                if k == KeyCode::Escape {
                    el.exit();
                }
                if !repeat && self.debug_key(k) {
                    return;
                }
                // netplay: no fight debug keys (cheats, the app's pause, the camera)
                if self.state.fight.is_none() || self.net.is_some() {
                    return;
                }
                // the fight's debug keys: cheats for the next step (the camera keys move the view only)
                let cheat = match k {
                    KeyCode::KeyP if !repeat => {
                        self.paused = !self.paused;
                        None
                    }
                    // frame advance while paused: one whole step
                    KeyCode::Period if self.paused => {
                        let pads = self.pads();
                        self.step([pads[0] | self.latch[0], pads[1] | self.latch[1]]);
                        self.latch = [0; 2];
                        None
                    }
                    KeyCode::BracketRight => Some(Cheat::NextMove),
                    KeyCode::BracketLeft => Some(Cheat::PrevMove),
                    KeyCode::KeyC if !repeat => Some(Cheat::Cpu),
                    KeyCode::KeyB if !repeat => Some(Cheat::Beast(0)),
                    KeyCode::KeyN if !repeat => Some(Cheat::Beast(1)),
                    _ => {
                        if let Some(v) = &mut self.view {
                            match k {
                                KeyCode::KeyQ => v.cam_yaw -= 0.1,
                                KeyCode::KeyE => v.cam_yaw += 0.1,
                                KeyCode::KeyZ => v.cam_dist = (v.cam_dist - 0.3).max(-1.5),
                                KeyCode::KeyX => v.cam_dist += 0.3,
                                _ => {}
                            }
                        }
                        None
                    }
                };
                self.cheats.extend(cheat);
            }
            WindowEvent::RedrawRequested => {
                // Android: the activity's input; no steps, loads or drawing while not in front or after a failure
                // (`android_idle`)
                #[cfg(target_os = "android")]
                {
                    self.android_input();
                    if self.droid.failed || self.droid.frozen() {
                        self.last = None;
                        return;
                    }
                }
                self.sync_fullscreen();
                let t = now();
                let pt = if self.perf.on { crate::perf_now() } else { 0.0 };
                if self.perf.on {
                    self.perf.frame(pt);
                }
                let dt = self.last.map(|l| (t - l).min(0.25)).unwrap_or(0.0);
                self.last = Some(t);
                let [pad, pad2] = self.pads();
                self.fps.frame(t);
                // debug OKB_DEBUGKEYS: the tools' keys due at this drawn frame
                for a in self.tools.as_ref().map(|t| t.due()).unwrap_or_default() {
                    self.tool_action(a, el);
                }
                // Backspace held (the debug tools): back instead of on, at the same 60 Hz
                let rewinding = self.tools.as_ref().is_some_and(|t| t.rewinding.is_some());
                self.net_connect_tick();
                self.net_poll();
                // netplay before its session: no steps (the connect screen)
                if self.net_ui.before_session() {
                    self.acc = 0.0;
                } else if rewinding || !self.paused {
                    self.acc = if !self.env.lockstep {
                        self.acc + dt
                    } else if self.renderer.as_ref().is_none_or(|r| r.presented) {
                        FRAME
                    } else {
                        0.0
                    };
                    while self.acc >= FRAME && rewinding {
                        self.rewind_tick();
                        self.acc -= FRAME;
                    }
                    'steps: while self.acc >= FRAME {
                        // netplay: this side's clock leads the other's, this step slot passes
                        if self.net.as_mut().is_some_and(|n| n.ahead()) {
                            self.acc -= FRAME;
                            continue;
                        }
                        // Android's TOUCH CONTROLS page: no session steps, the page's frame
                        #[cfg(target_os = "android")]
                        if self.droid.menu.is_some() {
                            self.touch_menu_tick([pad | self.latch[0], pad2 | self.latch[1]]);
                            self.latch = [0; 2];
                            self.acc -= FRAME;
                            continue;
                        }
                        // the NETWORK menu: no session steps, the page's frame (a connect it ran may begin a
                        // netplay session: its steps start with the next slot)
                        if self.net_ui.menu.is_some() {
                            self.net_menu_tick([pad | self.latch[0], pad2 | self.latch[1]]);
                            self.latch = [0; 2];
                            self.acc -= FRAME;
                            continue;
                        }
                        // a netplay session of the NETWORK menu that ends after this frame: no more steps of it
                        if self.net_ui.leaving() {
                            self.acc = 0.0;
                            break 'steps;
                        }
                        // the replay list: no session steps, the list's frame
                        if self.browser.is_some() {
                            self.list_tick([pad | self.latch[0], pad2 | self.latch[1]]);
                            self.latch = [0; 2];
                            self.acc -= FRAME;
                            continue;
                        }
                        // x4 (debug); OKB_FAST=n runs n× until close to a OKB_SHOTS frame
                        #[cfg_attr(target_arch = "wasm32", allow(unused_mut))]
                        let mut n = if self.fast { 4 } else { 1 };
                        #[cfg(not(target_arch = "wasm32"))]
                        if let (Some(v), Some(g)) = (self.env.fast, &self.state.fight) {
                            let f = g.m.frame;
                            let near = self.env.shots.iter().any(|&k| k >= f && k < f + 64);
                            if !near {
                                n = v;
                            }
                        }
                        // a playback: its controls (the pads) give the steps; a skip to its match or a seek runs fast
                        if self.player.is_some() {
                            n = self.playback_tick([pad | self.latch[0], pad2 | self.latch[1]]);
                            self.latch = [0; 2];
                            if self.player.is_none() {
                                // back on the list
                                self.acc = 0.0;
                                break 'steps;
                            }
                        }
                        let skipping = self.replay_skipping();
                        if skipping {
                            n = replays::SKIP_STEPS;
                        }
                        // OKB_FEFAST=n runs n front-end steps while the next OKB_FESHOTS frame is far (`fe_fast`)
                        #[cfg(not(target_arch = "wasm32"))]
                        let fe_fast = self.env.fefast.filter(|_| self.fe_fast());
                        #[cfg(not(target_arch = "wasm32"))]
                        if let Some(v) = fe_fast {
                            n = v;
                        }
                        for _ in 0..n {
                            #[cfg(not(target_arch = "wasm32"))]
                            let requests = self.requests;
                            let ran = self.step([pad | self.latch[0], pad2 | self.latch[1]]);
                            self.latch = [0; 2];
                            // OKB_FEFAST: no more steps in this drawn frame after one that made a request (what it
                            // asked for arrives after the redraw) or once the next one must be drawn
                            #[cfg(not(target_arch = "wasm32"))]
                            if fe_fast.is_some() && (self.requests != requests || !self.fe_fast()) {
                                break;
                            }
                            // netplay: the step waits for the other player; the slots meanwhile are caught up after
                            // it, a few at most
                            if self.net.is_some() && !ran {
                                self.acc = self.acc.min(netplay::CATCH_UP * FRAME);
                                break 'steps;
                            }
                            // a playback that waits for a load or fetch (delivered after this redraw), or whose skip
                            // just reached the match: no more steps in this drawn frame, and no catch-up after it
                            if self.player.is_some() && (!ran || skipping && !self.replay_skipping()) {
                                self.acc = 0.0;
                                break 'steps;
                            }
                            // the session asked for the replay list or the NETWORK menu (or left its netplay
                            // session): the steps stop with it
                            if self.browser.is_some() || self.net_ui.menu.is_some() || self.net_ui.leaving() {
                                break;
                            }
                        }
                        self.acc -= FRAME;
                    }
                }
                self.net_tick();
                #[cfg(not(target_arch = "wasm32"))]
                if self.net_quit_due() {
                    el.exit();
                    return;
                }
                // EXIT GAME (engine addition): the preferences saved as on any change, the memory card data is saved
                // whenever it changes
                if self.exit_req {
                    if self.saves() {
                        crate::platform::storage::write_prefs(self.prefs.to_stored());
                    }
                    el.exit();
                    return;
                }
                #[cfg(not(target_arch = "wasm32"))]
                if requests::presented(self.prefs).frame_cap {
                    self.next_due = next_slot(self.next_due, crate::perf_now());
                }
                if let Some((_, n)) = &mut self.toast {
                    *n = n.saturating_sub(1);
                    if *n == 0 {
                        self.toast = None;
                    }
                }
                let pd = if self.perf.on { crate::perf_now() } else { 0.0 };
                if let Some(r) = &mut self.renderer {
                    r.perf = self.perf.on;
                    // this frame's counts only (none when nothing is drawn; the GPU time only when one was read back)
                    r.stats = render::FrameStats::default();
                }
                // the frame's surface texture before anything of it is written: without one (an occluded window: the
                // acquire fails at once, nothing paces the redraws) the drawing is skipped and the next redraw waits for
                // its 60 Hz slot
                let shown = self.renderer.as_mut().is_none_or(|r| r.acquire());
                if shown {
                    self.draw(el);
                    if let Some(t) = &mut self.tools {
                        t.drawn += 1;
                    }
                }
                self.skipped = !shown;
                // a replay played to its end: once its last state has been drawn, back to the list it was played from,
                // or (--replay) quit
                if shown && self.player.as_ref().is_some_and(|pb| pb.ended) && self.end_playback([pad, pad2]) {
                    el.exit();
                    return;
                }
                // debug OKB_UIEXIT: quit 5 UI frames after the last OKB_UISHOTS frame
                if self.env.uiexit && self.env.uishots.iter().max().is_some_and(|&m| self.ui_frame > m + 5) {
                    el.exit();
                    return;
                }
                #[cfg(not(target_arch = "wasm32"))]
                if self.skipped {
                    self.next_due = crate::perf_now() + FRAME;
                }
                if let (true, Some(r)) = (self.perf.on, &self.renderer) {
                    // CPU: the steps, the draw lists (the drawing minus the renderer's time and its vsync wait), the
                    // renderer
                    let pe = crate::perf_now();
                    let game = self.view.as_ref().map_or([0, 0], |v| v.gpu_bytes());
                    let own = r.gpu_bytes();
                    self.perf.record(pe, (pd - pt) * 1000.0, ((pe - pd) * 1000.0 - r.stats.cpu_ms - r.stats.wait_ms).max(0.0), &r.stats, || {
                        (support::rss(), own[0] + game[0], own[1] + game[1])
                    });
                }
                // the fight loads after the VS screen has been drawn (the PS1 keeps that frame up while loading); not
                // while the debug tools rewind (each step back puts back what was under way)
                if self.load_req.is_some() && self.tools.as_ref().is_none_or(|t| t.rewinding.is_none()) {
                    self.load_wait += 1;
                    if self.load_wait >= 3 {
                        self.start_load();
                    }
                }
                // FRAME CAP / a skipped frame: `about_to_wait` asks for the next one in its slot
                if let (false, Some(w)) = (cfg!(not(target_arch = "wasm32")) && (self.prefs.frame_cap || self.skipped), &self.window) {
                    w.request_redraw();
                }
            }
            _ => {}
        }
    }
}

impl App {
    /// The browser's install of the imported game found by the start (`UserEvent::Br1`, before `Ready`): the game data's
    /// `Assets` read it, and a new session (not a replay's or netplay's start) offers it (engine addition, the import
    /// S9).
    fn br1_found(&mut self, br1: Option<(String, u32)>) {
        #[cfg(target_arch = "wasm32")]
        {
            for a in self.assets.iter_mut().chain(self.cfg.as_mut().map(|c| &mut c.assets)) {
                a.set_br1_opfs(br1.clone());
            }
            if self.player.is_none() && !self.net_ui.netplay {
                self.start.br1 = br1.map(|(_, id)| id);
            }
        }
        #[cfg(not(target_arch = "wasm32"))]
        let _ = br1;
    }

    /// The browser's Quake III Arena maps found by the start (engine addition, the extra stages; `UserEvent::Q3`, before
    /// `Ready`): the game data's `Assets` read them, and a new session's start (not a replay's or netplay's) offers the
    /// stages they have. Only the browser's start sends it (desktop and Android read the data folder in `start_up`).
    pub(super) fn q3_found(&mut self, q3: Option<(String, u8)>) {
        #[cfg(target_arch = "wasm32")]
        for a in self.assets.iter_mut().chain(self.cfg.as_mut().map(|c| &mut c.assets)) {
            a.set_q3_opfs(q3.clone());
        }
        if self.player.is_none() && !self.net_ui.netplay {
            self.start.set_extras(q3.map_or(0, |(_, mask)| mask));
        }
    }

    /// The game data's content id read by the start (`UserEvent::Content`, after its loads, before `Ready`): a new
    /// session's start carries it, and a netplay connect's (the command line's began before it was known: its handshake
    /// waits for it, `net_connect_tick`); a replay's start keeps its own, warned of when it is another data's.
    fn content_found(&mut self, id: u32) {
        log::info!("game data: content id {id:08x}");
        self.content = Some(id);
        if let Some(c) = &mut self.net_ui.connect {
            c.set_content(Some(id));
            // the tables are in now (the data's loads came first): the map checked against them
            c.set_buttons(crate::net::handshake::own_buttons());
        }
        if self.player.is_some() {
            if let Some(w) = crate::session::replay::content_warning(&self.start, Some(id)) {
                log::warn!("replay: {w}");
            }
        } else if !self.net_ui.netplay {
            self.start.content = Some(id);
        }
    }

    /// The startup's load: the front end's data (or a direct fight's match) and the renderer, then `UserEvent::Ready`
    /// (`resumed`; on Android once the game data is there, `android_idle`).
    pub(super) fn start_up(&mut self) {
        let Some(window) = self.window.clone() else { return };
        let cfg = self.cfg.as_ref().expect("config");
        let (assets, own_data, debug, perf) = (cfg.assets.clone(), cfg.own_data, cfg.debug, cfg.perf);
        self.assets = Some(assets.clone());
        // The imported game's install (engine addition, the import S9): a new session offers it when its content id is
        // known (desktop, Android: the data folder's, also after the setup screen added it; the browser: found below,
        // `UserEvent::Br1`); a replay's or netplay's start keeps its own
        #[cfg(not(target_arch = "wasm32"))]
        if self.player.is_none() && !self.net_ui.netplay {
            self.start.br1 = crate::assets::manifest::br1_install_id(&assets);
            // the extra stages' files as they are now (the setup screen may have added Quake III's maps)
            self.start.set_extras(crate::assets::extras::present(&assets));
        }
        self.overlay = debug;
        self.perf.on = perf || self.prefs.perf;
        // the setup screen's renderer, if it was up (a window has one surface)
        let have = self.renderer.take();
        // netplay (connecting, or its setup failed): the front end's data (the connect screen's font, the failure's
        // message); a direct fight's match once agreed
        let direct = match &self.start.entry {
            crate::session::Entry::DirectFight { p1, p2, stage } if !self.net_ui.netplay => Some((p1.clone(), p2.clone(), stage.clone())),
            _ => None,
        };
        let (proxy, costume, sim_version) = (self.proxy.clone(), self.costume, self.start.sim_version);
        // OKB_TEAM (engine addition, debug): a team battle's data instead
        let team = self.team_debug();
        let fut = async move {
            // the browser's install of the imported game (engine addition, the import S9): checked before the front end's
            // data, which reads its portraits
            #[cfg(target_arch = "wasm32")]
            let assets = {
                let mut assets = assets;
                if own_data && assets.opfs_dir().is_some() {
                    let br1 = crate::assets::manifest::opfs_br1_install().await;
                    assets.set_br1_opfs(br1.clone());
                    let _ = proxy.send_event(UserEvent::Br1(br1));
                    // Quake III Arena's maps (engine addition): the extra stages a direct fight or the select offers
                    let q3 = crate::assets::extras::opfs_q3_install().await;
                    assets.set_q3_opfs(q3.clone());
                    let _ = proxy.send_event(UserEvent::Q3(q3));
                }
                assets
            };
            let ev = match async {
                // the game data's content id (data without one: convert it again), before the data's own loads; told
                // after them (a netplay connect waits for it, and checks the joiner's map against their tables)
                let content = crate::assets::manifest::install_id(&assets).await.map_err(|e| (e, true))?;
                let (front, loaded) = if let Some(t) = team {
                    let chr = [0, 1, 2, 3].map(|k| t.chars[k].as_str());
                    (None, Some(fight_data::load_team(&assets, chr, t.costumes(), &t.stage, sim_version).await.map_err(|e| (e, false))?))
                } else if let Some((p1, p2, stage)) = direct {
                    (None, Some(fight_data::load_match(&assets, [&p1, &p2], costume, &stage, false, sim_version).await.map_err(|e| (e, true))?))
                } else {
                    (Some(frontend::load(&assets).await.map_err(|e| (e, true))?), None)
                };
                let _ = proxy.send_event(UserEvent::Content(content));
                let r = match have {
                    Some(r) => r,
                    None => render::Renderer::new(window).await.map_err(|e| (e, false))?,
                };
                Ok::<_, (String, bool)>((Some(r), front, loaded))
            }
            .await
            {
                Ok(v) => UserEvent::Ready(Box::new(v)),
                // the app's own game data could not be read: the setup screen
                Err((e, true)) if own_data => UserEvent::DataFailed(e),
                Err((e, _)) => UserEvent::Failed(e),
            };
            let _ = proxy.send_event(ev);
        };
        #[cfg(not(target_arch = "wasm32"))]
        pollster::block_on(fut);
        #[cfg(target_arch = "wasm32")]
        wasm_bindgen_futures::spawn_local(fut);
    }
}
