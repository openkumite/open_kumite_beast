//! The match globals the decision reads and writes (`World`), the HUD flash, the beast revert.
//!
//! State: serialised: part of `GameState` (field changes are `STATE_VERSION` changes)
//!
//! The match globals the decision reads and writes (`World`), the HUD flash request and the beast revert.

use crate::fight::fighter::*;
use crate::fight::input;
use crate::fight::side::{PerSlot, Slot};
use serde::{Deserialize, Serialize};

/// Globals of the match (the match block in the scratchpad and fixed RAM) that the decision reads or writes.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct World {
    /// Address of the match object passed as the second argument (the scratchpad base, `script_link::M_BASE`).
    /// One branch ANDs F58 with it; kept as the game's constant.
    pub match_addr: u32,
    /// byte +4 of *
    pub c0_b4: u8,
    /// word +8 of *: bit 10 enables the L2/R2 taunts, bit 13 …
    pub c4_w8: u32,
    /// stage ring, outer area (by pointer) and the wall block
    pub stage: crate::fight::stage::Stage,
    /// per-player halfwords of the fade records, 0xBC bytes a player (+0, +2, +6)
    pub p4b18: PerSlot<[i16; 4]>,
    /// The stage darkening {delay, mode, level 0..255, step} (op 0x2B sub 0)
    pub g4a58: [i16; 4],
    /// training display flag (bit 0) and its per-player flash bytes
    pub hud_5378: u8,
    pub hud_flash: PerSlot<[u8; 4]>,
    /// sound requests of the decision, the sound queue's (a0, a1): the match takes them after it from simulation
    /// version 4 on (before, they piled up unplayed)
    pub sounds: Vec<(u16, i32)>,
    /// effect requests of the decision (move-script effect op bytes), taken by the match after it (version 4 on);
    /// empty between frames
    #[serde(skip)]
    pub effects: Vec<[u8; 6]>,
}

impl World {
    pub fn new() -> Self {
        Self { match_addr: crate::fight::fx::script_link::M_BASE, ..Default::default() }
    }
}

pub(super) fn hud_flash(w: &mut World, p: Slot, a: u8, b: u8) {
    if w.hud_5378 & 1 != 0 {
        w.hud_flash[p] = [a, b, a, 1];
    }
}

/// Drop back to human form (gauge empty / hit while transforming).
pub fn revert_human(f: &mut Fighter, w: &mut World) {
    w.sounds.push((0x400, 7)); // the sound request (0x400, 7, 0)
    f.gauge = 0;
    f.o_d8 = 0;
    f.form = 0;
    f.flags[F6C] &= 0xFFFF_7FFF;
    f.o_ae = f.o_ac;
    // the model switches to the human form; the burst is spawned
    w.effects.push([0x17, 0x0F, 0, 0, 0, 0]); // = op17 sub F
    input::reset_chain(f);
    input::reset_generic(f);
    input::reset_attack(f);
}
