//! Serving a step's requests: match loads, music, fetched files, preferences, the card store.
//!
//! What a step asks the app for (`request`): the match load (`start_load`), music (`load_music`), fetched files,
//! the preferences (`apply_prefs`, `sync_fullscreen`), EXIT GAME, the replay list, the NETWORK menu; the card store
//! (`store_card`).

#[cfg(not(target_arch = "wasm32"))]
use super::draw::clamp_to_monitor;
use super::{App, UserEvent, perf};
use crate::assets::roster;
use crate::fight::fight_data;
use crate::formats;
use crate::frontend::{LoadSpec, options};
use crate::present::fight_scene;
use crate::session::{FrameInput, Request};
use std::sync::Arc;
use winit::window::Fullscreen;

/// The preferences as the window, the surface and the pacing use them (engine addition): on Android vsync'd, uncapped
/// and with no fullscreen / window size requests (the activity is always full screen; winit's Android backend ignores
/// them). GRAPHICS hides WINDOW, WINDOW SIZE, VSYNC and
/// FRAME CAP there, so a value saved by an earlier build or set in a desktop host's netplay session (whose rows show
/// them) would otherwise stay with no way to reset it. The saved values and the front end's copy are left as they are.
pub(super) fn presented(p: options::Prefs) -> options::Prefs {
    if cfg!(target_os = "android") { options::Prefs { fullscreen: false, vsync: true, frame_cap: false, ..p } } else { p }
}

impl App {
    /// A step's request.
    pub(super) fn request(&mut self, r: Request) {
        match r {
            Request::Load(spec) => {
                self.load_req = Some(LoadSpec::Match(spec));
                self.load_wait = 0;
                self.load_seq = self.load_seq.wrapping_add(1);
                self.store.fight = None;
            }
            // the team battle (engine addition): loaded as a match is, after its loading screen has been drawn
            Request::LoadTeam(spec) => {
                self.load_req = Some(LoadSpec::Team(spec));
                self.load_wait = 0;
                self.load_seq = self.load_seq.wrapping_add(1);
                self.store.fight = None;
            }
            Request::Fight => {
                if let (Some(r), Some(data), true) = (&mut self.renderer, &self.store.fight, self.state.fight.is_some() || self.state.team.is_some()) {
                    self.view = Some(fight_scene::FightView::new(r, data, &mut self.audio));
                }
            }
            Request::TeamOver { .. } => {
                self.view = None;
                self.store.fight = None;
            }
            Request::FightOver { backdrop, .. } => {
                let (view, data) = (self.view.take(), self.store.fight.take());
                if backdrop {
                    self.backdrop_view = view;
                    self.store.backdrop = data;
                }
            }
            Request::Music(None) => {}
            Request::Music(Some(_)) if self.replay_silent() => {}
            Request::Music(Some((track, looped))) => self.load_music(track, looped, 0.0),
            Request::Fetch(names) => {
                let Some(assets) = self.assets.clone() else { return };
                for n in &names {
                    self.store.files.remove(n);
                }
                self.fetch_seq = self.fetch_seq.wrapping_add(1);
                let (proxy, epoch, seq) = (self.proxy.clone(), self.restore_gen, self.fetch_seq);
                let fut = async move {
                    let mut files = Vec::new();
                    for n in names {
                        // a movie's music: its `.xa` first; an imported ending's from that install (`Assets::route`)
                        let Some((a, rel)) = assets.route(&n) else {
                            log::warn!("fetch {n}: no data of the imported game");
                            continue;
                        };
                        match a.load_music(rel).await {
                            Ok(b) => files.push((n, b)),
                            Err(e) => log::warn!("fetch {n}: {e}"),
                        }
                    }
                    let _ = proxy.send_event(UserEvent::Fetched(files, epoch, seq));
                };
                #[cfg(not(target_arch = "wasm32"))]
                pollster::block_on(fut);
                #[cfg(target_arch = "wasm32")]
                wasm_bindgen_futures::spawn_local(fut);
            }
            // a playback's preferences are the file's (the logic's) and the player's own (the window's); netplay's
            // the session's (both sides), the window keeps this player's
            Request::Prefs(_) if self.player.is_some() => {}
            Request::Prefs(p) if self.net.is_some() => {
                if let Some(n) = &mut self.net {
                    n.set_prefs(p);
                }
            }
            Request::Prefs(p) => self.apply_prefs(p),
            // a playback's EXIT GAME does not quit, nor its REPLAY open the list (the file's inputs go on)
            Request::Exit | Request::Replays | Request::Network if self.player.is_some() => {}
            Request::Exit => self.exit_req = true,
            // netplay: the list is this side's alone (the session would wait for it on one side only)
            Request::Replays if self.net.is_some() => self.toast = Some(("the replay list is off in netplay".into(), 180)),
            Request::Replays => {
                let held = self.pads();
                self.open_list(held);
            }
            // a netplay session of the NETWORK menu ends (both sides step the same press)
            Request::Network if self.net.is_some() => self.net_leave_request(),
            Request::Network => {
                let held = self.pads();
                self.open_net_menu(held);
            }
            // CHANGE GAME DATA (v9): not in a playback (the file's inputs go on) nor in netplay (the other side's
            // session would wait)
            Request::ChangeData if self.player.is_some() => {}
            Request::ChangeData if self.net.is_some() => self.toast = Some(("CHANGE GAME DATA is off in netplay".into(), 180)),
            Request::ChangeData => self.change_data(),
        }
    }

    /// XA track `track` loaded and played from `secs` in (`UserEvent::Music`).
    pub(super) fn load_music(&mut self, track: u8, looped: bool, secs: f64) {
        let Some(assets) = self.assets.clone() else { return };
        let proxy = self.proxy.clone();
        let epoch = self.restore_gen;
        let fut = async move {
            let path = formats::music::track_file(track);
            match assets.load_music(&path).await.ok().and_then(formats::music::decode) {
                Some(m) => {
                    let _ = proxy.send_event(UserEvent::Music { m: Arc::new(m), looped, secs, epoch });
                }
                None => log::warn!("music track {track:#x} ({path}) not found"),
            }
        };
        #[cfg(not(target_arch = "wasm32"))]
        pollster::block_on(fut);
        #[cfg(target_arch = "wasm32")]
        wasm_bindgen_futures::spawn_local(fut);
    }

    /// New preferences (GRAPHICS, F5, Alt+Enter, the ` key): saved and applied to the window, the renderer, the
    /// performance overlay and the front end's copy (engine addition).
    pub(super) fn apply_prefs(&mut self, p: options::Prefs) {
        let old = std::mem::replace(&mut self.prefs, p);
        if old == p {
            return;
        }
        // never while a replay plays (6a review) nor in netplay: the next change outside them saves them
        if self.saves() {
            crate::platform::storage::write_prefs(p.to_stored());
        }
        // the window and the pacing follow the presented values (`presented`), the front end's copy the saved ones
        let (np, op) = (presented(p), presented(old));
        if let Some(r) = &mut self.renderer {
            r.set_fit(p.aspect.fit());
            r.set_vsync(np.vsync);
            r.render_scale = p.render_scale();
            r.hud_scale = p.hud_scale();
        }
        if let Some(w) = &self.window {
            if np.fullscreen != op.fullscreen {
                w.set_fullscreen(np.fullscreen.then_some(Fullscreen::Borderless(None)));
            }
            #[cfg(not(target_arch = "wasm32"))]
            if !np.fullscreen && np.window != op.window && self.env.window.is_none() {
                let (ww, wh) = np.window_size();
                let _ = w.request_inner_size(clamp_to_monitor(w.current_monitor(), ww, wh));
            }
        }
        #[cfg(not(target_arch = "wasm32"))]
        if np.frame_cap && !op.frame_cap {
            // the slots start from now
            self.next_due = crate::perf_now();
        }
        if p.perf != old.perf {
            // the measurements restarted
            self.perf = perf::Perf::default();
            self.perf.on = p.perf;
        }
        // the front end's copy: through the next step (`session::Assets::prefs`)
    }

    /// A window taken to / out of fullscreen by the system (the title bar button, the browser's Esc): GRAPHICS →
    /// WINDOW follows. Checked on every resize and redraw (macOS reports the change only once its animation ends,
    /// with no resize guaranteed after it), and only when the window's state changed, so a request still under way
    /// is not undone.
    pub(super) fn sync_fullscreen(&mut self) {
        let Some(fs) = self.window.as_ref().map(|w| w.fullscreen().is_some()) else { return };
        if fs == self.seen_fs {
            return;
        }
        self.seen_fs = fs;
        if fs != self.prefs.fullscreen {
            self.prefs.fullscreen = fs;
            if self.saves() {
                crate::platform::storage::write_prefs(self.prefs.to_stored());
            }
        }
    }

    /// The memory card written by the step: its file to the disk / localStorage; the result goes into the next
    /// step's input (`FrameInput::CARD_STORED` / `CARD_FAILED`).
    pub(super) fn store_card(&mut self) {
        if self.card_held() {
            return;
        }
        let Some(file) = self.state.front.as_ref().and_then(|f| f.save.memory_card.file()) else { return };
        let ok = crate::platform::storage::store_card(file);
        self.flags |= FrameInput::CARD_STORED | if ok { 0 } else { FrameInput::CARD_FAILED };
    }

    pub(super) fn start_load(&mut self) {
        let (Some(load), Some(assets)) = (self.load_req.take(), self.assets.clone()) else { return };
        let arcade_bgm = self.state.front.as_ref().is_some_and(|f| f.arcade_bgm());
        let (proxy, epoch, seq, sim_version) = (self.proxy.clone(), self.restore_gen, self.load_seq, self.state.sim_version);
        let fut = async move {
            let loaded = match load {
                LoadSpec::Match(spec) => {
                    let chr = spec.chr.map(roster::code);
                    fight_data::load_match(&assets, chr, spec.costume, fight_data::stage_code(spec.stage), arcade_bgm, sim_version).await
                }
                LoadSpec::Team(spec) => {
                    let chr = spec.chr.map(roster::code);
                    fight_data::load_team(&assets, chr, spec.costume, fight_data::stage_code(spec.stage), sim_version).await
                }
            };
            let ev = match loaded {
                Ok(l) => UserEvent::Loaded(Box::new(l), epoch, seq),
                Err(e) => UserEvent::Failed(e),
            };
            let _ = proxy.send_event(ev);
        };
        #[cfg(not(target_arch = "wasm32"))]
        pollster::block_on(fut);
        #[cfg(target_arch = "wasm32")]
        wasm_bindgen_futures::spawn_local(fut);
        self.last = None; // no catch-up after the blocking load
    }
}
