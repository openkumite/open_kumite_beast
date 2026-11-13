//! Match logic without rendering: two fighters, input, move decision, move scripts.
//!
//! State: serialised: `Match` is part of `GameState` (field changes are `STATE_VERSION` changes)

use crate::assets::roster::CHARACTERS;
use crate::fight::duel;
use crate::fight::fight_data::Loaded;
use crate::fight::fighter::Fighter;
use crate::fight::move_decision::World;
use glam::Vec3;
use serde::{Deserialize, Serialize};

pub(crate) mod br1;
#[cfg(test)]
mod br1_tests;
pub(crate) mod frame;
mod ko_replay;
mod setup;

pub use crate::fight::rules::{Outcome, Rules};
pub use ko_replay::Replay;

#[derive(Clone, Serialize, Deserialize)]
pub struct Match {
    pub fighters: [Fighter; 2],
    pub world: World,
    /// Raw pad words (PS1 layout) for both players, set by the app from keyboard / gamepad.
    pub pads: [u16; 2],
    pub buttons: [[u16; 8]; 2],
    /// Screen-right direction in glTF space (unit vector P1 → P2 when the fight starts); sets the input swap.
    pub right: Vec3,
    pub rand: u32,
    pub frame: u64,
    /// Log of hits (for the window title / sim output; debug, not state).
    #[serde(skip)]
    pub events: Vec<String>,
    pub round: crate::fight::round::Round,
    /// Players driven by the CPU routine.
    pub cpu: [bool; 2],
    /// CPU level (state +0xA0; 0 easiest .. 0xFF).
    pub cpu_level: u16,
    pub ai: [Option<crate::fight::ai::Ai>; 2],
    /// Shared AI random table (in RAM), reshuffled every round.
    #[serde(with = "crate::base::serde_util::big_array")]
    pub ai_rnd: [u8; 256],
    /// SFX queue (32 entries of id, delay) and the sounds started this frame (≤ 1)
    pub sound_queue: Vec<(i16, i16)>,
    pub sounds_started: Vec<i16>,
    /// effect tasks (hit sparks, flash)
    pub fx: crate::fight::fx::hit_effects::Fx,
    /// the effect bytecode VM (scheduler list 4)
    pub vm: crate::fight::fx::script_vm::Vm,
    /// the fight camera
    pub cam: crate::fight::camera::Camera,
    pub replay: Replay,
    /// false: a finished match is not restarted; `result` holds the winner for the front end
    pub auto_rematch: bool,
    pub result: Option<Outcome>,
    /// last frame's raw pads (new presses for the round flow)
    prev_pads: [u16; 2],
    /// both ports' pads unmasked (the app zeroes `pads` of CPU sides): WATCH's camera buttons and pause
    pub watch_pads: [u16; 2],
    prev_watch: [u16; 2],
    /// M.8E: the attract demo's camera timer (a new pick every 240 fight frames)
    pub demo_cam: i16,
    /// max HP percentage (opt.8 ATTACK LEVEL → f.DE)
    pub hp_pct: i32,
    /// costumes (F.4)
    pub costume: [u8; 2],
    /// the settings block M.C4 (CHARGE / RECOVER SPEED, flags: walls hidden, side step, any cancel …)
    pub custom: crate::fight::rules::Custom,
    /// delayed EFT_89 tasks: side, bone, frames left
    pub fx_delayed: Vec<(usize, u8, i16)>,
    /// native effect tasks and the fight-effect state
    pub fxn: crate::fight::fx::native_tasks::State,
    /// debug: (fighter, move) requested at the next move decision
    pub force_move: Option<(usize, i16)>,
    /// pause → CONTROLLER CONFIG.: the app opens the button-mapping screen
    pub config_request: bool,
    /// frames of round state 4 (fighting) in this match: the arcade ranking's clear time
    pub play_frames: u32,
    /// PRACTICE (M.9C == 5, M.9B == 4): no rounds, no KO, SELECT resets
    pub practice: bool,
    /// the challenger interrupt: (side, frame)
    pub challenger: Option<(usize, i32)>,
    /// the pause menu (m.82 while it is open or closing)
    pub pause: crate::fight::pause::Pause,
    /// M.D4: the practice CPU behaviour (COM. ACTION and the guard options)
    pub practice_d4: u8,
    /// VIBRATION option on
    pub vibration: bool,
    /// motors per port: [large, small] as (frames left, value)
    pub vib: [[(u8, u8); 2]; 2],
    /// fighter colour flash: (pattern, step, frames left) per side
    pub flash_task: [Option<(u8, u16, i16)>; 2],
    /// the back (ambient) colour of each fighter this frame (4096 = 1.0), None = the stage default
    pub back_colour: [Option<[i32; 3]>; 2],
    /// the stage CLUT rows the stage routine copied last (simulation version 6, `stage::anim::Rows`)
    pub stage_rows: crate::fight::stage::anim::Rows,
    /// a round started or the match-end reset ran during this frame's logic: that frame draws no stage
    /// (`stage_anim_step`); set and taken within one `tick`
    #[serde(skip)]
    stage_skip: bool,
    /// engine addition (the import S7, simulation v15): the imported fighters' cores and state (`br1.rs`);
    /// None without imported fighters
    #[serde(default)]
    pub br1: Option<Box<crate::fight::br1::bridge::Br1Match>>,
}

impl Match {
    /// M.9E this frame: the special KO's value, else 1 in CUSTOM AFTERIMAGE MODE.
    pub fn m9e(&self) -> u8 {
        if self.round.m9e != 0 {
            self.round.m9e
        } else if self.custom.flags & 0x800 != 0 {
            1
        } else {
            0
        }
    }

    pub fn new(data: &Loaded) -> Self {
        let mut m = Self {
            fighters: Self::fresh_fighters(data, 0, None, 100),
            world: World {
                stage: {
                    let mut s = match &data.br1_stage {
                        Some(b) => crate::fight::stage::Stage::br1(data.stage_id, b),
                        None => crate::fight::stage::Stage::new(&data.tables, data.stage_id, data.gates()),
                    };
                    s.debris_count = data.debris.len();
                    s
                },
                ..World::new()
            },
            pads: [0; 2],
            buttons: [crate::assets::game_tables::default_buttons(); 2],
            right: Vec3::X,
            rand: 1,
            frame: 0,
            events: Vec::new(),
            round: crate::fight::round::Round::new(),
            cpu: [false, false],
            cpu_level: 0x80,
            ai: [None, None],
            ai_rnd: [0; 256],
            sound_queue: Vec::new(),
            sounds_started: Vec::new(),
            cam: crate::fight::camera::Camera::default(),
            replay: Replay::new(),
            vm: duel::setup::effect_vm(data),
            auto_rematch: true,
            result: None,
            prev_pads: [0; 2],
            watch_pads: [0; 2],
            prev_watch: [0; 2],
            demo_cam: 0,
            hp_pct: 100,
            costume: data.costume,
            custom: crate::fight::rules::Custom::normal(),
            fx_delayed: Vec::new(),
            fxn: Default::default(),
            force_move: None,
            config_request: false,
            play_frames: 0,
            practice: false,
            challenger: None,
            pause: Default::default(),
            practice_d4: 0,
            vibration: true,
            vib: [[(0, 0); 2]; 2],
            flash_task: [None; 2],
            back_colour: [None; 2],
            stage_rows: Default::default(),
            stage_skip: false,
            fx: crate::fight::fx::hit_effects::Fx::new(duel::setup::rings(data), crate::fight::fx::hit_effects::Arena { half: data.ring }),
            br1: Self::br1_new(data),
        };
        if data.gates().sparks_arena {
            // the sparks' arena is the ring the stage uses (the edge parts' record; the extra stages'
            // engine-made bounds, `stage::EXTRA_BOUNDS`)
            m.fx.arena.half = (m.world.stage.ring.hx as i32, m.world.stage.ring.hz as i32);
        }
        m.fx.keep_slot = data.gates().keep_slot;
        // boot: copy and shuffle with seed 0
        m.ai_rnd.copy_from_slice(&data.trig.random[..256]);
        crate::fight::ai::shuffle(&mut m.ai_rnd, &data.trig.random, 0);
        m.br1_fresh(data, 0);
        m.round.new_match();
        m.start_round(data);
        m.stage_skip = false;
        m
    }

    /// A decoded state's ids and indices into `data` and the fixed tables are in range: the characters, costumes,
    /// moves, move-script positions and attack chains of both fighters, their AI blocks, the effect task lists, the
    /// KO replay's log, slots and snapshots, the imported stage's sounds for the session. The data is a match's
    /// (two sides, not a team battle's four) and the effect tasks name fighters 0 / 1 only.
    fn check(&self, data: &Loaded) -> Result<(), String> {
        if data.sides() != 2 {
            return Err("not a match's data".into());
        }
        if self.fx_delayed.iter().any(|&(side, ..)| side >= 2) {
            return Err("delayed effects".into());
        }
        self.fxn.check(2, &data.tables)?;
        Self::br1_check(&self.br1, data)?;
        duel::setup::check_world(&self.world, self.br1.is_some(), data)?;
        Self::check_fighters(&self.fighters, &self.ai, data)?;
        if !self.costume.iter().all(|&c| c < 4) {
            return Err("costumes".into());
        }
        self.fx.check()?;
        self.replay.check(data)
    }

    /// The fighters' and AI blocks' part of `check` (also run on the KO replay's snapshots).
    fn check_fighters(fighters: &[Fighter; 2], ai: &[Option<crate::fight::ai::Ai>; 2], data: &Loaded) -> Result<(), String> {
        for (i, f) in fighters.iter().enumerate() {
            if data.br1[i].is_some() {
                // an imported face: its core is checked (`br1_check`); no main-game script or AI
                if f.player > 1 || (f.chr as usize) >= CHARACTERS.len() || ai[i].is_some() {
                    return Err(format!("fighter {i}"));
                }
                continue;
            }
            let ani = &data.fighters[i].ani;
            if !duel::setup::fighter_fits(f, ani) {
                return Err(format!("fighter {i}"));
            }
            if ai[i].as_ref().is_some_and(|a| !a.valid(ani)) {
                return Err(format!("AI {i}"));
            }
        }
        Ok(())
    }

    /// After deserialising: the static data the state refers to, from the fight's `data` (the effect VM's script
    /// blobs, static tables and model records, the hit-spark ring tables, also in the KO replay's snapshots).
    /// Err when the effect VM's static data does not match `data` (`Vm::rebind`).
    pub fn rebind(&mut self, data: &Loaded) -> Result<(), String> {
        self.check(data)?;
        self.vm.rebind(&duel::setup::effect_vm(data))?;
        let rings = duel::setup::rings(data);
        self.fx.rebind(&rings);
        self.replay.rebind(&rings);
        Ok(())
    }

    /// The effect code's view of the match (`fx::host`): each fighter's opponent the other one.
    pub(crate) fn fx_host(&mut self) -> crate::fight::fx::host::FxHost<'_> {
        crate::fight::fx::host::FxHost {
            fighters: &mut self.fighters,
            opp: &[1, 0],
            vm: &mut self.vm,
            fxn: &mut self.fxn,
            fx: &mut self.fx,
            fx_delayed: &mut self.fx_delayed,
            flash_task: &mut self.flash_task,
            sound_queue: &mut self.sound_queue,
            stage: &self.world.stage,
            cam: &self.cam,
            costume: &self.costume,
            pads: &self.pads,
            custom: &self.custom,
            round_state: self.round.state,
            rand: self.rand,
            ext: None,
        }
    }

    /// Vibration request: motor 0 large (0..255), 1 small (on / off), for `frames`; human players only.
    pub fn vibrate(&mut self, side: usize, motor: usize, frames: u8, val: u8) {
        if self.vibration && !self.cpu[side] {
            self.vib[side][motor] = (frames, val);
        }
    }

    /// Switch a player between human and CPU control.
    pub fn set_cpu(&mut self, i: usize, on: bool) {
        self.cpu[i] = on;
        // practice: the dummy (0x40 | M.D4); otherwise the normal AI (mode 0)
        let mode = if self.practice { (self.practice_d4 | 0x40) as u16 } else { 0 };
        self.fighters[i].o_01 = if on { mode as u8 + 1 } else { 0 };
        if let Some(ai) = self.ai[i].as_mut() {
            ai.state = mode;
        }
    }

    /// Model position in glTF space (game (x, y, z) → (x, −y, −z) / 300).
    pub fn world_pos(&self, i: usize) -> Vec3 {
        let t = self.fighters[i].m_t;
        Vec3::new(t[0] as f32, -t[1] as f32, -t[2] as f32) / 300.0
    }

    /// Model transform in glTF space (the renderer's; the logic's up to simulation version 2, then
    /// `portable::transform`).
    pub fn transform(&self, i: usize) -> glam::Mat4 {
        duel::transform(&self.fighters[i])
    }

    /// WATCH mode (M.9C = 2): CPU against CPU outside the attract demo and practice.
    pub fn watching(&self) -> bool {
        self.cpu[0] && self.cpu[1] && !self.round.demo && !self.practice
    }

    /// A second player pressed START during a 1P fight: the fight freezes for the announcement.
    pub fn challenge(&mut self, side: usize) {
        if self.challenger.is_none() && self.result.is_none() {
            self.challenger = Some((side, 0));
        }
    }

    /// The world as the presentation draws it (`fight::scene`).
    pub fn scene(&self, data: &Loaded) -> crate::fight::scene::SceneRef<'_> {
        crate::fight::scene::SceneRef {
            fighters: &self.fighters,
            opp: &[1, 0],
            cam: &self.cam,
            stage: &self.world.stage,
            darkening: self.world.g4a58,
            fades: crate::fight::scene::Fades::BySlot(&self.world.p4b18),
            fx: &self.fx,
            vm: &self.vm,
            fxn: &self.fxn,
            back_colour: &self.back_colour,
            custom: &self.custom,
            br1: self.br1.as_deref().map(crate::fight::br1::bridge::Br1Match::view),
            frame: self.frame,
            round_index: self.round.index,
            round_state: self.round.state,
            m9e: self.m9e(),
            stage_view: self.stage_view(data),
        }
    }

    /// What the stage routine shows this frame: the state (simulation version 6) or the frame count.
    pub fn stage_view(&self, data: &Loaded) -> crate::fight::stage::anim::View {
        if data.gates().stage_state {
            crate::fight::stage::anim::View::State(self.world.stage.anim, self.stage_rows)
        } else {
            crate::fight::stage::anim::View::Frame(self.frame)
        }
    }
}
