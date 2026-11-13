//! The team battle: four fighters on one stage, 2 VS 2 or FREE FOR ALL (engine addition).
//!
//! Engine addition; the per-fighter routines are the match's (`fight::duel`, `duel::hits`, `throw`)
//! State: serialised: `TeamFight` is part of `GameState` (`GameState::team`; field changes are `STATE_VERSION` changes)
//!
//! `TeamFight` is a world of its own beside `Match` (whose layout the replay traces hash): four fighters, each
//! with its target (`target.rs`) as the opponent the per-fighter steps see, the frame (`frame.rs`) and the
//! round flow (`round.rs`). Fighter `k` plays as player `k & 1` (its slot) with the second copy of the per-player
//! resources when `k >> 1` (its bank) is 1: the effect VM's groups 5 / 6, EffTab 100..139, sound slots 7 / 8,
//! the AI's random index + 64, and the per-fighter `PlayerExt` swapped into the `World`'s slot around its steps.

use crate::assets::roster::CHARACTERS;
use crate::fight::ai::Ai;
use crate::fight::duel;
use crate::fight::fight_data::Loaded;
use crate::fight::fighter::Fighter;
use crate::fight::move_decision::World;
use serde::{Deserialize, Serialize};

pub mod arcade;
pub mod br1;
#[cfg(test)]
mod br1_tests;
mod frame;
pub mod hud;
pub mod round;
pub mod target;
#[cfg(test)]
mod tests;
pub mod watch;

pub use arcade::ArcadeRules;
pub use hud::TeamHud;
pub use round::TeamRound;

/// The fighters of a team battle.
pub const N: usize = 4;

/// The two sub-modes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum TeamMode {
    /// 2 VS 2: P1 + P3 against P2 + P4 (the team is the slot)
    Vs2,
    /// FREE FOR ALL: every fighter on its own
    Ffa,
}

impl TeamMode {
    /// Fighter `k`'s team: its slot in 2 VS 2, itself in FREE FOR ALL.
    pub fn team(self, k: usize) -> usize {
        match self {
            TeamMode::Vs2 => crate::fight::side::Fid::of(k).slot().index(),
            TeamMode::Ffa => k,
        }
    }

    /// The number of teams.
    pub fn teams(self) -> usize {
        match self {
            TeamMode::Vs2 => 2,
            TeamMode::Ffa => N,
        }
    }
}

/// How a team battle ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum TeamOutcome {
    /// the team (2 VS 2: 0 = P1 + P3, 1 = P2 + P4; FREE FOR ALL: the fighter) that won the match
    Winner(usize),
    /// pause → QUIT → YES (engine addition, T4: the pause of the team battle)
    Quit,
    /// (T7, simulation v25 `version::v::DRAW_GAME`) several teams reached the rounds needed at once: DRAW GAME, nobody won
    Draw,
    /// (TEAM ARCADE A3, simulation v26) the humans' team lost and pressed START during the CONTINUE countdown
    Continue,
    /// (TEAM ARCADE A3, simulation v26) the CONTINUE countdown ran out: GAME OVER has been shown
    GameOver,
}

/// The per-player state of the game that is per fighter in the team battle, swapped into the `World`'s
/// slot `k & 1` around fighter `k`'s steps: the palette fade record (`World::p4b18`) and the HUD flash bytes.
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize)]
pub struct PlayerExt {
    pub p4b18: [i16; 4],
    pub hud_flash: [u8; 4],
}

#[derive(Clone, Serialize, Deserialize)]
pub struct TeamFight {
    pub mode: TeamMode,
    pub fighters: [Fighter; N],
    pub world: World,
    /// raw pad words (PS1 layout) by fighter (the CPU's are 0)
    pub pads: [u16; N],
    prev_pads: [u16; N],
    pub buttons: [[u16; 8]; N],
    pub rand: u32,
    pub frame: u64,
    /// log of hits (debug, not state)
    #[serde(skip)]
    pub events: Vec<String>,
    pub round: TeamRound,
    pub cpu: [bool; N],
    pub cpu_level: u16,
    pub ai: [Option<Ai>; N],
    /// the shared AI random table (in RAM), reshuffled every round
    #[serde(with = "crate::base::serde_util::big_array")]
    pub ai_rnd: [u8; 256],
    /// each fighter's opponent: its target (`target.rs`)
    pub target: [usize; N],
    /// the targets' locks: L2's, a thrown fighter's thrower (`target::Locks`)
    pub locked: target::Locks,
    /// the fighter each one hit last (`combo_flags`: the combo count restarts when that one is in no combo)
    pub last_victim: [usize; N],
    /// the SFX queue and the sounds started this frame
    pub sound_queue: Vec<(i16, i16)>,
    pub sounds_started: Vec<i16>,
    pub fx: crate::fight::fx::hit_effects::Fx,
    pub vm: crate::fight::fx::script_vm::Vm,
    pub cam: crate::fight::camera::Camera,
    pub result: Option<TeamOutcome>,
    pub hp_pct: i32,
    pub costume: [u8; N],
    pub custom: crate::fight::rules::Custom,
    pub fx_delayed: Vec<(usize, u8, i16)>,
    pub fxn: crate::fight::fx::native_tasks::State,
    /// fighters 2 / 3's part of `fxn` (`FxHost::ext`)
    pub fxn_ext: crate::fight::fx::native_tasks::SideExt,
    pub flash_task: [Option<(u8, u16, i16)>; N],
    pub back_colour: [Option<[i32; 3]>; N],
    pub ext: [PlayerExt; N],
    pub stage_rows: crate::fight::stage::anim::Rows,
    /// the HUD (`hud.rs`), stepped by the session after each frame (as a match's `Fight::hud`)
    pub hud: TeamHud,
    /// the pause menu (engine addition, T4: the match's `fight::pause`, its normal page; CONTROLLER CONFIG. does nothing
    /// here), driven by the human slots' presses (`raw_pads`; every pad's when all are CPU)
    pub pause: crate::fight::pause::Pause,
    /// the four pads as read, CPUs' too (the pause's input, as a match's WATCH pads), and the frame before's
    pub raw_pads: [u16; N],
    prev_raw: [u16; N],
    /// a round started during this frame's logic: that frame draws no stage (as `Match::stage_skip`)
    #[serde(skip)]
    stage_skip: bool,
    /// (T6, simulation v24 `version::v::TEAM_CONFIG`) the pause's CONTROLLER CONFIG. was chosen this frame: the session opens its
    /// button-mapping screen right after the tick (`session::TeamConfig`; never kept past the step)
    #[serde(skip)]
    pub config_request: bool,
    /// engine addition (T5, simulation v23): the imported fighters' cores and state (`br1.rs`); None without them
    #[serde(default)]
    pub br1: Option<Box<br1::TeamBr1>>,
    /// engine addition (TEAM ARCADE A3, simulation v26): a TEAM ARCADE battle's per-fighter CPU levels and continue
    /// (`arcade.rs`); None in every other team battle
    #[serde(default)]
    pub arcade: Option<ArcadeRules>,
}

impl TeamFight {
    /// A team battle on `data` (`fight_data::load_team`): every fighter a CPU, best of 3 with the 60 s timer.
    pub fn new(data: &Loaded, mode: TeamMode) -> Self {
        Self::with_cpu(data, mode, [true; N])
    }

    /// `new` with fighter `k` a CPU where `cpu[k]`, else a human playing pad `k`.
    pub fn with_cpu(data: &Loaded, mode: TeamMode, cpu: [bool; N]) -> Self {
        Self::with_setup(data, mode, cpu, 0x80, [crate::assets::game_tables::default_buttons(); N], None)
    }

    /// `with_cpu` with the CPUs' level and each fighter's button map (the front end's: OPTIONS' difficulty, P1 / P2's
    /// CONTROLLER CONFIG.; pads 3 / 4 the default map) and, in a TEAM ARCADE battle (2 VS 2, simulation v26), its rules
    /// (`ArcadeRules`: the AI blocks take their levels from it).
    pub fn with_setup(data: &Loaded, mode: TeamMode, cpu: [bool; N], cpu_level: u16, buttons: [[u16; 8]; N], arcade: Option<ArcadeRules>) -> Self {
        debug_assert!(arcade.is_none() || (mode == TeamMode::Vs2 && data.gates().team_arcade), "TEAM ARCADE: 2 VS 2 from v26");
        assert_eq!(data.sides(), N, "the team battle's data");
        // BIG STAGE (T5): the imported game's own ring and walls at their size, its debris as a match's
        let stage = match &data.br1_stage {
            Some(b) => {
                let mut s = crate::fight::stage::Stage::br1(data.stage_id, b);
                s.debris_count = data.debris.len();
                s
            }
            None => crate::fight::stage::Stage::team(&data.tables, data.stage_id, data.gates()),
        };
        let mut m = Self {
            mode,
            fighters: std::array::from_fn(|k| duel::setup::fresh_fighter(data, k, 0, None, 100)),
            world: World { stage, ..World::new() },
            pads: [0; N],
            prev_pads: [0; N],
            buttons,
            rand: 1,
            frame: 0,
            events: Vec::new(),
            round: TeamRound::new(),
            cpu,
            cpu_level,
            ai: Default::default(),
            ai_rnd: [0; 256],
            target: [1, 0, 3, 2],
            locked: Default::default(),
            last_victim: [1, 0, 3, 2],
            sound_queue: Vec::new(),
            sounds_started: Vec::new(),
            fx: crate::fight::fx::hit_effects::Fx::new(duel::setup::rings(data), crate::fight::fx::hit_effects::Arena { half: data.ring }),
            vm: Self::effect_vm(data),
            cam: crate::fight::camera::Camera::default(),
            result: None,
            hp_pct: 100,
            costume: data.team.as_ref().map_or([0; N], |t| t.costume),
            custom: crate::fight::rules::Custom::normal(),
            fx_delayed: Vec::new(),
            fxn: Default::default(),
            fxn_ext: Default::default(),
            flash_task: [None; N],
            back_colour: [None; N],
            ext: [PlayerExt::default(); N],
            stage_rows: Default::default(),
            hud: TeamHud::default(),
            stage_skip: false,
            config_request: false,
            pause: Default::default(),
            raw_pads: [0; N],
            prev_raw: [0; N],
            br1: Self::br1_new(data),
            arcade,
        };
        m.fx.arena.half = (m.world.stage.ring.hx as i32, m.world.stage.ring.hz as i32);
        m.fx.keep_slot = true;
        m.ai_rnd.copy_from_slice(&data.trig.random[..256]);
        crate::fight::ai::shuffle(&mut m.ai_rnd, &data.trig.random, 0);
        // as `Match::configure` leaves them: F.1 = 1 for a CPU before the round start
        for k in 0..N {
            m.fighters[k].o_01 = m.cpu[k] as u8;
        }
        m.br1_fresh(data, 0);
        m.round.new_match();
        m.start_round(data);
        m.stage_skip = false;
        m
    }

    /// The HUD's frame after the battle's (as `Fight::tick` steps a match's): built at the first round of the match,
    /// skipped while the round flow hides it.
    pub fn hud_step(&mut self) {
        let r = &self.round;
        // an imported fighter's gauge on its own marks and its own name (T5, as a match's: `bridge::hud_gauge`)
        // (only those fighters copied)
        let own: [Option<Fighter>; N] = std::array::from_fn(|k| {
            self.br1.as_ref().filter(|b| b.side[k])?;
            let mut f = self.fighters[k].clone();
            f.gauge = crate::fight::br1::bridge::hud_gauge(f.gauge, f.form);
            Some(f)
        });
        let fs: [&Fighter; N] = std::array::from_fn(|k| own[k].as_ref().unwrap_or(&self.fighters[k]));
        let names: [Option<&str>; N] = std::array::from_fn(|k| {
            let b = self.br1.as_ref().filter(|b| b.side[k])?;
            crate::formats::br1_anm::CHARACTERS.get(b.fs[k].f.chr as usize).and_then(|c| crate::assets::roster::br1_name(c))
        });
        if r.index == 0 && r.state == 0 && r.cnt <= 1 {
            self.hud.build(fs);
        }
        if !self.round.hud_hidden {
            self.hud.frame(fs, &self.last_victim, &self.round, self.mode, names);
        }
    }

    /// What `present` draws of the team battle's world (as `Match::scene`): each fighter's opponent its target.
    pub fn scene(&self) -> crate::fight::scene::SceneRef<'_> {
        crate::fight::scene::SceneRef {
            fighters: &self.fighters,
            opp: &self.target,
            cam: &self.cam,
            stage: &self.world.stage,
            darkening: self.world.g4a58,
            fades: crate::fight::scene::Fades::ByFighter(&self.ext),
            fx: &self.fx,
            vm: &self.vm,
            fxn: &self.fxn,
            back_colour: &self.back_colour,
            custom: &self.custom,
            br1: self.br1_scene(),
            frame: self.frame,
            round_index: self.round.index,
            round_state: self.round.state,
            m9e: 0,
            stage_view: crate::fight::stage::anim::View::State(self.world.stage.anim, self.stage_rows),
        }
    }

    /// Fighter `k`'s CPU level (its AI block's): a TEAM ARCADE battle's per fighter (`ArcadeRules::levels`), else the
    /// battle's one `cpu_level`.
    pub fn level(&self, k: usize) -> u16 {
        self.arcade.as_ref().map_or(self.cpu_level, |a| a.levels[k])
    }

    /// Fighter `k`'s team (`TeamMode::team`).
    pub fn team(&self, k: usize) -> usize {
        self.mode.team(k)
    }

    /// Fighter `k` is out of the round (KO'd or rung out): no target, no hits, no push, not in the camera.
    pub fn out(&self, k: usize) -> bool {
        self.round.out[k]
    }

    /// (T6) The pause's CONTROLLER CONFIG. is greyed and refused: from simulation v24 (`version::v::TEAM_CONFIG`) with no human on pad
    /// 1 or 2, as a WATCH match's (its screen edits P1 / P2's maps); before v24 never (the item plays its sound and does
    /// nothing). One rule for the pause's step (`frame.rs` `pause_step`) and its drawing (`present`).
    pub fn config_greyed(&self, g: crate::fight::version::Gates) -> bool {
        g.team_config && self.cpu[0] && self.cpu[1]
    }

    /// The effect VM of a match (`duel::setup::effect_vm`) with bank 1's groups: scripts 5 / 6 = &EffTab[110 / 130] (P3's /
    /// P4's `<CHR>EFT` 11..20, as P1's / P2's at 50 / 70), emitters 5 / 6 = &EffTab[100 / 120].
    fn effect_vm(data: &Loaded) -> crate::fight::fx::script_vm::Vm {
        use crate::fight::side::{Bank, Slot};
        let mut vm = duel::setup::effect_vm(data);
        for s in [Slot::P1, Slot::P2] {
            vm.set_group(Bank::B1.vm_group(s), &data.eff_tab[Bank::B1.eft_base(s) + 10..]);
        }
        for s in [Slot::P1, Slot::P2] {
            vm.set_model_group(Bank::B1.vm_group(s), &data.eff_tab[Bank::B1.eft_base(s)..]);
        }
        vm
    }

    /// The effect code's view of the team battle (`fx::host`): each fighter's opponent its target.
    pub(crate) fn fx_host(&mut self) -> crate::fight::fx::host::FxHost<'_> {
        crate::fight::fx::host::FxHost {
            fighters: &mut self.fighters,
            opp: &self.target,
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
            ext: Some(&mut self.fxn_ext),
        }
    }

    /// A decoded state's ids and indices are in range for `data`: the fighters and AI blocks, the targets, the
    /// costumes, the effect tasks.
    fn check(&self, data: &Loaded) -> Result<(), String> {
        if data.sides() != N || data.team.is_none() {
            return Err("not a team battle's data".into());
        }
        self.br1_check(data)?;
        duel::setup::check_world(&self.world, self.br1.is_some(), data)?;
        for (k, f) in self.fighters.iter().enumerate() {
            let ani = &data.fighters[k].ani;
            if data.br1[k].is_some() {
                // an imported face: its core is checked (`br1_check`); no main-game script or AI
                if f.player > 1 || (f.chr as usize) >= CHARACTERS.len() || self.ai[k].is_some() {
                    return Err(format!("fighter {k}"));
                }
                continue;
            }
            if !duel::setup::fighter_fits(f, ani) {
                return Err(format!("fighter {k}"));
            }
            if self.ai[k].as_ref().is_some_and(|a| !a.valid(ani)) {
                return Err(format!("AI {k}"));
            }
        }
        // never itself (the duel's steps take the fighter and its target as a pair, `duel::pair_mut`), nor a teammate
        // (2 VS 2: `target::update` and the hits only ever pick an enemy)
        let own = |ts: &[usize; N]| ts.iter().enumerate().any(|(k, &t)| t >= N || self.mode.team(t) == self.mode.team(k));
        if own(&self.target) || own(&self.last_victim) || !self.costume.iter().all(|&c| c < 4) {
            return Err("targets".into());
        }
        self.round.check(self.mode)?;
        // (TEAM ARCADE, v26) its rules only in a 2 VS 2 battle of v26; the continue and GAME OVER states only with them
        match &self.arcade {
            Some(_) if self.mode != TeamMode::Vs2 || !data.gates().team_arcade => return Err("arcade rules".into()),
            Some(a) => a.check()?,
            None if self.round.state >= 0xF => return Err("round".into()),
            None => {}
        }
        if self.fx_delayed.iter().any(|&(side, ..)| side >= N) {
            return Err("delayed effects".into());
        }
        self.fxn.check(N, &data.tables)?;
        self.fx.check()
    }

    /// After deserialising: the static data the state refers to, from `data` (as `Match::rebind`).
    pub fn rebind(&mut self, data: &Loaded) -> Result<(), String> {
        self.check(data)?;
        self.vm.rebind(&Self::effect_vm(data))?;
        self.fx.rebind(&duel::setup::rings(data));
        Ok(())
    }
}
