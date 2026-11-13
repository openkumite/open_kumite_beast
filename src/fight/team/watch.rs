//! A check of the team battle's state after each frame (engine addition, T7): no fighter hanging in the air, none
//! through a wall.
//!
//! Engine addition: a debug and regression check, `sim TEAMCHECK=1`, `fight::team::tests`
//! State: none (`Watch` is the checker's own, not part of the battle)
//!
//! While the round is fought (state 4), for each fighter still in it: its root (`m_t`) above the floor by more than
//! `AIR_Y` for more than `AIR_FRAMES` frames in a row outside a throw (as thrower or victim, either engine's) is a
//! fighter stuck in the air; on a ring whose four sides are walls, a body sphere's centre (the walls test every one but
//! the first, `physics::ring_walls`) outside the ring's box or past an extra stage's chamfer (`Extra::chamfers`, walls
//! from simulation v25 `version::v::CHAMFERS` only) is a fighter through a wall by more than `WALL_SLACK`: the walls keep each centre
//! its radius inside.

use super::{N, TeamFight};
use crate::fight::fight_data::Loaded;
use crate::fight::fighter::F58;

/// How far above the floor a root counts as airborne.
pub const AIR_Y: i32 = 64;

/// How many frames in a row in the air (outside a throw) make a fighter stuck: longer than any jump, launch or juggle.
pub const AIR_FRAMES: u32 = 240;

/// How far past a wall a body sphere's centre may reach for a frame: the walls leave
/// small overshoots of a long limb or a fast knock-back (up to 111 seen in the T7 matrix), not bodies inside the stone.
pub const WALL_SLACK: i32 = 0x80;

/// The checker's counters: each fighter's frames in the air so far.
#[derive(Clone, Debug, Default)]
pub struct Watch {
    air: [u32; N],
    reported: [bool; N],
}

impl Watch {
    /// The findings of the frame `t` (on `data`) has just run, one line each (a stuck fighter once per stay in the air).
    pub fn step(&mut self, t: &TeamFight, data: &Loaded) -> Vec<String> {
        let mut out = Vec::new();
        let st = &t.world.stage;
        let walled = (0..4).all(|k| st.walls.present(k)) && st.ring.kind == 0;
        // the chamfers are walls from simulation v25 (`frame.rs`, T7) only: a v22..v24 battle may stand in them
        let chamfers: Vec<_> = match crate::fight::stage::extra::by_stage(st.id) {
            Some(e) if data.gates().chamfers => e.chamfers(data.gates()).collect(),
            _ => Vec::new(),
        };
        let (hx, hz) = (st.ring.hx as i32 + WALL_SLACK, st.ring.hz as i32 + WALL_SLACK);
        // (n has length √2: the slack along it is 181 / 128 of a side's)
        let slack = (WALL_SLACK * 181) >> 7;
        for k in 0..N {
            let f = &t.fighters[k];
            let thrown = f.flags[F58] & 0x4010_0000 != 0 || t.br1.as_ref().is_some_and(|b| b.throws.involves(k));
            if t.round.state != 4 || t.round.out[k] || thrown || f.m_t[1] - f.o_94 as i32 >= -AIR_Y {
                (self.air[k], self.reported[k]) = (0, false);
            } else {
                self.air[k] += 1;
                if self.air[k] > AIR_FRAMES && !self.reported[k] {
                    self.reported[k] = true;
                    out.push(format!("frame {}: P{} in the air for {} frames (root y {}, move {:#x})", t.frame, k + 1, self.air[k], f.m_t[1], f.move_cur));
                }
            }
            if t.round.state != 4 || t.round.out[k] || !walled {
                continue;
            }
            let through = t
                .bodies(data, k)
                .iter()
                .skip(1)
                .filter_map(|s| f.parts.get(s.node as usize))
                .find(|p| p[0].abs() > hx || p[2].abs() > hz || chamfers.iter().any(|c| c.n[0] * p[0] + c.n[1] * p[2] > c.c + slack));
            if let Some(p) = through {
                out.push(format!(
                    "frame {}: P{} through a wall at ({}, {}) (root ({}, {}), move {:#x})",
                    t.frame,
                    k + 1,
                    p[0],
                    p[2],
                    f.m_t[0],
                    f.m_t[2],
                    f.move_cur
                ));
            }
        }
        out
    }
}
