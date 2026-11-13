//! One team battle frame: `TeamFight::tick`, the match's frame steps over four fighters (engine addition).
//!
//! State: none of its own: steps the serialised `TeamFight`
//!
//! The steps in the match's order (`fight::sim::frame`), with the targets first: `target_step`, input, throws
//! (`throw_step`: each fighter and its target, a grab locking the pair), the fighters (`duel::fighter` against the
//! target, the fighter's `PlayerExt` in its slot), the movement (a throw pair attached), the skeletons, the effect
//! commands, the hits (every enemy pair, `hits_step`), the push-apart (every pair still in the round), the stage, the
//! angles to the target, the swing nodes, the darkening, the round flow (`TeamRound`), the effect lists, the camera
//! over the fighters still in the round, the sounds; before them the pause (`pause_step`, T4). Not run: the KO replay, the challenger, the attract
//! demo, the scripted cameras (a move script's camera ops write a scratch copy), the vibration.

use super::round::Request;
use super::target::in_throw;
use super::{N, TeamFight, TeamOutcome};
use crate::fight::duel::{self, hits};
use crate::fight::fight_data::Loaded;
use crate::fight::fighter::{F5C, F6C, F58, F68, Fighter};
use crate::fight::move_script::{self, Ctx};
use crate::fight::physics;
use crate::fight::side::{Bank, Fid, Slot};

impl TeamFight {
    /// One 60 Hz frame, then the stage routine's part of the draw (simulation version 6 on: always here).
    pub fn tick(&mut self, data: &Loaded) {
        if self.result.is_some() {
            return;
        }
        if !self.pause_step(data.gates()) {
            self.frame_step(data);
        }
        // (T5) BIG STAGE: a frame's sounds of the imported game's stage for the session (`Match::stage_anim_step`)
        let br1 = data.br1_stage.as_ref().filter(|_| data.gates().br1_debris);
        if br1.is_some() {
            self.world.stage.br1.sounds.clear();
        }
        if std::mem::take(&mut self.stage_skip) || self.world.g4a58[2] == 255 {
            return;
        }
        crate::fight::stage::anim::step(data.stage_no, &mut self.world.stage.anim, &mut self.stage_rows);
        // (T5) an imported stage's routine on the fighter it watches (`Br1Scene::watched`): Uriko's
        // stage's walls, then the imported game's debris of the broken walls and their sounds
        if let Some(s) = br1 {
            let k = self.br1_scene().map_or(0, |b| b.watched());
            let d = duel::frame::br1_stage_debris(&mut self.world.stage, s, self.fighters[k].form, self.cam.ang[1]);
            self.br1_stage_sounds(s.g94, d.stage, d.sounds, d.splash);
        }
    }

    fn frame_step(&mut self, data: &Loaded) {
        self.colour_flash_step(data);
        self.target_step(data);
        // (T5) the faces' writes since the last frame into the imported cores, the stage's walls into theirs
        self.br1_absorb();
        let walls0 = self.br1_walls_in();
        self.input_step(data);
        self.throw_step(data);
        self.br1_swap_in();
        let effect_reqs = self.fighter_steps(data);
        self.integrate_step(data);
        for k in 0..N {
            // (an imported core landed in its move; a posed throw victim lands with its proxy)
            if !self.br1_body(k) && !self.puppeted(k) {
                physics::landing(&mut self.fighters[k]);
            }
        }
        let nodes = self.skeleton_step(data);
        crate::fight::fx::script_link::refresh(&mut self.fx_host(), data);
        for (i, b) in effect_reqs {
            crate::fight::fx::script_link::effect_op(&mut self.fx_host(), data, i, b);
        }
        self.hits_step(data, &nodes);
        self.combo_flags();
        self.br1_absorb();
        let th = self.push_apart_step(data);
        self.stage_step(data);
        for k in 0..N {
            // (an imported thrower across the engines and its posed victim: their pair's angles, `after_hits`)
            if !self.puppeted(k) && !(self.br1_body(k) && self.in_cross_throw(k)) {
                physics::angles(&mut self.fighters[k], &data.trig, th[k]);
            }
        }
        self.br1_absorb();
        self.br1_publish_all();
        for k in 0..N {
            if !self.is_br1(k) {
                for sw in self.fighters[k].swing.iter_mut() {
                    sw.step();
                }
            }
        }
        self.br1_finish(data, walls0);
        self.darkening_step();
        self.round_step(data);
        self.effect_lists_step(data);
        self.camera_step(data);
        self.sound_step();
        self.frame += 1;
    }

    /// The pause (engine addition, T4): the match's controller (its normal page) on the human slots' new presses
    /// (`raw_pads`; a CPU slot's pad does not drive it, as a match's; every pad when all four are CPU, as WATCH), from
    /// round state 4. True while it is open or closing: the battle stands still and the camera's shake decays (as a
    /// match's `frozen_step`). From simulation v24 (T6) CONTROLLER CONFIG. asks the session for the button-mapping
    /// screen (`config_request`), offered as a WATCH match offers it: only with a human on pad 1 or 2 (the screen
    /// edits P1 / P2's maps).
    fn pause_step(&mut self, g: crate::fight::version::Gates) -> bool {
        let watch = self.cpu.iter().all(|&c| c);
        let pressed = (0..N).filter(|&k| watch || !self.cpu[k]).fold(0, |a, k| a | (self.raw_pads[k] & !self.prev_raw[k]));
        self.prev_raw = self.raw_pads;
        let mut sounds = Vec::new();
        let config = g.team_config;
        let act = self.pause.tick(pressed, self.round.state == 4, false, self.config_greyed(g), 0, &mut sounds);
        self.sound_queue.extend(sounds);
        match act {
            crate::fight::pause::Action::Quit => self.result = Some(TeamOutcome::Quit),
            crate::fight::pause::Action::ControllerConfig if config => self.config_request = true,
            _ => {}
        }
        if !self.pause.frozen() {
            return false;
        }
        self.prev_pads = self.pads;
        self.frame += 1;
        for k in 0..2 {
            self.cam.shake[k] = ((-3 * self.cam.shake[k] as i32) / 4) as i16;
        }
        self.sounds_started.clear();
        true
    }

    /// The flash tasks of every fighter (`duel::frame::colour_flash`; this frame only runs unpaused).
    fn colour_flash_step(&mut self, data: &Loaded) {
        for k in 0..N {
            duel::frame::colour_flash(&data.trig, &mut self.flash_task[k], &mut self.back_colour[k], false);
        }
    }

    /// The targets (`target::update`), L2 of the human pads cycling them.
    fn target_step(&mut self, data: &Loaded) {
        let team = std::array::from_fn(|k| self.team(k));
        let l2 = std::array::from_fn(|k| !self.cpu[k] && self.pads[k] & !self.prev_pads[k] & crate::fight::input::pad::L2 != 0);
        // (T7, simulation v25) a thrown fighter keeps its thrower after the release until it is in a state both games
        // share after a throw (`bridge::victim_done`: lying, getting up, standing): its thrown move still reads its
        // opponent (an imported victim's places itself from it and waits on it); a thrower holding (F58
        // 0x40000000) and its victim keep each other while the victim is not in such a state (`target::thrown`).
        let keep = if data.gates().thrown_lock {
            let done = std::array::from_fn(|k| crate::fight::br1::bridge::victim_done(self.state_of(k)).is_some());
            super::target::thrown(&self.fighters, &self.target, &mut self.locked.thrown_by, done, &self.round.out)
        } else {
            [false; N]
        };
        super::target::update(&self.fighters, &mut self.target, &mut self.locked.l2, &team, &self.round.out, l2, keep);
        // (T5) the thrower and the victim of a throw across the engines keep each other
        if let Some(b) = &self.br1 {
            for (t, c) in b.throws.iter() {
                (self.target[t], self.target[c.victim as usize]) = (c.victim as usize, t);
            }
        }
    }

    /// Input of every fighter against its target (`duel::input`, an imported fighter's `br1_input`); L2 is the
    /// target cycle's, not the taunt's. The victim of a throw across the engines takes none (as a match's).
    fn input_step(&mut self, data: &Loaded) {
        let (cam_yaw, timer) = (self.cam.ang[1], self.round.timer);
        for k in 0..N {
            let pad = self.pads[k] & !crate::fight::input::pad::L2;
            if self.br1.as_ref().is_some_and(|b| b.throws.thrower_of(k).is_some()) {
                continue;
            }
            if self.is_br1(k) {
                self.br1_input(data, k, pad);
                continue;
            }
            duel::input(&mut self.fighters, k, self.target[k], data, pad, &self.buttons[k], self.ai[k].as_mut(), timer, cam_yaw, false);
        }
    }

    /// Throws of each fighter and its target in index order, once per pair; a pair with a fighter in
    /// another throw is skipped. A grab locks the thrower and the victim onto each other (`target::in_throw`).
    fn throw_step(&mut self, data: &Loaded) {
        let mut seen = [[false; N]; N];
        for k in 0..N {
            let t = self.target[k];
            if t == k || seen[k][t] || self.out(k) || self.out(t) || self.team(k) == self.team(t) {
                continue;
            }
            (seen[k][t], seen[t][k]) = (true, true);
            let busy = |a: usize, b: usize| in_throw(&self.fighters[a]) && self.target[a] != b;
            // (T5) a fighter in a throw across the engines: its thrower's engine holds it
            if busy(k, t) || busy(t, k) || self.in_cross_throw(k) || self.in_cross_throw(t) {
                continue;
            }
            let p = if k < t { [k, t] } else { [t, k] };
            let grabbed = |f: &Fighter| f.flags[F6C] & 0x80_0000 != 0;
            let before = p.map(|i| grabbed(&self.fighters[i]));
            if self.br1_body(k) || self.br1_body(t) {
                // (T5) a pair with an imported fighter: the imported game's resolution, or a match's mixed pair's
                self.br1_throw(data, p);
            } else {
                crate::fight::throw::resolve_pair(&mut self.fighters, p);
            }
            for (j, &v) in p.iter().enumerate() {
                if !before[j] && grabbed(&self.fighters[v]) {
                    let a = p[1 - j];
                    (self.target[v], self.target[a]) = (a, v);
                    self.events.push(format!("frame {}: P{}'s grab on P{}", self.frame, a + 1, v + 1));
                }
            }
        }
    }

    /// Every fighter's step against its target (`duel::fighter`), its `PlayerExt` in its slot of the `World`; the
    /// move scripts' camera ops write a scratch copy of the camera (not used in the team battle).
    fn fighter_steps(&mut self, data: &Loaded) -> Vec<(usize, [u8; 6])> {
        let mut effect_reqs = Vec::new();
        for k in 0..N {
            // (T5) an imported fighter by its engine; a main-game victim of an imported throw by its
            // thrower's (`br1_fighter`); a main-game throw's proxy in its imported victim's slot as a
            // main-game fighter, the damage it takes onto the victim's face (`br1_take_damage`)
            if self.br1_body(k) {
                self.br1_fighter(data, k);
                continue;
            }
            if self.puppeted(k) {
                continue;
            }
            let proxy = self.swapped(k);
            if proxy {
                self.br1_take_damage(k);
            }
            self.in_slot(k, true, |t| {
                let enemies: Vec<usize> = (0..N).filter(|&j| t.team(j) != t.team(k) && !t.out(j)).collect();
                let mut cam = t.cam.clone();
                let mut force = None;
                let mut p = duel::Parts {
                    world: &mut t.world,
                    cam: &mut cam,
                    rand: &mut t.rand,
                    sound_queue: &mut t.sound_queue,
                    custom: &t.custom,
                    force_move: &mut force,
                    vib: None,
                    round_state: t.round.state,
                    round_index: t.round.index,
                    frame: t.frame,
                    bank: Fid::of(k).bank(),
                    enemies: &enemies,
                };
                duel::fighter(&mut t.fighters, k, t.target[k], data, &mut p, &mut effect_reqs);
            });
            if proxy {
                self.br1_take_damage(k);
            }
        }
        effect_reqs
    }

    /// The pair step's movement: a throw pair (the victim attached by F6C 0x20000 / 0x40000) through
    /// `physics::integrate_pair_of` (the lower index in P1's place), every other fighter alone, in index order.
    fn integrate_step(&mut self, data: &Loaded) {
        let attached = |f: &Fighter| f.flags[F6C] & 0x6_0000 != 0;
        let mut done = [false; N];
        for k in 0..N {
            if done[k] {
                continue;
            }
            // (T5) an imported core by its engine (`br1_integrate`); a posed throw victim moves with its proxy
            if self.br1_body(k) {
                self.br1_integrate(data, k, &mut done);
                continue;
            }
            if self.puppeted(k) {
                done[k] = true;
                continue;
            }
            let t = self.target[k];
            if t != k && !done[t] && self.target[t] == k && !self.br1_body(t) && (attached(&self.fighters[k]) || attached(&self.fighters[t])) {
                physics::integrate_pair_of(&mut self.fighters, if k < t { [k, t] } else { [t, k] }, &data.trig);
                done[t] = true;
            } else {
                physics::integrate(&mut self.fighters[k], &data.trig);
            }
            done[k] = true;
        }
    }

    /// The skeletons (`Match::skeleton_step` for four main-game fighters, each against its target).
    fn skeleton_step(&mut self, data: &Loaded) -> Vec<Vec<[i32; 3]>> {
        // (T5) an imported fighter's nodes: its core's world walk's, on its face (a main-game throw's proxy in
        // its slot: the victim's, as a match's); its head, chest, pelvis, springs are its core's
        let br1: [bool; N] = std::array::from_fn(|i| self.is_br1(i));
        let (nodes, rots): (Vec<_>, Vec<_>) = (0..N)
            .map(|i| {
                let f = &self.fighters[i];
                if br1[i] { (f.parts.clone(), f.part_rots.clone()) } else { duel::skeleton(f, data, i, self.target[i]) }
            })
            .unzip();
        for i in (0..N).filter(|&i| !br1[i]) {
            let m = &data.fighters[i].models[self.fighters[i].beast() as usize];
            duel::frame::skeleton_parts(&mut self.fighters[i], m, &nodes[i], &rots[i]);
        }
        let rsqrt = data.tables.rsqrt_fn();
        for i in (0..N).filter(|&i| !br1[i]) {
            let (f, opp) = (&self.fighters[i], &self.fighters[self.target[i]]);
            let h = crate::fight::anim::pose::head_world(f, opp, &data.fighters[i].skels[f.beast() as usize], &data.trig, &rsqrt);
            self.fighters[i].head_w = h;
        }
        // (the team battle is simulation v22 on: always the portable path)
        for i in (0..N).filter(|&i| !br1[i]) {
            duel::frame::skeleton_tail(&mut self.fighters[i], &data.fighters[i].extras, &nodes[i], &rots[i], true);
        }
        nodes
    }

    /// The hits: every ordered pair of enemies still in the round probed in index
    /// order, the trade rule for each mutual pair, then every hit landed (one attack hits each enemy it touches). A
    /// third party's hit on a throw victim does damage only; one on the thrower breaks the throw.
    fn hits_step(&mut self, data: &Loaded, nodes: &[Vec<[i32; 3]>]) {
        // (T5) each attacker tested by its engine (`probe_on_br1`: the main game's on an imported core, `probe_br1`:
        // the imported game's with its direction bits and contact point); a posed throw victim attacks no one, and an
        // imported thrower and its victim across the engines strike in the thrower's pair (`cross_br1_thrower`)
        type Probed = Option<(hits::Probe, u32, Option<(u32, [i16; 3])>)>;
        let mut probes: [[Probed; N]; N] = Default::default();
        for a in 0..N {
            for d in 0..N {
                if a == d || self.team(a) == self.team(d) || self.out(a) || self.out(d) {
                    continue;
                }
                if self.puppeted(a) || (self.puppeted(d) && self.cross_pair(a, d)) {
                    continue;
                }
                let p = match (self.br1_body(a), self.br1_body(d)) {
                    (false, false) => hits::probe(&mut self.fighters, a, d, data, nodes).map(|p| (p, None)),
                    (false, true) => self.probe_on_br1(data, a, d, nodes).map(|p| (p, None)),
                    (true, _) => self.probe_br1(data, a, d).map(|(p, dir, pos)| (p, Some((dir, pos)))),
                };
                if let Some((p, b)) = p {
                    probes[a][d] = Some((p, self.fighters[d].flags[F68], b));
                }
            }
        }
        let kind = |p: &Probed| p.map_or(0, |(p, _, _)| p.kind);
        for a in 0..N {
            for d in a + 1..N {
                if kind(&probes[a][d]) != 0 && kind(&probes[d][a]) != 0 {
                    // trade: the weaker hit is cancelled
                    let (da, dd) = (probes[a][d].unwrap().0.dmg, probes[d][a].unwrap().0.dmg);
                    if da < dd {
                        probes[a][d] = None;
                    } else if dd < da {
                        probes[d][a] = None;
                    }
                }
            }
        }
        let mut guard_fx = Vec::new();
        let mut broken = Vec::new();
        for a in 0..N {
            for d in 0..N {
                let Some((p, f68, b1)) = probes[a][d].filter(|(p, _, _)| p.kind != 0) else { continue };
                let f58 = self.fighters[d].flags[F58];
                let third = self.target[d] != a;
                let (only, thrower) = (third && f58 & 0x10_0000 != 0, third && f58 & 0x4000_0000 != 0);
                let guarded = if let Some((dir, pos16)) = b1 {
                    // (T5) an imported attacker's hit, by its engine (its effects are the imported game's requests)
                    let guarded = self.land_br1(data, a, d, &p, dir, pos16, only);
                    self.last_victim[a] = d;
                    let what = if guarded { "guarded by" } else { "hits" };
                    self.events.push(format!("frame {}: P{} {} P{} (the imported game)", self.frame, a + 1, what, d + 1));
                    guarded
                } else {
                    let r = if self.br1_body(d) {
                        // (T5) a main-game hit on an imported core
                        self.land_on_br1(data, a, d, &p, f68, only)
                    } else if only {
                        // a throw victim hit by a third party: damage only, its throw goes on
                        let saved = self.fighters[d].clone();
                        self.fighters[d].flags[F68] = f68;
                        let r = hits::land(&mut self.fighters, a, d, p.mult, p.kind, data);
                        let hurt = std::mem::replace(&mut self.fighters[d], saved);
                        let v = &mut self.fighters[d];
                        (v.o_c8, v.o_c0, v.o_ca, v.o_cc, v.o_d8) = (hurt.o_c8, hurt.o_c0, hurt.o_ca, hurt.o_cc, hurt.o_d8);
                        r
                    } else {
                        self.fighters[d].flags[F68] = f68;
                        hits::land(&mut self.fighters, a, d, p.mult, p.kind, data)
                    };
                    self.last_victim[a] = d;
                    hits::hit_fx(&mut self.fx, &self.custom, data, &self.fighters, a, d, &r, p.pos, &mut guard_fx);
                    self.events.push(format!(
                        "frame {}: P{} {} P{} for {} (level {})",
                        self.frame,
                        a + 1,
                        if r.guarded { "guarded by" } else { "hits" },
                        d + 1,
                        r.damage,
                        r.level
                    ));
                    r.guarded
                };
                // a hit on a thrower by a third party breaks its throw unless guarded
                if !only && thrower && !guarded {
                    broken.push(d);
                }
            }
        }
        hits::guard_fx(&mut self.fx_host(), &data.tables, guard_fx);
        for t in broken {
            self.break_throw(data, t);
        }
    }

    /// A hit on thrower `t` breaks its throw: its victim escapes as from the throw-escape input (F6C 0x4000000:
    /// its decision plays 0x7E) while its hips are near the floor (op 0x02 cond −5) and it is not down, else it drops
    /// out of the throw lying (as the cross-engine hand-back, `sim::br1`): F58 0x100000 and F6C 0x860000 cleared,
    /// move 0x67 started and its script run once.
    fn break_throw(&mut self, data: &Loaded, t: usize) {
        // (T5) a throw the imported game's code holds (`br1_break`)
        if self.br1.is_some() && self.br1_break(data, t) {
            return;
        }
        let v = self.target[t];
        let f = &mut self.fighters[v];
        if v == t || f.flags[F58] & 0x10_0000 == 0 {
            return;
        }
        let low = (f.o_94 as i32) - f.hips[1] < 0xE7;
        self.events.push(format!("frame {}: P{}'s throw on P{} broken", self.frame, t + 1, v + 1));
        if low && f.flags[F5C] & 0x4_0000 == 0 {
            f.flags[F6C] |= 0x400_0000;
            return;
        }
        duel::release(f, &data.fighters[v].ani, 0x67);
        self.run_start_move(data, v);
    }

    /// `combo_drop` for every fighter, then each restarts its combo count when the fighter
    /// it hit last is in no combo.
    fn combo_flags(&mut self) {
        for f in self.fighters.iter_mut() {
            hits::combo_drop(f);
        }
        for a in 0..N {
            hits::combo_reset(&mut self.fighters, a, self.last_victim[a]);
        }
    }

    /// The pair step's angles and push-apart over four fighters: θ to the target from the roots (returned, for the angles), the
    /// distance and height to the target (+0x76 / +0x78), the push-apart of every pair still in the round in index
    /// order (`physics::body_push`), then the ring walls (`Stage::team`: before simulation v25 the enlarged rings are
    /// open, the ring-outs; from v25 walled, `version::v::TEAM_WALLS`) with a throw pair moving together.
    fn push_apart_step(&mut self, data: &Loaded) -> [i32; N] {
        let fs = &self.fighters;
        let th: [i32; N] = std::array::from_fn(|k| physics::root_angle_of(&fs[k], &fs[self.target[k]], &data.trig));
        let bodies: [Vec<crate::formats::act_ani::Sphere>; N] = std::array::from_fn(|k| self.bodies(data, k));
        let before: [[i32; 3]; N] = std::array::from_fn(|k| fs[k].m_t);
        for k in 0..N {
            let t = self.target[k];
            let g = physics::body_measure(&self.fighters[k], &self.fighters[t], th[k], &data.trig);
            let f = &mut self.fighters[k];
            f.o_76 = g.dist;
            f.o_78 = g.dy as i16;
            f.flags[F5C] &= !0x80_0000;
        }
        let rsqrt = data.tables.rsqrt_fn();
        for a in 0..N {
            for b in a + 1..N {
                // (T5) an imported thrower and its victim across the engines: pushed apart in their pair (`after_hits`)
                if self.out(a) || self.out(b) || (self.cross_pair(a, b) && (self.puppeted(a) || self.puppeted(b))) {
                    continue;
                }
                let tab = physics::root_angle_of(&self.fighters[a], &self.fighters[b], &data.trig);
                let g = physics::body_measure(&self.fighters[a], &self.fighters[b], tab, &data.trig);
                let (na, nb) = (self.fighters[a].parts.clone(), self.fighters[b].parts.clone());
                physics::body_push(&mut self.fighters, (a, b), &g, tab, &data.trig, [&bodies[a], &bodies[b]], [&na, &nb], &rsqrt);
            }
        }
        for k in 0..N {
            let f = &mut self.fighters[k];
            let (dx, dz) = (f.m_t[0] - before[k][0], f.m_t[2] - before[k][2]);
            for p in f.parts.iter_mut() {
                p[0] += dx;
                p[2] += dz;
            }
            f.head_w[0] += dx;
            f.head_w[2] += dz;
        }
        // (T5) an imported core follows its face's push (`Match::br1_follow_push`)
        self.br1_follow(data, before);
        // ring walls in index order (an imported core's, `br1_walls`); a thrower and its
        // victim move together
        let st: [u8; N] = std::array::from_fn(|k| self.fighters[self.target[k]].o_00);
        let mut walls = [(0u32, 0i32, 0i32); N];
        for k in 0..N {
            walls[k] = if self.br1.is_none() {
                physics::ring_walls(&mut self.fighters[k], st[k], &bodies[k], &mut self.world.stage)
            } else {
                self.br1_ring_walls(data, k, st[k], &bodies[k])
            };
            // (T7, simulation v25) Q3DM6's chamfered corners, added to the walls' push (a throw pair moves together); a
            // imported throw across the engines, whose walls are its pair's `after_hits`, is pushed below
            if data.gates().chamfers && !self.puppeted(k) && !(self.br1_body(k) && self.in_cross_throw(k)) {
                let (hit, dx, dz) = physics::chamfers(&self.fighters[k], st[k], &bodies[k], &self.world.stage, data.gates());
                if hit {
                    walls[k] = (walls[k].0 | 0x100, walls[k].1 + dx, walls[k].2 + dz);
                }
            }
        }
        let mut done = [false; N];
        for k in 0..N {
            if done[k] {
                continue;
            }
            let t = self.target[k];
            let pair = t != k && self.target[t] == k && {
                let (a, b) = (&self.fighters[k], &self.fighters[t]);
                let throw = |a: &Fighter, b: &Fighter| a.flags[F58] & 0x4000_0000 != 0 && b.flags[F58] & 0x10_0000 != 0;
                throw(a, b) || throw(b, a)
            };
            if pair && (walls[k].0 != 0 || walls[t].0 != 0) {
                let (dx, dz) = (walls[k].1 + walls[t].1, walls[k].2 + walls[t].2);
                self.shift_one(data, k, dx, dz);
                self.shift_one(data, t, dx, dz);
                done[t] = true;
            } else if walls[k].0 != 0 {
                self.shift_one(data, k, walls[k].1, walls[k].2);
            }
            done[k] = true;
        }
        // (T7) the chamfers for an imported throw across the engines: its thrower's core and its posed victim (with
        // the proxy) each on its own (an imported victim of a main-game throw: `br1_finish`, by its own spheres)
        let cross: Vec<usize> = (0..N).filter(|&k| self.puppeted(k) || self.br1_cross_thrower(k)).collect();
        for k in cross {
            if self.puppeted(k) {
                if !data.gates().posed_inside {
                    continue;
                }
                // the posed victim's own body (the proxy's walls saw the proxy's) inside the walls and chamfers at once
                let (dx, dz) = physics::inside(&self.fighters[k], &bodies[k], &self.world.stage, data.gates());
                if dx != 0 || dz != 0 {
                    self.shift_posed(data, k, dx, dz);
                }
                continue;
            }
            if !data.gates().chamfers {
                continue;
            }
            let (hit, dx, dz) = physics::chamfers(&self.fighters[k], st[k], &bodies[k], &self.world.stage, data.gates());
            if hit {
                self.shift_one(data, k, dx, dz);
            }
        }
        th
    }

    /// Fighter `k`'s body spheres (the push-apart's and the walls'): its ANI's for its form, an imported core's its
    /// ANM's (the same records, T5).
    pub fn bodies(&self, data: &Loaded, k: usize) -> Vec<crate::formats::act_ani::Sphere> {
        match (self.br1_body(k), data.br1_res(k), &self.br1) {
            (true, Some(r), Some(b)) => r.anm.spheres[b.fs[k].cur as usize & 1].clone(),
            _ => data.fighters[k].ani.spheres(self.fighters[k].beast() as usize),
        }
    }

    /// The walls' push of fighter `k` (`physics::shift`; an imported core's too, `Match::shift_one`).
    fn shift_one(&mut self, data: &Loaded, k: usize, dx: i32, dz: i32) {
        physics::shift(&mut self.fighters[k], dx, dz);
        if self.br1_body(k)
            && let (Some(b), Some(res)) = (self.br1.as_deref_mut(), data.br1_res(k))
        {
            crate::fight::br1::physics::push_out(&mut b.fs[k], res, dx, dz);
        }
    }

    /// The stage update: the walls' hp, shake and break, the debris (`Match::stage_step`: on BIG STAGE,
    /// T5, a broken wall falls into the imported game's debris). No other team stage breaks a wall (the open rings have
    /// none, Q3DM6's hold): the main-game part mirrors `Match::stage_step` for a team stage with breakable walls.
    /// From simulation v25 (T7) the walls round an enlarged ring shake and lose hp as a match's but never break: they
    /// stay locked (the stage step breaks a wall only while the lock is 0; a blow into a wall clears it).
    fn stage_step(&mut self, data: &Loaded) {
        if data.gates().team_walls && data.br1_stage.is_none() {
            self.world.stage.walls.lock = 1;
        }
        duel::frame::stage_step(&mut self.world.stage, data, true, &mut self.vm, &mut self.sound_queue, &mut self.events, self.frame);
    }

    /// The Beast Drive darkening and the fades, each fighter's palette fade from
    /// its `PlayerExt`.
    fn darkening_step(&mut self) {
        duel::frame::stage_fade_step(&mut self.world.g4a58, &self.fighters);
        for k in 0..N {
            duel::frame::fade_step(&mut self.ext[k].p4b18, Some(&mut self.fighters[k]));
        }
    }

    /// The round flow (`TeamRound`) and its requests.
    fn round_step(&mut self, data: &Loaded) {
        let pressed = (0..N).filter(|&k| !self.cpu[k]).fold(0u16, |a, k| a | (self.fighters[k].pressed & 0xE8) | (self.pads[k] & !self.prev_pads[k] & 0x800));
        // (TEAM ARCADE) the humans' raw presses (a human's pad word is its raw pad): the countdown's and GAME OVER's
        self.round.raw_press = (0..N).filter(|&k| !self.cpu[k]).fold(0, |a, k| a | (self.pads[k] & !self.prev_pads[k]));
        self.prev_pads = self.pads;
        let mut sounds = Vec::new();
        let req = self.round.tick(&mut self.fighters, self.mode, &mut sounds, pressed, data.gates(), self.arcade.as_mut());
        self.sound_queue.extend(sounds);
        match req {
            Request::NextRound(mode) => {
                let prev = self.fighters.clone();
                self.fighters = std::array::from_fn(|k| duel::setup::fresh_fighter(data, k, mode, Some(&prev[k]), self.hp_pct));
                for k in 0..N {
                    self.fighters[k].o_01 = prev[k].o_01 & 0x7F;
                }
                self.br1_fresh(data, mode);
                self.start_round(data);
            }
            Request::MatchOver(w) => self.result = Some(TeamOutcome::Winner(w)),
            Request::MatchDraw => self.result = Some(TeamOutcome::Draw),
            Request::Continue => self.result = Some(TeamOutcome::Continue),
            Request::GameOver => self.result = Some(TeamOutcome::GameOver),
            Request::None => {}
        }
    }

    /// The effect lists (pool 1) as the match's.
    fn effect_lists_step(&mut self, data: &Loaded) {
        let frame = self.frame;
        crate::fight::fx::effect_lists(&mut self.fx_host(), data, frame);
    }

    /// The camera: MOVE over the fighters still in the round (all four when fewer than two are, a KO'd body is
    /// dropped from it), the yaw from the two furthest apart (ties to the lower indices), the distance at most
    /// `ZOOM_MAX`.
    fn camera_step(&mut self, data: &Loaded) {
        let mut alive: Vec<usize> = (0..N).filter(|&k| !self.out(k)).collect();
        if alive.len() < 2 {
            alive = (0..N).collect();
        }
        // (TEAM ARCADE) CONTINUE? and GAME OVER: over the losing team's two fighters (the match's LOSE camera looks at
        // its loser), the winners hidden
        if let Some(a) = self.arcade.as_ref().filter(|_| self.round.state >= 0xF) {
            alive = (0..N).filter(|&k| self.team(k) == a.human_team as usize).collect();
        }
        let subj: Vec<_> = alive.iter().map(|&k| crate::fight::camera::Subject::of(&self.fighters[k])).collect();
        let (mut best, mut pair) = (-1i64, (0, 1));
        for a in 0..subj.len() {
            for b in a + 1..subj.len() {
                let (p, q) = (subj[a].root, subj[b].root);
                let (dx, dz) = ((q[0] - p[0]) as i64, (q[2] - p[2]) as i64);
                let d = dx * dx + dz * dz;
                if d > best {
                    (best, pair) = (d, (a, b));
                }
            }
        }
        self.cam.stage = data.stage_no as u8;
        // (T5) a ring larger than the main-game stages' ±2304 (BIG STAGE's ±3520) lets it go as much further out
        let r = (self.world.stage.ring.hx as i32).max(self.world.stage.ring.hz as i32);
        let zoom = if r <= 2304 { ZOOM_MAX } else { (ZOOM_MAX as i32 * r / 2304).min(0x7FFF) as i16 };
        self.cam.update_team(&data.trig, [&subj[pair.0], &subj[pair.1]], &subj, zoom);
    }

    /// The sound delays and the queue's SE part for the imported queue (`duel::frame::sound_tick`, as the match's).
    fn sound_step(&mut self) {
        duel::frame::sound_tick(&mut self.sound_queue, &mut self.sounds_started);
        if let Some(b) = self.br1.as_deref_mut() {
            duel::frame::sound_tick(&mut b.sound_queue, &mut b.sounds_started);
        }
    }

    /// Round start (as `Match::start_round`): the fighters placed, the round-start move run, the effect state and
    /// the walls reset, the intro lock, the camera snap, the AI table reshuffled and every AI block set up (bank 1's
    /// random index 64 further).
    pub(super) fn start_round(&mut self, data: &Loaded) {
        self.round.start();
        self.place(&data.trig);
        for k in 0..N {
            if self.is_br1(k) {
                // (T5) an imported fighter ran its state 0x28 in its reset (`br1_fresh`): its core to its place
                self.br1_place(data, k);
                continue;
            }
            self.fighters[k].node_scale.clear();
            move_script::start_move(&mut self.fighters[k], &data.fighters[k].ani, 0x28);
            self.run_start_move(data, k);
        }
        crate::fight::fx::native_tasks::reset(&mut self.fx_host(), data);
        self.world.stage.round_init(self.round.index as i32, self.round.need as i32);
        self.world.stage.anim.reset(data.stage_no);
        self.stage_skip = true;
        self.world.c0_b4 = self.round.need;
        self.world.g4a58 = [0; 4];
        self.world.p4b18 = Default::default();
        self.ext = Default::default();
        for f in self.fighters.iter_mut() {
            f.o_e5 = 0;
            f.flags[crate::fight::fighter::F60] |= 0x0001_8004;
        }
        self.target = [1, 0, 3, 2];
        self.locked = Default::default();
        self.last_victim = self.target;
        let team = std::array::from_fn(|k| self.team(k));
        for k in 0..N {
            if let Some(t) = super::target::nearest(&self.fighters, k, None, &team, &self.round.out) {
                self.target[k] = t;
            }
        }
        self.cam.snap = -1;
        self.cam.set_mode(crate::fight::camera::MOVE);
        let seed = move_script::rand(&mut self.rand, &data.trig.random) as u32;
        crate::fight::ai::shuffle(&mut self.ai_rnd, &data.trig.random, seed);
        for k in 0..N {
            let f = &mut self.fighters[k];
            f.o_01 = if self.cpu[k] { 0x81 } else { 0 };
            let mode = if f.o_01 != 0 { (f.o_01 & 0x7F) as i32 - 1 } else { 0 };
            let idx = Fid::of(k).ai_index();
            if self.is_br1(k) {
                // (T5) an imported fighter: its own AI
                self.ai[k] = None;
                self.br1_ai_init(data, k);
                continue;
            }
            self.ai[k] = Some(crate::fight::ai::Ai::new_at(&data.fighters[k].ani, &self.ai_rnd, idx, self.level(k), mode));
        }
    }

    /// The fighters' places at a round start: 2 VS 2 a team per side at (∓0x300, ±0x200), FREE FOR ALL the four
    /// corners at 0.6 of the ring; each faces the centre.
    fn place(&mut self, t: &crate::fight::physics::Trig) {
        let (hx, hz) = (self.world.stage.ring.hx as i32 * 6 / 10, self.world.stage.ring.hz as i32 * 6 / 10);
        for k in 0..N {
            let (x, z) = match self.mode {
                super::TeamMode::Vs2 => {
                    (if Fid::of(k).slot() == Slot::P1 { -0x300 } else { 0x300 }, if Fid::of(k).bank() == Bank::B0 { -0x200 } else { 0x200 })
                }
                super::TeamMode::Ffa => [(-hx, -hz), (hx, hz), (-hx, hz), (hx, -hz)][k],
            };
            let f = &mut self.fighters[k];
            f.pos = [x as i16, 0, z as i16];
            f.m_t = [x, 0, z];
            f.m_pos = f.pos;
            // the angle to the centre as `physics::root_angle_of` gives it (yaw 0x400 faces +x: P1's at a match's start)
            f.yaw_base = t.atan2(z, -x) as i16;
            f.yaw = f.yaw_base;
        }
    }

    /// A script step once right after fighter `k`'s reset, against its target (its sounds and effect requests dropped),
    /// its `PlayerExt` in its slot.
    pub(super) fn run_start_move(&mut self, data: &Loaded, k: usize) {
        self.in_slot(k, false, |t| {
            let acts = data.acts(k, t.target[k]);
            let (mut sounds, mut effects) = (Vec::new(), Vec::new());
            let mut cam = t.cam.clone();
            let mut ctx = Ctx {
                ani: &data.fighters[k].ani,
                acts,
                trig: &data.trig,
                rand: &mut t.rand,
                sounds: &mut sounds,
                cam: &mut cam,
                ring: (t.world.stage.ring.hx as i32, t.world.stage.ring.hz as i32),
                walls: [0, 1, 2, 3].map(|w| t.world.stage.walls.present(w)),
                effects: &mut effects,
                need: t.world.c0_b4,
                effects_off: t.round.state >= 0xF,
                stage: data.stage_no as i8,
                round: t.round.index,
                fades: &mut t.world.p4b18,
                stage_fade: &mut t.world.g4a58,
                gates: data.gates(),
            };
            move_script::run(&mut t.fighters, k, t.target[k], &mut ctx);
        });
    }

    /// Runs `f` with fighter `k`'s `PlayerExt` in its slot of the `World`: its palette
    /// fade record (and with `hud` its HUD flash bytes) copied into the slot before, back into `ext[k]` after. The
    /// fighter steps swap both; the round-start move swaps the fade record alone.
    fn in_slot<R>(&mut self, k: usize, hud: bool, f: impl FnOnce(&mut Self) -> R) -> R {
        let slot = Fid::of(k).slot();
        self.world.p4b18[slot] = self.ext[k].p4b18;
        if hud {
            self.world.hud_flash[slot] = self.ext[k].hud_flash;
        }
        let r = f(self);
        self.ext[k].p4b18 = self.world.p4b18[slot];
        if hud {
            self.ext[k].hud_flash = self.world.hud_flash[slot];
        }
        r
    }
}

/// The camera's largest distance in the team battle (four fighters at the far corners of the ±2304 ring).
pub const ZOOM_MAX: i16 = 0x1800;
