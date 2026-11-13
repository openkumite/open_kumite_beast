//! The parts of a fight the effect code reads and writes, lent by the match that runs it.
//!
//! State: none of its own: borrows the serialised `Match` (or the team battle's world)
//!
//! `script_link`, `native_tasks` and `move_effects` run on an `FxHost`, not on `Match`, so that the team battle
//! (engine addition) runs the same effect code over its four fighters: the fighters by
//! index, each one's opponent (`opp`: the other one in a match, its target in the team battle), and the per-side
//! state of fighters 2 and 3 (`ext`), which `native_tasks::State` keeps only for two.

use crate::fight::camera::Camera;
use crate::fight::fighter::Fighter;
use crate::fight::fx::hit_effects::Fx;
use crate::fight::fx::native_tasks::{SideExt, State};
use crate::fight::fx::script_vm::Vm;
use crate::fight::side::Fid;
use crate::fight::stage::Stage;

/// The effect code's view of a fight (`Match::fx_host`).
pub struct FxHost<'a> {
    /// the fighters by index (`Loaded::fighters`)
    pub fighters: &'a mut [Fighter],
    /// each fighter's opponent (the opponent pointer of its block, the emitters aimed at it, the tongue's victim)
    pub opp: &'a [usize],
    pub vm: &'a mut Vm,
    pub fxn: &'a mut State,
    pub fx: &'a mut Fx,
    /// delayed EFT_89 tasks: side, bone, frames left
    pub fx_delayed: &'a mut Vec<(usize, u8, i16)>,
    /// the colour flash tasks by fighter
    pub flash_task: &'a mut [Option<(u8, u16, i16)>],
    /// the SFX queue
    pub sound_queue: &'a mut Vec<(i16, i16)>,
    pub stage: &'a Stage,
    pub cam: &'a Camera,
    /// costumes (F.4) by fighter
    pub costume: &'a [u8],
    /// the raw pads by fighter (the match block's M.6C / M.6E, a team battle's bank 1 after the block)
    pub pads: &'a [u16],
    pub custom: &'a crate::fight::rules::Custom,
    pub round_state: u8,
    /// the match's random seed (read only)
    pub rand: u32,
    /// fighters 2 and 3's per-side state (the team battle; None in a match)
    pub ext: Option<&'a mut SideExt>,
}

impl FxHost<'_> {
    /// Fighter colour flash `pattern` (from the flash pattern table) for `dur` frames; restarts a running one.
    pub fn colour_flash(&mut self, side: usize, dur: i16, pattern: u8) {
        if self.round_state >= 0xF {
            return;
        }
        self.flash_task[side] = Some((pattern, 0, dur));
    }

    /// Stun's aura script of fighter `i`.
    pub fn aura(&mut self, i: usize) -> &mut u32 {
        match (i, self.ext.as_deref_mut()) {
            (0 | 1, _) | (_, None) => &mut self.fxn.aura[Fid::of(i).slot()],
            (_, Some(e)) => &mut e.aura[Fid::of(i).slot()],
        }
    }

    /// Fighter `i`'s gauge of the previous frame (the pop-up edge).
    pub fn prev_gauge(&mut self, i: usize) -> &mut i16 {
        match (i, self.ext.as_deref_mut()) {
            (0 | 1, _) | (_, None) => &mut self.fxn.prev_gauge[Fid::of(i).slot()],
            (_, Some(e)) => &mut e.prev_gauge[Fid::of(i).slot()],
        }
    }

    /// Fighter `i`'s F6C as last written to the VM's match block (`script_link::write_back`).
    pub fn f6c(&mut self, i: usize) -> &mut u32 {
        match (i, self.ext.as_deref_mut()) {
            (0 | 1, _) | (_, None) => &mut self.fxn.f6c[Fid::of(i).slot()],
            (_, Some(e)) => &mut e.f6c[Fid::of(i).slot()],
        }
    }
}
