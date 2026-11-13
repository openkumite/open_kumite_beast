//! The debug tools: pause and frame step, save slots, rewind (`OKB_DEBUG=1`, `--debug`).
//!
//! Engine addition.
//!
//! The debug tools (engine additions, `OKB_DEBUG=1` / `--debug` / `?debug=1`; game-state plan stage 5), off by
//! default: pause and frame step on every screen (P, `.`), save slots (F12 saves, Shift+F12 loads, Shift+F1..F4
//! picks slot 1..4, again slot 0), hold Backspace to rewind (Shift+Backspace twice as fast). A restored state is the
//! logic only: the app then re-syncs what lives outside it from the state (the fight's GPU view and VRAM, the front
//! end's VRAM through `VramView`, the music at its frame, a movie's media and music), and from then on the memory
//! card is no longer written to the disk (the restored card is not the player's). Debug OKB_DEBUGKEYS (desktop)
//! presses the tools' keys at drawn frames (`parse_keys`).

use super::App;
use crate::present::fight_scene;
use crate::session::debug_history::{Rewind, Slot};
use crate::session::rollback::Held;
use crate::session::{self, Cheat, FrameInput, GameState};
use std::sync::Arc;
use winit::event_loop::ActiveEventLoop;
use winit::keyboard::KeyCode;

/// What the app has under way between steps besides its store, kept with a point to go on from it: the completions
/// for the next step's input, a match waiting to load after the VS screen.
#[derive(Clone, Copy, Debug, Default)]
pub(super) struct Pending {
    pub flags: u8,
    pub load_req: Option<crate::frontend::LoadSpec>,
    pub load_wait: u32,
}

/// A frame of the rewind history: its input and cheats, and the app's part as it started.
pub(super) type Frame = (FrameInput, Vec<Cheat>, Pending);

/// A tool key (or a OKB_DEBUGKEYS entry).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(target_arch = "wasm32", allow(dead_code))]
pub(super) enum Action {
    Pause,
    Step,
    Save,
    Load,
    /// the slot F12 / Shift+F12 use (the same one again: slot 0)
    Slot(usize),
    /// Backspace held (frames back per drawn frame), released (None)
    Rewind(Option<usize>),
    /// OKB_DEBUGKEYS only: an F7 screenshot named by the drawn frame, quit
    Shot,
    Quit,
}

pub(super) const SLOTS: usize = 5;

pub(super) struct Tools {
    slots: [Option<Slot<Pending>>; SLOTS],
    slot: usize,
    pub rewind: Rewind<Frame>,
    /// Backspace held: frames back per drawn frame
    pub rewinding: Option<usize>,
    /// frames stepped while paused: the sound is re-synced when play goes on
    stepped: bool,
    /// a state was put back: the memory card is no longer stored (told once)
    restored: bool,
    card_noted: bool,
    /// the steps back since the key went down: count, total and longest ms
    ticks: (u32, f64, f64),
    /// the history filled up once (its memory logged)
    full_noted: bool,
    /// OKB_DEBUGKEYS and the drawn frames counted for it
    script: Vec<(u64, Action)>,
    pub drawn: u64,
    /// a OKB_DEBUGKEYS screenshot's file name
    pub shot_name: Option<String>,
}

impl Tools {
    pub fn new(script: Vec<(u64, Action)>) -> Self {
        Self {
            slots: Default::default(),
            slot: 0,
            rewind: Rewind::new(Rewind::<Frame>::EVERY, Rewind::<Frame>::KEEP),
            rewinding: None,
            stepped: false,
            restored: false,
            card_noted: false,
            ticks: (0, 0.0, 0.0),
            full_noted: false,
            script,
            drawn: 0,
            shot_name: None,
        }
    }

    /// The OKB_DEBUGKEYS entries due at the current drawn frame.
    pub fn due(&self) -> Vec<Action> {
        self.script.iter().filter(|(f, _)| *f == self.drawn).map(|(_, a)| *a).collect()
    }

    /// After a live step: once the history is full, its memory to the log.
    pub fn note_history(&mut self) {
        if !self.full_noted && self.rewind.len() >= Rewind::<Frame>::KEEP {
            self.full_noted = true;
            log::info!("debug: rewind history full: {}", usage_text(&self.rewind));
        }
    }
}

/// OKB_DEBUGKEYS="frame:key,…" (frames drawn since the start; several keys at one frame in order): P, `.` (step),
/// F12 (save), SF12 (load), S1..S4 (Shift+F1..F4), BS / SBS (Backspace / Shift+Backspace down), /BS (up), F7
/// (screenshot `okb_dbg_<frame>.png`), Q (quit). Unknown entries are skipped with a warning.
#[cfg_attr(target_arch = "wasm32", allow(dead_code))]
pub(super) fn parse_keys(s: &str) -> Vec<(u64, Action)> {
    let mut out = Vec::new();
    for e in s.split(',').map(str::trim).filter(|e| !e.is_empty()) {
        let parsed = e.split_once(':').and_then(|(f, k)| {
            let a = match k {
                "P" => Action::Pause,
                "." => Action::Step,
                "F12" => Action::Save,
                "SF12" => Action::Load,
                "S1" | "S2" | "S3" | "S4" => Action::Slot(k[1..].parse().ok()?),
                "BS" => Action::Rewind(Some(1)),
                "SBS" => Action::Rewind(Some(2)),
                "/BS" => Action::Rewind(None),
                "F7" => Action::Shot,
                "Q" => Action::Quit,
                _ => return None,
            };
            Some((f.parse::<u64>().ok()?, a))
        });
        match parsed {
            Some(x) => out.push(x),
            None => log::warn!("OKB_DEBUGKEYS: {e:?} skipped"),
        }
    }
    out
}

fn usage_text<T>(r: &Rewind<T>) -> String {
    let u = r.usage();
    format!(
        "{} frames ({:.1} s), {} snapshots {} KB + log {} KB = {:.2} MB",
        u.frames,
        u.frames as f64 / 60.0,
        u.keys,
        u.key_bytes / 1024,
        u.log_bytes / 1024,
        u.total() as f64 / (1 << 20) as f64
    )
}

impl App {
    /// A key of the tools (pressed: `down`); true when it was one (it does not reach the pads: Backspace is P1's
    /// SELECT without the tools).
    pub(super) fn tool_key(&mut self, k: KeyCode, down: bool, repeat: bool, el: &ActiveEventLoop) -> bool {
        // a replay played from the list: its own controls (Backspace is SELECT, the HUD key, there)
        if self.player.is_some() {
            return false;
        }
        let shift = self.mods.shift_key();
        let a = match k {
            KeyCode::Backspace if !down => Some(Action::Rewind(None)),
            KeyCode::Backspace if !repeat => Some(Action::Rewind(Some(if shift { 2 } else { 1 }))),
            KeyCode::Backspace => None,
            _ if !down => return false,
            KeyCode::KeyP if !repeat => Some(Action::Pause),
            KeyCode::Period => Some(Action::Step),
            KeyCode::F12 if !repeat => Some(if shift { Action::Load } else { Action::Save }),
            KeyCode::F1 | KeyCode::F2 | KeyCode::F3 | KeyCode::F4 if shift => {
                (!repeat).then(|| Action::Slot([KeyCode::F1, KeyCode::F2, KeyCode::F3, KeyCode::F4].iter().position(|&f| f == k).unwrap_or(0) + 1))
            }
            _ => return false,
        };
        if let Some(a) = a {
            self.tool_action(a, el);
        }
        true
    }

    pub(super) fn tool_action(&mut self, a: Action, el: &ActiveEventLoop) {
        let Some(t) = &mut self.tools else { return };
        if self.player.is_some() && a != Action::Quit && a != Action::Shot {
            return;
        }
        match a {
            Action::Pause => self.toggle_pause(),
            // frame advance while paused: one whole step
            Action::Step if self.paused => {
                t.stepped = true;
                let pads = self.pads();
                self.step([pads[0] | self.latch[0], pads[1] | self.latch[1]]);
                self.latch = [0; 2];
            }
            Action::Step => {}
            Action::Slot(n) => {
                t.slot = if t.slot == n { 0 } else { n };
                let text = format!("slot {} {}", t.slot, if t.slots[t.slot].is_some() { "(saved)" } else { "(empty)" });
                self.tool_toast(text);
            }
            Action::Save => self.save_slot(),
            Action::Load => self.load_slot(),
            Action::Rewind(Some(n)) => {
                if t.rewinding.is_none() {
                    t.ticks = (0, 0.0, 0.0);
                }
                t.rewinding = Some(n);
                self.audio.set_paused(true);
            }
            Action::Rewind(None) => self.end_rewind(),
            Action::Shot => {
                t.shot_name = Some(format!("okb_dbg_{:05}.png", t.drawn));
                self.shot_req = true;
            }
            Action::Quit => el.exit(),
        }
    }

    /// P (F9): the pause; with the tools the sound holds too, and frames stepped meanwhile re-sync it on resume.
    pub(super) fn toggle_pause(&mut self) {
        self.paused = !self.paused;
        let Some(t) = &mut self.tools else { return };
        let stepped = std::mem::take(&mut t.stepped);
        let hold = self.paused || t.rewinding.is_some();
        if !self.paused && stepped {
            self.resync_audio();
        }
        self.audio.set_paused(hold);
        self.last = None; // no catch-up for the time paused
    }

    fn tool_toast(&mut self, t: String) {
        log::info!("debug: {t}");
        self.toast = Some((t, 180));
    }

    /// F12: the state between steps into the current slot (with the store and what is under way), and as a
    /// save-state file `debug_slot<n>.okbs` beside the save (desktop).
    fn save_slot(&mut self) {
        let Some(t) = &mut self.tools else { return };
        let n = t.slot;
        let held = Held::of(&self.store, self.prefs);
        let slot = Slot::save(&self.state, held, Pending { flags: self.flags, load_req: self.load_req, load_wait: self.load_wait });
        let kb = slot.bytes.len() as f64 / 1024.0;
        t.slots[n] = Some(slot);
        #[allow(unused_mut)]
        let mut text = format!("slot {n} saved at frame {} ({kb:.1} KB)", self.state.frame);
        #[cfg(not(target_arch = "wasm32"))]
        {
            let path = crate::platform::storage::debug_path(&format!("debug_slot{n}.okbs"));
            let ok = path.parent().is_some_and(|d| std::fs::create_dir_all(d).is_ok()) && std::fs::write(&path, session::save_file(&self.state)).is_ok();
            text += &if ok { format!(", file {}", path.display()) } else { format!(", file {} not written", path.display()) };
        }
        self.tool_toast(text);
    }

    /// Shift+F12: the current slot back (or, when this run saved none there, its file with the data the app holds
    /// now); the history restarts there and the sound is re-synced.
    fn load_slot(&mut self) {
        let Some(t) = &self.tools else { return };
        let n = t.slot;
        let loaded = match &t.slots[n] {
            Some(slot) => slot.load().map(|s| (s, slot.held.clone(), slot.app)),
            None => self.load_slot_file(n),
        };
        match loaded {
            Ok((s, held, p)) => {
                let frame = s.frame;
                self.restore(s, held, p);
                if let Some(t) = &mut self.tools {
                    t.rewind.start(&self.state);
                }
                self.fetch_again();
                self.resync_audio();
                self.tool_toast(format!("slot {n} loaded: frame {frame}"));
            }
            Err(e) => self.tool_toast(format!("slot {n} not loaded: {e}")),
        }
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn load_slot_file(&self, n: usize) -> Result<(GameState, Held, Pending), String> {
        // an earlier build's slot file (`.br2s`) when this build wrote none
        let path = crate::platform::storage::debug_path(&format!("debug_slot{n}.okbs"));
        let path = if path.exists() { path } else { crate::platform::storage::debug_path(&format!("debug_slot{n}.br2s")) };
        let meta = std::fs::metadata(&path).map_err(|e| format!("{}: {e}", path.display()))?;
        if meta.len() > session::MAX_STATE_BYTES as u64 {
            return Err(format!("{}: {} bytes", path.display(), meta.len()));
        }
        let b = std::fs::read(&path).map_err(|e| format!("{}: {e}", path.display()))?;
        let s = session::load_file(&b, &self.store.assets(self.prefs))?;
        Ok((s, Held::of(&self.store, self.prefs), Pending::default()))
    }

    #[cfg(target_arch = "wasm32")]
    fn load_slot_file(&self, _n: usize) -> Result<(GameState, Held, Pending), String> {
        Err("empty".into())
    }

    /// One 1/60 s of Backspace held: back by its frames (the nearest snapshot loaded, the frames up to the target
    /// re-simulated muted), the views re-synced; the sound stays held.
    pub(super) fn rewind_tick(&mut self) {
        let Some(t) = &mut self.tools else { return };
        let Some(k) = t.rewinding else { return };
        let t0 = crate::perf_now();
        let r = t.rewind.back(k, |s, held, (i, c, _)| session::step(s, &held.assets(), *i, c, &mut session::Out::default()));
        match r {
            Ok(Some((s, held, (_, _, p)))) => {
                self.restore(s, held, p);
                if let Some(t) = &mut self.tools {
                    let ms = (crate::perf_now() - t0) * 1000.0;
                    t.ticks = (t.ticks.0 + 1, t.ticks.1 + ms, t.ticks.2.max(ms));
                }
            }
            // the history's start: it stays there
            Ok(None) => {}
            Err(e) => {
                self.end_rewind();
                self.tool_toast(format!("rewind failed: {e}"));
            }
        }
    }

    /// Backspace up: play goes on from the state rewound to (when it went back at all: the files it waits for asked
    /// for again, its sound re-synced); the history and the cost logged.
    pub(super) fn end_rewind(&mut self) {
        let Some(t) = &mut self.tools else { return };
        if t.rewinding.take().is_none() {
            return;
        }
        let (n, sum, max) = t.ticks;
        log::info!(
            "debug: rewound {n} ticks to frame {}, {:.2} ms per tick (max {max:.2}); history {}",
            self.state.frame,
            if n > 0 { sum / n as f64 } else { 0.0 },
            usage_text(&t.rewind)
        );
        if n > 0 {
            self.fetch_again();
            self.resync_audio();
        }
        self.audio.set_paused(self.paused);
        self.last = None;
    }

    /// After a state was put back (not while rewinding): the files it waits for, asked for again.
    pub(super) fn fetch_again(&mut self) {
        if let (_, Some(names)) = session::debug_history::awaited(&self.state, self.flags) {
            log::info!("debug: fetch asked for again: {names:?}");
            self.request(session::Request::Fetch(names));
        }
    }

    /// The logic put back to `s` (a slot, a rewind): the store as the next frame starts it, what was under way then,
    /// and the presentation re-synced from the state: the fight's and the backdrop's GPU views (kept when their
    /// data is the same, built when not), the VRAM (the fight's, the backdrop stage's, or the front end's through
    /// `VramView`). Loads and fetches still under way are dropped when they complete (`App::restore_gen`); the match
    /// the state waits for is asked for again here, its files by the callers (`fetch_again`), which also re-sync the
    /// sound (not while rewinding).
    fn restore(&mut self, s: GameState, held: Held, p: Pending) {
        self.record_stop("a debug load / rewind put a state back");
        self.put_state(s, held, p);
        if let Some(t) = &mut self.tools {
            t.restored = true;
        }
    }

    /// `restore` without its debug side (the recording and the card go on): the live session back after a replay
    /// played from the list, a playback's seek.
    pub(super) fn put_state(&mut self, s: GameState, held: Held, p: Pending) {
        let mut old = [(self.store.fight.take(), self.view.take()), (self.store.backdrop.take(), self.backdrop_view.take())];
        self.store = held.into_store();
        self.state = s;
        self.flags = p.flags & !(FrameInput::FORCED[0] | FrameInput::FORCED[1]);
        self.load_req = p.load_req;
        self.load_wait = p.load_wait;
        self.cheats.clear();
        self.latch = [0; 2];
        self.restore_gen = self.restore_gen.wrapping_add(1);
        if let (Some(spec), None) = (session::debug_history::awaited(&self.state, self.flags).0, self.load_req) {
            self.load_req = Some(spec);
            self.load_wait = 0;
            self.store.fight = None;
        }
        let mut view =
            |data: &Option<Arc<crate::fight::fight_data::Loaded>>, wanted: bool, r: &mut Option<crate::render::Renderer>, audio: &mut crate::audio::Audio| {
                let data = data.as_ref().filter(|_| wanted)?;
                let kept = old.iter_mut().find(|(d, v)| v.is_some() && d.as_ref().is_some_and(|d| Arc::ptr_eq(d, data))).and_then(|(_, v)| v.take());
                kept.or_else(|| r.as_mut().map(|r| fight_scene::FightView::new(r, data, audio)))
            };
        let fighting = self.state.fight.is_some() || self.state.team.is_some();
        self.view = view(&self.store.fight, fighting, &mut self.renderer, &mut self.audio);
        self.backdrop_view = view(&self.store.backdrop, self.state.backdrop.is_some(), &mut self.renderer, &mut self.audio);
        let stage = if fighting { self.store.fight.as_ref() } else { self.store.backdrop.as_ref().filter(|_| self.state.backdrop.is_some()) };
        if let (Some(r), Some(data)) = (&mut self.renderer, stage) {
            fight_scene::upload_vram(r, data);
        }
        self.vram_view.invalidate();
        // a movie's stream follows the state (its music decoded with the sound's re-sync)
        if let Some(f) = &self.state.front {
            self.vram_view.follow(f, &self.store.files);
        }
        if let Some(rb) = &mut self.rollback {
            rb.start(&self.state);
        }
    }

    /// The sound brought to the state: the effects cut, the sound options and the XA volume, the music at its frame
    /// (from the fight's or the front end's data, else loaded), then a playing movie's media and music at its frame.
    pub(super) fn resync_audio(&mut self) {
        self.audio.stop_voices();
        if let Some(f) = &self.state.front {
            let s = &f.save.settings;
            self.audio.set_options(s.master, s.bgm_volume, s.output != 0);
        }
        self.audio.set_music_scale(self.state.music_volume as f32 / 0x7F as f32);
        match self.state.music {
            None => self.audio.stop_music(),
            Some(m) => {
                let secs = m.frames as f64 / 60.0;
                let have = self.store.fight.as_ref().and_then(|d| d.music.iter().find(|(t, _)| *t == m.track).map(|(_, x)| x.clone()));
                match have.or_else(|| self.store.front.as_ref().and_then(|d| d.music(m.track))) {
                    Some(pcm) => self.audio.play_music_from(pcm, m.looped, secs),
                    None => {
                        self.audio.stop_music();
                        self.load_music(m.track, m.looped, secs);
                    }
                }
                log::info!("debug: music {:#x} re-synced at {secs:.2} s (looped {})", m.track, m.looped);
            }
        }
        self.vram_view.forget_movie();
        if let Some(f) = &self.state.front
            && let Some((m, at)) = self.vram_view.step(f, &self.store.files)
        {
            log::info!("debug: movie music re-synced at {at:.2} s");
            self.audio.play_music_from(m, false, at);
        }
    }

    /// The memory card store of a step after a state was put back: not written (the card is the restored state's,
    /// not the player's), reported stored so the game goes on. False when the card is stored as usual.
    pub(super) fn card_held(&mut self) -> bool {
        let Some(t) = &mut self.tools else { return false };
        if !t.restored {
            return false;
        }
        if !std::mem::replace(&mut t.card_noted, true) {
            self.tool_toast("memory card not saved after a load / rewind (until the app restarts)".into());
        }
        self.flags |= FrameInput::CARD_STORED;
        true
    }

    /// The tools' panel lines (under the performance panel): paused / rewinding with the frame, the slot, the
    /// history and the cost of a step back (with the performance overlay or while rewinding).
    pub(super) fn tool_lines(&self) -> Vec<String> {
        let Some(t) = &self.tools else { return Vec::new() };
        let mut v = Vec::new();
        let what = match t.rewinding {
            Some(n) => format!("REWIND x{n}"),
            None if self.paused => "PAUSED".into(),
            None if self.perf.on => "PLAY".into(),
            None => return v,
        };
        v.push(format!("{what} frame {}", self.state.frame));
        v.push(format!("slot {}{}", t.slot, if t.slots[t.slot].is_some() { " saved" } else { "" }));
        if t.restored {
            v.push("card not saved".into());
        }
        if self.perf.on || t.rewinding.is_some() {
            let u = t.rewind.usage();
            v.push(format!("history {:.1} s {:.2} MB", u.frames as f64 / 60.0, u.total() as f64 / (1 << 20) as f64));
            if t.ticks.0 > 0 {
                v.push(format!("back {:.2} ms (max {:.2})", t.ticks.1 / t.ticks.0 as f64, t.ticks.2));
            }
        }
        v
    }
}

#[cfg(test)]
mod tests {
    use super::{Action, parse_keys};

    #[test]
    fn debug_keys_parse() {
        assert_eq!(
            parse_keys("10:P, 11:., 12:F12,12:SF12,13:S3,20:BS,30:/BS,31:SBS,40:F7,50:Q,60:X,abc"),
            vec![
                (10, Action::Pause),
                (11, Action::Step),
                (12, Action::Save),
                (12, Action::Load),
                (13, Action::Slot(3)),
                (20, Action::Rewind(Some(1))),
                (30, Action::Rewind(None)),
                (31, Action::Rewind(Some(2))),
                (40, Action::Shot),
                (50, Action::Quit),
            ]
        );
    }
}
