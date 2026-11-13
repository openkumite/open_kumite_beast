//! The match rules the fight runs with: the CUSTOM settings block, the rules from the options, how a match ended.
//!
//! State: serialised: part of `GameState` (field changes are `STATE_VERSION` changes)

use serde::{Deserialize, Serialize};

/// The CUSTOM settings block (`cb = opt + 0x38`, used by the fight through M.C4 in CUSTOM games).
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct Custom {
    /// +0 GAME START: 0 VS HUMAN, 1 VS COMPUTER, 2 VS COMPUTER ALL MEMBERS, 3 PRACTICE
    pub game: u8,
    /// +1 MODEL TYPE: NORMAL / BIG HEAD / KIDS / BIG ARMS
    pub model: u8,
    /// +2 BLOOD EFFECT
    pub blood: u8,
    /// +3 CHARGE SPEED 0..8 ("x1" default)
    pub charge: u8,
    /// +4 RECOVER SPEED 0..8
    pub recover: u8,
    /// +8 flags: bit 6 walls hidden, 10 side step, 11 afterimage, 13 any cancel point
    pub flags: u32,
}

impl Custom {
    /// The "normal" block opt+0x2C used outside CUSTOM games: blood on, ×1, ×1, no flags.
    pub fn normal() -> Self {
        Self { game: 0, model: 0, blood: 1, charge: 1, recover: 1, flags: 0 }
    }
}

impl Default for Custom {
    fn default() -> Self {
        Self { game: 0, model: 0, blood: 1, charge: 1, recover: 1, flags: 0x8000_0000 }
    }
}

/// Match rules from the options.
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub struct Rules {
    /// rounds to win (MATCH POINT + 1)
    pub need: u8,
    /// round time in frames, −1 = infinite
    pub time_limit: i16,
    pub hp_pct: i32,
    pub cpu_level: u16,
    /// the settings block M.C4: the CUSTOM block in CUSTOM games, the normal block otherwise
    pub custom: Custom,
    /// CUSTOM GAME START = PRACTICE
    pub practice: bool,
    /// the attract demo (its own fight step): no round flow, no HUD
    pub demo: bool,
}

impl Rules {
    /// opt.8 ATTACK LEVEL 0..7 → HP %
    pub const HP_PCT: [i32; 8] = [100, 90, 80, 75, 70, 65, 60, 55];
    /// opt.9 TIME LIMIT 0..4 → frames
    pub const TIME: [i16; 5] = [0x4B0, 0x960, 0xE10, 0x1518, -1];
}

/// How a match ended, for the front end.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Outcome {
    Winner(usize),
    /// the human lost and pressed START on the continue screen
    Continue,
    /// the continue countdown ran out and the game-over screen ended
    GameOver,
    /// a second player joined (the challenger interrupt ran its 300 frames): side of the challenger
    Challenger(usize),
    /// pause → QUIT → YES
    Quit,
    /// engine addition (the team battle T7, simulation v25): DRAW GAME, nobody won (`fight::team::TeamOutcome::Draw`)
    Draw,
}
