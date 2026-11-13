//! The rollback test: restores the state of N frames ago from its bytes and re-steps it.
//!
//! Engine addition.
//!
//! The rollback test (debug `ROLLBACK=N` in `sim` / `fetest`, `OKB_ROLLBACK=N` in the app; game-state plan stage
//! 4): proves that the state is complete and its steps deterministic. After every frame the state of N frames
//! before is restored **from its snapshot bytes** (`load_state`, so a field serde skips or `rebind` rebuilds wrongly
//! shows), the N logged frames run again on it with the static data they saw, muted and with their output thrown
//! away, and the result must hash as the state the frame left. A mismatch is reported with the section hashes of
//! both states, the first differing section and the first differing fields, and both states are dumped as JSON.

use super::{AssetStore, Assets, FileCache, GameState, hash, load_state, save_state, section_hashes};
use crate::fight::fight_data::Loaded;
use crate::frontend::FrontData;
use std::collections::VecDeque;
use std::io::Write;
use std::sync::Arc;
use std::time::{Duration, Instant};

/// The static data one frame ran with: shared handles to the app's store at the frame's start (the store changes
/// as matches load and end, files are fetched and dropped).
#[derive(Clone)]
pub struct Held {
    pub front: Option<Arc<FrontData>>,
    pub fight: Option<Arc<Loaded>>,
    pub backdrop: Option<Arc<Loaded>>,
    pub files: FileCache,
    pub prefs: crate::frontend::options::Prefs,
}

impl Held {
    pub fn of(store: &AssetStore, prefs: crate::frontend::options::Prefs) -> Self {
        Self { front: store.front.clone(), fight: store.fight.clone(), backdrop: store.backdrop.clone(), files: store.files.clone(), prefs }
    }

    pub fn assets(&self) -> Assets<'_> {
        Assets { front: self.front.as_ref(), fight: self.fight.as_deref(), backdrop: self.backdrop.as_deref(), files: &self.files, prefs: self.prefs }
    }

    /// The store as it was (a restored point goes on with it; the preferences stay the app's).
    pub fn into_store(self) -> AssetStore {
        AssetStore { front: self.front, fight: self.fight, backdrop: self.backdrop, files: self.files }
    }
}

/// The test's history and counts. `T` is a frame's log as its driver keeps it (the input and cheats of the step,
/// and whatever else the driver does to the state in that frame); its `resim` closure runs a logged frame again.
pub struct Rollback<T> {
    n: usize,
    /// the snapshots after the last frames, oldest first (at most N + 1 with the one taken after a check)
    snaps: VecDeque<Vec<u8>>,
    /// the frames run since the oldest snapshot: the data each ran with and its log
    frames: VecDeque<(Held, T)>,
    /// where the report goes: the file named by `<var>_LOG`, else stderr (a sim's own output stays as it is)
    out: Box<dyn Write>,
    /// where a mismatch's two states are written: `<var>_DUMP`, else the temp dir
    dump_dir: std::path::PathBuf,
    checks: u64,
    mismatches: u64,
    /// the test's time per frame: the hash, the restore, the N steps, the new snapshot
    total: Duration,
    max: Duration,
}

impl<T> Rollback<T> {
    /// Largest N accepted (a minute of frames; the history holds N snapshots and N frames' data).
    pub const MAX_N: usize = 10_000;

    /// From the env var `var` (N > 0 frames; unset, 0 or not a number: off; above `MAX_N`: off, with a warning).
    pub fn from_env(var: &str) -> Option<Self> {
        let n = std::env::var(var).ok()?.parse::<usize>().ok().filter(|&n| n > 0)?;
        if n > Self::MAX_N {
            log::warn!("{var}={n}: at most {} frames, the rollback test is off", Self::MAX_N);
            return None;
        }
        let out: Box<dyn Write> = match std::env::var(format!("{var}_LOG")) {
            Ok(p) if !p.is_empty() => match std::fs::File::create(&p) {
                Ok(f) => Box::new(std::io::LineWriter::new(f)),
                Err(e) => {
                    log::warn!("{var}_LOG: {p}: {e}");
                    Box::new(std::io::stderr())
                }
            },
            _ => Box::new(std::io::stderr()),
        };
        let dump_dir = std::env::var_os(format!("{var}_DUMP")).map(Into::into).unwrap_or_else(std::env::temp_dir);
        Some(Self::new(n, out, dump_dir))
    }

    /// `n` is capped at `MAX_N`.
    pub fn new(n: usize, out: Box<dyn Write>, dump_dir: std::path::PathBuf) -> Self {
        let n = n.min(Self::MAX_N);
        Self {
            n,
            snaps: VecDeque::with_capacity(n + 1),
            frames: VecDeque::with_capacity(n + 1),
            out,
            dump_dir,
            checks: 0,
            mismatches: 0,
            total: Duration::ZERO,
            max: Duration::ZERO,
        }
    }

    /// The session (re)starts from `s`: the history before it is dropped.
    pub fn start(&mut self, s: &GameState) {
        self.snaps.clear();
        self.frames.clear();
        self.snaps.push_back(save_state(s));
    }

    /// After a frame that ran with `held` (the data at its start) as logged in `log`, leaving `s`: the oldest
    /// snapshot (N frames back, fewer at the start) is loaded from its bytes and the logged frames run again on it
    /// with `resim`, muted; false on a mismatch (reported; the first one dumped). The first call without `start`
    /// only starts.
    pub fn check(&mut self, s: &GameState, held: Held, log: T, mut resim: impl FnMut(&mut GameState, &Assets, &T)) -> bool {
        if self.snaps.is_empty() {
            self.start(s);
            return true;
        }
        self.frames.push_back((held, log));
        if self.frames.len() > self.n {
            self.frames.pop_front();
            self.snaps.pop_front();
        }
        let frame = s.frame;
        let t = Instant::now();
        let h = hash(s);
        let first = &self.frames[0].0;
        let ok = match load_state(&self.snaps[0], &first.assets()) {
            Ok(mut c) => {
                crate::base::trace::muted(|| {
                    for (held, log) in &self.frames {
                        resim(&mut c, &held.assets(), log);
                    }
                });
                if hash(&c) == h { Ok(()) } else { Err(Ok(c)) }
            }
            Err(e) => Err(Err(e)),
        };
        self.snaps.push_back(save_state(s));
        let dt = t.elapsed();
        self.total += dt;
        self.max = self.max.max(dt);
        self.checks += 1;
        match ok {
            Ok(()) => true,
            Err(c) => {
                self.mismatches += 1;
                match c {
                    _ if self.mismatches > MAX_LINES => {}
                    Err(e) => {
                        let n = self.frames.len();
                        let _ = writeln!(self.out, "rollback: MISMATCH at frame {frame}: the snapshot of {n} frames back does not load: {e}");
                    }
                    // the first in full; later ones (often the same fault seen again) in a line each
                    Ok(c) if self.mismatches == 1 => self.report(s, &c),
                    Ok(c) => {
                        let (a, b) = (section_hashes(s), section_hashes(&c));
                        let diff: Vec<_> = a.iter().zip(&b).skip(1).filter(|(x, y)| x != y).map(|(x, _)| x.0).collect();
                        let _ = writeln!(self.out, "rollback: MISMATCH at frame {} in {diff:?}", s.frame);
                    }
                }
                false
            }
        }
    }

    /// The first mismatch: the section hashes of both, the first differing section and fields, both states dumped.
    fn report(&mut self, s: &GameState, c: &GameState) {
        let (a, b) = (section_hashes(s), section_hashes(c));
        let n = self.frames.len();
        let _ = writeln!(self.out, "rollback: MISMATCH at frame {}: the state re-simulated over the last {n} frames differs", s.frame);
        let _ = writeln!(self.out, "rollback:   {:12} {:16} {:16}", "section", "run", "re-simulated");
        for (x, y) in a.iter().zip(&b) {
            let _ = writeln!(self.out, "rollback:   {:12} {:016x} {:016x}{}", x.0, x.1, y.1, if x.1 != y.1 { "  *" } else { "" });
        }
        if a.len() != b.len() {
            let _ =
                writeln!(self.out, "rollback:   sections {:?} / {:?}", a.iter().map(|x| x.0).collect::<Vec<_>>(), b.iter().map(|x| x.0).collect::<Vec<_>>());
        }
        if let Some((x, _)) = a.iter().zip(&b).skip(1).find(|(x, y)| x != y) {
            let _ = writeln!(self.out, "rollback:   first differing section: {}", x.0);
        }
        let (ja, jb) = (serde_json::to_value(s), serde_json::to_value(c));
        if let (Ok(ja), Ok(jb)) = (&ja, &jb) {
            let mut diffs = Vec::new();
            json_diff(ja, jb, &mut String::new(), &mut diffs);
            for (path, x, y) in &diffs {
                let _ = writeln!(self.out, "rollback:   {path}: {x} / {y}");
            }
        }
        for (state, json, what) in [(s, ja, "run"), (c, jb, "resim")] {
            let path = self.dump_dir.join(format!("rollback_{}_{what}.json", s.frame));
            // JSON (one field per line, for a diff); a state JSON cannot hold goes out as its CBOR value's Debug
            let text = match json {
                Ok(v) => serde_json::to_string_pretty(&v).unwrap_or_default(),
                Err(_) => format!("{:#?}", ciborium::Value::serialized(state)),
            };
            match std::fs::write(&path, text) {
                Ok(()) => {
                    let _ = writeln!(self.out, "rollback:   dumped {}", path.display());
                }
                Err(e) => {
                    let _ = writeln!(self.out, "rollback:   dump {}: {e}", path.display());
                }
            }
        }
    }

    /// The summary line; the number of mismatches.
    pub fn finish(&mut self) -> u64 {
        let mean = if self.checks > 0 { self.total.as_secs_f64() * 1e3 / self.checks as f64 } else { 0.0 };
        let _ = writeln!(
            self.out,
            "rollback: N={} {} frames checked, {} mismatches; {mean:.3} ms per frame (max {:.3} ms) to restore, re-simulate, hash",
            self.n,
            self.checks,
            self.mismatches,
            self.max.as_secs_f64() * 1e3
        );
        let _ = self.out.flush();
        self.mismatches
    }
}

/// At most this many mismatches reported (the first in full, with the dumps).
const MAX_LINES: u64 = 100;

/// At most this many differing fields reported.
const MAX_DIFFS: usize = 20;

/// The leaves where `a` and `b` differ (path, a's value, b's value), in key order (serde_json's maps are sorted).
fn json_diff(a: &serde_json::Value, b: &serde_json::Value, path: &mut String, out: &mut Vec<(String, String, String)>) {
    use serde_json::Value;
    if out.len() >= MAX_DIFFS || a == b {
        return;
    }
    let len = path.len();
    match (a, b) {
        (Value::Object(x), Value::Object(y)) => {
            for (k, v) in x {
                path.push('.');
                path.push_str(k);
                json_diff(v, y.get(k).unwrap_or(&Value::Null), path, out);
                path.truncate(len);
            }
            for (k, v) in y.iter().filter(|(k, _)| !x.contains_key(*k)) {
                path.push('.');
                path.push_str(k);
                json_diff(&Value::Null, v, path, out);
                path.truncate(len);
            }
        }
        (Value::Array(x), Value::Array(y)) if x.len() == y.len() => {
            for (i, (v, w)) in x.iter().zip(y).enumerate() {
                path.push_str(&format!("[{i}]"));
                json_diff(v, w, path, out);
                path.truncate(len);
            }
        }
        _ => {
            let short = |v: &Value| {
                let t = v.to_string();
                if t.len() > 120 { format!("{}… ({} chars)", &t[..t.floor_char_boundary(120)], t.len()) } else { t }
            };
            out.push((if path.is_empty() { "state".into() } else { path.clone() }, short(a), short(b)));
        }
    }
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use super::*;
    use crate::session::{FrameInput, Out, step};

    fn held() -> Held {
        Held { front: None, fight: None, backdrop: None, files: FileCache::default(), prefs: Default::default() }
    }

    /// A change the log does not hold (made outside `step` at frame 5) is caught on that frame and the N − 1 after
    /// it, whose re-simulations start before it; from the snapshot that holds it on, they match again. The report
    /// names the session section and the field, and both states are dumped.
    #[test]
    fn catches_unlogged_change() {
        const N: usize = 4;
        let dir = std::env::temp_dir().join(format!("okb_rollback_test_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let report = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        struct Shared(std::sync::Arc<std::sync::Mutex<Vec<u8>>>);
        impl Write for Shared {
            fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
                self.0.lock().unwrap().extend_from_slice(b);
                Ok(b.len())
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }
        let mut rb = Rollback::<FrameInput>::new(N, Box::new(Shared(report.clone())), dir.clone());
        let mut s = GameState::default();
        rb.start(&s);
        let resim = |s: &mut GameState, a: &Assets, i: &FrameInput| step(s, a, *i, &[], &mut Out::default());
        let mut failed = Vec::new();
        for _ in 0..20 {
            let i = FrameInput::default();
            let h = held();
            step(&mut s, &h.assets(), i, &[], &mut Out::default());
            if s.frame == 5 {
                s.music_volume = 0x30; // not in the log
            }
            if !rb.check(&s, h, i, resim) {
                failed.push(s.frame);
            }
        }
        assert_eq!(failed, (5..5 + N as u32).collect::<Vec<_>>());
        assert_eq!(rb.finish(), N as u64);
        let text = String::from_utf8(report.lock().unwrap().clone()).unwrap();
        assert!(text.contains("MISMATCH at frame 5") && text.contains("first differing section: session"), "{text}");
        assert!(text.contains(".music_volume: 48 / 127"), "{text}");
        assert!(dir.join("rollback_5_run.json").exists() && dir.join("rollback_5_resim.json").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn json_diff_paths() {
        let a = serde_json::json!({"x": 1, "f": [{"hp": 3}, {"hp": 4}], "s": "a"});
        let b = serde_json::json!({"x": 1, "f": [{"hp": 3}, {"hp": 5}], "s": "b", "n": true});
        let mut d = Vec::new();
        json_diff(&a, &b, &mut String::new(), &mut d);
        let paths: Vec<_> = d.iter().map(|x| x.0.as_str()).collect();
        assert_eq!(paths, [".f[1].hp", ".s", ".n"]);
    }
}
