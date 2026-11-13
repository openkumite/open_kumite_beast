//! One 60 Hz step of the app: the pads, the step's input and cheats, `session::step` and its output.
//!
//! A 60 Hz step: the pads (`pads`), the step's input (`frame_input`, the env cheats), `session::step` and its output
//! (`step`); OKB_FEFAST's test (`fe_fast`); FRAME CAP's redraw slots (`next_slot`).

#[cfg(not(target_arch = "wasm32"))]
use super::FRAME;
#[cfg(not(target_arch = "wasm32"))]
use super::capture;
use super::{App, tools};
use crate::session::{self, Cheat, FrameInput};

/// FRAME CAP's next redraw slot after a redraw at `t` (monotonic seconds; engine addition): one 1/60 s step on, kept
/// within one frame of `t`, so redraws coming faster (a live resize, the cap just turned on) never push it further
/// ahead and a late one restarts the slots from now.
#[cfg(not(target_arch = "wasm32"))]
pub(super) fn next_slot(due: f64, t: f64) -> f64 {
    (due + FRAME).clamp(t, t + FRAME)
}

impl App {
    /// The pads as read now: keyboard and gamepads (`gamepads::Pads`; Android: also the touch controls); pads 3 / 4
    /// (gamepads only, engine addition: the team battle) into `ext`.
    pub(super) fn pads(&mut self) -> [u16; 2] {
        let g = self.gamepads.poll();
        #[cfg_attr(not(target_os = "android"), allow(unused_mut))]
        let mut pads = [self.keys | g[0], self.keys2 | g[1]];
        self.ext = [g[2], g[3]];
        // Android: the activity's keys and gamepads (the second gamepad player 2's, the third and fourth pads 3 / 4) and
        // the touch controls
        #[cfg(target_os = "android")]
        {
            let d = self.droid.bits();
            pads = [pads[0] | d[0], pads[1] | d[1]];
            self.ext = [self.ext[0] | d[2], self.ext[1] | d[3]];
        }
        pads
    }

    /// One 60 Hz frame: `session::step` with these pads, the scripted ones and the cheats; then its output (sound,
    /// rumble, requests, the card store).
    /// In a playback (`--replay`) the input, cheats and preferences are the file's, and a step waits for the loads
    /// the recording had by then. False when no step ran (a playback waiting, ended or stopped).
    pub(super) fn step(&mut self, pads: [u16; 2]) -> bool {
        let pending = tools::Pending { flags: self.flags, load_req: self.load_req, load_wait: self.load_wait };
        let (i, cheats, prefs) = if self.player.is_some() {
            self.cheats.clear();
            let Some(x) = self.replay_input() else { return false };
            x
        } else if self.net.is_some() {
            // netplay: the lockstep's input; no cheats
            self.cheats.clear();
            let Some(i) = self.net_input(pads) else { return false };
            (i, Vec::new(), self.logic_prefs())
        } else {
            // the buttons that closed the replay list reach the session only once released and pressed again
            self.swallow &= pads[0] | pads[1];
            let pads = pads.map(|p| p & !self.swallow);
            let i = self.frame_input(pads);
            let mut cheats = std::mem::take(&mut self.cheats);
            self.env_cheats(&mut cheats);
            (i, cheats, self.prefs)
        };
        let mut out = session::Out::new(self.audio.logging());
        let mut held = (self.rollback.is_some() || self.tools.is_some()).then(|| session::rollback::Held::of(&self.store, prefs));
        session::step(&mut self.state, &self.store.assets(prefs), i, &cheats, &mut out);
        if let Some(n) = &mut self.net {
            n.stepped(&self.state, &out);
        }
        self.hashes.log(&self.state);
        self.record_step(i, &cheats, prefs);
        self.replay_stepped();
        if let (Some(t), None) = (&mut self.tools, &self.player) {
            // the store's copy moves into the history unless the rollback test takes it too
            let h = if self.rollback.is_some() { held.clone() } else { held.take() };
            if let Some(h) = h {
                t.rewind.push(h, (i, cheats.clone(), pending), &self.state);
                t.note_history();
            }
        }
        if let (Some(rb), Some(held)) = (&mut self.rollback, held) {
            rb.check(&self.state, held, (i, cheats), |s, a, (i, c)| session::step(s, a, *i, c, &mut session::Out::default()));
        }
        // a replay skipping, seeking, paused or not at ×1 is silent (the sound is brought to the state after)
        let silent = self.replay_silent();
        if !silent {
            self.audio.apply(&mut out.audio);
        }
        // a movie's own audio follows the state (`VramView`)
        if let Some(f) = &self.state.front
            && let Some((m, at)) = self.vram_view.step(f, &self.store.files)
            && !silent
        {
            self.audio.play_music_from(m, false, at);
        }
        if let Some(v) = out.vib {
            self.gamepads.rumble(v);
        }
        for r in out.requests {
            #[cfg(not(target_arch = "wasm32"))]
            {
                self.requests = self.requests.wrapping_add(1);
            }
            self.record_request(&r);
            self.request(r);
        }
        if self.state.backdrop.is_none() {
            self.backdrop_view = None;
            self.store.backdrop = None;
        }
        self.store.prune(&self.state);
        // a playback never writes the card (the file's input says how its stores went), nor netplay (the lockstep
        // reports the store done)
        if out.card_dirty && self.saves() {
            self.store_card();
        }
        true
    }

    /// OKB_FEFAST: whether the next step may run without the frame before it drawn: a front-end step (no fight, no
    /// playback, netplay, replay list or debug tools), no match load counting its drawn frames (`load_wait`), and a
    /// OKB_FESHOTS frame still to save 64 or more front-end frames ahead (none after the last: OKB_FEEXIT's frame
    /// and the save it leaves stay the same).
    #[cfg(not(target_arch = "wasm32"))]
    pub(super) fn fe_fast(&self) -> bool {
        let Some(f) = &self.state.front else { return false };
        self.state.fight.is_none()
            && self.state.team.is_none()
            && self.load_req.is_none()
            && self.player.is_none()
            && self.net.is_none()
            && self.browser.is_none()
            && self.net_ui.menu.is_none()
            && self.tools.is_none()
            && capture::next_front(&self.env, self.shot_done).is_some_and(|s| s >= f.frame_no().saturating_add(64))
    }

    /// The step's input: the pads, the debug scripts (OKB_SCRIPT / OKB_SCRIPT2 at the fight's or team battle's frame, OKB_FESCRIPT
    /// at the front end's) and what completed since the last step.
    fn frame_input(&mut self, pads: [u16; 2]) -> FrameInput {
        // pads 3 / 4 only from the team battle's simulation version (an older replay's session never had them)
        let ext = if crate::fight::version::Gates::of(self.state.sim_version).team { self.ext } else { [0; 2] };
        let mut i = FrameInput { pad: pads, forced: [0; 2], flags: std::mem::take(&mut self.flags), ext, ..Default::default() };
        let e = &self.env;
        let (scripts, at) = if let Some(g) = &self.state.fight {
            ([&e.script, &e.script2, &e.script3, &e.script4], g.m.frame as usize)
        } else if let Some(t) = &self.state.team {
            ([&e.script, &e.script2, &e.script3, &e.script4], t.frame as usize)
        } else if let Some(f) = &self.state.front {
            ([&e.fescript[0], &e.fescript[1], &e.fescript[2], &e.fescript[3]], f.frame_no() as usize)
        } else {
            return i;
        };
        for (k, script) in scripts.iter().enumerate() {
            if let Some(&p) = script.get(at) {
                if k < 2 {
                    i.forced[k] = p;
                    i.flags |= FrameInput::FORCED[k];
                } else if self.state.fight.is_none() && crate::fight::version::Gates::of(self.state.sim_version).team {
                    i.forced_ext[k - 2] = p;
                    i.flags |= FrameInput::FORCED_EXT[k - 2];
                }
            }
        }
        i
    }

    /// The env debug hooks that change the fight at a frame: OKB_P2DMG=d@frame[,…], OKB_FORCEMOVE, OKB_BREAK.
    fn env_cheats(&self, cheats: &mut Vec<Cheat>) {
        let Some(g) = &self.state.fight else { return };
        let fr = g.m.frame;
        for &(d, at) in &self.env.p2dmg {
            if at == fr {
                cheats.push(Cheat::P2Dmg(d));
            }
        }
        if let Some((at, mv)) = self.env.forcemove {
            if fr >= at {
                let l = crate::fight::fx::native_tasks::debug_line(&g.m.fxn);
                if !l.is_empty() {
                    log::info!("f{fr}{l}");
                }
            }
            if fr == at {
                cheats.push(Cheat::ForceMove(mv));
            }
        }
        if let Some(side) = self.env.brk
            && fr == 330
        {
            cheats.push(Cheat::Break(side));
        }
    }
}
