//! A round's and a match's setup: round start, CUSTOM / practice rules, fresh fighters, match-end reset.
//!
//! State: none of its own: steps the serialised `Match`
//!
//! A round's and a match's setup: the round start (`start_round`, from `new`, `configure` and the round flow),
//! the CUSTOM / practice rules (`configure`), the fighters as a round starts them (`fresh_fighters`) and the
//! match-end reset (`win_reset`).

use super::{Match, Rules};
use crate::fight::duel;
use crate::fight::fight_data::Loaded;
use crate::fight::fighter::Fighter;
use crate::fight::move_script::{self, Ctx};

impl Match {
    /// Round start: reshuffle the AI table and set up both AI blocks.
    pub(super) fn start_round(&mut self, data: &Loaded) {
        self.round.start();
        self.round.win_reset = data.gates().win_reset;
        // MODEL TYPE: F54 bits for both fighters, CPU included; practice: F54 |= 2
        let bits = [0u32, 4, 8, 0x10][self.custom.model.min(3) as usize];
        for (i, f) in self.fighters.iter_mut().enumerate() {
            f.flags[crate::fight::fighter::F54] |= bits;
            if self.practice {
                f.flags[crate::fight::fighter::F54] |= 2;
            }
            let _ = i;
        }
        // the fighter reset ends with a move start and a script step: the round-start move 0x28 runs once right away (its
        // shared entry calls the MODEL TYPE shaping subroutine, op 0x1D)
        for i in 0..2 {
            if self.is_br1(i) {
                // an imported fighter ran its state 0x28 in its reset (`br1_fresh`)
                continue;
            }
            self.fighters[i].node_scale.clear();
            move_script::start_move(&mut self.fighters[i], &data.fighters[i].ani, 0x28);
            self.run_start_move(data, i);
        }
        // the fight-effect state is reset at every round start: pools wiped, one-shot effects
        crate::fight::fx::native_tasks::reset(&mut self.fx_host(), data);
        // walls restored every round; E5 = rounds lost so far; rounds needed
        self.world.stage.round_init(self.round.index as i32, self.round.need as i32);
        if data.gates().stage_state {
            // ... ending with the stage init table
            self.world.stage.anim.reset(data.stage_no);
            self.stage_skip = true;
        }
        self.world.c0_b4 = self.round.need;
        // fades start clear every round
        self.world.g4a58 = [0; 4];
        self.world.p4b18 = Default::default();
        for i in 0..2 {
            self.fighters[i].o_e5 = self.round.lost[i];
        }
        // fighters stand still until "FIGHT!!": intro lock (invulnerable, no throws, pre-fight idle)
        for f in self.fighters.iter_mut() {
            f.flags[crate::fight::fighter::F60] |= 0x0001_8004;
        }
        // the camera snaps behind the fighters, P1 on the left
        self.cam.snap = -1;
        self.cam.set_mode(crate::fight::camera::MOVE);
        let seed = move_script::rand(&mut self.rand, &data.trig.random) as u32;
        crate::fight::ai::shuffle(&mut self.ai_rnd, &data.trig.random, seed);
        for i in 0..2 {
            let f = &mut self.fighters[i];
            // AI paused until GET READY ends; practice: (M.D4 | 0x40) + 1, running from the first frame
            // demo (attract state 1): P1.1 = 0x11, P2.1 = 0x12 (AI modes 0x10 / 0x11), running from the first frame
            f.o_01 = if !self.cpu[i] {
                0
            } else if self.practice {
                (self.practice_d4 | 0x40) + 1
            } else if self.round.demo {
                0x11 + i as u8
            } else {
                0x81
            };
            let mode = if f.o_01 != 0 { (f.o_01 & 0x7F) as i32 - 1 } else { 0 };
            // an imported fighter: its own AI (`br1_ai_init`), no main-game one
            self.ai[i] = (!self.is_br1(i)).then(|| crate::fight::ai::Ai::new(&data.fighters[i].ani, &self.ai_rnd, i as u8, self.cpu_level, mode));
            if self.is_br1(i) {
                self.br1_ai_init(data, i);
            }
        }
    }

    /// Apply the match rules and restart the match with them (before the first frame).
    pub fn configure(&mut self, data: &Loaded, rules: Rules) {
        self.round.need = rules.need;
        self.round.time_limit = rules.time_limit;
        self.hp_pct = rules.hp_pct;
        self.cpu_level = rules.cpu_level;
        self.custom = rules.custom;
        self.practice = rules.practice;
        self.round.practice = rules.practice;
        self.round.demo = rules.demo;
        self.demo_cam = 0;
        if rules.demo {
            self.round.time_limit = -1;
        }
        if rules.practice {
            self.round.need = 0;
            self.round.time_limit = -1;
        }
        self.world.c4_w8 = rules.custom.flags;
        let cpu = self.cpu;
        self.fighters = Self::fresh_fighters(data, 0, None, self.hp_pct);
        for i in 0..2 {
            self.fighters[i].o_01 = cpu[i] as u8;
        }
        self.br1_fresh(data, 0);
        self.round.new_match();
        self.start_round(data);
        self.stage_skip = false;
    }

    /// Both fighters at a round start (loader state 6): P1 at x = −0x100
    /// facing +0x400, P2 at +0x100 facing −0x400, move 0x28 (stand). `mode` (match +0x9A): 0 first round
    /// (full beast gauge), 2 next round (gauge and beast form carried over), 3 sudden death (no health left),
    /// 4 the match-end reset (`win_reset`: damage, gauge and form kept, the move request restarted).
    pub fn fresh_fighters(data: &Loaded, mode: u8, prev: Option<&[Fighter; 2]>, hp_pct: i32) -> [Fighter; 2] {
        [0, 1].map(|i| duel::setup::fresh_fighter(data, i, mode, prev.map(|p| &p[i]), hp_pct))
    }

    /// Round state 0xD / 0xE at cnt 0 (simulation version 6): the effect reset (1) clears the effect pools,
    /// the fades are cleared (M.9F = 0) and the fighter reset (mode 4) re-initialises the winner, then the loser: placed as at
    /// a round start, damage, gauge and form kept, its AI block set up again and its move request run once.
    pub(super) fn win_reset(&mut self, data: &Loaded) {
        self.world.g4a58 = [0; 4];
        self.world.p4b18 = Default::default();
        let w = self.round.winner.unwrap_or(0);
        for i in [w, 1 - w] {
            // built when its turn comes: the winner's start move may change the loser before
            self.fighters[i] = duel::setup::fresh_fighter(data, i, 4, Some(&self.fighters[i]), self.hp_pct);
            // F.E5 = F.E4 (rounds lost)
            self.fighters[i].o_e5 = self.round.lost[i];
            if self.is_br1(i) {
                // an imported fighter: its reset (mode 1), no AI or start move of the main game
                self.br1_fresh_one(data, i, 4);
                self.ai[i] = None;
                self.br1_ai_init(data, i);
                continue;
            }
            let f = &self.fighters[i];
            let mode = if f.o_01 != 0 { (f.o_01 & 0x7F) as i32 - 1 } else { 0 };
            self.ai[i] = Some(crate::fight::ai::Ai::new(&data.fighters[i].ani, &self.ai_rnd, i as u8, self.cpu_level, mode));
            self.run_start_move(data, i);
        }
        // the effect state's reset (the effect reset only sets its state; the effect task restarts later in the frame)
        crate::fight::fx::native_tasks::reset(&mut self.fx_host(), data);
        self.stage_skip = true;
    }

    /// A script step once right after the fighter's reset (its sounds and effect requests are dropped).
    pub(super) fn run_start_move(&mut self, data: &Loaded, i: usize) {
        let acts = data.acts(i, 1 - i);
        let (mut sounds, mut effects) = (Vec::new(), Vec::new());
        let mut ctx = Ctx {
            ani: &data.fighters[i].ani,
            acts,
            trig: &data.trig,
            rand: &mut self.rand,
            sounds: &mut sounds,
            cam: &mut self.cam,
            ring: (self.world.stage.ring.hx as i32, self.world.stage.ring.hz as i32),
            walls: [0, 1, 2, 3].map(|k| self.world.stage.walls.present(k)),
            effects: &mut effects,
            need: self.world.c0_b4,
            effects_off: self.round.state >= 0xF,
            stage: data.stage_no as i8,
            round: self.round.index,
            fades: &mut self.world.p4b18,
            stage_fade: &mut self.world.g4a58,
            gates: data.gates(),
        };
        move_script::run(&mut self.fighters, i, 1 - i, &mut ctx);
    }
}
