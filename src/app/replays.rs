//! The app's replays: the always-on recording, the replay store's matches, playback.
//!
//! Engine addition.
//!
//! The app's replays (game-state plan stage 6): the always-on recording of the session (`session::replay::Recorder`;
//! stopped for good when the debug tools put a state back), each finished match added to the replay store
//! (`replay_store`: the newest `KEEP_LAST` kept, KEEP-marked ones always), debug OKB_RECORD's whole-session file at
//! exit, and playback: `--replay FILE`, or PLAY on the replay list (`replay_menu`, opened by the mode menu's REPLAY;
//! the live session is put aside meanwhile and comes back after). A playback's steps take the file's inputs (never
//! the pads, the debug scripts or the env cheats), wait for the loads the recording had done by then, run fast and
//! silent up to the match's VS screen ("SKIPPING n%"), and stop with "replay out of sync" at a checkpoint that
//! differs. Its controls (`playback`): pause and frame step, speeds, seeks back and on (snapshots every second of the
//! watched part, the frames after one re-simulated fast and silent), round starts, HUD and input display.

use super::App;
use super::playback::Controls;
use super::replay_menu::{Browser, Cmd};
use super::replay_store::Imported;
use super::tools::Pending;
use crate::frontend::options::Prefs;
use crate::session::replay::{Player, Pos, Replay};
use crate::session::rollback::Held;
use crate::session::{self, Cheat, FrameInput, GameState, Request};

/// Steps per drawn frame while a playback skips to the match's VS screen, or seeks.
pub const SKIP_STEPS: i32 = 600;
/// Drawn frames a playback waits for a load or fetch the recording had by then before it gives up (a step that
/// waits ends the drawn frame's steps: `App::step`'s false).
const MAX_WAIT: u32 = 600;
/// A seek snapshot every this many steps of the watched part, at most `MAX_KEYS` of them in `KEY_BYTES` (then
/// every other one goes). A fight's snapshot takes up to ~44 KB (a front-end one ~1.5 KB): the budget holds about
/// six minutes of a fight at one a second.
const KEY_EVERY: u32 = 60;
const MAX_KEYS: usize = 1200;
const KEY_BYTES: usize = 16 << 20;

/// Where a skip or a seek runs to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Target {
    /// this frame
    Frame(u32),
    /// the first frame of a round after the one of this index (`Round::index`; −1 = sudden death), or the end
    After(i8),
}

/// A seek snapshot: the state after a step, the data the next step reads, what the app had under way, the file's
/// position.
struct Key {
    frame: u32,
    snap: Vec<u8>,
    held: Held,
    pending: Pending,
    pos: Pos,
}

/// A replay played back (`--replay`, or PLAY on the list).
pub(super) struct Playback {
    pub p: Player,
    /// a skip (to the match's VS screen) or seek under way: fast, silent
    pub target: Option<Target>,
    /// where it started (its progress)
    pub skip_from: u32,
    /// out of sync: no more steps
    pub stopped: bool,
    /// the inputs ran out: back to the list (or, `--replay`, the app quits) after drawing the last state
    pub ended: bool,
    /// its session exists (`App::playback_started`): no steps before (a direct fight's match still loading, or
    /// `--replay` before the app's `Ready`)
    pub started: bool,
    waited: u32,
    /// PLAY on the replay list (its key): the end goes back there
    pub from_list: Option<String>,
    pub ctl: Controls,
    /// the last step's input (the input display)
    pub last: FrameInput,
    keys: Vec<Key>,
    /// (round index, its first frame) as seen in the watched part, by frame (a session of several fights has
    /// each fight's)
    pub rounds: Vec<(i8, u32)>,
    /// the round index of the last state (None: no fight): a change is a round's start
    round: Option<i8>,
}

impl Playback {
    /// `held`: the pads held as it starts (the button that chose PLAY is no control press).
    pub fn new(r: Replay, from_list: Option<String>, held: [u16; 2]) -> Self {
        let from = r.header.from;
        Self {
            p: Player::new(r),
            target: (from > 0).then_some(Target::Frame(from)),
            skip_from: 0,
            stopped: false,
            ended: false,
            started: false,
            waited: 0,
            from_list,
            ctl: Controls::new(held),
            last: FrameInput::default(),
            keys: Vec::new(),
            rounds: Vec::new(),
            round: None,
        }
    }

    /// The watched part's first frame.
    pub fn from(&self) -> u32 {
        self.p.replay.header.from
    }

    /// The skip's progress (0..=100).
    pub fn progress(&self) -> u32 {
        let to = match self.target {
            Some(Target::Frame(n)) => n,
            _ => self.p.replay.header.frames,
        };
        let (done, all) = (self.p.frame().saturating_sub(self.skip_from), to.saturating_sub(self.skip_from).max(1));
        (done as u64 * 100 / all as u64).min(100) as u32
    }
}

/// What PLAY put aside: the live session as it was between two steps, its recording.
pub(super) struct Live {
    state: GameState,
    held: Held,
    pending: Pending,
    rec: Option<session::replay::Recorder>,
}

impl App {
    /// The preferences the logic runs with: the replay's in a playback, the session's in netplay, else the app's.
    pub(super) fn logic_prefs(&self) -> Prefs {
        match (&self.player, &self.net) {
            (Some(pb), _) => pb.p.prefs(),
            (None, Some(n)) => n.prefs(),
            (None, None) => self.prefs,
        }
    }

    /// A playback's next step: its input, cheats and preferences; None while it waits for a load or fetch that
    /// the recording had completed by this step, and once it ended or stopped.
    pub(super) fn replay_input(&mut self) -> Option<(FrameInput, Vec<Cheat>, Prefs)> {
        let pb = self.player.as_mut()?;
        // nothing before the session's state exists (the app's steps before that would run on the empty default
        // state that `playback_started` replaces)
        if pb.stopped || pb.ended || !pb.started {
            return None;
        }
        let Some(next) = pb.p.peek() else {
            pb.ended = true;
            pb.target = None;
            log::info!("replay: the end at frame {}", pb.p.frame());
            return None;
        };
        let want = next.flags & (FrameInput::LOADED | FrameInput::FETCHED);
        if want & !self.flags != 0 {
            pb.waited += 1;
            if pb.waited > MAX_WAIT {
                self.replay_stop(&format!("frame {}: waits for a load or fetch that does not come", self.state.frame));
            } else if want & FrameInput::LOADED != 0 && self.load_req.is_some() {
                // the recording had the match by now: its load starts without waiting for the VS screen's frames
                self.start_load();
            }
            return None;
        }
        pb.waited = 0;
        // the completions the recording saw at this step (one that came sooner waits for its step)
        self.flags &= !want;
        let x = pb.p.next_step();
        if let Some((i, ..)) = &x {
            pb.last = *i;
        }
        x
    }

    /// After a playback's step: its checkpoint compared; the watched part's seek snapshots and round starts noted;
    /// a skip or seek ends at its target (the sound brought to the state there).
    pub(super) fn replay_stepped(&mut self) {
        let Some(pb) = &mut self.player else { return };
        if let Err(e) = pb.p.check(&self.state) {
            self.replay_stop(&e);
            return;
        }
        self.replay_note();
        let Some(pb) = &mut self.player else { return };
        let frame = self.state.frame;
        let reached = match pb.target {
            Some(Target::Frame(n)) => frame >= n,
            Some(Target::After(r)) => self.state.fight.as_ref().is_some_and(|g| g.m.round.index != r) && frame >= pb.from(),
            None => false,
        };
        if reached {
            pb.target = None;
            log::info!("replay: watching from frame {frame}");
            self.playback_sound();
        }
    }

    /// The state as a playback stands at (its start, or after a step): a round's start noted, and every
    /// `KEY_EVERY` frames of the watched part (from its first) a seek snapshot.
    fn replay_note(&mut self) {
        let Some(pb) = &mut self.player else { return };
        let frame = self.state.frame;
        let round = self.state.fight.as_ref().map(|g| g.m.round.index);
        if round != pb.round {
            pb.round = round;
            if let (Some(r), true) = (round, frame >= pb.from())
                && let Err(k) = pb.rounds.binary_search_by_key(&frame, |r| r.1)
            {
                pb.rounds.insert(k, (r, frame));
            }
        }
        if frame >= pb.from() && (frame - pb.from()).is_multiple_of(KEY_EVERY) && pb.keys.last().is_none_or(|k| k.frame < frame) {
            let held = Held::of(&self.store, pb.p.prefs());
            let pending = Pending { flags: self.flags, load_req: self.load_req, load_wait: self.load_wait };
            pb.keys.push(Key { frame, snap: session::save_state(&self.state), held, pending, pos: pb.p.pos() });
            if pb.keys.len() > MAX_KEYS || pb.keys.iter().map(|k| k.snap.len()).sum::<usize>() > KEY_BYTES {
                let mut k = 0;
                pb.keys.retain(|_| {
                    k += 1;
                    k % 2 == 1
                });
            }
        }
    }

    /// The playback's session exists (`start_session`, or `Ready` for `--replay`): its steps may run; a file
    /// watched from frame 0 gets its first seek snapshot here.
    pub(super) fn playback_started(&mut self) {
        let Some(pb) = &mut self.player else { return };
        pb.started = true;
        pb.round = None;
        self.replay_note();
    }

    /// Skipping to the match or seeking: no sound, no music loads.
    pub(super) fn replay_skipping(&self) -> bool {
        self.player.as_ref().is_some_and(|pb| pb.target.is_some())
    }

    /// A playback's steps make no sound: skipping, seeking, paused or stepping, or not at ×1.
    pub(super) fn replay_silent(&self) -> bool {
        self.player.as_ref().is_some_and(|pb| pb.target.is_some() || pb.ctl.quiet())
    }

    /// The sound as the playback stands: held while silent, else brought to the state.
    pub(super) fn playback_sound(&mut self) {
        if self.replay_silent() {
            self.audio.set_paused(true);
        } else {
            self.resync_audio();
            self.audio.set_paused(false);
        }
    }

    fn replay_stop(&mut self, why: &str) {
        log::error!("replay out of sync: {why}");
        if let Some(pb) = &mut self.player {
            pb.stopped = true;
            pb.target = None;
        }
        self.toast = Some(("replay out of sync".into(), u32::MAX));
        self.audio.set_paused(true);
    }

    /// A seek of the playback: to a frame (kept within the watched part), back to the latest snapshot at or before
    /// it (also forward, when one is beyond the current frame), then the frames up to it re-simulated fast and
    /// silent; or on to the next round's start. A snapshot that does not load leaves the playback where it is.
    pub(super) fn replay_seek(&mut self, target: Target) {
        let Some(pb) = &mut self.player else { return };
        let frame = self.state.frame;
        log::info!("replay: seek {target:?} at frame {frame} (rounds seen {:?})", pb.rounds);
        let to = match target {
            Target::Frame(n) => n.clamp(pb.from(), pb.p.replay.header.frames),
            // on until the next round starts
            Target::After(_) => {
                pb.skip_from = frame;
                pb.target = Some(target);
                self.playback_sound();
                return;
            }
        };
        if to == frame {
            return;
        }
        let key = pb.keys.iter().rposition(|k| k.frame <= to).filter(|&k| to < frame || pb.keys[k].frame > frame);
        if let Some(k) = key {
            let k = &pb.keys[k];
            let loaded = session::load_state(&k.snap, &k.held.assets());
            let (held, pending, pos) = (k.held.clone(), k.pending, k.pos);
            match loaded {
                Ok(s) => {
                    pb.p.seek(pos);
                    pb.ended = false;
                    pb.waited = 0;
                    pb.round = s.fight.as_ref().map(|g| g.m.round.index);
                    self.put_state(s, held, pending);
                    self.fetch_again();
                }
                Err(e) => {
                    log::warn!("replay: seek failed: {e}");
                    self.toast = Some((format!("seek failed: {e}"), 180));
                    return;
                }
            }
        } else if to < frame {
            return;
        }
        let Some(pb) = &mut self.player else { return };
        pb.skip_from = self.state.frame;
        pb.target = (self.state.frame < to).then_some(Target::Frame(to));
        self.playback_sound();
    }

    /// After a live step with this input, cheats and preferences: the recording goes on, or stops (logged).
    pub(super) fn record_step(&mut self, i: FrameInput, cheats: &[Cheat], prefs: Prefs) {
        let Some(rec) = &mut self.rec else { return };
        if let Err(e) = rec.step(i, cheats, prefs, &self.state) {
            self.record_stop(&e);
        }
    }

    /// The recording ends for this session (a state put back by the debug tools does not follow it).
    pub(super) fn record_stop(&mut self, why: &str) {
        if self.rec.take().is_some() {
            log::warn!("replay recording stopped: {why}");
        }
    }

    /// A live step's request as the recording sees it: a match's VS screen, its fight's start, its end (the
    /// match's file saved).
    pub(super) fn record_request(&mut self, r: &Request) {
        let time = self.rec_time();
        let Some(rec) = &mut self.rec else { return };
        match *r {
            Request::Load(_) | Request::LoadTeam(_) => rec.match_load(&self.state),
            // the team battle (engine addition): its file in FORMAT 2
            Request::TeamOver { outcome, rounds } => {
                if let Some(r) = rec.team_end(&self.state, outcome, rounds, time) {
                    self.save_match(r);
                }
            }
            Request::Fight => rec.match_start(&self.state),
            Request::FightOver { outcome, rounds, demo, .. } => {
                if let Some(r) = rec.match_end(&self.state, outcome, rounds, demo, time) {
                    self.save_match(r);
                }
            }
            _ => {}
        }
    }

    /// Debug OKB_RECORD=path (at exit): the whole session as a replay file.
    #[cfg(not(target_arch = "wasm32"))]
    pub(super) fn write_record(&self) {
        let Some(path) = &self.env.record else { return };
        let Some(rec) = self.rec.as_ref().or(self.live.as_ref().and_then(|l| l.rec.as_ref())) else {
            log::warn!("OKB_RECORD: no recording to write (stopped, or a playback)");
            return;
        };
        let state = self.live.as_ref().map_or(&self.state, |l| &l.state);
        let b = rec.replay(state, self.rec_time()).encode();
        match std::fs::write(path, &b) {
            Ok(()) => log::info!("OKB_RECORD: {} steps, {} bytes written to {path}", rec.frames(), b.len()),
            Err(e) => log::warn!("OKB_RECORD: {path}: {e}"),
        }
    }

    /// A finished match's file into the store, then the rotation: desktop on a thread of its own (the rotation reads
    /// the folder's files: no hitch at the KO), one save at a time; the browser's storage at once.
    fn save_match(&mut self, r: Replay) {
        #[cfg(not(target_arch = "wasm32"))]
        {
            self.join_save();
            let mut st = self.replays.clone();
            let save = std::thread::Builder::new().name("replay save".into()).spawn(move || match st.add(&r) {
                Ok((key, _)) => log::info!("replay saved: {key} ({} steps)", r.header.frames),
                Err(e) => log::warn!("replay not saved: {e}"),
            });
            match save {
                Ok(h) => self.saving = Some(h),
                Err(e) => log::warn!("replay not saved: {e}"),
            }
        }
        #[cfg(target_arch = "wasm32")]
        match self.replays.add(&r) {
            Ok((key, None)) => log::info!("replay saved: {key} ({} steps)", r.header.frames),
            Ok((_, Some(why))) => self.toast = Some((format!("replay: {why}: kept until the page closes"), 240)),
            Err(e) => self.toast = Some((format!("replay: {e}"), 240)),
        }
    }

    /// The match file being saved, finished (before the next one, the list, and at exit).
    #[cfg(not(target_arch = "wasm32"))]
    pub(super) fn join_save(&mut self) {
        if let Some(h) = self.saving.take() {
            let _ = h.join();
        }
    }

    #[cfg(target_arch = "wasm32")]
    pub(super) fn join_save(&mut self) {}

    // ------------------------------------------------------------------ the replay list

    /// The mode menu's REPLAY: the list opens over the waiting session.
    pub(super) fn open_list(&mut self, held: [u16; 2]) {
        self.join_save();
        self.browser = Some(Browser::new(self.replays.list(), held));
    }

    /// One 60 Hz frame of the list with these pads: its sounds through the front end's banks, its command run.
    pub(super) fn list_tick(&mut self, pads: [u16; 2]) {
        let pads = self.ui_pads(pads);
        let Some(b) = &mut self.browser else { return };
        let mut sounds = Vec::new();
        let cmd = b.tick(pads, &mut sounds);
        if let Some(f) = &self.state.front {
            let mut q = crate::output::sound::Queue::new(false);
            for id in sounds {
                f.play_se(&mut q, id);
            }
            self.audio.apply(&mut q);
        }
        let toast = |s: &mut Self, t: String, n: u32| {
            log::info!("replays: {t}");
            s.toast = Some((t, n));
        };
        match cmd {
            Cmd::None => {}
            Cmd::Close => {
                self.browser = None;
                // an IMPORT left to the next key press or click: not after the list
                #[cfg(target_arch = "wasm32")]
                super::web_files::unarm();
                // the △ / × that closed it does not reach the mode menu
                self.swallow = pads[0] | pads[1];
            }
            Cmd::Play(key) => match self.replays.read(&key).and_then(|b| Replay::decode(&b)).and_then(|r| {
                let install = self.assets.as_ref().and_then(crate::assets::manifest::br1_install_id);
                session::replay::br1_fits(&r.header.start, install).map(|()| r)
            }) {
                Ok(r) => {
                    // over other game data: played anyway
                    if let Some(w) = session::replay::content_warning(&r.header.start, self.content) {
                        log::warn!("replay {key}: {w}");
                    }
                    self.play_from_list(r, key, pads)
                }
                Err(e) => toast(self, format!("cannot play: {e}"), 300),
            },
            Cmd::Keep(key, keep) => self.edit_replay(&key, |h| h.keep = keep, if keep { "marked KEEP" } else { "KEEP cleared" }),
            Cmd::Rename(key, name) => self.edit_replay(&key, |h| h.name = Some(name), "renamed"),
            Cmd::Delete(key) => {
                match self.replays.remove(&key) {
                    Ok(()) => toast(self, "replay deleted".into(), 180),
                    Err(e) => toast(self, format!("not deleted: {e}"), 300),
                }
                let items = self.replays.list();
                if let Some(b) = &mut self.browser {
                    b.refresh(items, None);
                }
            }
            Cmd::Export(key) => self.export_replay(&key),
            Cmd::Import => self.import_replays(),
        }
    }

    /// KEEP / RENAME written back into the file's header.
    fn edit_replay(&mut self, key: &str, f: impl FnOnce(&mut session::replay::Header), done: &str) {
        let r = self.replays.read(key).and_then(|b| super::replay_store::edit(&b, f)).and_then(|b| self.replays.write(key, &b));
        self.toast = Some((
            match r {
                Ok(None) => format!("replay {done}"),
                Ok(Some(why)) => format!("replay {done} until the page closes ({why})"),
                Err(e) => format!("replay not changed: {e}"),
            },
            180,
        ));
        let items = self.replays.list();
        if let Some(b) = &mut self.browser {
            b.refresh(items, Some(key));
        }
    }

    /// EXPORT: desktop a copy in `replays/export` (its path in the toast), the browser a download.
    fn export_replay(&mut self, key: &str) {
        #[cfg(not(target_arch = "wasm32"))]
        let msg = match self.replays.export(key) {
            Ok(p) => format!("exported to {p}"),
            Err(e) => format!("not exported: {e}"),
        };
        #[cfg(target_arch = "wasm32")]
        let msg = match self.replays.read(key).and_then(|b| super::web_files::download(key, &b)) {
            Ok(()) => format!("downloading {key}"),
            Err(e) => format!("not exported: {e}"),
        };
        log::info!("replays: {msg}");
        self.toast = Some((msg, 300));
    }

    /// IMPORT: desktop the `.okbr` files of `replays/import`, one at a time (each one taken moved to
    /// `import/done`); the browser a file picker (its files come back as `UserEvent::Imported`), which a gamepad
    /// press cannot open (the browser wants a key or a click: the next one opens it).
    fn import_replays(&mut self) {
        #[cfg(not(target_arch = "wasm32"))]
        {
            let paths = self.replays.import_paths();
            if paths.is_empty() {
                let d = self.replays.import_dir();
                self.toast = Some((format!("put .okbr files in {} and IMPORT again", d.display()), 420));
                return;
            }
            let mut done = Vec::new();
            for p in paths {
                let name = p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                let r = crate::platform::storage::read_capped(&p, session::replay::MAX_BYTES).and_then(|b| self.replays.import(&b));
                if r.is_ok()
                    && let Err(e) = self.replays.import_done(&p)
                {
                    log::warn!("import {name}: not moved: {e}");
                }
                done.push((name, r));
            }
            self.import_report(done);
        }
        #[cfg(target_arch = "wasm32")]
        match super::web_files::pick(self.proxy.clone()) {
            Ok(true) => {}
            Ok(false) => self.toast = Some(("press a key or click to choose the files".into(), 420)),
            Err(e) => self.toast = Some((format!("import: {e}"), 300)),
        }
    }

    /// The browser's imported files (name, bytes): each validated in full, a valid one added marked KEEP unless
    /// the store has it.
    #[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
    pub(super) fn imported(&mut self, files: Vec<(String, Result<Vec<u8>, String>)>) {
        let done = files.into_iter().map(|(name, b)| (name, b.and_then(|b| self.replays.import(&b)))).collect();
        self.import_report(done);
    }

    /// An IMPORT's outcome: the counts and the first refusal in the toast, the list refreshed.
    fn import_report(&mut self, done: Vec<(String, Result<Imported, String>)>) {
        let (mut ok, mut there, mut bad, mut mem) = (0, 0, Vec::new(), Vec::new());
        for (name, r) in done {
            match r {
                Ok(Imported::Added(_, None)) => ok += 1,
                Ok(Imported::Added(key, Some(why))) => {
                    log::warn!("import {name}: {key} kept until the page closes ({why})");
                    ok += 1;
                    mem.push(why);
                }
                Ok(Imported::There(key)) => {
                    log::info!("import {name}: already there as {key}");
                    there += 1;
                }
                Err(e) => {
                    log::warn!("import {name}: {e}");
                    bad.push(format!("{name}: {e}"));
                }
            }
        }
        let mut msg = format!("imported {ok}");
        if let Some(why) = mem.first() {
            msg += &format!(" ({} until the page closes: {why})", mem.len());
        }
        if there > 0 {
            msg += &format!(", {there} already there");
        }
        if let Some(first) = bad.first() {
            msg += &format!(", refused {} ({first})", bad.len());
        }
        log::info!("replays: {msg}");
        self.toast = Some((msg, 420));
        let items = self.replays.list();
        if let Some(b) = &mut self.browser {
            b.refresh(items, None);
        }
    }

    /// PLAY with these pads held: the live session put aside (with its recording), the replay's session started in
    /// its place; a direct fight's match is loaded first (`UserEvent::ReplayData`; no steps meanwhile).
    fn play_from_list(&mut self, r: Replay, key: String, pads: [u16; 2]) {
        self.put_aside();
        self.browser = None;
        #[cfg(target_arch = "wasm32")]
        super::web_files::unarm();
        log::info!("replay {key}: {} steps from frame {} (engine {}, simulation v{})", r.header.frames, r.header.from, r.engine, r.sim_version);
        let direct = match &r.header.start.entry {
            session::Entry::DirectFight { p1, p2, stage } => Some((p1.clone(), p2.clone(), stage.clone())),
            _ => None,
        };
        let (costume, sim_version) = (r.header.costume, r.sim_version);
        self.player = Some(Playback::new(r, Some(key), pads));
        match direct {
            None => self.start_session(),
            Some((p1, p2, stage)) => {
                let Some(assets) = self.assets.clone() else { return };
                let (proxy, epoch) = (self.proxy.clone(), self.restore_gen);
                let fut = async move {
                    let ev = match crate::fight::fight_data::load_match(&assets, [&p1, &p2], costume, &stage, false, sim_version).await {
                        Ok(l) => super::UserEvent::ReplayData(Box::new(l), epoch),
                        Err(e) => super::UserEvent::ReplayFailed(e, epoch),
                    };
                    let _ = proxy.send_event(ev);
                };
                #[cfg(not(target_arch = "wasm32"))]
                pollster::block_on(fut);
                #[cfg(target_arch = "wasm32")]
                wasm_bindgen_futures::spawn_local(fut);
            }
        }
    }

    /// The live session put aside as it is between two steps, with its recording (PLAY on the replay list, a netplay
    /// session of the NETWORK menu): the store keeps the front end's data only, nothing of it plays on.
    pub(super) fn put_aside(&mut self) {
        let held = Held::of(&self.store, self.prefs);
        let pending = Pending { flags: self.flags, load_req: self.load_req, load_wait: self.load_wait };
        let state = std::mem::take(&mut self.state);
        self.live = Some(Box::new(Live { state, held, pending, rec: self.rec.take() }));
        // a new epoch: loads and fetches of the live session still under way are dropped
        self.restore_gen = self.restore_gen.wrapping_add(1);
        (self.flags, self.load_req, self.load_wait) = (0, None, 0);
        self.audio.stop_voices();
        self.audio.stop_music();
        self.vram_view.invalidate();
        let front = self.store.front.clone();
        self.store = session::AssetStore { front, ..Default::default() };
        (self.view, self.backdrop_view) = (None, None);
    }

    /// The live session back as it was put aside (its recording, its sound re-synced); false when there was none.
    pub(super) fn bring_back(&mut self) -> bool {
        let Some(live) = self.live.take() else { return false };
        let live = *live;
        self.rec = live.rec;
        // the live store's front-end data (a direct fight's playback dropped it)
        self.put_state(live.state, live.held, live.pending);
        self.vram_view.invalidate();
        self.fetch_again();
        self.resync_audio();
        self.audio.set_paused(self.paused);
        self.last = None;
        true
    }

    /// The playback's session from its start (`Ready` does the same for `--replay`), with the store as it is (a
    /// direct fight's match in it, or the front end's data only).
    pub(super) fn start_session(&mut self) {
        let Some(pb) = &self.player else { return };
        let start = pb.p.replay.start();
        let prefs = pb.p.prefs();
        if matches!(start.entry, session::Entry::DirectFight { .. }) {
            self.store.front = None;
        }
        let mut out = session::Out::new(self.audio.logging());
        self.state = session::new_state(&start, &self.store.assets(prefs), &mut out);
        if let (Some(r), Some(data)) = (&mut self.renderer, &self.store.fight) {
            self.view = Some(crate::present::fight_scene::FightView::new(r, data, &mut self.audio));
        }
        if !self.replay_silent() {
            self.audio.apply(&mut out.audio);
        }
        self.playback_started();
        self.playback_sound();
    }

    /// A playback's end (its last state drawn, △, or out of sync and left): `--replay` quits; from the list the
    /// live session comes back as it was (its sound re-synced) and the list opens again on the replay played.
    /// True when the app must quit.
    pub(super) fn end_playback(&mut self, held: [u16; 2]) -> bool {
        let Some(pb) = self.player.take() else { return false };
        let Some(key) = pb.from_list else { return true };
        if self.toast.as_ref().is_some_and(|t| t.1 == u32::MAX) {
            self.toast = None;
        }
        if !self.bring_back() {
            return true;
        }
        if let Some(v) = &mut self.view {
            v.hide_hud = false;
        }
        let mut b = Browser::new(Vec::new(), held);
        b.refresh(self.replays.list(), Some(&key));
        self.browser = Some(b);
        false
    }
}
