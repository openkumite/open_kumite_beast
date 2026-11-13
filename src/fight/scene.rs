//! A read-only view of a fight's world for the presentation: the fighters, the camera, the stage, the effects.
//!
//! State: none of its own: borrows the serialised `Match`
//!
//! `present::fight_scene` draws a fight's world (stage, fighters, shadows, effects) through a `SceneRef` built by
//! `Match::scene`, not from `Match` itself, so that a world with more fighters (engine addition, the team battle)
//! can be drawn by the same code. The HUD, the status labels and the pause menu are the
//! 1v1 fight's own and still read `Fight`.

use crate::fight::br1::bridge::Br1Match;
use crate::fight::br1::object::Br1Fighter;
use crate::fight::br1::script::Request;
use crate::fight::camera::Camera;
use crate::fight::fighter::Fighter;
use crate::fight::fx::hit_effects::Fx;
use crate::fight::fx::native_tasks;
use crate::fight::fx::script_vm::Vm;
use crate::fight::rules::Custom;
use crate::fight::side::{PerSlot, Slot};
use crate::fight::stage::Stage;
use crate::fight::stage::anim::View;
use crate::fight::team::PlayerExt;

/// What a fight's world shows this frame.
#[derive(Clone, Copy)]
pub struct SceneRef<'a> {
    /// the fighters, by side (the indices of `Loaded::fighters`)
    pub fighters: &'a [Fighter],
    /// each fighter's opponent (a match: the other side; the team battle: its target): the throw clips it plays
    pub opp: &'a [usize],
    pub cam: &'a Camera,
    /// the stage (ring, walls, debris, the stage routine's counters)
    pub stage: &'a Stage,
    /// the stage darkening (`World::g4a58`: {delay, mode, level 0..255, step})
    pub darkening: [i16; 4],
    /// the fighters' palette fades, read through `fade` / `body` only (not visible outside `fight`: the World's records
    /// are by slot, the drawing goes by fighter)
    pub(in crate::fight) fades: Fades<'a>,
    pub fx: &'a Fx,
    pub vm: &'a Vm,
    pub fxn: &'a native_tasks::State,
    /// each fighter's back (ambient) colour this frame, None = the stage default
    pub back_colour: &'a [Option<[i32; 3]>],
    /// the settings block M.C4 (WALL DISPLAY OFF)
    pub custom: &'a Custom,
    /// the imported fighters' cores (engine addition, the import)
    pub br1: Option<Br1Scene<'a>>,
    pub frame: u64,
    /// the round (M.99) and the round flow's state (the imported props and chains restart with a round)
    pub round_index: i8,
    pub round_state: u8,
    /// M.9E this frame (`Match::m9e`)
    pub m9e: u8,
    /// what the stage routine shows (`Match::stage_view`)
    pub stage_view: View,
}

/// Where a world keeps its fighters' palette fade records (P.4).
#[derive(Clone, Copy)]
pub(in crate::fight) enum Fades<'a> {
    /// a match's: the World's, by player (`World::p4b18`)
    BySlot(&'a PerSlot<[i16; 4]>),
    /// the team battle's (T8): each fighter's own (`TeamFight::ext`), not the World slot it last stepped in
    ByFighter(&'a [PlayerExt]),
}

/// The imported fighters of a world (engine addition, the import; four sides in the team battle, T5): by side,
/// whether it is an imported fighter and its core (a main-game side's entry: its last imported view,
/// `bridge::shadow`), and the requests of the imported game's code for the drawing (`Br1Match::fx_log`).
#[derive(Clone, Copy)]
pub struct Br1Scene<'a> {
    pub side: &'a [bool],
    pub fs: &'a [Br1Fighter],
    pub fx_log: &'a [(u64, u8, Request)],
}

impl Br1Match {
    /// The match's imported fighters as a world shows them.
    pub fn view(&self) -> Br1Scene<'_> {
        Br1Scene { side: &self.side, fs: &self.fs, fx_log: &self.fx_log }
    }
}

impl Br1Scene<'_> {
    /// The fighter an imported stage watches (its Uriko's glow and counter): a match's P2 if P2 is the imported game's
    /// Uriko (F.03 8), else P1; the team battle's (T5) first such Uriko in index order, else fighter 0.
    pub fn watched(&self) -> usize {
        if self.side.len() == 2 {
            return (self.side[1] && self.fs[1].f.chr == 8) as usize;
        }
        self.uriko().unwrap_or(0)
    }

    /// The first imported Uriko (F.03 8) in index order.
    pub fn uriko(&self) -> Option<usize> {
        (0..self.side.len()).find(|&k| self.side[k] && self.fs[k].f.chr == 8)
    }
}

impl SceneRef<'_> {
    /// Fighter `i`'s palette fade record (P.4, `World::p4b18`): a match's by its player, the team battle's (T8) its
    /// own, as the team battle keeps it (`PlayerExt`) and not the World slot `i & 1` it last stepped in.
    pub fn fade(&self, i: usize) -> [i16; 4] {
        match self.fades {
            Fades::ByFighter(e) => e[i].p4b18,
            Fades::BySlot(p) => p[Slot::of_player(self.fighters[i].player)],
        }
    }

    /// Fighter `i`'s body as drawn: its fade level (P.4[2] of its own record, `fade`, subtracted from its palettes by
    /// the fade step) and whether the body is hidden (BKOUT: F6C 0x400 with fade mode P.4[1] 0, i.e. level 255 of op
    /// 0x2B; the shadow is still drawn).
    pub fn body(&self, i: usize) -> (u8, bool) {
        let fade = self.fade(i);
        let bkout = self.fighters[i].flags[crate::fight::fighter::F6C] & 0x400 != 0 && fade[1] == 0;
        (fade[2].clamp(0, 255) as u8, bkout)
    }

    /// Fighter `i`'s model transform in glTF space.
    pub fn transform(&self, i: usize) -> glam::Mat4 {
        crate::fight::duel::transform(&self.fighters[i])
    }
}
