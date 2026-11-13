//! Checkpoints: the replay's own `sync_hash` (the same in every later build) and the full state hash.
//!
//! Engine addition.
//! State: the replay file's format (`sync_hash`'s values are part of `FORMAT` 1)

use crate::base::hash::{FNV_BASIS, fnv1a};
use crate::session::{GameState, hash};

/// The replay's own checkpoint hash: FNV-1a 64 of values every later build computes the same way whatever its state
/// layout (part of `FORMAT` 1). The session's frame, the match asked for (sides, costumes, stage, CPU sides), whether
/// it waits loaded, the music track; the front end's `Front::sync_key` (frame, random state, ladder fight, play
/// time, screen, cursors, mode); the fight's frame, random state, round state / index / lost rounds / timer, and per
/// fighter the position, damage, max HP, beast gauge, form and move; in a session with a team battle (engine addition,
/// simulation v22) its spec and the battle's values as a match's; with a TEAM ARCADE run (simulation v26) its ladder.
pub fn sync_hash(s: &GameState) -> u64 {
    let mut v = Vec::with_capacity(160);
    v.extend(s.frame.to_le_bytes());
    match &s.spec {
        Some(m) => v.extend([1, m.chr[0], m.chr[1], m.costume[0], m.costume[1], m.stage, m.cpu[0] as u8, m.cpu[1] as u8]),
        None => v.push(0),
    }
    v.push(s.pending as u8);
    match &s.music {
        Some(m) => v.extend([1, m.track, m.looped as u8]),
        None => v.push(0),
    }
    match &s.front {
        Some(f) => {
            v.push(1);
            for w in f.sync_key() {
                v.extend(w.to_le_bytes());
            }
        }
        None => v.push(0),
    }
    match &s.fight {
        Some(g) => {
            let (m, r) = (&g.m, &g.m.round);
            v.push(1);
            v.extend(m.frame.to_le_bytes());
            v.extend(m.rand.to_le_bytes());
            v.extend([r.state, r.index as u8, r.lost[0], r.lost[1]]);
            v.extend(r.timer.to_le_bytes());
            for f in &m.fighters {
                for p in f.pos {
                    v.extend(p.to_le_bytes());
                }
                v.extend(f.o_ac.to_le_bytes());
                v.extend(f.o_b0.to_le_bytes());
                v.extend(f.gauge.to_le_bytes());
                v.push(f.form);
                v.extend(f.move_cur.to_le_bytes());
            }
        }
        None => v.push(0),
    }
    // the team battle (engine addition, simulation v22, replay FORMAT 2): only when there is one, so the values of every
    // other session stay as they were; its spec, frame, random state, round and per fighter as a match's
    if let Some(t) = &s.team_spec {
        v.push(2);
        v.extend([t.mode as u8, t.stage]);
        v.extend(t.chr);
        v.extend(t.costume);
        v.extend(t.cpu.map(|c| c as u8));
    }
    // TEAM ARCADE's ladder (engine addition, simulation v26): only while a run is on (`Front::end_team_arcade` clears it)
    if let Some(l) = s.front.as_ref().and_then(|f| f.team_ladder_sync()) {
        v.push(4);
        v.extend(l);
    }
    if let Some(t) = &s.team {
        v.push(3);
        v.extend(t.frame.to_le_bytes());
        v.extend(t.rand.to_le_bytes());
        v.extend([t.round.state, t.round.index as u8]);
        v.extend(t.round.wins);
        v.extend(t.round.timer.to_le_bytes());
        for f in &t.fighters {
            for p in f.pos {
                v.extend(p.to_le_bytes());
            }
            v.extend(f.o_ac.to_le_bytes());
            v.extend(f.gauge.to_le_bytes());
            v.push(f.form);
            v.extend(f.move_cur.to_le_bytes());
        }
    }
    fnv1a(FNV_BASIS, &v)
}

pub(super) fn checkpoint(s: &GameState) -> (u32, u64, u64) {
    (s.frame, hash(s), sync_hash(s))
}
