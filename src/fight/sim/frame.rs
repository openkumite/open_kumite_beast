//! One fight frame: `Match::tick` and the steps of `frame_step` in the game's order.
//!
//! State: none of its own: steps the serialised `Match`
//!
//! One fight frame: `tick` (the game frame `frame_step`, then from simulation version 6 the stage routine's part of
//! the draw, `stage_anim_step`) and the steps of `frame_step`, in the game's order:
//!
//! | step | what it does |
//! |---|---|
//! | `swap_debug` | debug OKB_SWAPDBG (engine addition) |
//! | `challenger_step` | the challenger interrupt |
//! | `vibration_step` | the motors |
//! | `pause_step` | the pause controller |
//! | `colour_flash_step` | the flash tasks |
//! | `frozen_step` | paused (m.82): only the camera decays |
//! | `replay_step` | the KO replay (`ko_replay.rs`) |
//! | `demo_camera` | the attract demo's camera (state 2) |
//! | `input_step` | input |
//! | `throw::resolve` | throws |
//! | `fighter_steps` | per fighter: decision and move script, from v13 the head turn |
//! | `physics::integrate_pair`, `landing` | move, land |
//! | `skeleton_step` | the skeleton's nodes; from v13 node 2's integer world position |
//! | `script_link::refresh`, `effect_op` | the move scripts' effect commands |
//! | `hits`, `combo_flags` | hits (below, through `fight::duel::hits`) |
//! | `push_apart_step`, `physics::opponent_angles` | the pair's angles, the push-apart |
//! | `stage_step` | the stage update |
//! | the swing nodes | the extra nodes of type 3 |
//! | `darkening_step` | the Beast Drive darkening, the fades |
//! | `round_step` | the round flow |
//! | `replay_log_step` | the KO replay's recording |
//! | `effect_lists_step` | the effect lists |
//! | `camera_step` | the fight camera |
//! | `sound_step` | the sound delays |
//!
//! `input_step` and `fighter_steps` run each fighter against the other through `fight::duel` (index-explicit: the team
//! battle runs the same code), the hits through `duel::hits`'s pair functions.

use super::{Match, Outcome};
use crate::fight::duel::{self, hits};
use crate::fight::fight_data::Loaded;
use crate::fight::fighter::{F58, Fighter};
use crate::fight::move_script;
use crate::fight::physics;
use glam::Vec3;

impl Match {
    /// One 60 Hz game frame (`frame_step`), then, from simulation version 6, the stage routine's part of the
    /// frame's draw (`stage_anim_step`).
    pub fn tick(&mut self, data: &Loaded) {
        self.frame_step(data);
        if data.gates().stage_state {
            self.stage_anim_step(data);
        }
    }

    /// The stage draw runs once per displayed fight frame, also while paused; it skips the per-stage
    /// routine (counters, CLUT copies) under M.9E and M.9F, the darkening at 255. The
    /// frames a later round starts on and the match-end reset runs on draw no fight (recorded, ST3 / ST5: W.26 / W.28
    /// = 0 on the round-start frame, then 0x14; unchanged on the frame of round state 0xD at cnt 0).
    fn stage_anim_step(&mut self, data: &Loaded) {
        let br1 = data.br1_stage.as_ref().filter(|_| data.gates().br1_stage_sound);
        if br1.is_some() && data.gates().br1_debris {
            self.world.stage.br1.sounds.clear();
        }
        if std::mem::take(&mut self.stage_skip) || self.m9e() != 0 || self.world.g4a58[2] == 255 {
            return;
        }
        crate::fight::stage::anim::step(data.stage_no, &mut self.world.stage.anim, &mut self.stage_rows);
        // an imported stage's routine (in the imported game's draw): the block it watches is P2's if P2
        // is the imported game's Uriko (F.03 8), else P1's; F.05 as its face holds it (a main-game side's own form)
        if let Some(s) = br1 {
            let k = self.br1.as_ref().map_or(0, |b| b.view().watched());
            if !data.gates().br1_debris {
                self.world.stage.br1.step(s.g94, self.fighters[k].form);
                return;
            }
            // simulation v18: Uriko's stage's walls (its shatters into the imported game's debris), then
            // the stage debris (from the main game's wall breaks too: `stage_step`) and their sounds
            let d = duel::frame::br1_stage_debris(&mut self.world.stage, s, self.fighters[k].form, self.cam.ang[1]);
            self.br1_stage_sounds(s.g94, d.stage, d.sounds, d.splash);
        }
    }

    /// One 60 Hz game frame (input for both fighters, then decision + script per fighter), the phases in the
    /// game's order.
    fn frame_step(&mut self, data: &Loaded) {
        #[cfg(not(target_arch = "wasm32"))]
        self.swap_debug();
        if self.challenger_step() {
            return;
        }
        self.vibration_step();
        let watch_press = self.pause_step(data);
        self.colour_flash_step(data);
        if self.frozen_step(data) {
            return;
        }
        let freeze = false;
        if self.round.state == 4 {
            self.play_frames += 1;
        }
        self.replay_step(data);
        self.demo_camera(data);
        if self.br1.as_ref().is_some_and(|b| b.both()) {
            // engine addition (the import S7): the imported game's fighters run the imported game's frame (`br1.rs`)
            self.br1_steps(data);
            self.stage_step(data);
        } else if self.br1.is_some() {
            // an imported and a main-game fighter: each by its own engine (`br1.rs`)
            self.br1_mixed_steps(data, freeze);
        } else {
            self.br2_steps(data, freeze);
        }
        self.darkening_step();
        let d = self.world_pos(1) - self.world_pos(0);
        let d = Vec3::new(d.x, 0.0, d.z);
        if d.length() > 0.05 {
            self.right = d.normalize();
        }
        self.round_step(data, watch_press);
        self.replay_log_step();
        self.effect_lists_step(data);
        self.camera_step(data);
        self.sound_step();
        self.frame += 1;
    }

    /// The fighters' part of `frame_step` (main-game fighters): input, throws, decision and scripts, movement,
    /// skeleton, effect commands, hits, push-apart, stage, opponent angles, swing nodes.
    fn br2_steps(&mut self, data: &Loaded, freeze: bool) {
        self.input_step(data, freeze);
        // throws
        crate::fight::throw::resolve(&mut self.fighters);
        let effect_reqs = self.fighter_steps(data);
        // move both, land, angles (hits and push-apart come later)
        physics::integrate_pair(&mut self.fighters, &data.trig);
        for i in 0..2 {
            physics::landing(&mut self.fighters[i]);
        }
        let nodes = self.skeleton_step(data);
        // effect commands of the move scripts (ops 0x16 / 0x17 / 0x18), with this frame's skeleton
        crate::fight::fx::script_link::refresh(&mut self.fx_host(), data);
        for (i, b) in effect_reqs {
            crate::fight::fx::script_link::effect_op(&mut self.fx_host(), data, i, b);
        }
        self.hits(data, &nodes);
        self.combo_flags();
        let th = self.push_apart_step(data, &nodes);
        self.stage_step(data);
        physics::opponent_angles(&mut self.fighters, &data.trig, th);
        // the extra nodes of type 3: the script-driven extra nodes (the beast's jaw) ease toward their targets
        for f in self.fighters.iter_mut() {
            for sw in f.swing.iter_mut() {
                sw.step();
            }
        }
    }

    /// Debug trace (OKB_SWAPDBG) of the input swap every 30 frames.
    #[cfg(not(target_arch = "wasm32"))]
    fn swap_debug(&self) {
        if (crate::base::trace::flag("OKB_SWAPDBG") || crate::base::trace::flag("BR2_SWAPDBG"))
            && !crate::base::trace::is_muted()
            && self.frame.is_multiple_of(30)
        {
            let p = [self.world_pos(0), self.world_pos(1)];
            eprintln!(
                "frame {} cam yaw {} yaw {} {} x {:.2} {:.2} swap {} {}",
                self.frame,
                self.cam.ang[1],
                self.fighters[0].yaw,
                self.fighters[1].yaw,
                p[0].x,
                p[1].x,
                duel::facing_left(&self.fighters[0], self.cam.ang[1]),
                duel::facing_left(&self.fighters[1], self.cam.ang[1])
            );
        }
    }

    /// The challenger interrupt (round-flow prologue): the fight is frozen (m.82 = −1), the jingle at
    /// frame 2, "Here Comes A Challenger !!!" until frame 300. True while it runs (the rest of the frame is skipped).
    fn challenger_step(&mut self) -> bool {
        if let Some((side, cnt)) = self.challenger {
            self.frame += 1;
            self.round.message = crate::fight::round::Message::Challenger;
            self.round.music = (cnt == 2).then_some(2);
            self.round.music_stop = false;
            self.round.gover_voice = None;
            self.sounds_started.clear();
            if cnt >= 300 {
                self.result = Some(Outcome::Challenger(side));
            }
            self.challenger = Some((side, cnt + 1));
            return true;
        }
        false
    }

    /// The vibration motors run every frame, also while paused.
    fn vibration_step(&mut self) {
        for port in self.vib.iter_mut() {
            for mo in port.iter_mut() {
                if mo.0 == 0 {
                    mo.1 = 0;
                } else {
                    mo.0 -= 1;
                }
            }
        }
    }

    /// The pause controller, from round state 4 (and the practice flow). Returns WATCH's new presses
    /// of both pads (for the WATCH camera in the round flow).
    fn pause_step(&mut self, data: &Loaded) -> [u16; 2] {
        let human_press = [0, 1].map(|i| if self.cpu[i] { 0 } else { self.pads[i] & !self.prev_pads[i] });
        let watch = self.cpu[0] && self.cpu[1];
        // WATCH (M.9C = 2): both pads drive the pause and the camera although both sides are CPU
        let watch_press = if self.watching() { [0, 1].map(|i| self.watch_pads[i] & !self.prev_watch[i]) } else { [0; 2] };
        self.prev_watch = self.watch_pads;
        let pressed = human_press[0] | human_press[1] | watch_press[0] | watch_press[1];
        let can_open = self.round.state == 4 && self.result.is_none();
        let mut psounds = Vec::new();
        let act = self.pause.tick(pressed, can_open, self.practice, watch, self.practice_d4, &mut psounds);
        self.sound_queue.extend(psounds);
        match act {
            crate::fight::pause::Action::Quit => self.result = Some(Outcome::Quit),
            // the button-mapping screen over the paused fight (run by the app)
            crate::fight::pause::Action::ControllerConfig => self.config_request = true,
            crate::fight::pause::Action::Reset(v) => {
                // practice settings changed: m.9A = 2, m.1 = 6 (the round is rebuilt)
                self.practice_d4 = v;
                self.fighters = Self::fresh_fighters(data, 0, None, self.hp_pct);
                self.br1_fresh(data, 0);
                self.start_round(data);
            }
            _ => {}
        }
        watch_press
    }

    /// The flash tasks: set the back colour, cycle the pattern; the duration only runs unpaused.
    fn colour_flash_step(&mut self, data: &Loaded) {
        let frozen = self.pause.frozen();
        for side in 0..2 {
            duel::frame::colour_flash(&data.trig, &mut self.flash_task[side], &mut self.back_colour[side], frozen);
        }
    }

    /// While paused (m.82): fighters, hits, round flow, effects and the replay recorder stop; the camera only decays
    /// its shake, except the TV camera (0x20), which runs on (only MOVE checks the freeze). True when frozen.
    fn frozen_step(&mut self, data: &Loaded) -> bool {
        if self.pause.frozen() {
            self.prev_pads = self.pads;
            self.frame += 1;
            if self.cam.mode & 0x7F == 0x20 {
                let s = self.cam_subjects();
                self.cam.update(&data.trig, [&s[0], &s[1]], true, 0);
            } else {
                for k in 0..2 {
                    self.cam.shake[k] = ((-3 * self.cam.shake[k] as i32) / 4) as i16;
                }
            }
            self.sounds_started.clear();
            return true;
        }
        false
    }

    /// The attract demo's camera (attract state 2, before the fight step): every 240 frames TV
    /// (0x20) or MOVE at random, then a random TV preset — two RNG calls either way.
    fn demo_camera(&mut self, data: &Loaded) {
        if self.round.demo {
            if self.demo_cam == 0 {
                self.demo_cam = 0xF0;
                let m = if move_script::rand(&mut self.rand, &data.trig.random) & 1 != 0 { 0x20 } else { crate::fight::camera::MOVE };
                self.cam.set_mode(m);
                self.cam.preset = (move_script::rand(&mut self.rand, &data.trig.random) & 7) as i16;
            }
            self.demo_cam -= 1;
        }
    }

    /// Input for both fighters.
    fn input_step(&mut self, data: &Loaded, freeze: bool) {
        for i in 0..2 {
            self.input_one(data, freeze, i);
        }
    }

    /// `input_step` of fighter `i` against the other (`duel::input`).
    pub(super) fn input_one(&mut self, data: &Loaded, freeze: bool, i: usize) {
        let (pad, cam_yaw, timer) = (self.pads[i], self.cam.ang[1], self.round.timer);
        duel::input(&mut self.fighters, i, 1 - i, data, pad, &self.buttons[i], self.ai[i].as_mut(), timer, cam_yaw, freeze);
    }

    /// Per fighter: health and gauge, the state flags, the automatic turn, the move
    /// decision, the move script, the frame step and the animation, the velocity. Returns the effect requests of
    /// the move scripts (ops 0x16 / 0x17 / 0x18).
    fn fighter_steps(&mut self, data: &Loaded) -> Vec<(usize, [u8; 6])> {
        let mut effect_reqs: Vec<(usize, [u8; 6])> = Vec::new();
        for i in 0..2 {
            self.fighter_one(data, i, &mut effect_reqs);
        }
        effect_reqs
    }

    /// `fighter_steps` of fighter `i` against the other (`duel::fighter`), its effect requests added to `effect_reqs`.
    pub(super) fn fighter_one(&mut self, data: &Loaded, i: usize, effect_reqs: &mut Vec<(usize, [u8; 6])>) {
        let opp = [1 - i];
        let mut p = duel::Parts {
            world: &mut self.world,
            cam: &mut self.cam,
            rand: &mut self.rand,
            sound_queue: &mut self.sound_queue,
            custom: &self.custom,
            force_move: &mut self.force_move,
            vib: (self.vibration && !self.cpu[i]).then_some(&mut self.vib[i]),
            round_state: self.round.state,
            round_index: self.round.index,
            frame: self.frame,
            bank: crate::fight::side::Bank::B0,
            enemies: &opp,
        };
        duel::fighter(&mut self.fighters, i, 1 - i, data, &mut p, effect_reqs);
    }

    /// Skeleton node positions (game units) for hits and push-apart, the parts for the camera / throws / AI, the
    /// pelvis lock and the spring chains. Returns the node positions.
    pub(super) fn skeleton_step(&mut self, data: &Loaded) -> Vec<Vec<[i32; 3]>> {
        let mut rots = Vec::new();
        let nodes: Vec<Vec<[i32; 3]>> = (0..2)
            .map(|i| {
                let f = &self.fighters[i];
                if self.is_br1(i) {
                    // an imported fighter: its own world walk's nodes (published on its face)
                    rots.push(f.part_rots.clone());
                    return f.parts.clone();
                }
                let (nodes, r) = duel::skeleton(f, data, i, 1 - i);
                rots.push(r);
                nodes
            })
            .collect();
        // parts 2 / 3 / 4 (head, chest, pelvis: model +0x124 / +0x174) for the camera, throws and the AI, the nodes
        for i in 0..2 {
            if !self.is_br1(i) {
                let m = &data.fighters[i].models[self.fighters[i].beast() as usize];
                duel::frame::skeleton_parts(&mut self.fighters[i], m, &nodes[i], &rots[i]);
            }
        }
        // node 2 by the game's integer world walk, for the next frame's head turn (simulation version 13)
        if data.gates().head_turn {
            let rsqrt = data.tables.rsqrt_fn();
            for i in 0..2 {
                if self.is_br1(i) {
                    continue;
                }
                let (f, opp) = (&self.fighters[i], &self.fighters[1 - i]);
                let h = crate::fight::anim::pose::head_world(f, opp, &data.fighters[i].skels[f.beast() as usize], &data.trig, &rsqrt);
                self.fighters[i].head_w = h;
            }
        }
        // op 07.03's pelvis lock, the spring chains (extra nodes of type 2)
        for i in 0..2 {
            if !self.is_br1(i) {
                duel::frame::skeleton_tail(&mut self.fighters[i], &data.fighters[i].extras, &nodes[i], &rots[i], data.portable(), data.gates().spring_rest);
            }
        }
        nodes
    }

    /// The pair's angles and push-apart: θ from the roots, then distance / height and the push-apart, then the ring
    /// walls and from simulation v14 an extra stage's solid parts (`physics::solids`, engine addition).
    /// Returns θ.
    pub(super) fn push_apart_step(&mut self, data: &Loaded, nodes: &[Vec<[i32; 3]>]) -> i32 {
        let b0 = self.body_spheres(data, 0);
        let b1 = self.body_spheres(data, 1);
        let before = [self.fighters[0].m_t, self.fighters[1].m_t];
        let th = physics::root_angle(&self.fighters, &data.trig);
        let rsqrt = data.tables.rsqrt_fn();
        physics::body_pair(&mut self.fighters, th, &data.trig, [&b0, &b1], [&nodes[0], &nodes[1]], &rsqrt);
        for i in 0..2 {
            let f = &mut self.fighters[i];
            let (dx, dz) = (f.m_t[0] - before[i][0], f.m_t[2] - before[i][2]);
            for p in f.parts.iter_mut() {
                p[0] += dx;
                p[2] += dz;
            }
            f.head_w[0] += dx;
            f.head_w[2] += dz;
        }
        // an imported fighter (a mixed pair) moves its core with the push (engine addition)
        self.br1_follow_push(data, before);
        // ring walls for P1 then P2; a thrower and its victim move together; an
        // imported fighter's are its own (engine addition)
        let st = [self.fighters[0].o_00, self.fighters[1].o_00];
        let (t_a, mut ax, mut az) = self.ring_walls_one(data, 0, st[1], &b0);
        let (t_b, mut bx, mut bz) = self.ring_walls_one(data, 1, st[0], &b1);
        let (mut t_a, mut t_b) = (t_a != 0, t_b != 0);
        // simulation v14 (engine addition): an extra stage's solid parts push as walls do, added to the walls' push
        if data.gates().extra_solids {
            let stage = &self.world.stage;
            let (s_a, x, z) = physics::solids(&self.fighters[0], st[1], &b0, stage, data.gates());
            (t_a, ax, az) = (t_a || s_a, ax + x, az + z);
            let (s_b, x, z) = physics::solids(&self.fighters[1], st[0], &b1, stage, data.gates());
            (t_b, bx, bz) = (t_b || s_b, bx + x, bz + z);
        }
        let throw = |a: &Fighter, b: &Fighter| a.flags[F58] & 0x4000_0000 != 0 && b.flags[F58] & 0x10_0000 != 0;
        if (throw(&self.fighters[0], &self.fighters[1]) || throw(&self.fighters[1], &self.fighters[0])) && (t_a || t_b) {
            for f in self.fighters.iter_mut() {
                physics::shift(f, ax + bx, az + bz);
            }
        } else {
            if t_a {
                self.shift_one(data, 0, ax, az);
            }
            if t_b {
                self.shift_one(data, 1, bx, bz);
            }
        }
        th
    }

    /// Fighter `i`'s body spheres for the push-apart and the walls: its ANI's for its form, an imported
    /// fighter's ANM's (the same records).
    fn body_spheres(&self, data: &Loaded, i: usize) -> Vec<crate::formats::act_ani::Sphere> {
        match (self.br1_body(i), data.br1_res(i), &self.br1) {
            (true, Some(r), Some(b)) => r.anm.spheres[b.fs[i].cur as usize & 1].clone(),
            _ => data.fighters[i].ani.spheres(self.fighters[i].beast() as usize),
        }
    }

    /// Stage update (`duel::frame::stage_step`): wall hp, shake, the break; the broken walls' events and
    /// sound. In AFTERIMAGE mode (M.9E != 0) it returns at once: no wall damage, shake or break.
    pub(super) fn stage_step(&mut self, data: &Loaded) {
        let run = self.m9e() == 0;
        duel::frame::stage_step(&mut self.world.stage, data, run, &mut self.vm, &mut self.sound_queue, &mut self.events, self.frame);
    }

    /// The Beast Drive darkening, then the fade steps.
    fn darkening_step(&mut self) {
        duel::frame::stage_fade_step(&mut self.world.g4a58, &self.fighters);
        // the per-fighter palette fades, each blacking out the fighter of its player
        for (k, p) in self.world.p4b18.iter_mut().enumerate() {
            duel::frame::fade_step(p, self.fighters.iter_mut().find(|f| f.player as usize == k));
        }
    }

    /// The round flow (new presses, the WATCH camera) and its requests.
    fn round_step(&mut self, data: &Loaded, watch_press: [u16; 2]) {
        // new presses of the human pads: buttons only (the logical START bit is synthetic here)
        let pressed = self.fighters.iter().filter(|f| f.o_01 == 0).fold(0u16, |a, f| a | (f.pressed & 0xE8));
        let mut sounds = Vec::new();
        self.round.raw_press = [0, 1].map(|i| if self.cpu[i] { 0 } else { self.pads[i] & !self.prev_pads[i] });
        self.prev_pads = self.pads;
        let pressed = pressed | (self.round.raw_press[0] | self.round.raw_press[1]) & 0x800;
        if self.watching() {
            self.round.watch_camera(&mut self.cam, watch_press[0] | watch_press[1]);
        }
        let req = self.round.tick(&mut self.fighters, &mut sounds, &mut self.cam, pressed);
        self.sound_queue.extend(sounds);
        match req {
            crate::fight::round::Request::WinReset => {
                self.win_reset(data);
                self.round.win_tail(&mut self.fighters, &mut self.cam, pressed);
            }
            crate::fight::round::Request::NextRound(mode) => {
                let cpu = [self.fighters[0].o_01, self.fighters[1].o_01];
                self.fighters = Self::fresh_fighters(data, mode, Some(&self.fighters), self.hp_pct);
                for i in 0..2 {
                    self.fighters[i].o_01 = cpu[i] & 0x7F;
                }
                self.br1_fresh(data, mode);
                self.start_round(data);
            }
            crate::fight::round::Request::NewMatch if !self.auto_rematch => {
                self.result = Some(Outcome::Winner(self.round.winner.unwrap_or(0)));
            }
            crate::fight::round::Request::Continue => self.result = Some(Outcome::Continue),
            crate::fight::round::Request::GameOver => self.result = Some(Outcome::GameOver),
            crate::fight::round::Request::NewMatch => {
                self.fighters = Self::fresh_fighters(data, 0, None, self.hp_pct);
                self.br1_fresh(data, 0);
                self.round.new_match();
                if data.gates().stage_state {
                    // a new match loads the stage again, its CLUT rows with it
                    self.stage_rows = Default::default();
                }
                self.start_round(data);
            }
            crate::fight::round::Request::None => {}
        }
        self.br1_key_cut(data);
    }

    /// Effect lists (pool 1), after the hit detection of this frame; the effect scripts (list 4) run
    /// before the effect lists 6..14.
    fn effect_lists_step(&mut self, data: &Loaded) {
        let frame = self.frame;
        crate::fight::fx::effect_lists(&mut self.fx_host(), data, frame);
    }

    /// The fight camera.
    fn camera_step(&mut self, data: &Loaded) {
        let [s0, s1] = self.cam_subjects();
        self.cam.stage = data.stage_no as u8;
        let r = if matches!(self.cam.mode & 0x7F, 4 | 5 | 8 | 9 | 10 | 11) && self.cam.timer == 0 {
            move_script::rand(&mut self.rand, &data.trig.random)
        } else {
            0
        };
        self.cam.update(&data.trig, [&s0, &s1], false, r);
    }

    /// Count the delays down, start at most one sound effect this frame; then the queue's SE part for
    /// the imported queue (`duel::frame::sound_tick` both).
    fn sound_step(&mut self) {
        duel::frame::sound_tick(&mut self.sound_queue, &mut self.sounds_started);
        if let Some(b) = self.br1.as_deref_mut() {
            duel::frame::sound_tick(&mut b.sound_queue, &mut b.sounds_started);
        }
    }

    /// What the camera reads from both fighters.
    fn cam_subjects(&self) -> [crate::fight::camera::Subject; 2] {
        self.fighters.each_ref().map(crate::fight::camera::Subject::of)
    }
}

impl Match {
    /// Every frame after the hits: `combo_drop` for both, then an attacker whose opponent
    /// is in no combo restarts its combo count (`combo_reset`), so the damage scaling of `hit::apply` only applies
    /// within one combo.
    pub(super) fn combo_flags(&mut self) {
        for f in self.fighters.iter_mut() {
            hits::combo_drop(f);
        }
        for d in 0..2 {
            hits::combo_reset(&mut self.fighters, 1 - d, d);
        }
    }

    /// Hit checks of both fighters, the trade rule and the hit application.
    pub(super) fn hits(&mut self, data: &Loaded, nodes: &[Vec<[i32; 3]>]) {
        let mut kinds = [0i32; 2];
        let mut dmg = [0i32; 2];
        let mut mult = [0x80i32; 2];
        let mut hit_pos = [[0i32; 3]; 2];
        for a in 0..2 {
            if let Some(p) = hits::probe(&mut self.fighters, a, 1 - a, data, nodes) {
                (kinds[a], dmg[a], mult[a], hit_pos[a]) = (p.kind, p.dmg, p.mult, p.pos);
            }
        }
        if kinds[0] != 0 && kinds[1] != 0 {
            // trade: the weaker hit is cancelled
            if dmg[0] < dmg[1] {
                kinds[0] = 0;
            } else if dmg[1] < dmg[0] {
                kinds[1] = 0;
            }
        }
        let mut guard_fx: Vec<(usize, [i32; 3])> = Vec::new();
        for a in 0..2 {
            if kinds[a] == 0 {
                continue;
            }
            let d = 1 - a;
            let r = hits::land(&mut self.fighters, a, d, mult[a], kinds[a], data);
            self.hit_fx(data, a, d, &r, hit_pos[a], &mut guard_fx);
            self.events.push(format!(
                "frame {}: P{} {} P{} for {} (level {})",
                self.frame,
                a + 1,
                if r.guarded { "guarded by" } else { "hits" },
                d + 1,
                r.damage,
                r.level
            ));
        }
        hits::guard_fx(&mut self.fx_host(), &data.tables, guard_fx);
    }

    /// The effects of attacker `a`'s hit `r` on `d` at `pos` (clean hit,
    /// guarded); a beast attacker's guard spark into `guard_fx`.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn hit_fx(&mut self, data: &Loaded, a: usize, d: usize, r: &crate::fight::hit::HitResult, pos: [i32; 3], guard_fx: &mut Vec<(usize, [i32; 3])>) {
        hits::hit_fx(&mut self.fx, &self.custom, data, &self.fighters, a, d, r, pos, guard_fx);
    }
}
