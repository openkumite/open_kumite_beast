//! Each fighter's target in the team battle: the opponent its per-fighter steps see (engine addition).
//!
//! State: `Locks` (`TeamFight::locked`, serialised); updates `TeamFight::target` / `locked`
//!
//! At the start of every frame, before the input: a fighter in a throw keeps its partner (thrower F58 0x40000000,
//! victim F58 0x100000, or the grab requests F6C 0xC00000 of this frame's `throw::resolve_pair`), so that the
//! throw's ops and the victim's ACT see it; from simulation v25 (T7) a thrown fighter also keeps its thrower after the
//! release until it lies, gets up or stands (`thrown`: its thrown move still reads its opponent); a target chosen
//! with L2 stays until it is out; otherwise the nearest enemy still in the round (squared x / z distance, ties to the
//! lower index), the current one kept unless another is nearer by more than `HYSTERESIS`. A new L2 press moves to the
//! next enemy in index order and locks it.

use super::N;
use crate::fight::br1::bridge::THROW_FRAMES;
use crate::fight::fighter::{F6C, F58, Fighter};
use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// The targets' locks (`TeamFight::locked`): a target chosen with L2, kept until that fighter is KO'd (or L2 is
/// pressed again); (T7, simulation v25) each thrown fighter's thrower and the frames since the release (`thrown`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Locks {
    pub l2: [bool; N],
    pub thrown_by: [Option<(u8, u16)>; N],
}

/// `Locks` by fighter as serialised: `Free` / `L2` encode as the `false` / `true` of the v22..v24 `[bool; N]`, so a
/// battle without a thrown fighter's lock keeps its bytes (the team fixtures' traces hash them).
#[derive(Serialize, Deserialize)]
enum Cell {
    Free,
    L2,
    Thrown(u8, u16),
    L2Thrown(u8, u16),
}

impl Serialize for Locks {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        let cells: [Cell; N] = std::array::from_fn(|k| match (self.l2[k], self.thrown_by[k]) {
            (false, None) => Cell::Free,
            (true, None) => Cell::L2,
            (false, Some((a, n))) => Cell::Thrown(a, n),
            (true, Some((a, n))) => Cell::L2Thrown(a, n),
        });
        cells.serialize(s)
    }
}

impl<'de> Deserialize<'de> for Locks {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let mut l = Locks::default();
        for (k, c) in <[Cell; N]>::deserialize(d)?.into_iter().enumerate() {
            (l.l2[k], l.thrown_by[k]) = match c {
                Cell::Free => (false, None),
                Cell::L2 => (true, None),
                Cell::Thrown(a, n) if (a as usize) < N && n < THROW_FRAMES => (false, Some((a, n))),
                Cell::L2Thrown(a, n) if (a as usize) < N && n < THROW_FRAMES => (true, Some((a, n))),
                _ => return Err(serde::de::Error::custom("a thrower or its frame count out of range")),
            };
        }
        Ok(l)
    }
}

/// The current target is kept unless another enemy is nearer by more than this (game units).
pub const HYSTERESIS: i64 = 256;

/// Fighter `f` is in a throw (as the thrower or the victim, or grabbed this frame).
pub fn in_throw(f: &Fighter) -> bool {
    f.flags[F58] & 0x4010_0000 != 0 || f.flags[F6C] & 0xC0_0000 != 0
}

/// The x / z distance between two fighters' roots.
fn dist(a: &Fighter, b: &Fighter) -> i64 {
    let (dx, dz) = ((a.m_t[0] - b.m_t[0]) as i64, (a.m_t[2] - b.m_t[2]) as i64);
    (dx * dx + dz * dz).isqrt()
}

/// The enemies of fighter `k` still in the round (`team` by fighter, `out` the fighters out of it), in index order.
fn enemies(k: usize, team: [usize; N], out: [bool; N]) -> impl Iterator<Item = usize> {
    (0..N).filter(move |&j| j != k && team[j] != team[k] && !out[j])
}

/// The nearest enemy of fighter `k` (ties to the lower index), the current target `cur` kept unless another is nearer
/// by more than `HYSTERESIS`; None when no enemy is left.
pub fn nearest(fs: &[Fighter; N], k: usize, cur: Option<usize>, team: &[usize; N], out: &[bool; N]) -> Option<usize> {
    let mut best: Option<(i64, usize)> = None;
    for j in enemies(k, *team, *out) {
        let d = dist(&fs[k], &fs[j]);
        if best.is_none_or(|(bd, _)| d < bd) {
            best = Some((d, j));
        }
    }
    let (bd, b) = best?;
    match cur.filter(|&c| c != k && team[c] != team[k] && !out[c]) {
        Some(c) if dist(&fs[k], &fs[c]) <= bd + HYSTERESIS => Some(c),
        _ => Some(b),
    }
}

/// (T7, simulation v25) One frame of the thrown fighters' throwers (`Locks::thrown_by`), CPU or human alike:
/// fighter k grabbed (F58 0x100000) by its target t holding it (F58 0x40000000, t's target k) gets `(t, 0)`; after the
/// release that stays, counting the frames, until k is `done` (lying, getting up or standing, `bridge::victim_done`),
/// t is out, or for `THROW_FRAMES`. Returns `update`'s `keep`: a thrown fighter's target while it is its thrower, and
/// a thrower and its victim holding each other (each the other's target) while the victim is not done.
pub fn thrown(fs: &[Fighter; N], target: &[usize; N], thrown_by: &mut [Option<(u8, u16)>; N], done: [bool; N], out: &[bool; N]) -> [bool; N] {
    let holds = |f: &Fighter| f.flags[F58] & 0x4000_0000 != 0;
    for k in 0..N {
        let t = target[k];
        let grabbed = fs[k].flags[F58] & 0x10_0000 != 0 && t != k && target[t] == k && holds(&fs[t]);
        thrown_by[k] = match thrown_by[k] {
            _ if grabbed => Some((t as u8, 0)),
            Some((a, n)) if !done[k] && !out[a as usize] && n + 1 < THROW_FRAMES => Some((a, n + 1)),
            _ => None,
        };
    }
    std::array::from_fn(|k| {
        let t = target[k];
        let victim = if holds(&fs[t]) { k } else { t };
        let held = t != k && target[t] == k && (holds(&fs[k]) || holds(&fs[t])) && !done[victim];
        held || thrown_by[k].is_some_and(|(a, _)| a as usize == t)
    })
}

/// One frame of the targets (`l2`: fighter k pressed L2 this frame, a human's new press), fighter k's target kept this
/// frame where `keep[k]` while that target is still in the round (T7, simulation v25: `thrown`; before, all false).
pub fn update(fs: &[Fighter; N], target: &mut [usize; N], locked: &mut [bool; N], team: &[usize; N], out: &[bool; N], l2: [bool; N], keep: [bool; N]) {
    for k in 0..N {
        let t = target[k];
        if (in_throw(&fs[k]) && t != k && in_throw(&fs[t])) || (keep[k] && !out[t]) {
            continue;
        }
        if out[t] || team[t] == team[k] {
            locked[k] = false;
        }
        if l2[k] {
            // the next enemy after the current one in index order, locked
            let next = (1..N).map(|d| (t + d) % N).find(|&j| enemies(k, *team, *out).any(|e| e == j));
            if let Some(j) = next {
                target[k] = j;
                locked[k] = true;
            }
            continue;
        }
        if locked[k] {
            continue;
        }
        if let Some(j) = nearest(fs, k, Some(t), team, out) {
            target[k] = j;
        }
    }
}
