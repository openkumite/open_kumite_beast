//! The KO replay: snapshots of the fight simulation and the pad log they re-run with.
//!
//! State: serialised: part of `GameState` (field changes are `STATE_VERSION` changes)

use super::Match;
use crate::fight::duel;
use crate::fight::fight_data::Loaded;
use crate::fight::fighter::Fighter;
use crate::fight::move_decision::World;
use glam::Vec3;
use serde::{Deserialize, Serialize};

/// A replay snapshot (3 slots): everything the fight simulation needs to re-run.
#[derive(Clone, Serialize, Deserialize)]
struct Snapshot {
    frame: usize,
    fighters: [Fighter; 2],
    world: World,
    rand: u32,
    ai: [Option<crate::fight::ai::Ai>; 2],
    #[serde(with = "crate::base::serde_util::big_array")]
    ai_rnd: [u8; 256],
    fx: crate::fight::fx::hit_effects::Fx,
    cam: crate::fight::camera::Camera,
    right: Vec3,
    /// the imported fighters' state (engine addition, the import S7)
    #[serde(default)]
    br1: Option<Box<crate::fight::br1::bridge::Br1Match>>,
}

/// KO replay recorder (match +0xCC): mode 1 records (pads + snapshots), 2 logs the pads only (from the KO on),
/// 3 playing back, 4 played to the end.
#[derive(Clone, Default, Serialize, Deserialize)]
pub struct Replay {
    pub mode: u8,
    slot: usize,
    cool: i32,
    snaps: [Option<Box<Snapshot>>; 3],
    /// the special KO: the KO moment kept over its replays, and the snapshot they play from
    end_snap: Option<Box<Snapshot>>,
    special_idx: Option<usize>,
    frame: usize,
    end: usize,
    log: Vec<[u16; 2]>,
}

impl Replay {
    /// A decoded state's pad log is the 600 frames it is indexed by, the slots are of the 3 snapshots, and the
    /// snapshots' fighters, AI blocks, imported parts and effect lists are sound.
    pub(super) fn check(&self, data: &Loaded) -> Result<(), String> {
        let snaps = self.snaps.iter().chain(std::iter::once(&self.end_snap)).flatten();
        if self.log.len() != 600 || self.slot >= 3 || self.special_idx.is_some_and(|i| i >= 3) {
            return Err("KO replay".into());
        }
        for s in snaps {
            Match::check_fighters(&s.fighters, &s.ai, data).map_err(|e| format!("KO replay: {e}"))?;
            Match::br1_check(&s.br1, data).map_err(|e| format!("KO replay: {e}"))?;
            duel::setup::check_world(&s.world, s.br1.is_some(), data).map_err(|e| format!("KO replay: {e}"))?;
            s.fx.check()?;
        }
        Ok(())
    }

    /// After deserialising: the snapshots' static ring tables (`hit_effects::Fx::rebind`).
    pub(super) fn rebind(&mut self, rings: &std::sync::Arc<[u8]>) {
        for s in self.snaps.iter_mut().chain(std::iter::once(&mut self.end_snap)).flatten() {
            s.fx.rebind(rings);
        }
    }
}

impl Match {
    fn snapshot(&self) -> Snapshot {
        Snapshot {
            frame: self.replay.frame,
            fighters: self.fighters.clone(),
            world: self.world.clone(),
            rand: self.rand,
            ai: self.ai.clone(),
            ai_rnd: self.ai_rnd,
            fx: self.fx.clone(),
            cam: self.cam.clone(),
            right: self.right,
            br1: self.br1.clone(),
        }
    }

    /// The KO replay recorder before the fighters move (and its pad hook): start and stop recording,
    /// the special KO's replay commands, the rewind of a normal KO replay, the snapshots and the pad playback.
    pub(super) fn replay_step(&mut self, data: &Loaded) {
        let rs = self.round.state;
        if rs == 3 && self.round.cnt == 1 && self.replay.mode != 3 {
            self.replay.mode = 1;
            self.replay.cool = 0;
            self.replay.snaps = Default::default();
        }
        if matches!(rs, 5..=8) && self.replay.mode == 1 {
            // at the KO: no more snapshots, but the pad hook keeps logging (cc.4), so a playback runs
            // through the KO pose up to the frame it starts
            self.replay.mode = 2;
        }
        // the special KO's replays; the camera is not part of the
        // game's snapshot, so it stays as the round set it
        if let Some(cmd) = self.round.replay_cmd.take() {
            use crate::fight::round::ReplayCmd;
            let cam = self.cam.clone();
            let load = |m: &mut Self, s: &Snapshot| {
                m.fighters = s.fighters.clone();
                m.world = s.world.clone();
                m.rand = s.rand;
                m.ai = s.ai.clone();
                m.ai_rnd = s.ai_rnd;
                m.fx = s.fx.clone();
                m.right = s.right;
                m.br1 = s.br1.clone();
            };
            match cmd {
                ReplayCmd::Start | ReplayCmd::Restart => {
                    if cmd == ReplayCmd::Start {
                        self.replay.end = self.replay.frame;
                        self.replay.end_snap = Some(Box::new(self.snapshot()));
                        // the middle snapshot (n = 1), else the nearest older / newer one
                        let r = &self.replay;
                        self.replay.special_idx = [(r.slot + 1) % 3, r.slot, (r.slot + 2) % 3].into_iter().find(|&i| r.snaps[i].is_some());
                    }
                    match self.replay.special_idx.and_then(|i| self.replay.snaps[i].clone()) {
                        Some(s) => {
                            load(self, &s);
                            self.replay.frame = s.frame;
                            self.replay.mode = 3;
                            crate::fight::fx::native_tasks::reset(&mut self.fx_host(), data); // the effect reset (1)
                            self.world.stage.debris.clear();
                        }
                        None => {
                            self.replay.mode = 4;
                            self.round.replay_done = true;
                        }
                    }
                }
                ReplayCmd::Restore => {
                    if let Some(s) = self.replay.end_snap.take() {
                        load(self, &s);
                        self.replay.frame = s.frame;
                    }
                    self.replay.mode = 2;
                }
            }
            self.cam = cam;
        }
        if matches!(rs, 9 | 10) && self.replay.mode == 2 {
            // back to the snapshot two slots before the newest, re-simulate with the logged pads
            let r = &mut self.replay;
            // newest = slot − 1, two back = slot (the oldest); empty slots are skipped
            let idx = (0..3).map(|k| (r.slot + k) % 3).find(|&i| r.snaps[i].is_some());
            if let Some(snap) = idx.and_then(|i| r.snaps[i].clone()) {
                let s = *snap;
                r.end = r.frame;
                r.frame = s.frame;
                r.mode = 3;
                self.fighters = s.fighters;
                self.world = s.world;
                self.rand = s.rand;
                self.ai = s.ai;
                self.ai_rnd = s.ai_rnd;
                self.fx = s.fx;
                self.cam = s.cam;
                self.right = s.right;
                self.br1 = s.br1;
            } else {
                self.replay.mode = 4;
            }
        }
        if self.replay.mode == 1 {
            if self.replay.cool == 0 {
                let s = self.snapshot();
                let slot = self.replay.slot;
                self.replay.snaps[slot] = Some(Box::new(s));
                self.replay.slot = (slot + 1) % 3;
                self.replay.cool = 20;
            } else {
                self.replay.cool -= 1;
            }
        }
        let playing = self.replay.mode == 3;
        for i in 0..2 {
            self.fighters[i].pad_override = if playing { Some(self.replay.log[self.replay.frame % 600][i]) } else { None };
        }
    }

    /// Replay bookkeeping after the round flow: log the processed pads, advance, detect the end of a playback.
    pub(super) fn replay_log_step(&mut self) {
        if matches!(self.replay.mode, 1 | 2) {
            let k = self.replay.frame % 600;
            self.replay.log[k] = [self.fighters[0].pad_record, self.fighters[1].pad_record];
        }
        if matches!(self.replay.mode, 1..=3) {
            self.replay.frame += 1;
        }
        if self.replay.mode == 3 && self.replay.frame >= self.replay.end {
            self.replay.mode = 4;
            self.round.replay_done = true;
        }
        if self.replay.mode == 4 && !matches!(self.round.state, 9 | 10) && self.round.special == 0 {
            self.replay.mode = 0;
        }
    }
}

impl Replay {
    /// An idle recorder with its 600-frame pad log.
    pub(super) fn new() -> Self {
        Replay { log: vec![[0; 2]; 600], ..Default::default() }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The KO replay's snapshots' worlds are checked as the live world (`duel::setup::check_world`): imported stage
    /// sounds in a snapshot of a match whose imported fighters' queue takes them are refused; without one (two
    /// main-game fighters, simulation v18) up to 32 non-negative ids pass (data-gated).
    #[test]
    fn snapshot_stage_sounds_checked() {
        for (p1, p2, ok) in [("MIT", "FOX", 0), ("ALI", "YUG", 32)] {
            let Some(d) = super::super::br1_tests::data_on(p1, p2, "ZIN00", crate::fight::version::v::BR1_DEBRIS) else {
                eprintln!("skipped: no okb_export / okb_export/br1");
                return;
            };
            let mut m = Match::new(&d);
            m.replay.snaps[0] = Some(Box::new(m.snapshot()));
            assert!(m.replay.check(&d).is_ok(), "{p1} {p2}: a clean snapshot");
            fn snap(m: &mut Match) -> &mut Vec<i16> {
                &mut m.replay.snaps[0].as_mut().expect("the snapshot").world.stage.br1.sounds
            }
            snap(&mut m).extend(std::iter::repeat_n(0x81, ok));
            assert!(m.replay.check(&d).is_ok(), "{p1} {p2}: {ok} sounds");
            snap(&mut m).push(0x81);
            assert!(m.replay.check(&d).is_err_and(|e| e.contains("stage sounds")), "{p1} {p2}: {} sounds", ok + 1);
            snap(&mut m).clear();
            snap(&mut m).push(-1);
            assert!(m.replay.check(&d).is_err(), "{p1} {p2}: a negative id");
        }
    }
}
