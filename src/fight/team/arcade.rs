//! TEAM ARCADE's rules of a battle (engine addition): the CPU level of each fighter and the continue countdown.
//!
//! Engine addition (TEAM ARCADE A3); the countdown is the match's (`round::countdown`), the continue and GAME OVER
//!   states are modelled on the match's 0xF / 0x10
//! State: serialised: part of `TeamFight` (`TeamFight::arcade`; field changes are `STATE_VERSION` changes)
//!
//! A TEAM ARCADE battle (simulation v26, `version::v::TEAM_ARCADE`) is a 2 VS 2 `TeamFight` with `arcade` set by the
//! session from the front end's run (`session::step` `start_team`): the AI blocks take each fighter's level from
//! `levels` instead of `TeamFight::cpu_level`, and a battle the humans' team did not win (a DRAW GAME too) goes from
//! the end of `TeamRound`'s state 0xD to the CONTINUE countdown (0xF) and from there to GAME OVER (0x10). Every other
//! team battle has `arcade` None and runs as before.

use super::N;
use serde::{Deserialize, Serialize};

/// A TEAM ARCADE battle's rules (`TeamFight::arcade`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArcadeRules {
    /// the humans' team: 0 P1 + P3, 1 P2 + P4 (its two slots read the countdown's presses; YOU WIN / YOU LOSE)
    pub human_team: u8,
    /// each fighter's CPU level (the AI blocks', `Ai::new_at` / the imported game's `init`): the enemies' from the rank
    /// table's column of the stage, a CPU partner's the boss column's (`Front::team_arcade_levels`)
    pub levels: [u16; N],
    /// the continue countdown's frames 0..999 (the match's +0xB2, `round::countdown`)
    pub b2: u16,
    /// the GAME OVER screen's frames (the match's +0x8E: 900, or 0x1734 once skipped)
    pub go_cnt: i32,
}

impl ArcadeRules {
    pub fn new(human_team: usize, levels: [u16; N]) -> Self {
        ArcadeRules { human_team: human_team.min(1) as u8, levels, b2: 0, go_cnt: 0 }
    }

    /// The match's winner team (None: a DRAW GAME) is not the humans': a loss (a DRAW GAME counts as one; every loss
    /// goes to the CONTINUE countdown, continues are unlimited).
    pub fn lost(&self, winner: Option<usize>) -> bool {
        winner != Some(self.human_team as usize)
    }

    /// A decoded value is one the battle can hold: a team of 2 VS 2, CPU levels in the rank table's range (`* 16`, at
    /// most 0xFF with L1+L2+R1+R2), the counters in their ranges (`round::countdown` stops at 999; GAME OVER resets it
    /// at its end).
    pub fn check(&self) -> Result<(), String> {
        let ok = self.human_team < 2 && self.levels.iter().all(|&l| l <= 0xFF) && self.b2 <= 999 && (0..=0x1734).contains(&self.go_cnt);
        if ok { Ok(()) } else { Err("arcade rules".into()) }
    }
}
