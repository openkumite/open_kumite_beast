//! A match's imported fighters (engine addition, the import S7): their part of the frame, their round starts,
//! the throws across the engines, the state checks.
//!
//! State: none of its own: steps the serialised `Match` (its `br1`)
//!
//! When both fighters are the imported game's, the imported game's whole fight-frame logic (the inputs and throws, each
//! fighter's decision and script, the pair's movement, hits, push-apart, walls and angles) runs in place of the
//! main game's input .. opponent-angle steps, with the main game's health and gauge rules (`bridge::health`), on the stage's
//! ring and walls; everything around it (the pause, the round flow, the camera, the effects, the sound) is the
//! main game's, reading the faces. A mixed pair runs each fighter by its own engine (`br1_mixed_steps`, `hits_mixed`);
//! a throw across the engines puppets the victim from a proxy of the thrower's game (`cross_*`).

use super::Match;
use crate::fight::br1::ai::Table;
use crate::fight::br1::bridge::{self, Br1Match};
use crate::fight::br1::decision::Opts;
use crate::fight::br1::frame::{self, World};

use crate::fight::br1::physics::Ring;
use crate::fight::br1::script::{self, Ctx, Request};
use crate::fight::br1::{hit, init, physics};
use crate::fight::duel;
use crate::fight::duel::br1::{G98, br1_stage_byte, think};
use crate::fight::fight_data::Loaded;

/// Engine addition (the import S10, a decision of the import): the main game's PRACTICE settings (M.D4: COM. ACTION in bits
/// 0..2, COM. GUARD bit 3, COM. GUARD AND ESCAPE bit 4, DEFENSIVE FALL bit 5; `fight::pause`) as the imported game's AI init
/// mode of its practice dummies (the AI init's mode table, states 10..21): STANDING no input (1, state 9), with a
/// guard the guarding dummy (4, state 10: turned to the opponent, a low guard against low attacks), with DEFENSIVE FALL
/// the recovering one (15, state 21: rolls up, recovers in the air); BLOCKS AFTER TAKING DAMAGE 7 (state 13); CROUCHING
/// 13 (state 19), with a guard 14 (state 20, crouching on with the guard); JUMPING 6 (state 12); DODGING 8 (state 14:
/// out of guard stun, side steps, jumps over slow lows). Imported has no separate GUARD AND ESCAPE: it counts as a
/// guard.
fn practice_mode(d4: u8) -> i32 {
    let guard = d4 & 0x18 != 0;
    match d4 & 7 {
        0 if guard => 4,
        0 if d4 & 0x20 != 0 => 15,
        0 => 1,
        1 => 7,
        2 if guard => 14,
        2 => 13,
        3 => 6,
        4 => 8,
        _ => 1,
    }
}

impl Match {
    /// Whether side `i` is an imported fighter.
    pub fn is_br1(&self, i: usize) -> bool {
        self.br1.as_ref().is_some_and(|b| b.side[i])
    }

    /// The match end's key cut for an imported loser (engine addition, the import S9, simulation v19): the match end's win
    /// reset (`Round::win_tail`, round state 0xD / 0xE at cnt 2: the frame whose round step leaves cnt at 3) ends the
    /// loser's current key on the next frame. Main-game cuts its script state's +0xC2 / +0xC8 / +0xCA by F.88; a
    /// imported core's are S.CA total, S.D0 key duration, S.D2 time left and F.80 its step (`br1::pose::Anim`):
    /// total += step − left, duration = left = step. The face's fields, which main-game cuts, are not the core's.
    pub(super) fn br1_key_cut(&mut self, data: &Loaded) {
        let r = &self.round;
        if !data.gates().br1_key_cut || !r.win_reset || !matches!(r.state, 0xD | 0xE) || r.cnt != 3 {
            return;
        }
        let l = if r.state == 0xD { 1 } else { 0 };
        let Some(b) = self.br1.as_mut().filter(|b| b.side[l]) else { return };
        let (a, step) = (&mut b.fs[l].anim, b.fs[l].f.step);
        a.total = a.total.wrapping_add(step.wrapping_sub(a.left));
        (a.d0, a.left) = (step, step);
    }

    /// Whether side `i`'s body in the push-apart and the walls is an imported core's (not while a main-game
    /// throw's proxy stands in its slot).
    pub(super) fn br1_body(&self, i: usize) -> bool {
        self.br1.as_ref().is_some_and(|b| b.side[i] && b.swapped != Some(i as u8))
    }

    /// The match's imported state for `data` (None without imported fighters): each imported side's
    /// block as the loader leaves it (`init::new`), the stream G seeded with 1 (the main game's `Match::rand` start).
    pub(super) fn br1_new(data: &Loaded) -> Option<Box<Br1Match>> {
        if data.br1.iter().all(Option::is_none) {
            return None;
        }
        let mut b = Br1Match { side: [0, 1].map(|i| data.br1[i].is_some()), rand: 1, ..Default::default() };
        for i in 0..2 {
            if let Some(r) = data.br1_res(i) {
                let chr = crate::formats::br1_anm::CHARACTERS.iter().position(|c| *c == r.code).unwrap_or(0) as u8;
                b.fs[i] = init::new(r, i as u8, chr, 0);
                b.fs[i].f.f4c = 1;
            }
        }
        // the AI's random table as the boot leaves it (simulation v20)
        if let Some(r) = data.br1_res(0).or(data.br1_res(1)).filter(|_| data.gates().br1_ai) {
            b.ai_rnd = Table::boot(&r.tables);
        }
        Some(Box::new(b))
    }

    /// The stage's ring as the imported game's records {hips, edge}: an imported stage's own two (`Loaded::br1_stage`,
    /// simulation v16), else the main-game stage's box for both (the main game has one table).
    fn br1_rings(&self, data: &Loaded) -> [Ring; 2] {
        duel::br1::rings(data, &self.world.stage.ring)
    }

    /// After `fresh_fighters(data, mode, …)`: the imported sides reset as imported starts a round
    /// (the main game's mode 0 / 2 / 3 → 0 / 2 / 3, the match-end reset 4 → 1), their state
    /// 0x28 started and run once (mode 4: the move kept), and their faces: the proxy's `chr`, the core published, the
    /// health by the main game's rules (`HP` × the ATTACK LEVEL %; mode 3 at its end; mode 4 kept).
    pub(super) fn br1_fresh(&mut self, data: &Loaded, mode: u8) {
        // before the resets, the AI's table shuffled by one draw (stream A there, the
        // fighters' stream G here: a user decision; simulation v20)
        if let (Some(b), Some(r)) = (self.br1.as_deref_mut(), data.br1_res(0).or(data.br1_res(1)))
            && data.gates().br1_ai
        {
            let seed = r.tables.table("math.random_bytes").u8(b.rand as usize);
            b.rand = b.rand.wrapping_add(1);
            b.ai_rnd.shuffle(&r.tables, seed as u32);
        }
        for i in 0..2 {
            if self.is_br1(i) {
                self.br1_fresh_one(data, i, mode);
            }
        }
    }

    /// The AI init as the round reset calls it (simulation v20): side `i`'s AI block for the round, its mode
    /// from F.01 as the main game's round start set it (a CPU's 0x81: mode 0, the normal AI; a human's 0: mode 0, not
    /// run), its level the match's CPU level (G.9C's place). PRACTICE's dummy (F.01 (M.D4 | 0x40) + 1)
    /// takes the imported game's practice dummy for the PRACTICE settings (`practice_mode`).
    pub fn br1_ai_init(&mut self, data: &Loaded, i: usize) {
        let (Some(b), Some(r)) = (self.br1.as_deref_mut(), data.br1_res(i)) else { return };
        if !data.gates().br1_ai {
            return;
        }
        let c = self.fighters[i].o_01;
        let mode = match c {
            0 => 0,
            _ if self.practice => practice_mode(self.practice_d4),
            _ => (c & 0x7F) as i32 - 1,
        };
        b.ai[i].init(&r.anm.bytes, b.fs[i].f.player, self.cpu_level, mode);
    }

    /// `br1_fresh` for side `i`.
    pub(super) fn br1_fresh_one(&mut self, data: &Loaded, i: usize, mode: u8) {
        let rings = self.br1_rings(data);
        let (Some(b), Some(res)) = (self.br1.as_deref_mut(), data.br1_res(i)) else { return };
        let opp = data.br1_res(1 - i).unwrap_or(res);
        let res2 = if i == 0 { [res, opp] } else { [opp, res] };
        // a throw across the engines does not outlast a round
        b.throw = None;
        let face = &mut self.fighters[i];
        let f = &mut b.fs[i];
        f.f.cpu = face.o_01;
        f.f.fda = self.round.lost[i];
        if mode != 4 {
            f.f.f4c = face.flags[crate::fight::fighter::F54];
            f.s.state_req = 0x28;
        }
        init::reset_input(f);
        let mut out = Vec::new();
        let mut ctx = Ctx {
            res: res2,
            rand: &mut b.rand,
            stage: br1_stage_byte(data),
            round: self.round.index,
            walls: [0, 1, 2, 3].map(|k| self.world.stage.walls.present(k)),
            need: self.round.need,
            g_ad: 0,
            g_e8: 0,
            out: &mut out,
            trace: None,
            rings: Some(rings),
        };
        let setup = init::Setup { fast_regen: false, practice: self.practice, vitality: init::VITALITY };
        let m1 = match mode {
            4 => 1,
            m => m,
        };
        init::reset(&mut b.fs, i, m1, &setup, &mut ctx);
        let f = &mut b.fs[i];
        let face = &mut self.fighters[i];
        face.chr = bridge::br2_proxy(f.f.chr);
        bridge::publish(f, face);
        if mode != 4 {
            face.o_b0 = (bridge::HP * self.hp_pct / 100) as i16;
            (face.o_ac, face.o_ae, face.o_aa) = if mode == 3 { (face.o_b0, face.o_b0, face.o_b0) } else { (0, 0, 0) };
            (face.o_c8, face.o_d8, face.o_ca) = (0, 0, 0);
        } else {
            // the value load in mode 4 returns before the ATTACK LEVEL scaling
            face.o_b0 = bridge::HP as i16;
        }
        bridge::write_health(f, face, &mut b.synced[i]);
    }

    /// The imported part of `frame_step` (both fighters the imported game's): from the input to the opponent angles.
    pub(super) fn br1_steps(&mut self, data: &Loaded) {
        let rings = self.br1_rings(data);
        let Some(b) = self.br1.as_deref_mut() else { return };
        let (Some(r0), Some(r1)) = (data.br1_res(0), data.br1_res(1)) else { return };
        let res = [r0, r1];
        // the faces' writes since the last frame (the round flow, the cheats), the stage's walls
        for i in 0..2 {
            bridge::absorb(&mut b.fs[i], &self.fighters[i]);
        }
        let walls0 = self.br1_walls_in();
        let Some(b) = self.br1.as_deref_mut() else { return };
        let w = World {
            pads: self.pads,
            cam: self.cam.ang[1],
            // the AI runs inside the inputs (`frame::inputs_ai`)
            ai: [0; 2],
            stage: br1_stage_byte(data),
            opts: Opts { g98: G98, need: self.round.need, opt38: 0, rave: false },
            opt2c: 0,
            frozen: false,
            replay: [self.fighters[0].pad_override, self.fighters[1].pad_override],
        };
        let mut out = Vec::new();
        let mut ctx = duel::br1::ctx(res, &mut b.rand, data, (self.round.index, self.round.need), &b.walls, &mut out, Some(rings));
        let (on, timer) = (data.gates().br1_ai, self.round.timer);
        let (ai, rnd) = (&mut b.ai, &b.ai_rnd);
        let rec = frame::inputs_ai(&mut b.fs, &mut ctx, &w, &mut |fs, k| {
            let o = &fs[1 - k];
            think(on, &mut ai[k], rnd, &fs[k], &o.f, o.hips_y(), res[k], timer)
        });
        for me in 0..2 {
            let ko = bridge::health(&mut b.fs[me], me, &mut self.fighters[me], &mut b.synced[me], self.custom.charge as i16, self.custom.recover, ctx.out);
            frame::decide(&mut b.fs, me, &mut ctx, &w, &mut b.walls);
            Self::br1_forced(&mut self.force_move, &mut b.fs[me], me);
            ctx.walls = b.walls.hp.map(|h| h >= 0);
            script::step(&mut b.fs, me, &mut ctx);
            frame::after_script(&mut b.fs, me, &mut ctx, ko);
        }
        frame::pair_move(&mut b.fs, res);
        hit::hits(&mut b.fs, &mut ctx, G98);
        physics::after_hits(&mut b.fs, res, rings[0], &mut b.walls);
        for i in 0..2 {
            bridge::publish(&b.fs[i], &mut self.fighters[i]);
            self.fighters[i].pad_record = rec[i];
        }
        self.br1_walls_out(walls0);
        self.br1_requests(data, out);
    }

    /// The match context of the imported game's code in a mixed pair (the pads, the camera, the stage, the options; the
    /// data, `Ctx::res`, is the caller's: the imported side's in both entries, or with a throw's proxy's).
    fn br1_world(&self, data: &Loaded) -> World {
        World {
            pads: self.pads,
            cam: self.cam.ang[1],
            ai: [0; 2],
            stage: br1_stage_byte(data),
            opts: Opts { g98: G98, need: self.round.need, opt38: 0, rave: false },
            opt2c: 0,
            frozen: false,
            replay: [self.fighters[0].pad_override, self.fighters[1].pad_override],
        }
    }

    /// The fighters' part of `frame_step` for a mixed pair (engine addition, the import S7): the frame order both
    /// games share, each fighter run by its own engine against the other's face (a main-game fighter's
    /// imported-game view: `bridge::shadow`): the inputs, the throws (none across the engines yet: each side's throw test
    /// finds the other unthrowable), each fighter's decision and script, the moves, the skeletons, the effect
    /// commands, the hits across the engines (`hits_mixed`), the main game's push-apart with both fighters' spheres,
    /// each fighter's walls, the stage, the angles.
    pub(super) fn br1_mixed_steps(&mut self, data: &Loaded, freeze: bool) {
        let rings = self.br1_rings(data);
        let k = if self.is_br1(0) { 0 } else { 1 };
        let Some(res) = data.br1_res(k) else { return };
        let w = self.br1_world(data);
        let walls0 = self.br1_walls_in();
        if self.br1.as_ref().is_some_and(|b| b.throw.is_some()) {
            self.cross_steps(data, freeze, k, walls0);
            return;
        }
        // the faces' writes since the last frame
        if let Some(b) = self.br1.as_deref_mut() {
            bridge::absorb(&mut b.fs[k], &self.fighters[k]);
        }
        let mut out = Vec::new();
        // inputs, P1 then P2
        for i in 0..2 {
            if i == k {
                let b = self.br1.as_deref_mut().expect("the imported side");
                b.fs[1 - k] = bridge::shadow(&self.fighters[1 - k]);
                // the AI against the main-game fighter's view, its hips (model +0x174) its own
                let ai = think(data.gates().br1_ai, &mut b.ai[k], &b.ai_rnd, &b.fs[k], &b.fs[1 - k].f, self.fighters[1 - k].hips[1], res, self.round.timer);
                let pad = crate::fight::br1::input::Pad { raw: w.pads[i], cam: w.cam, ai, replay: w.replay[i] };
                self.fighters[k].pad_record = crate::fight::br1::input::process(&mut b.fs, k, &res.anm, pad);
                bridge::publish(&b.fs[k], &mut self.fighters[k]);
            } else {
                self.input_one(data, freeze, i);
            }
        }
        // throws: each engine's resolution, the other fighter its face / its view; a grab
        // across the engines starts a `CrossThrow` (imported throws unescapable: no escape across the engines)
        {
            let f6c = self.fighters[k].flags[crate::fight::fighter::F6C];
            crate::fight::throw::resolve(&mut self.fighters);
            let br2_grab = self.fighters[k].flags[crate::fight::fighter::F6C] & !f6c & 0x80_0000 != 0;
            if br2_grab {
                // The main game's resolution runs first (P1's, then P2's): its grab wins over an imported attempt
                // of the same frame, which is not resolved
                self.cross_start_br2(data, k);
            }
        }
        if self.br1.as_ref().is_some_and(|b| b.throw.is_none()) {
            let b = self.br1.as_deref_mut().expect("the imported side");
            // what the main game's resolution wrote on the face (its whiff's flip)
            bridge::absorb(&mut b.fs[k], &self.fighters[k]);
            b.fs[1 - k] = bridge::shadow(&self.fighters[1 - k]);
            let f64_before = b.fs[1 - k].f.f64;
            let mut r = b.rand;
            let mut ctx = duel::br1::ctx([res, res], &mut r, data, (self.round.index, self.round.need), &b.walls, &mut out, Some(rings));
            crate::fight::br1::throw::resolve(&mut b.fs, &mut ctx);
            b.rand = r;
            let br1_grab = b.fs[1 - k].f.f64 & !f64_before & 0x80_0000 != 0;
            bridge::publish(&b.fs[k], &mut self.fighters[k]);
            if br1_grab {
                self.cross_start_br1(data, k);
            }
        }
        if self.br1.as_ref().is_some_and(|b| b.throw.is_some()) {
            self.cross_rest(data, k, walls0, out);
            return;
        }
        // each fighter's frame, P1 then P2
        let mut effect_reqs = Vec::new();
        for i in 0..2 {
            if i != k {
                self.fighter_one(data, i, &mut effect_reqs);
                continue;
            }
            let b = self.br1.as_deref_mut().expect("the imported side");
            b.fs[1 - k] = bridge::shadow(&self.fighters[1 - k]);
            let ko = bridge::health(&mut b.fs[k], k, &mut self.fighters[k], &mut b.synced[k], self.custom.charge as i16, self.custom.recover, &mut out);
            let mut r = b.rand;
            let mut ctx = duel::br1::ctx([res, res], &mut r, data, (self.round.index, self.round.need), &b.walls, &mut out, Some(rings));
            frame::decide(&mut b.fs, k, &mut ctx, &w, &mut b.walls);
            Self::br1_forced(&mut self.force_move, &mut b.fs[k], k);
            ctx.walls = b.walls.hp.map(|h| h >= 0);
            script::step(&mut b.fs, k, &mut ctx);
            frame::after_script(&mut b.fs, k, &mut ctx, ko);
            b.rand = r;
            bridge::publish(&b.fs[k], &mut self.fighters[k]);
        }
        // the moves (no attachment across the engines), the landings
        {
            let o = 1 - k;
            crate::fight::physics::integrate(&mut self.fighters[o], &data.trig);
            crate::fight::physics::landing(&mut self.fighters[o]);
            let b = self.br1.as_deref_mut().expect("the imported side");
            physics::integrate_one(&mut b.fs[k], res);
            physics::landing(&mut b.fs[k]);
            if b.fs[k].f.f50 & 0x1000_0000 != 0 {
                b.fs[k].f.f54 |= 0x4000_0000;
            }
            bridge::publish(&b.fs[k], &mut self.fighters[k]);
        }
        let nodes = self.skeleton_step(data);
        crate::fight::fx::script_link::refresh(&mut self.fx_host(), data);
        for (i, e) in effect_reqs {
            crate::fight::fx::script_link::effect_op(&mut self.fx_host(), data, i, e);
        }
        self.hits_mixed(data, &nodes, k, &mut out);
        self.combo_flags();
        if let Some(b) = self.br1.as_deref_mut() {
            bridge::absorb(&mut b.fs[k], &self.fighters[k]);
        }
        let th = self.push_apart_step(data, &nodes);
        self.stage_step(data);
        crate::fight::physics::opponent_angles(&mut self.fighters, &data.trig, th);
        for sw in self.fighters[1 - k].swing.iter_mut() {
            sw.step();
        }
        let b = self.br1.as_deref_mut().expect("the imported side");
        bridge::absorb(&mut b.fs[k], &self.fighters[k]);
        bridge::publish(&b.fs[k], &mut self.fighters[k]);
        self.br1_walls_out(walls0);
        self.br1_requests(data, out);
    }

    /// The side-ordered data of imported pair (`k`'s, the other's).
    fn res2<'a>(k: usize, a: &'a crate::fight::br1::res::Br1Res, b: &'a crate::fight::br1::res::Br1Res) -> [&'a crate::fight::br1::res::Br1Res; 2] {
        if k == 0 { [a, b] } else { [b, a] }
    }

    /// An imported throw by side `k` grabbed the main-game fighter's view (its F.64 0x800000, S.34): the
    /// victim's imported proxy (`Loaded::br1_ghost`) reset as a round starts it in its slot, then
    /// placed and posed as the victim, the grab's request taken over; the victim puppeted from here on.
    fn cross_start_br1(&mut self, data: &Loaded, k: usize) {
        let o = 1 - k;
        let rings = self.br1_rings(data);
        let (Some(res), Some(gres)) = (data.br1_res(k), data.br1_ghost[o].as_deref()) else { return };
        let face = &self.fighters[o];
        let Some(b) = self.br1.as_deref_mut() else { return };
        let thrown_by = b.fs[o].s.thrown_by;
        b.fs[o] = init::new(gres, o as u8, bridge::br1_proxy(face.chr), face.o_01);
        // the proxy's reset is unseen: on a copy of the stream G (the match's not advanced) and its sounds dropped
        let mut r = b.rand;
        let mut dropped = Vec::new();
        let mut ctx = duel::br1::ctx(Self::res2(k, res, gres), &mut r, data, (self.round.index, self.round.need), &b.walls, &mut dropped, Some(rings));
        let setup = init::Setup { fast_regen: false, practice: false, vitality: init::VITALITY };
        init::reset(&mut b.fs, o, 0, &setup, &mut ctx);
        duel::br1::pose_br1_ghost(&mut b.fs[o], gres, face, thrown_by);
        let g = std::mem::take(&mut b.fs[o]);
        b.throw = Some(bridge::CrossThrow { victim: o as u8, ghost: bridge::Ghost::Br1(Box::new(g)), frames: 0 });
    }

    /// A main-game throw grabbed imported side `k`'s face (F6C 0x800000, `s_2c`): the victim's
    /// main-game proxy (its script file in `Loaded::fighters[k]`) made from the face (`bridge::br2_ghost`); the victim
    /// puppeted from here on.
    fn cross_start_br2(&mut self, data: &Loaded, k: usize) {
        let Some(b) = self.br1.as_deref_mut() else { return };
        let g = duel::br1::br2_ghost(&mut b.fs[k], &mut self.fighters[k], &data.fighters[k].ani);
        b.throw = Some(bridge::CrossThrow { victim: k as u8, ghost: bridge::Ghost::Br2(Box::new(g)), frames: 0 });
    }

    /// A frame of a mixed pair during a `CrossThrow`, from its inputs (`k` the imported side; `walls0` the walls
    /// as they came in): the thrower's input (the puppeted victim and the proxy take none), then `cross_rest`.
    fn cross_steps(&mut self, data: &Loaded, freeze: bool, k: usize, walls0: crate::fight::br1::physics::Walls) {
        let Some(res) = data.br1_res(k) else { return };
        let w = self.br1_world(data);
        let victim = self.br1.as_ref().and_then(|b| b.throw.as_ref()).map_or(k, |t| t.victim as usize);
        if let Some(b) = self.br1.as_deref_mut() {
            bridge::absorb(&mut b.fs[k], &self.fighters[k]);
        }
        if victim == k {
            self.input_one(data, freeze, 1 - k);
        } else {
            let b = self.br1.as_deref_mut().expect("the imported side");
            let Some(bridge::CrossThrow { ghost: bridge::Ghost::Br1(g), .. }) = b.throw.as_mut() else { return };
            std::mem::swap(&mut b.fs[1 - k], &mut **g);
            // an imported thrower's AI sees the proxy in the victim's slot (the puppeted victim takes no input, its
            // AI does not run)
            let ai = think(data.gates().br1_ai, &mut b.ai[k], &b.ai_rnd, &b.fs[k], &b.fs[1 - k].f, b.fs[1 - k].hips_y(), res, self.round.timer);
            let pad = crate::fight::br1::input::Pad { raw: w.pads[k], cam: w.cam, ai, replay: w.replay[k] };
            self.fighters[k].pad_record = crate::fight::br1::input::process(&mut b.fs, k, &res.anm, pad);
            let Some(bridge::CrossThrow { ghost: bridge::Ghost::Br1(g), .. }) = b.throw.as_mut() else { return };
            std::mem::swap(&mut b.fs[1 - k], &mut **g);
            bridge::publish(&b.fs[k], &mut self.fighters[k]);
        }
        self.cross_rest(data, k, walls0, Vec::new());
    }

    /// A `CrossThrow`'s frame from the fighters' frames on: the thrower's engine runs the pair of the thrower and the
    /// proxy (in the victim's slot), the victim is posed and placed from the proxy, its health by its own rules
    /// with the proxy's damage; no hits; at the end the victim goes on in the proxy's state.
    fn cross_rest(&mut self, data: &Loaded, k: usize, walls0: crate::fight::br1::physics::Walls, out: Vec<(u8, Request)>) {
        let br1_thrower = matches!(self.br1.as_ref().and_then(|b| b.throw.as_ref()), Some(bridge::CrossThrow { ghost: bridge::Ghost::Br1(_), .. }));
        let out = if br1_thrower { self.cross_br1_thrower(data, k, out) } else { self.cross_br2_thrower(data, k, out) };
        self.br1_walls_out(walls0);
        self.br1_requests(data, out);
    }

    /// `cross_rest` of an imported thrower (side `k`): the imported game's pair (the thrower, the proxy victim)
    /// through its fighters' frames, the move and `after_hits`; the main-game victim's health by its rules, the
    /// proxy's queued damage (× 200 / 187) and gauge change onto it, its pose and position the proxy's.
    fn cross_br1_thrower(&mut self, data: &Loaded, k: usize, mut out: Vec<(u8, Request)>) -> Vec<(u8, Request)> {
        let o = 1 - k;
        let rings = self.br1_rings(data);
        let w = self.br1_world(data);
        let (Some(res), Some(gres)) = (data.br1_res(k), data.br1_ghost[o].as_deref()) else { return out };
        let res2 = Self::res2(k, res, gres);
        // the victim's health (the fighter step's start)
        let rate = duel::recover_rate(self.custom.recover, &data.fighters[o].ani);
        let b = self.br1.as_deref_mut().expect("the imported side");
        let Some(bridge::CrossThrow { ghost: bridge::Ghost::Br1(g), .. }) = b.throw.as_mut() else { return out };
        std::mem::swap(&mut b.fs[o], &mut **g);
        for i in 0..2 {
            let mut rr = b.rand;
            let mut ctx = duel::br1::ctx(res2, &mut rr, data, (self.round.index, self.round.need), &b.walls, &mut out, Some(rings));
            let ko = if i == k {
                bridge::health(&mut b.fs[k], k, &mut self.fighters[k], &mut b.synced[k], self.custom.charge as i16, self.custom.recover, ctx.out)
            } else {
                // what the proxy queued last frame (its script, the strikes) applied this frame, as either engine does
                duel::br1::proxy_damage(&mut b.fs[o], &mut self.fighters[o]);
                crate::fight::hit::health(&mut self.fighters[o], self.custom.charge as i16, rate);
                false
            };
            frame::decide(&mut b.fs, i, &mut ctx, &w, &mut b.walls);
            if i == k {
                Self::br1_forced(&mut self.force_move, &mut b.fs[k], k);
            }
            ctx.walls = b.walls.hp.map(|h| h >= 0);
            script::step(&mut b.fs, i, &mut ctx);
            frame::after_script(&mut b.fs, i, &mut ctx, ko);
            b.rand = rr;
        }
        frame::pair_move(&mut b.fs, res2);
        // a throw's strikes (the pair's hits, the proxy's hurt spheres its own)
        let mut rr = b.rand;
        let mut ctx = duel::br1::ctx(res2, &mut rr, data, (self.round.index, self.round.need), &b.walls, &mut out, Some(rings));
        hit::hits(&mut b.fs, &mut ctx, G98);
        b.rand = rr;
        physics::after_hits(&mut b.fs, res2, rings[0], &mut b.walls);
        bridge::pose_face(&mut self.fighters[o], &b.fs[o]);
        bridge::publish(&b.fs[k], &mut self.fighters[k]);
        let state = b.fs[o].s.state_cur;
        let down = b.fs[o].f.down_timer;
        let Some(bridge::CrossThrow { ghost: bridge::Ghost::Br1(g), .. }) = b.throw.as_mut() else { return out };
        std::mem::swap(&mut b.fs[o], &mut **g);
        self.skeleton_step(data);
        crate::fight::fx::script_link::refresh(&mut self.fx_host(), data);
        self.stage_step(data);
        let next = self.br1.as_deref_mut().and_then(|b| b.throw.as_mut()).and_then(|t| t.handover(state));
        if let Some(state) = next {
            // the victim goes on in the proxy's state (0x67 lying) in its own engine: the state started and its script
            // run once (as a round start does), before its own decision sees it
            if let Some(b) = self.br1.as_deref_mut()
                && let Some(bridge::CrossThrow { ghost: bridge::Ghost::Br1(g), .. }) = b.throw.as_mut()
            {
                // the proxy's damage still queued (this frame's strikes) goes with the victim
                duel::br1::proxy_damage(g, &mut self.fighters[o]);
            }
            self.fighters[o].o_c0 = down;
            duel::release(&mut self.fighters[o], &data.fighters[o].ani, state);
            self.run_start_move(data, o);
            if let Some(b) = self.br1.as_deref_mut() {
                b.throw = None;
            }
        }
        out
    }

    /// A main-game throw's proxy (in the victim's slot `k`): its queued damage and gauge change moved onto the
    /// victim's face (the proxy is never KO'd), its last damage kept as `hit::health` keeps it (op 19 subs 0 / 1
    /// take 3/10, 1/10 of it).
    fn cross_take_damage(&mut self, k: usize) {
        let Some(bridge::CrossThrow { ghost: bridge::Ghost::Br2(face), .. }) = self.br1.as_deref_mut().and_then(|b| b.throw.as_mut()) else { return };
        duel::br1::br2_proxy_damage(&mut self.fighters[k], face);
    }

    /// `cross_rest` of a main-game thrower (imported side `k` the victim): the main game's pair (the
    /// thrower, the proxy victim in the victim's slot) through its fighters' frames and the move; the imported
    /// victim's health by the main game's rules with the proxy's damage and gauge change, posed and placed from the
    /// proxy (`physics::puppet`).
    fn cross_br2_thrower(&mut self, data: &Loaded, k: usize, out: Vec<(u8, Request)>) -> Vec<(u8, Request)> {
        let rings = self.br1_rings(data);
        let Some(res) = data.br1_res(k) else { return out };
        let Some(b) = self.br1.as_deref_mut() else { return out };
        let Some(bridge::CrossThrow { ghost: bridge::Ghost::Br2(g), .. }) = b.throw.as_mut() else { return out };
        // the proxy sees the victim's skeleton (the thrower's scripts read it)
        duel::br1::proxy_sees(g, &self.fighters[k]);
        std::mem::swap(&mut self.fighters[k], &mut **g);
        b.swapped = Some(k as u8);
        let mut effect_reqs = Vec::new();
        for i in 0..2 {
            if i == k {
                // what the thrower's script queued on the proxy (a thrower-side damage op)
                self.cross_take_damage(k);
            }
            self.fighter_one(data, i, &mut effect_reqs);
            if i == k {
                self.cross_take_damage(k);
            }
        }
        // a thrower after the proxy (P2 throwing P1) queued its damage op on it this frame
        self.cross_take_damage(k);
        crate::fight::physics::integrate_pair(&mut self.fighters, &data.trig);
        for i in 0..2 {
            crate::fight::physics::landing(&mut self.fighters[i]);
        }
        // the skeletons (the proxy's: the victim's nodes), the effect commands, a throw's strikes (the proxy's hurt
        // spheres at the victim's nodes), the combo flags
        let nodes = self.skeleton_step(data);
        crate::fight::fx::script_link::refresh(&mut self.fx_host(), data);
        for (i, e) in effect_reqs {
            crate::fight::fx::script_link::effect_op(&mut self.fx_host(), data, i, e);
        }
        self.hits(data, &nodes);
        self.combo_flags();
        // The main game's push-apart and walls for the pair (the proxy's body spheres), the stage, the angles
        let th = self.push_apart_step(data, &nodes);
        self.stage_step(data);
        crate::fight::physics::opponent_angles(&mut self.fighters, &data.trig, th);
        let b = self.br1.as_deref_mut().expect("the imported side");
        b.swapped = None;
        let Some(bridge::CrossThrow { ghost: bridge::Ghost::Br2(g), .. }) = b.throw.as_mut() else { return out };
        std::mem::swap(&mut self.fighters[k], &mut **g);
        let g = (**g).clone();
        let mut out = out;
        bridge::health(&mut b.fs[k], k, &mut self.fighters[k], &mut b.synced[k], self.custom.charge as i16, self.custom.recover, &mut out);
        duel::br1::puppet_from(&mut b.fs[k], res, &g);
        bridge::publish(&b.fs[k], &mut self.fighters[k]);
        let next = b.throw.as_mut().and_then(|t| t.handover(g.move_cur));
        if let Some(state) = next {
            // the victim goes on in the proxy's state (0x67 lying) in its own engine: the state started and its script
            // run once (as the round-start reset does), before its own decision sees it
            b.fs[k].f.down_timer = g.o_c0;
            duel::br1::release(&mut b.fs[k], res, state);
            b.fs[1 - k] = bridge::shadow(&self.fighters[1 - k]);
            let mut r = b.rand;
            let mut ctx = duel::br1::ctx([res, res], &mut r, data, (self.round.index, self.round.need), &b.walls, &mut out, Some(rings));
            script::run(&mut b.fs, k, &mut ctx);
            b.rand = r;
            b.throw = None;
        }
        bridge::publish(&b.fs[k], &mut self.fighters[k]);
        out
    }

    /// The stage's walls into the imported walls' state (absent −1, indestructible 0x7FFF; the hit strength and
    /// its lock); returns them as they came in (`br1_walls_out`).
    fn br1_walls_in(&mut self) -> crate::fight::br1::physics::Walls {
        let Some(b) = self.br1.as_deref_mut() else { return Default::default() };
        duel::br1::walls_in(&self.world.stage.walls, &mut b.walls)
    }

    /// What the imported game's code changed of the walls since `br1_walls_in` (`before`) back into the stage (its
    /// update breaks and shakes them).
    fn br1_walls_out(&mut self, before: crate::fight::br1::physics::Walls) {
        let Some(b) = self.br1.as_deref() else { return };
        duel::br1::walls_out(&b.walls, before, &mut self.world.stage.walls);
    }

    /// The requests of the imported game's code this frame (`duel::br1::requests`): the sounds queued, the stage sound on
    /// an imported stage, the effects, camera words and presentation calls logged for the drawing.
    fn br1_requests(&mut self, data: &Loaded, out: Vec<(u8, Request)>) {
        let frame = self.frame;
        let Some(b) = self.br1.as_deref_mut() else { return };
        duel::br1::requests(&mut b.host(), data, out, frame, &self.world.stage.br1);
    }

    /// The hits of a mixed pair: each attacker tested by its own engine
    /// (the main game's against the imported fighter's hurt spheres and nodes; the imported game's against the
    /// main-game fighter's view, its ANI's hurt spheres at its nodes), the weaker hit of a trade dropped (both
    /// in the main game's damage units), then P1's and P2's hit applied by the attacker's engine: the main game's
    /// `apply` on the imported face with the imported game's guard gate, its result into the core
    /// (`bridge::take_hit`); the imported game's `apply` on the view, its result onto the face (`bridge::give_hit`).
    fn hits_mixed(&mut self, data: &Loaded, nodes: &[Vec<[i32; 3]>], k: usize, out: &mut Vec<(u8, Request)>) {
        let res = data.br1_res(k).expect("the imported side");
        let o = 1 - k;
        // (kind, dmg in the main game's units, pos, the main game: mult / the imported game: (dir, mult, pos16))
        let mut cand: [Option<(i32, i32, [i32; 3], i32, u32, [i16; 3])>; 2] = [None, None];
        for a in 0..2 {
            let d = 1 - a;
            if a == o {
                let cur = self.br1.as_deref().expect("the imported side").fs[k].cur as usize & 1;
                if let Some(p) = duel::br1::probe_on_core(&mut self.fighters, a, d, nodes, &res.anm.spheres[2 + cur]) {
                    cand[a] = Some((p.kind, p.dmg, p.pos, p.mult, 0, [0; 3]));
                }
            } else {
                let hurt = data.fighters[o].ani.spheres(2 + self.fighters[o].beast() as usize);
                let b = self.br1.as_deref_mut().expect("the imported side");
                b.fs[o] = bridge::shadow(&self.fighters[o]);
                // F.9: the view's hit sphere count (its ANI's table)
                b.fs[o].f.hit_spheres = hurt.len() as u8;
                let probe = duel::br1::probe_core(&mut b.fs, a, &hurt);
                // the attacker's F.54 0x14 (can_hit clears it whatever its result) on its face, so that the absorb
                // after the combo flags keeps it; F.54 only (a whole publish would drop a main-game attacker's
                // direction and counter bits just written on the face's F68)
                self.fighters[k].flags[crate::fight::fighter::F5C] = b.fs[k].f.f54;
                if let Some((p, dir, pos)) = probe {
                    cand[a] = Some((p.kind, p.dmg, [0; 3], p.mult, dir, pos));
                }
            }
        }
        if let [Some(x), Some(y)] = &mut cand
            && x.0 != 0
            && y.0 != 0
        {
            if x.1 < y.1 {
                x.0 = 0;
            } else if y.1 < x.1 {
                y.0 = 0;
            }
        }
        // a main-game hit on the imported face still to come after the imported game's (the imported side P1)
        let br2_next = k < o && cand[o].is_some_and(|c| c.0 != 0);
        let mut guard_fx = Vec::new();
        for (a, c) in cand.into_iter().enumerate() {
            let Some((kind, _, pos, mult, dir, pos16)) = c else { continue };
            if kind == 0 {
                continue;
            }
            if a == o {
                // The main game's hit on the imported face, the imported game's guard gate (F.54 0x2000)
                let b = self.br1.as_deref_mut().expect("the imported side");
                let r = duel::br1::land_on_face(&mut self.fighters, a, k, b.fs[k].f.f54, mult, kind, data);
                bridge::take_hit(&mut b.fs[k], &self.fighters[k]);
                // the core's masked result word on the face (the absorb after the combo flags would undo the mask)
                bridge::publish(&b.fs[k], &mut self.fighters[k]);
                self.hit_fx(data, a, 1 - a, &r, pos, &mut guard_fx);
                self.events.push(format!("frame {}: P{} {} P{} for {}", self.frame, a + 1, if r.guarded { "guarded by" } else { "hits" }, 2 - a, r.damage));
            } else {
                let n = data.fighters[o].ani.spheres(2 + self.fighters[o].beast() as usize).len() as u8;
                let b = self.br1.as_deref_mut().expect("the imported side");
                b.fs[o] = bridge::shadow(&self.fighters[o]);
                b.fs[o].f.hit_spheres = n;
                let mut r = b.rand;
                let mut ctx = duel::br1::ctx([res, res], &mut r, data, (self.round.index, self.round.need), &b.walls, out, None);
                hit::apply(&mut b.fs, a, dir, mult, pos16, kind, &mut ctx, G98);
                b.rand = r;
                bridge::give_hit(&b.fs[o], &mut self.fighters[o]);
                // the attacker's face; its F68 kept for a main-game hit still to be applied on it (the direction
                // and counter bits its candidate wrote, which the core's F.60 does not have)
                let f68 = self.fighters[k].flags[crate::fight::fighter::F68];
                bridge::publish(&b.fs[k], &mut self.fighters[k]);
                if br2_next {
                    self.fighters[k].flags[crate::fight::fighter::F68] = f68;
                }
                self.events.push(format!("frame {}: P{} hits P{} (the imported game, result {:08x})", self.frame, a + 1, 2 - a, b.fs[o].f.f60));
            }
        }
        duel::hits::guard_fx(&mut self.fx_host(), &data.tables, guard_fx);
    }

    /// After the push-apart moved the faces (`before` their roots): an imported fighter's core follows its face.
    pub(super) fn br1_follow_push(&mut self, data: &Loaded, before: [[i32; 3]; 2]) {
        for i in 0..2 {
            if !self.br1_body(i) || self.br1.as_ref().is_some_and(|b| b.both()) {
                continue;
            }
            let (Some(b), Some(res)) = (self.br1.as_deref_mut(), data.br1_res(i)) else { continue };
            let f = &self.fighters[i];
            let (dx, dz) = (f.m_t[0] - before[i][0], f.m_t[2] - before[i][2]);
            if dx != 0 || dz != 0 {
                physics::push_out(&mut b.fs[i], res, dx, dz);
            }
        }
    }

    /// The ring walls of fighter `i` (`opp` the opponent's status, `bodies` its body spheres): the main game's
    /// The ring walls; an imported fighter's own on its core. Returns (touched, push x, push z).
    pub(super) fn ring_walls_one(&mut self, data: &Loaded, i: usize, opp: u8, bodies: &[crate::formats::act_ani::Sphere]) -> (u32, i32, i32) {
        if !self.br1_body(i) {
            return crate::fight::physics::ring_walls(&mut self.fighters[i], opp, bodies, &mut self.world.stage);
        }
        let hips = self.br1_rings(data)[0];
        let (Some(b), Some(res)) = (self.br1.as_deref_mut(), data.br1_res(i)) else { return (0, 0, 0) };
        // the push-apart's distance and height on the face first (the publish below would drop them)
        bridge::absorb(&mut b.fs[i], &self.fighters[i]);
        let (t, o) = physics::walls(&mut b.fs[i], res, opp, hips, &mut b.walls);
        bridge::publish(&b.fs[i], &mut self.fighters[i]);
        (t, o[0] as i32, o[1] as i32)
    }

    /// The walls' push of fighter `i`: the main game's model and base shift, an imported fighter's on its core.
    pub(super) fn shift_one(&mut self, data: &Loaded, i: usize, dx: i32, dz: i32) {
        crate::fight::physics::shift(&mut self.fighters[i], dx, dz);
        if self.br1_body(i)
            && let (Some(b), Some(res)) = (self.br1.as_deref_mut(), data.br1_res(i))
        {
            physics::push_out(&mut b.fs[i], res, dx, dz);
        }
    }

    /// Debug (tests, FORCEMOVE): a state forced on imported side `me` after its decision, requested as a decided
    /// one (an attack state clears F.54 0x10006000 and sets S.1C).
    fn br1_forced(force: &mut Option<(usize, i16)>, f: &mut crate::fight::br1::object::Br1Fighter, me: usize) {
        if let Some((_, st)) = force.take_if(|(k, _)| *k == me) {
            f.s.state_req = st;
            if st >= 0x80 {
                f.f.f54 &= 0xEFFF_9FFF;
                f.s.attack_index = st - 0x80;
            }
        }
    }

    /// Simulation v18: an imported stage's sounds of this frame (`duel::br1::stage_sounds`), the debris' splashes
    /// logged under the frame number this tick's `frame_step` started from (it runs from `stage_anim_step`, after
    /// `frame_step`'s `self.frame += 1`).
    pub(super) fn br1_stage_sounds(&mut self, g94: u8, stage: Vec<bool>, raw: Vec<i16>, splash: Vec<[i32; 3]>) {
        let frame = self.frame.saturating_sub(1);
        let h = self.br1.as_deref_mut().map(Br1Match::host);
        duel::br1::stage_sounds(h, &mut self.world.stage.br1, g94, stage, raw, splash, frame);
    }

    /// A decoded state's imported part `br1` (the match's, a KO replay snapshot's) fits `data`: the same sides,
    /// each core's indices in range, a cross-engine throw's victim the side of the other engine than its thrower's and
    /// its proxy fitting the proxy's data (`Loaded::br1_ghost`, the main-game proxy's ANI), the AI blocks runnable.
    pub(super) fn br1_check(br1: &Option<Box<Br1Match>>, data: &Loaded) -> Result<(), String> {
        let want = [0, 1].map(|i| data.br1[i].is_some());
        let err = || Err("Imported fighters".into());
        let b = match br1 {
            None if want == [false; 2] => return Ok(()),
            Some(b) if b.side == want => b,
            _ => return err(),
        };
        for i in 0..2 {
            if let Some(r) = data.br1_res(i) {
                bridge::check(&b.fs[i], r)?;
            }
        }
        if !b.ai.iter().all(|a| a.valid()) {
            return Err("Imported AI".into());
        }
        let Some(t) = &b.throw else { return Ok(()) };
        let v = t.victim as usize;
        if v >= 2 || b.both() {
            return err();
        }
        match &t.ghost {
            bridge::Ghost::Br1(g) => match (data.br1[v].is_none(), data.br1_ghost[v].as_deref()) {
                (true, Some(r)) => bridge::check(g, r),
                _ => err(),
            },
            bridge::Ghost::Br2(g) if data.br1[v].is_some() && duel::setup::fighter_fits(g, &data.fighters[v].ani) => Ok(()),
            bridge::Ghost::Br2(_) => err(),
        }
    }
}
