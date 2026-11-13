//! The debug tools' state side: save slots and the rewind history.
//!
//! Engine addition.
//! State: none of its own: holds runtime snapshots of `GameState`
//!
//! The debug tools' state side (game-state plan stage 5; the app's `OKB_DEBUG=1` / `--debug`): save slots and the
//! rewind history. Both hold runtime snapshots (`save_state`) with the static data their frames ran with (`Held`),
//! so a point is restored whatever the app's store holds now (another fight, the front end); `T` is what the app
//! keeps with a point besides the state (its requests under way). Restoring re-simulates logged frames muted with
//! their output thrown away; the app then re-syncs what lives outside the logic (VRAM, music, a movie, the fight's
//! GPU view) from the restored state.

use super::rollback::Held;
use super::{FrameInput, GameState, load_state, save_state};
use crate::frontend::LoadSpec;
use std::collections::VecDeque;

/// A saved point: the state's snapshot, the data it ran with and the app's part.
pub struct Slot<T> {
    pub bytes: Vec<u8>,
    pub held: Held,
    pub app: T,
}

impl<T> Slot<T> {
    /// `s` as it is between steps, `held` the store then.
    pub fn save(s: &GameState, held: Held, app: T) -> Self {
        Self { bytes: save_state(s), held, app }
    }

    /// The saved state (rebound to the slot's data).
    pub fn load(&self) -> Result<GameState, String> {
        load_state(&self.bytes, &self.held.assets())
    }
}

/// What a state put back waits for from the app, which no longer has it under way (the loads and fetches of before
/// are dropped): the match of the VS / demo-load screen not loaded yet, the files asked for and not fetched; none
/// whose completion `flags` (the next step's input) already reports.
pub fn awaited(s: &GameState, flags: u8) -> (Option<LoadSpec>, Option<Vec<String>>) {
    let load =
        s.front.as_ref().and_then(|f| f.load_wanted()).filter(|_| (s.spec.is_some() || s.team_spec.is_some()) && !s.pending && flags & FrameInput::LOADED == 0);
    let fetch = s.fetching.clone().filter(|_| flags & FrameInput::FETCHED == 0);
    (load, fetch)
}

/// The last `keep` frames: a snapshot every `every` frames and every frame's log (the data it ran with and `T`, its
/// input and cheats as the app keeps them). A frame back is the nearest snapshot at or before it plus the logged
/// frames after that, re-simulated; the frames after it are dropped (play goes on from there).
pub struct Rewind<T> {
    every: usize,
    keep: usize,
    /// the state's frame at the first snapshot (where the log starts)
    first: u32,
    /// (frame, snapshot) at `first`, `first + every`, …
    keys: VecDeque<(u32, Vec<u8>)>,
    /// `log[k]` ran from frame `first + k` to `first + k + 1`
    log: VecDeque<(Held, T)>,
}

/// The history's memory, for the overlay and the log.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Usage {
    pub keys: usize,
    pub key_bytes: usize,
    pub frames: usize,
    /// the log's own memory (its entries and the file tables they copy; the static data itself is shared)
    pub log_bytes: usize,
}

impl Usage {
    pub fn total(&self) -> usize {
        self.key_bytes + self.log_bytes
    }
}

impl<T> Rewind<T> {
    /// The app's history: a snapshot every 30 frames over the last 10 s.
    pub const EVERY: usize = 30;
    pub const KEEP: usize = 600;

    pub fn new(every: usize, keep: usize) -> Self {
        let every = every.max(1);
        Self { every, keep: keep.max(every), first: 0, keys: VecDeque::new(), log: VecDeque::new() }
    }

    /// The history (re)starts at `s` (a session start, a loaded slot).
    pub fn start(&mut self, s: &GameState) {
        self.keys.clear();
        self.log.clear();
        self.first = s.frame;
        self.keys.push_back((s.frame, snapshot(s)));
    }

    /// After a live step that ran with `held` (the store at its start) as logged in `log`, leaving `s`. A step that
    /// does not follow the history (none yet, a state put in from elsewhere) starts it again at `s`.
    pub fn push(&mut self, held: Held, log: T, s: &GameState) {
        if self.keys.is_empty() || s.frame != self.newest().wrapping_add(1) {
            self.start(s);
            return;
        }
        self.log.push_back((held, log));
        if self.log.len().is_multiple_of(self.every) {
            self.keys.push_back((s.frame, snapshot(s)));
        }
        // the oldest snapshot goes once the next one is `keep` frames back
        while self.keys.len() > 1 && self.newest().wrapping_sub(self.keys[1].0) as usize >= self.keep {
            self.keys.pop_front();
            self.log.drain(..self.every);
            self.first = self.keys[0].0;
        }
    }

    /// The frame of the newest state (the one the last push left).
    pub fn newest(&self) -> u32 {
        self.first.wrapping_add(self.log.len() as u32)
    }

    /// Frames that can be gone back.
    pub fn len(&self) -> usize {
        self.log.len()
    }

    pub fn is_empty(&self) -> bool {
        self.log.is_empty()
    }

    /// The state `k` frames before the newest (at most `len`): the snapshot at or before it, loaded with the data
    /// of the frame after it, and the logged frames up to it run again with `resim`, muted. The frames after it
    /// leave the history; returned with it is the log of the frame that followed it (the store and the app's part
    /// as that frame started, for the app to go on from). `Ok(None)` when there is nothing to go back to.
    pub fn back(&mut self, k: usize, mut resim: impl FnMut(&mut GameState, &Held, &T)) -> Result<Option<(GameState, Held, T)>, String> {
        let k = k.min(self.log.len());
        if k == 0 {
            return Ok(None);
        }
        let at = self.log.len() - k; // the target's index: log[at] ran from it
        let key = at / self.every;
        let from = key * self.every;
        let mut s = load_state(&self.keys[key].1, &self.log[from].0.assets())?;
        crate::base::trace::muted(|| {
            for (held, log) in self.log.range(from..at) {
                resim(&mut s, held, log);
            }
        });
        self.keys.truncate(key + 1);
        let next = self.log.drain(at..).next().expect("the frame after the target");
        Ok(Some((s, next.0, next.1)))
    }

    pub fn usage(&self) -> Usage {
        let entry = std::mem::size_of::<(Held, T)>();
        let file = std::mem::size_of::<(String, std::sync::Arc<[u8]>)>() + 16;
        Usage {
            keys: self.keys.len(),
            key_bytes: self.keys.iter().map(|(_, b)| b.capacity() + std::mem::size_of::<(u32, Vec<u8>)>()).sum(),
            frames: self.log.len(),
            log_bytes: self.log.iter().map(|(h, _)| entry + h.files.keys().map(|n| file + n.len()).sum::<usize>()).sum(),
        }
    }
}

/// A snapshot kept for long: no spare capacity.
fn snapshot(s: &GameState) -> Vec<u8> {
    let mut b = save_state(s);
    b.shrink_to_fit();
    b
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use super::*;
    use crate::session::{AssetStore, Cheat, Entry, FrameInput, Out, Request, SessionStart, hash, new_state, step};
    use std::sync::Arc;

    fn export() -> Option<crate::assets::Assets> {
        let root = crate::base::env::var("EXPORT").unwrap_or_else(|_| concat!(env!("CARGO_MANIFEST_DIR"), "/../okb_export").into());
        std::path::Path::new(&root).join("manifest.json").exists().then(|| crate::assets::Assets::new(root))
    }

    /// A frame as the app logs it: the input and the cheats.
    type Frame = (FrameInput, Vec<Cheat>);

    /// A session driven as the app does: the store follows the requests (loads at once, fetches at once, a fight's
    /// data dropped or kept as the backdrop), completions go into the next input.
    struct Run<'a> {
        assets: &'a crate::assets::Assets,
        store: AssetStore,
        s: GameState,
        flags: u8,
    }

    impl<'a> Run<'a> {
        fn new(assets: &'a crate::assets::Assets, store: AssetStore, start: &SessionStart) -> Self {
            let s = new_state(start, &store.assets(Default::default()), &mut Out::default());
            Self { assets, store, s, flags: 0 }
        }

        /// One step with pad 1 = `pad` and `cheats`; returns the store it started with and its log.
        fn step(&mut self, pad: u16, cheats: Vec<Cheat>) -> (Held, Frame) {
            let i = FrameInput { pad: [pad, 0], forced: [0; 2], flags: std::mem::take(&mut self.flags), ..Default::default() };
            let held = Held::of(&self.store, Default::default());
            let mut out = Out::default();
            step(&mut self.s, &held.assets(), i, &cheats, &mut out);
            for r in out.requests {
                match r {
                    Request::Load(spec) => self.serve_load(spec),
                    Request::Fetch(names) => {
                        for n in names {
                            if let Ok(b) = pollster::block_on(self.assets.fetch(&n)) {
                                self.store.files.insert(n, b.into());
                            }
                        }
                        self.flags |= FrameInput::FETCHED;
                    }
                    Request::FightOver { backdrop, .. } => {
                        let data = self.store.fight.take();
                        if backdrop {
                            self.store.backdrop = data;
                        }
                    }
                    _ => {}
                }
            }
            if self.s.backdrop.is_none() {
                self.store.backdrop = None;
            }
            self.store.prune(&self.s);
            (held, (i, cheats))
        }

        /// The match loaded into the store, reported to the next step.
        fn serve_load(&mut self, spec: crate::frontend::MatchSpec) {
            let chr = spec.chr.map(crate::assets::roster::code);
            let l = pollster::block_on(crate::fight::fight_data::load_match(
                self.assets,
                chr,
                spec.costume,
                crate::fight::fight_data::stage_code(spec.stage),
                false,
                crate::session::SIM_VERSION,
            ));
            self.store.fight = Some(Arc::new(l.expect("the match")));
            self.flags |= FrameInput::LOADED;
        }

        /// The point the app would restore: the state, the store as the next frame starts it, the pending flags.
        fn restore(&mut self, s: GameState, held: Held, flags: u8) {
            self.s = s;
            self.store = held.into_store();
            self.flags = flags;
        }
    }

    fn resim(s: &mut GameState, held: &Held, (i, c): &Frame) {
        step(s, &held.assets(), *i, c, &mut Out::default());
    }

    /// The fight start (YUG vs ALI, P2 the CPU) and the front end from the title (START, ARCADE, the select, a
    /// character confirmed → the VS screen, the match loaded and fought) as (store, start, pad and cheats by step).
    #[allow(clippy::type_complexity)]
    fn sessions(assets: &crate::assets::Assets) -> Vec<(&'static str, AssetStore, SessionStart, Box<dyn Fn(usize) -> (u16, Vec<Cheat>)>)> {
        let data = Arc::new(pollster::block_on(crate::session::start::load(assets, "YUG", "ALI", "ST0")).expect("the fight"));
        let script = crate::fight::input_script::parse_script("340:,20:R,1:J,12:,1:K,12:,1:DK,20:,1:R,1:RJK,30:,1:C,90:,1:J,12:,1:K,200:");
        let fight = SessionStart { seed: 1, entry: Entry::DirectFight { p1: "YUG".into(), p2: "ALI".into(), stage: "ST0".into() }, ..Default::default() };
        let front = Arc::new(pollster::block_on(crate::frontend::load(assets)).expect("front data"));
        let title = SessionStart { seed: 1, entry: Entry::Title, ..Default::default() };
        let fe = |k: usize| match k {
            40 => (crate::fight::input::pad::START, Vec::new()),
            70 => (crate::fight::input::pad::CROSS, Vec::new()),
            k if (150..400).contains(&k) && k % 40 == 0 => (crate::fight::input::pad::SQUARE, Vec::new()),
            _ => (0, Vec::new()),
        };
        vec![
            (
                "fight",
                AssetStore { fight: Some(data), ..Default::default() },
                fight,
                Box::new(move |k: usize| (script.get(k).copied().unwrap_or(0), if k == 500 { vec![Cheat::Gauges] } else { Vec::new() })),
            ),
            ("front end", AssetStore { front: Some(front), ..Default::default() }, title, Box::new(fe)),
        ]
    }

    /// A slot saved at step x, y more steps, the slot loaded (its state and store), the same y steps again: the
    /// same hash after each as the first time; in a direct fight and in the front end (title → select → VS → the
    /// match loaded and started: the store changes under the slot).
    #[test]
    fn slot_load_resim_same_hash() {
        let Some(assets) = export() else {
            eprintln!("skipped: no okb_export");
            return;
        };
        for (name, store, start, input) in sessions(&assets) {
            for (x, y) in [(0, 60), (200, 300), (340, 120)] {
                let mut run = Run::new(&assets, store.clone(), &start);
                for k in 0..x {
                    let (pad, c) = input(k);
                    run.step(pad, c);
                }
                let slot = Slot::save(&run.s, Held::of(&run.store, Default::default()), run.flags);
                let mut seen = Vec::new();
                for k in x..x + y {
                    let (pad, c) = input(k);
                    run.step(pad, c);
                    seen.push(hash(&run.s));
                }
                let what = run.s.front.as_ref().map(|f| f.describe()).unwrap_or_default();
                let s = slot.load().expect("the slot loads");
                run.restore(s, slot.held.clone(), slot.app);
                for (n, k) in (x..x + y).enumerate() {
                    let (pad, c) = input(k);
                    run.step(pad, c);
                    assert_eq!(hash(&run.s), seen[n], "{name}: slot of step {x}, step {k} after the load");
                }
                eprintln!("{name}: slot at {x}, {y} steps again: same hashes (now {what}{})", if run.s.fight.is_some() { " +fight" } else { "" });
            }
        }
    }

    /// What a restored state asks for again (`awaited`): files asked for and not fetched; and a slot file saved
    /// while the VS screen waits for its match, loaded where that load is no longer under way (the store without
    /// the match, no completion pending), asks for the match, and with it served the session reaches the fight.
    #[test]
    fn restored_vs_wait_reaches_fight() {
        let names = vec!["engine/sty/PIC00.DAT".to_string()];
        let s = GameState { fetching: Some(names.clone()), ..Default::default() };
        assert_eq!(awaited(&s, 0).1, Some(names));
        assert_eq!(awaited(&s, FrameInput::FETCHED).1, None);
        let Some(assets) = export() else {
            eprintln!("skipped: no okb_export");
            return;
        };
        let (_, store, start, input) = sessions(&assets).pop().expect("the front end");
        let mut run = Run::new(&assets, store.clone(), &start);
        let mut k = 0;
        while awaited(&run.s, 0).0.is_none() {
            assert!(k < 600, "no VS screen");
            let (pad, c) = input(k);
            run.step(pad, c);
            k += 1;
        }
        // the load under way completes: nothing asked again
        assert!(run.flags & FrameInput::LOADED != 0 && awaited(&run.s, run.flags).0.is_none());
        let what = run.s.front.as_ref().map(|f| f.describe()).unwrap_or_default();
        let file = crate::session::save_file(&run.s);
        let s = crate::session::load_file(&file, &store.assets(Default::default())).expect("the file loads");
        run.restore(s, Held::of(&store, Default::default()), 0);
        let (Some(crate::frontend::LoadSpec::Match(spec)), None) = awaited(&run.s, 0) else { panic!("{what}: the match not asked for again") };
        run.serve_load(spec);
        let end = k + 600;
        while run.s.fight.is_none() {
            assert!(k < end, "{what}: no fight after the restored VS screen");
            let (pad, c) = input(k);
            run.step(pad, c);
            k += 1;
        }
        eprintln!("restored at {what}: the fight at step {k}");
    }

    /// Going back k frames gives the state recorded k frames before (hash), across snapshots and the history's
    /// 10 s edge, again from the shortened history; play goes on from there exactly as the first time. Reports the
    /// history's memory and the time of a step back.
    #[test]
    fn rewind_k_frames_same_hash() {
        let Some(assets) = export() else {
            eprintln!("skipped: no okb_export");
            return;
        };
        for (name, store, start, input) in sessions(&assets) {
            let n = 900;
            let mut run = Run::new(&assets, store, &start);
            let mut rw = Rewind::<Frame>::new(Rewind::<Frame>::EVERY, Rewind::<Frame>::KEEP);
            rw.start(&run.s);
            let mut hashes = vec![hash(&run.s)];
            let mut flags = vec![0u8];
            for k in 0..n {
                let (pad, c) = input(k);
                let (held, log) = run.step(pad, c);
                rw.push(held, log, &run.s);
                hashes.push(hash(&run.s));
                flags.push(run.flags);
            }
            assert_eq!(rw.newest() as usize, n);
            assert_eq!(rw.len(), Rewind::<Frame>::KEEP + rw.newest() as usize % Rewind::<Frame>::EVERY, "{name}: frames kept");
            let u = rw.usage();
            eprintln!(
                "{name}: history {} snapshots {} KB + {} frames {} KB = {:.2} MB",
                u.keys,
                u.key_bytes / 1024,
                u.frames,
                u.log_bytes / 1024,
                u.total() as f64 / (1 << 20) as f64
            );
            assert!(u.total() < 16 << 20, "{name}: {u:?}");
            // going back by 1 (one display frame), by 2, a jump of 45 (over a snapshot), then to the oldest
            let mut at = n;
            let mut worst = 0.0f64;
            for k in [1, 2, 1, 45, 29, 30, 31, 1, 2, 7] {
                let t = std::time::Instant::now();
                let (s, held, (i, _)) = rw.back(k, resim).expect("loads").expect("a frame back");
                worst = worst.max(t.elapsed().as_secs_f64() * 1e3);
                at -= k;
                assert_eq!(s.frame as usize, at);
                assert_eq!(hash(&s), hashes[at], "{name}: back {k} to frame {at}");
                assert_eq!(rw.newest() as usize, at);
                assert_eq!(i.flags & !(FrameInput::FORCED[0] | FrameInput::FORCED[1]), flags[at], "{name}: the next frame's flags");
                run.restore(s, held, i.flags);
            }
            eprintln!("{name}: a step back takes at most {worst:.2} ms");
            // play on from there to the end: the same states as the first time
            for k in at..n {
                let (pad, c) = input(k);
                let (held, log) = run.step(pad, c);
                rw.push(held, log, &run.s);
                assert_eq!(hash(&run.s), hashes[k + 1], "{name}: frame {} played on after the rewind", k + 1);
            }
            // as far back as the history goes
            let oldest = rw.newest() as usize - rw.len();
            let (s, ..) = rw.back(usize::MAX, resim).expect("loads").expect("a frame back");
            assert_eq!((s.frame as usize, hash(&s)), (oldest, hashes[oldest]), "{name}: the oldest frame");
            assert!(rw.back(1, resim).expect("nothing to load").is_none());
        }
    }

    /// The history without the export: the frames kept between `keep` and `keep + every - 1`, going back one
    /// frame at a time over its whole length gives every recorded state again, then nothing.
    #[test]
    fn rewind_bookkeeping() {
        let held = || Held { front: None, fight: None, backdrop: None, files: Default::default(), prefs: Default::default() };
        let mut rw = Rewind::<FrameInput>::new(4, 10);
        let mut s = GameState::default();
        rw.push(held(), FrameInput::default(), &s); // no history yet: starts it
        assert_eq!((rw.newest(), rw.len()), (0, 0));
        let mut hashes = vec![hash(&s)];
        for k in 0..23u16 {
            let i = FrameInput { pad: [k, 0], ..Default::default() };
            let h = held();
            step(&mut s, &h.assets(), i, &[], &mut Out::default());
            rw.push(h, i, &s);
            hashes.push(hash(&s));
            assert!(rw.len() >= 10.min(s.frame as usize) && rw.len() < 14, "{} frames at {}", rw.len(), s.frame);
        }
        assert_eq!((rw.newest(), rw.len(), rw.usage().keys), (23, 11, 3));
        let mut at = 23;
        while let Some((s, _, i)) = rw.back(1, |s, h, i| step(s, &h.assets(), *i, &[], &mut Out::default())).unwrap() {
            at -= 1;
            assert_eq!((s.frame, hash(&s), i.pad[0]), (at, hashes[at as usize], at as u16));
        }
        assert_eq!(at, 12);
        // a push that does not follow (a state from elsewhere) starts the history again
        rw.push(held(), FrameInput::default(), &GameState { frame: 100, ..Default::default() });
        assert_eq!((rw.newest(), rw.len()), (100, 0));
    }
}
