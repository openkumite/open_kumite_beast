//! Cross-platform determinism check: step logs re-run on any target, the hash after every step.
//!
//! Engine addition.
//! State: the step log file (`BR2T`); not part of `GameState`
//!
//! Cross-platform determinism check (game-state stage 7): the state hash after every step must come out the same on
//! every target (macOS / Linux / Windows native, wasm32 in the browser). A headless run's step log (`STEPLOG=file`
//! of `sim` / `fetest`: its data, its start state and everything it did to the state, in order) is re-run here on
//! any target; `replay --trace` prints the hashes of a step log or of a replay, as the STATEHASH trace
//! (the determinism matrix diffs them across targets).

use super::{AssetStore, Cheat, FrameInput, GameState, Out, hash, load_state, save_state};
use crate::fight::sim::Outcome;
use crate::frontend::options::Prefs;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

/// A step log file's first bytes (a historical format id; then postcard `StepLog`, this build only: its start state is a runtime snapshot).
pub const MAGIC: &[u8; 4] = b"BR2T";
/// Largest step log accepted (a replay's cap; a 10 000-step run takes well under 1 MB).
const MAX_BYTES: usize = super::replay::MAX_BYTES;

/// The static data a logged run reads.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum Data {
    /// `sim`: the direct fight's (`fight_data::load_match`)
    Fight { p1: String, p2: String, stage: String, costume: [u8; 2] },
    /// `fetest`: the front end's (`frontend::load`)
    Front,
}

/// What a logged run did to the state, in order.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum Op {
    /// `session::step` with this input and these cheats
    Step(FrameInput, Vec<Cheat>),
    /// `session::inputs` with these flags (fetest: a load or a fetch completing within the frame)
    Inputs(u8),
    /// these files (re)loaded into the store from the assets (one that fails to load left out)
    Fetch(Vec<String>),
    /// `AssetStore::prune`
    Prune,
    /// fetest's fight result: 2400 frames of play time, then `session::end_match`
    EndMatch(Outcome),
    /// fetest's team battle result (engine addition): `session::end_team`
    EndTeam(crate::fight::team::TeamOutcome),
    /// the card writes made after the step taken (`Card::take_dirty`)
    TakeDirty,
    /// the preferences from here on (`Request::Prefs`)
    Prefs(Prefs),
    /// sim's P2DMG: P2's damage set to this
    P2Dmg(i16),
    /// the state's hash here (a STATEHASH trace line)
    Hash,
}

/// A headless run as `run` does it again.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct StepLog {
    pub data: Data,
    /// the simulation version run (the fight data is built for it)
    pub sim_version: u32,
    /// the preferences at the start
    pub prefs: Prefs,
    /// the start state (`save_state`)
    pub start: Vec<u8>,
    pub ops: Vec<Op>,
}

impl StepLog {
    pub fn new(data: Data, prefs: Prefs, s: &GameState) -> Self {
        Self { data, sim_version: s.sim_version, prefs, start: save_state(s), ops: Vec::new() }
    }

    pub fn push(&mut self, op: Op) {
        self.ops.push(op);
    }

    pub fn encode(&self) -> Vec<u8> {
        let mut b = MAGIC.to_vec();
        b.extend(postcard::to_allocvec(self).expect("the log serialises"));
        b
    }

    /// Err for anything but a step log within `MAX_BYTES` (never a panic).
    pub fn decode(b: &[u8]) -> Result<Self, String> {
        if b.len() > MAX_BYTES {
            return Err(format!("step log of {} bytes", b.len()));
        }
        let body = b.strip_prefix(MAGIC.as_slice()).ok_or("not a step log")?;
        postcard::from_bytes(body).map_err(|e| format!("step log: {e}"))
    }
}

/// `b` starts as a step log.
pub fn is_step_log(b: &[u8]) -> bool {
    b.starts_with(MAGIC)
}

/// The log run again with `assets`' data; `each(frame, hash)` at every `Op::Hash`. Returns the number of hashes.
pub async fn run(log: &StepLog, assets: &crate::assets::Assets, mut each: impl FnMut(u32, u64)) -> Result<u32, String> {
    let mut store = AssetStore::default();
    match &log.data {
        Data::Fight { p1, p2, stage, costume } => {
            store.fight = Some(Arc::new(crate::fight::fight_data::load_match(assets, [p1, p2], *costume, stage, false, log.sim_version).await?))
        }
        Data::Front => store.front = Some(Arc::new(crate::frontend::load(assets).await?)),
    }
    let mut prefs = log.prefs;
    let mut s = load_state(&log.start, &store.assets(prefs))?;
    let mut n = 0;
    for op in &log.ops {
        let mut out = Out::default();
        match op {
            Op::Step(i, cheats) => super::step(&mut s, &store.assets(prefs), *i, cheats, &mut out),
            Op::Inputs(flags) => super::inputs(&mut s, &store.assets(prefs), *flags, &mut out),
            Op::Fetch(names) => {
                for name in names {
                    store.files.remove(name);
                    if let Ok(b) = assets.fetch(name).await {
                        store.files.insert(name.clone(), b.into());
                    }
                }
            }
            Op::Prune => store.prune(&s),
            Op::EndTeam(w) => {
                // (TEAM ARCADE A4) a battle not run: its play time as a match's
                if let Some(f) = &mut s.front
                    && f.arcade_team().is_some()
                {
                    f.add_play_time(2400);
                }
                super::end_team(&mut s, &store.assets(prefs), *w, [0; 4], &mut out)
            }
            Op::EndMatch(w) => {
                if let Some(f) = &mut s.front {
                    f.add_play_time(2400);
                }
                super::end_match(&mut s, &store.assets(prefs), *w, [0, 0], false, &mut out);
            }
            Op::TakeDirty => {
                if let Some(f) = &mut s.front {
                    f.save.memory_card.take_dirty();
                }
            }
            Op::Prefs(p) => prefs = *p,
            Op::P2Dmg(d) => {
                let g = &mut s.fight.as_mut().ok_or("P2DMG without a fight")?.m.fighters[1];
                (g.o_ac, g.o_ae, g.o_aa) = (*d, *d, *d);
            }
            Op::Hash => {
                each(s.frame, hash(&s));
                n += 1;
            }
        }
    }
    Ok(n)
}

/// The trace line of a hash (the STATEHASH trace's format).
pub fn line(frame: u32, h: u64) -> String {
    format!("{frame} {h:016x}")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A step log survives its encoding; other bytes are refused, never a panic.
    #[test]
    fn encodes() {
        let s = GameState::default();
        let mut l = StepLog::new(Data::Fight { p1: "YUG".into(), p2: "ALI".into(), stage: "ST0".into(), costume: [0, 1] }, Prefs::default(), &s);
        l.push(Op::Step(FrameInput { pad: [0x40, 0], forced: [0x40, 0], flags: FrameInput::FORCED[0], ..Default::default() }, vec![Cheat::Timer]));
        l.push(Op::Hash);
        l.push(Op::P2Dmg(199));
        let b = l.encode();
        assert!(is_step_log(&b));
        let d = StepLog::decode(&b).expect("decodes");
        assert_eq!((d.sim_version, d.start.len(), d.ops.len()), (s.sim_version, l.start.len(), 3));
        assert!(matches!(d.ops[0], Op::Step(i, ref c) if i.pad[0] == 0x40 && c == &[Cheat::Timer]));
        for n in [0, 4, 5, b.len() / 2, b.len() - 1] {
            assert!(StepLog::decode(&b[..n]).is_err(), "{n} bytes");
        }
        assert!(StepLog::decode(b"BR2R....").is_err());
    }
}
