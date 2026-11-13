//! The team battle's imported fighters (engine addition, team battle T5).
//!
//! State: `TeamBr1` (serialised in `TeamFight::br1`)
//!
//! Every imported fighter of a team battle runs through a match's mixed path (`fight::sim::br1`), never through
//! the imported game's pair frame: its core against a view of its target (`View`): the target's core when that is
//! an imported fighter, else the main-game fighter's `bridge::shadow`, or the proxy of its throw across the
//! engines. The core stands in the pair at its slot (`k & 1`, its F.02; `pair_run`; the pair code reads `k & 1` as the
//! place, deliberately: a pair of two is the game's P1 / P2, `side::Fid::slot`), the view in the other place. The
//! face (`TeamFight::fighters`) is what the team battle's own steps read and write (`bridge::publish` / `absorb`), its
//! health the main game's (`bridge::health`). The hits are probed and landed by the attacker's engine for every
//! ordered pair of enemies (`probe_br1`, `probe_on_br1`, `land_br1`, `land_on_br1`); the throws across the engines are
//! a list keyed by thrower (`bridge::CrossThrows`), each played by the thrower's engine on a proxy of the victim as in
//! a match.

use super::{N, TeamFight};
use crate::fight::br1::ai::{Br1Ai, Table};
use crate::fight::br1::bridge::{self, CrossThrow, CrossThrows, Ghost};
use crate::fight::br1::decision::Opts;
use crate::fight::br1::frame::{self, World};
use crate::fight::br1::object::Br1Fighter;
use crate::fight::br1::physics::{Ring, Walls};
use crate::fight::br1::script::{self, Ctx, Request};
use crate::fight::br1::{hit, init, physics};
use crate::fight::duel;
use crate::fight::duel::br1::{G98, br1_stage_byte, think};
use crate::fight::duel::hits;
use crate::fight::fight_data::Loaded;
use crate::fight::fighter::{F5C, F6C, F54, F68};
use crate::fight::side::Fid;
use serde::{Deserialize, Serialize};

/// The imported fighters of a team battle and what their code keeps besides them (`Br1Match` for four).
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct TeamBr1 {
    /// the fighters that are the imported game's
    pub side: [bool; N],
    /// the cores by fighter (a main-game fighter's entry: its last imported view, `bridge::shadow`)
    pub fs: [Br1Fighter; N],
    /// the ring walls' state of the imported game's code (the stage block + 0x44)
    pub walls: Walls,
    /// the gameplay random stream G (one for all imported fighters, as a match's)
    pub rand: u8,
    /// the sound request queue and the sounds started this frame: the imported game's ids (bank 1's fighters'
    /// character ids moved to slots 5 / 6, `requests`)
    pub sound_queue: Vec<(i16, i16)>,
    pub sounds_started: Vec<i16>,
    /// per fighter, the core's damage, permanent damage and blue bar as `bridge::health` last wrote them
    pub synced: [[i16; 3]; N],
    /// the throws across the engines, keyed by thrower
    pub throws: CrossThrows,
    /// the AI blocks by fighter (a main-game fighter's entry unused) and their random table
    pub ai: [Br1Ai; N],
    pub ai_rnd: Table,
    /// this frame's requests of the imported game's code by fighter (`requests` takes them at the frame's end)
    #[serde(skip)]
    out: Vec<(u8, Request)>,
    /// the fighters whose face slot holds a main-game throw's proxy for the moment (within one frame only)
    #[serde(skip)]
    swapped: [bool; N],
    /// the effect, camera and presentation requests of the last `bridge::FX_FRAMES` frames with their frame and
    /// fighter, for the drawing (`present::br1_fx`); not part of the state
    #[serde(skip)]
    pub fx_log: Vec<(u64, u8, Request)>,
}

impl TeamBr1 {
    /// The team battle's imported state as its requests write it (`Br1Match::host`).
    fn host(&mut self) -> duel::br1::Br1Host<'_> {
        duel::br1::Br1Host { side: &self.side, fs: &self.fs, sound_queue: &mut self.sound_queue, fx_log: &mut self.fx_log }
    }
}

/// What stands in an imported fighter's pair opposite its core.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum View {
    /// fighter `t`'s core
    Core(usize),
    /// the imported view of the main-game body in fighter `t`'s slot (`bridge::shadow`)
    Shadow(usize),
    /// the proxy victim of thrower `k`'s throw across the engines
    Ghost(usize),
}

impl TeamFight {
    /// Whether fighter `k` is an imported fighter.
    pub fn is_br1(&self, k: usize) -> bool {
        self.br1.as_ref().is_some_and(|b| b.side[k])
    }

    /// Whether fighter `k`'s body is its imported core's (not while a main-game throw's proxy stands in its
    /// slot).
    pub(super) fn br1_body(&self, k: usize) -> bool {
        self.br1.as_ref().is_some_and(|b| b.side[k] && !b.swapped[k])
    }

    /// Whether fighter `k` (a main-game fighter) is the victim of an imported throw across the engines: posed
    /// by the thrower's proxy, it runs no step of its own.
    pub(super) fn puppeted(&self, k: usize) -> bool {
        self.br1.as_ref().is_some_and(|b| b.throws.thrower_of(k).is_some_and(|t| b.side[t]))
    }

    /// Whether `a` and `b` are the thrower and the victim of one throw across the engines.
    pub(super) fn cross_pair(&self, a: usize, b: usize) -> bool {
        self.br1.as_ref().is_some_and(|x| x.throws.thrower_of(a) == Some(b) || x.throws.thrower_of(b) == Some(a))
    }

    /// Whether fighter `k` is in a throw across the engines (as the thrower or the victim).
    pub(super) fn in_cross_throw(&self, k: usize) -> bool {
        self.br1.as_ref().is_some_and(|b| b.throws.involves(k))
    }

    /// An imported thrower's throw across the engines (its strikes, walls and angles are its pair's).
    pub(super) fn br1_cross_thrower(&self, k: usize) -> bool {
        self.br1.as_ref().is_some_and(|b| matches!(b.throws.of(k), Some(CrossThrow { ghost: Ghost::Br1(_), .. })))
    }

    /// The team battle's imported state for `data` (None without imported fighters): each imported
    /// fighter's block as the loader leaves it (`init::new`, its slot as F.02), the stream G seeded with 1, the AI's
    /// random table as the boot leaves it.
    pub(super) fn br1_new(data: &Loaded) -> Option<Box<TeamBr1>> {
        let first = (0..N).find_map(|k| data.br1_res(k))?;
        let mut b = TeamBr1 { side: std::array::from_fn(|k| data.br1[k].is_some()), rand: 1, ..Default::default() };
        for k in 0..N {
            if let Some(r) = data.br1_res(k) {
                let chr = crate::formats::br1_anm::CHARACTERS.iter().position(|c| *c == r.code).unwrap_or(0) as u8;
                b.fs[k] = init::new(r, (k & 1) as u8, chr, 0);
                b.fs[k].f.f4c = 1;
            }
        }
        b.ai_rnd = Table::boot(&first.tables);
        Some(Box::new(b))
    }

    /// The stage's ring as the imported game's records {hips, edge}: an imported stage's own (BIG STAGE), else the
    /// stage's box for both (`Match::br1_rings`).
    fn br1_rings(&self, data: &Loaded) -> [Ring; 2] {
        duel::br1::rings(data, &self.world.stage.ring)
    }

    /// (T7, simulation v25) Whether an imported core's walls take its spheres' radius as their margin
    /// (`physics::walls_by`), as the main-game fighters' beside it do (BIG STAGE too: its records and walls stay
    /// the imported game's, the bodies stop outside them).
    fn br1_wall_radius(data: &Loaded) -> bool {
        data.gates().br1_wall_radius
    }

    /// (T7) Fighter `k`'s state: its core's (S.10) for an imported body, else its move.
    pub(super) fn state_of(&self, k: usize) -> i16 {
        match self.br1.as_deref() {
            Some(b) if self.br1_body(k) => b.fs[k].s.state_cur,
            _ => self.fighters[k].move_cur,
        }
    }

    /// (T7) An imported throw's posed main-game victim `v` moved by (dx, dz) (Q3DM6's chamfers): its face
    /// and the proxy it is posed from.
    pub(super) fn shift_posed(&mut self, data: &Loaded, v: usize, dx: i32, dz: i32) {
        crate::fight::physics::shift(&mut self.fighters[v], dx, dz);
        let Some(b) = self.br1.as_deref_mut() else { return };
        let (Some(t), Some(res)) = (b.throws.thrower_of(v), data.br1_ghost[v].as_deref()) else { return };
        if let Some(CrossThrow { ghost: Ghost::Br1(g), .. }) = b.throws.of_mut(t) {
            physics::push_out(g, res, dx, dz);
        }
    }

    /// The imported game's match context for the decision (its options; `Match::br1_world`).
    fn br1_world(&self, data: &Loaded) -> World {
        World {
            pads: [self.pads[0], self.pads[1]],
            cam: self.cam.ang[1],
            ai: [0; 2],
            stage: br1_stage_byte(data),
            opts: Opts { g98: G98, need: self.round.need, opt38: 0, rave: false },
            opt2c: 0,
            frozen: false,
            replay: [None; 2],
        }
    }

    /// Fighter `k`'s view: its throw's proxy, its target's core, or the main-game body in its target's slot.
    fn br1_view(&self, k: usize) -> View {
        let t = self.target[k];
        if self.br1_cross_thrower(k) {
            View::Ghost(k)
        } else if self.br1_body(t) {
            View::Core(t)
        } else {
            View::Shadow(t)
        }
    }

    /// `pair_run_at` with fighter `k`'s core at its slot.
    fn pair_run<R>(&mut self, data: &Loaded, k: usize, view: View, run: impl FnOnce(&mut [Br1Fighter; 2], usize, &mut Ctx, &mut Walls) -> R) -> R {
        self.pair_run_at(data, k, k & 1, view, run)
    }

    /// Runs `run(pair, me, ctx, walls)` on fighter `k`'s pair: its core at place `me`, `view` in the other one (a
    /// shadow made for the run; a main-game fighter's kept in its entry as a match keeps it), the data by place
    /// (a view of a main-game fighter: `k`'s, as a match's mixed pair); the requests of the run tagged with the
    /// team battle's fighters, the cores and the proxy put back.
    fn pair_run_at<R>(
        &mut self,
        data: &Loaded,
        k: usize,
        me: usize,
        view: View,
        run: impl FnOnce(&mut [Br1Fighter; 2], usize, &mut Ctx, &mut Walls) -> R,
    ) -> R {
        let rings = self.br1_rings(data);
        let (round, need) = (self.round.index, self.round.need);
        let shadow = match view {
            View::Shadow(t) => Some(bridge::shadow(&self.fighters[t])),
            _ => None,
        };
        let res_k = data.br1_res(k).expect("an imported fighter");
        let b = self.br1.as_deref_mut().expect("Imported fighters");
        let (t, res_t) = match view {
            View::Core(t) => (t, data.br1_res(t).unwrap_or(res_k)),
            View::Shadow(t) => (t, res_k),
            View::Ghost(g) => {
                let v = b.throws.of(g).map_or(g, |c| c.victim as usize);
                (v, data.br1_ghost[v].as_deref().unwrap_or(res_k))
            }
        };
        let mut pair: [Br1Fighter; 2] = Default::default();
        pair[me] = std::mem::take(&mut b.fs[k]);
        pair[1 - me] = match view {
            View::Core(t) => std::mem::take(&mut b.fs[t]),
            View::Shadow(_) => shadow.unwrap_or_default(),
            View::Ghost(g) => match b.throws.of_mut(g) {
                Some(CrossThrow { ghost: Ghost::Br1(p), .. }) => std::mem::take(&mut **p),
                _ => Br1Fighter::default(),
            },
        };
        let mut res = [res_k; 2];
        res[1 - me] = res_t;
        let mut out = Vec::new();
        let mut rand = b.rand;
        let mut ctx = duel::br1::ctx(res, &mut rand, data, (round, need), &b.walls, &mut out, Some(rings));
        let r = run(&mut pair, me, &mut ctx, &mut b.walls);
        b.rand = rand;
        let [p0, p1] = pair;
        let (mine, other) = if me == 0 { (p0, p1) } else { (p1, p0) };
        b.fs[k] = mine;
        match view {
            View::Core(t) => b.fs[t] = other,
            View::Shadow(t) if !b.side[t] => b.fs[t] = other,
            View::Shadow(_) => {}
            View::Ghost(g) => {
                if let Some(CrossThrow { ghost: Ghost::Br1(p), .. }) = b.throws.of_mut(g) {
                    **p = other;
                }
            }
        }
        b.out.extend(out.into_iter().map(|(s, r)| (if s as usize == me { k } else { t } as u8, r)));
        r
    }

    /// Fighter `k`'s face from its core.
    fn br1_publish(&mut self, k: usize) {
        if let Some(b) = self.br1.as_deref() {
            bridge::publish(&b.fs[k], &mut self.fighters[k]);
        }
    }

    /// The faces' writes since the last publish into the cores of every imported body.
    pub(super) fn br1_absorb(&mut self) {
        for k in 0..N {
            if self.br1_body(k)
                && let Some(b) = self.br1.as_deref_mut()
            {
                bridge::absorb(&mut b.fs[k], &self.fighters[k]);
            }
        }
    }

    /// Every imported body's face from its core.
    pub(super) fn br1_publish_all(&mut self) {
        for k in 0..N {
            if self.br1_body(k) {
                self.br1_publish(k);
            }
        }
    }

    /// After `fresh_fighter` for every fighter: the AI's table shuffled by one stream-G draw, the throws
    /// across the engines ended, each imported fighter reset as a match's (`br1_fresh_one`).
    pub(super) fn br1_fresh(&mut self, data: &Loaded, mode: u8) {
        let Some(b) = self.br1.as_deref_mut() else { return };
        if let Some(r) = (0..N).find_map(|k| data.br1_res(k)) {
            let seed = r.tables.table("math.random_bytes").u8(b.rand as usize);
            b.rand = b.rand.wrapping_add(1);
            b.ai_rnd.shuffle(&r.tables, seed as u32);
        }
        // a throw across the engines does not outlast a round
        b.throws.clear();
        b.swapped = [false; N];
        for k in 0..N {
            if self.is_br1(k) {
                self.br1_fresh_one(data, k, mode);
            }
        }
        // the resets' requests are dropped (as a match's `br1_fresh_one`): nothing waits in `out` between frames
        if let Some(b) = self.br1.as_deref_mut() {
            b.out.clear();
        }
    }

    /// `Match::br1_fresh_one` for fighter `k` (modes 0 / 2 / 3; no rounds lost: the faces' F.E5 is 0 in the team
    /// battle): its reset against its target's view, its face the proxy's `chr`, the core published, the
    /// health by the main game's rules.
    fn br1_fresh_one(&mut self, data: &Loaded, k: usize, mode: u8) {
        let (cpu, f54) = (self.fighters[k].o_01, self.fighters[k].flags[F54]);
        let b = self.br1.as_deref_mut().expect("Imported fighters");
        let f = &mut b.fs[k];
        f.f.cpu = cpu;
        f.f.fda = 0;
        f.f.f4c = f54;
        f.s.state_req = 0x28;
        init::reset_input(f);
        let view = self.br1_view(k);
        self.pair_run(data, k, view, |fs, me, ctx, _| {
            let setup = init::Setup { fast_regen: false, practice: false, vitality: init::VITALITY };
            init::reset(fs, me, mode, &setup, ctx);
        });
        let b = self.br1.as_deref_mut().expect("Imported fighters");
        let (f, face) = (&mut b.fs[k], &mut self.fighters[k]);
        face.chr = bridge::br2_proxy(f.f.chr);
        bridge::publish(f, face);
        face.o_b0 = (bridge::HP * self.hp_pct / 100) as i16;
        (face.o_ac, face.o_ae, face.o_aa) = if mode == 3 { (face.o_b0, face.o_b0, face.o_b0) } else { (0, 0, 0) };
        (face.o_c8, face.o_d8, face.o_ca) = (0, 0, 0);
        bridge::write_health(f, face, &mut b.synced[k]);
    }

    /// After `place` at a round start: fighter `k`'s core at its face's place and facing (the reset placed it by
    /// its slot), its nodes posed there (`physics::puppet` with its own records), the face published.
    pub(super) fn br1_place(&mut self, data: &Loaded, k: usize) {
        let (Some(res), Some(b)) = (data.br1_res(k), self.br1.as_deref_mut()) else { return };
        let face = &self.fighters[k];
        let (x, z, yaw) = (face.pos[0], face.pos[2], face.yaw_base);
        let f = &mut b.fs[k];
        let (dx, dz) = (x as i32 - f.anim.pos[0] as i32, z as i32 - f.anim.pos[2] as i32);
        let m = f.world(0);
        let recs = f.form().rec.clone();
        let (t1, t4, y) = (f.anim.t1, f.anim.t4, f.anim.pos[1]);
        physics::puppet(f, res, &recs, t1, t4, [x, y, z], [m[0] + dx, m[1], m[2] + dz], yaw, yaw);
        self.br1_publish(k);
    }

    /// The AI init at a round start for fighter `k` (`Match::br1_ai_init`): its mode from the face's F.01, the match's
    /// CPU level (TEAM ARCADE: the fighter's, `TeamFight::level`); bank 1's random index 64 further (as the
    /// main-game fighters').
    pub(super) fn br1_ai_init(&mut self, data: &Loaded, k: usize) {
        let level = self.level(k);
        let (Some(b), Some(r)) = (self.br1.as_deref_mut(), data.br1_res(k)) else { return };
        let c = self.fighters[k].o_01;
        let mode = if c == 0 { 0 } else { (c & 0x7F) as i32 - 1 };
        b.ai[k].init(&r.anm.bytes, b.fs[k].f.player, level, mode);
        b.ai[k].idx = b.ai[k].idx.wrapping_add(Fid::of(k).bank().ai_offset());
    }

    /// The stage's walls into the imported game's walls state (`Match::br1_walls_in`); returns them as they came in.
    pub(super) fn br1_walls_in(&mut self) -> Walls {
        let Some(b) = self.br1.as_deref_mut() else { return Walls::default() };
        duel::br1::walls_in(&self.world.stage.walls, &mut b.walls)
    }

    /// What the imported game's code changed of the walls since `br1_walls_in` back into the stage (`Match::br1_walls_out`).
    fn br1_walls_out(&mut self, before: Walls) {
        let Some(b) = self.br1.as_deref() else { return };
        duel::br1::walls_out(&b.walls, before, &mut self.world.stage.walls);
    }

    /// Input of imported fighter `k`: its AI against its view (a main-game
    /// target's hips its own), its raw pad (L2 the target cycle's), the face published.
    pub(super) fn br1_input(&mut self, data: &Loaded, k: usize, pad: u16) {
        let Some(res) = data.br1_res(k) else { return };
        let view = self.br1_view(k);
        let hips = match view {
            View::Shadow(t) => Some(self.fighters[t].hips[1]),
            _ => None,
        };
        let (cam, timer) = (self.cam.ang[1], self.round.timer);
        let b = self.br1.as_deref_mut().expect("Imported fighters");
        let mut ai = std::mem::take(&mut b.ai[k]);
        let rnd = std::mem::take(&mut b.ai_rnd);
        let rec = self.pair_run(data, k, view, |fs, me, _, _| {
            let opp_hips = hips.unwrap_or(fs[1 - me].hips_y());
            let a = think(true, &mut ai, &rnd, &fs[me], &fs[1 - me].f, opp_hips, res, timer);
            crate::fight::br1::input::process(fs, me, &res.anm, crate::fight::br1::input::Pad { raw: pad, cam, ai: a, replay: None })
        });
        let b = self.br1.as_deref_mut().expect("Imported fighters");
        (b.ai[k], b.ai_rnd) = (ai, rnd);
        self.fighters[k].pad_record = rec;
        self.br1_publish(k);
    }

    /// The throws of a pair `p` (the lower index first) with an imported fighter: two
    /// cores by the imported game's resolution (the lower index in P1's place); a mixed pair as a match's (the main game's
    /// resolution first, the imported face standing in, then the imported game's against the other's view), a grab
    /// across the engines starting a `CrossThrow` and locking the pair's targets.
    pub(super) fn br1_throw(&mut self, data: &Loaded, p: [usize; 2]) {
        let [a, c] = p;
        if self.br1_body(a) && self.br1_body(c) {
            self.pair_run_at(data, a, 0, View::Core(c), |fs, _, ctx, _| crate::fight::br1::throw::resolve(fs, ctx));
            self.br1_publish(a);
            self.br1_publish(c);
            return;
        }
        let (k, o) = if self.br1_body(a) { (a, c) } else { (c, a) };
        let f6c = self.fighters[k].flags[F6C];
        crate::fight::throw::resolve_pair(&mut self.fighters, p);
        if self.fighters[k].flags[F6C] & !f6c & 0x80_0000 != 0 {
            // The main game's resolution runs first: its grab wins over an imported attempt of the same frame
            self.cross_start_br2(data, o, k);
            return;
        }
        // what the main game's resolution wrote on the face (its whiff's flip)
        if let Some(b) = self.br1.as_deref_mut() {
            bridge::absorb(&mut b.fs[k], &self.fighters[k]);
        }
        let grabbed = self.pair_run(data, k, View::Shadow(o), |fs, me, ctx, _| {
            let before = fs[1 - me].f.f64;
            crate::fight::br1::throw::resolve(fs, ctx);
            fs[1 - me].f.f64 & !before & 0x80_0000 != 0
        });
        self.br1_publish(k);
        if grabbed {
            self.cross_start_br1(data, k, o);
        }
    }

    /// An imported throw by `k` grabbed main-game fighter `o`'s view (`Match::cross_start_br1`): `o`'s
    /// imported-game proxy reset as a round starts it (unseen: a copy of stream G, its sounds dropped), placed and posed as
    /// the victim, the grab's request taken over.
    fn cross_start_br1(&mut self, data: &Loaded, k: usize, o: usize) {
        let rings = self.br1_rings(data);
        let (Some(res), Some(gres)) = (data.br1_res(k), data.br1_ghost[o].as_deref()) else { return };
        let (round, need) = (self.round.index, self.round.need);
        let face = &self.fighters[o];
        let b = self.br1.as_deref_mut().expect("Imported fighters");
        let thrown_by = b.fs[o].s.thrown_by;
        let me = k & 1;
        let mut pair: [Br1Fighter; 2] = Default::default();
        pair[me] = std::mem::take(&mut b.fs[k]);
        pair[1 - me] = init::new(gres, (o & 1) as u8, bridge::br1_proxy(face.chr), face.o_01);
        let mut res2 = [res; 2];
        res2[1 - me] = gres;
        let mut r = b.rand;
        let mut dropped = Vec::new();
        let mut ctx = duel::br1::ctx(res2, &mut r, data, (round, need), &b.walls, &mut dropped, Some(rings));
        let setup = init::Setup { fast_regen: false, practice: false, vitality: init::VITALITY };
        init::reset(&mut pair, 1 - me, 0, &setup, &mut ctx);
        let [p0, p1] = pair;
        let (mine, mut g) = if me == 0 { (p0, p1) } else { (p1, p0) };
        b.fs[k] = mine;
        duel::br1::pose_br1_ghost(&mut g, gres, face, thrown_by);
        b.throws.start(k, CrossThrow { victim: o as u8, ghost: Ghost::Br1(Box::new(g)), frames: 0 });
        (self.target[k], self.target[o]) = (o, k);
        self.events.push(format!("frame {}: P{}'s grab on P{} (across the engines: the imported fighter throws)", self.frame, k + 1, o + 1));
    }

    /// A main-game throw by `t` grabbed imported fighter `k`'s face (`Match::cross_start_br2`): `k`'s
    /// main-game proxy made from the face (`bridge::br2_ghost`), its core puppeted from here on.
    fn cross_start_br2(&mut self, data: &Loaded, t: usize, k: usize) {
        let Some(b) = self.br1.as_deref_mut() else { return };
        let g = duel::br1::br2_ghost(&mut b.fs[k], &mut self.fighters[k], &data.fighters[k].ani);
        b.throws.start(t, CrossThrow { victim: k as u8, ghost: Ghost::Br2(Box::new(g)), frames: 0 });
        (self.target[k], self.target[t]) = (t, k);
        self.events.push(format!("frame {}: P{}'s grab on P{} (across the engines: the imported fighter is thrown)", self.frame, t + 1, k + 1));
    }

    /// Before the fighters' steps: each main-game throw's proxy into its imported victim's slot (it sees the
    /// victim's skeleton), the victim's face kept in the throw (`Match::cross_br2_thrower`); `br1_finish` swaps back.
    pub(super) fn br1_swap_in(&mut self) {
        let Some(b) = self.br1.as_deref_mut() else { return };
        for t in b.throws.throwers() {
            let Some(CrossThrow { victim, ghost: Ghost::Br2(g), .. }) = b.throws.of_mut(t) else { continue };
            let v = *victim as usize;
            duel::br1::proxy_sees(g, &self.fighters[v]);
            std::mem::swap(&mut self.fighters[v], &mut **g);
            b.swapped[v] = true;
        }
    }

    /// A main-game throw's proxy in slot `v`: its queued damage and gauge change onto the victim's face kept in
    /// the throw (`Match::cross_take_damage`).
    pub(super) fn br1_take_damage(&mut self, v: usize) {
        let Some(b) = self.br1.as_deref_mut() else { return };
        let Some(t) = b.throws.thrower_of(v) else { return };
        let Some(CrossThrow { ghost: Ghost::Br2(face), .. }) = b.throws.of_mut(t) else { return };
        duel::br1::br2_proxy_damage(&mut self.fighters[v], face);
    }

    /// Whether fighter `v`'s slot holds a main-game throw's proxy this frame.
    pub(super) fn swapped(&self, v: usize) -> bool {
        self.br1.as_ref().is_some_and(|b| b.swapped[v])
    }

    /// Imported fighter `k`'s frame (by its parts, `Match::br1_mixed_steps`): the health by the
    /// main game's rules on its face, then the decision, the script and the velocity against its view; a thrower across the
    /// engines runs its pair with the proxy (`cross_br1_thrower`).
    pub(super) fn br1_fighter(&mut self, data: &Loaded, k: usize) {
        if self.br1_cross_thrower(k) {
            self.cross_br1_thrower(data, k);
            return;
        }
        let w = self.br1_world(data);
        let (charge, recover) = (self.custom.charge as i16, self.custom.recover);
        let b = self.br1.as_deref_mut().expect("Imported fighters");
        let ko = bridge::health(&mut b.fs[k], k, &mut self.fighters[k], &mut b.synced[k], charge, recover, &mut b.out);
        let view = self.br1_view(k);
        self.pair_run(data, k, view, |fs, me, ctx, walls| {
            frame::decide(fs, me, ctx, &w, walls);
            ctx.walls = walls.hp.map(|h| h >= 0);
            script::step(fs, me, ctx);
            frame::after_script(fs, me, ctx, ko);
        });
        self.br1_publish(k);
    }

    /// An imported thrower `k`'s frame during its throw across the engines (`Match::cross_br1_thrower`): the
    /// imported game's pair of the thrower and the proxy through their frames, the move, the throw's strikes and `after_hits`; the
    /// main-game victim's health by its rules with the proxy's damage, its pose and place the proxy's.
    fn cross_br1_thrower(&mut self, data: &Loaded, k: usize) {
        let Some(o) = self.br1.as_ref().and_then(|b| b.throws.of(k)).map(|t| t.victim as usize) else { return };
        let (Some(res), Some(gres)) = (data.br1_res(k), data.br1_ghost[o].as_deref()) else { return };
        let rings = self.br1_rings(data);
        let radius = Self::br1_wall_radius(data);
        let w = self.br1_world(data);
        let (charge, recover) = (self.custom.charge as i16, self.custom.recover);
        let (round, need) = (self.round.index, self.round.need);
        // the victim's health (the fighter step's start)
        let rate = duel::recover_rate(recover, &data.fighters[o].ani);
        let me = k & 1;
        let mut res2 = [res; 2];
        res2[1 - me] = gres;
        let b = self.br1.as_deref_mut().expect("Imported fighters");
        let mut pair: [Br1Fighter; 2] = Default::default();
        pair[me] = std::mem::take(&mut b.fs[k]);
        if let Some(CrossThrow { ghost: Ghost::Br1(g), .. }) = b.throws.of_mut(k) {
            pair[1 - me] = std::mem::take(&mut **g);
        }
        let mut out = Vec::new();
        for i in 0..2 {
            let mut rr = b.rand;
            let mut ctx = duel::br1::ctx(res2, &mut rr, data, (round, need), &b.walls, &mut out, Some(rings));
            let ko = if i == me {
                bridge::health(&mut pair[me], me, &mut self.fighters[k], &mut b.synced[k], charge, recover, ctx.out)
            } else {
                // what the proxy queued last frame (its script, the strikes) applied this frame, as either engine does
                duel::br1::proxy_damage(&mut pair[i], &mut self.fighters[o]);
                crate::fight::hit::health(&mut self.fighters[o], charge, rate);
                false
            };
            frame::decide(&mut pair, i, &mut ctx, &w, &mut b.walls);
            ctx.walls = b.walls.hp.map(|h| h >= 0);
            script::step(&mut pair, i, &mut ctx);
            frame::after_script(&mut pair, i, &mut ctx, ko);
            b.rand = rr;
        }
        frame::pair_move(&mut pair, res2);
        // a throw's strikes (the pair's hits, the proxy's hurt spheres its own)
        let mut rr = b.rand;
        let mut ctx = duel::br1::ctx(res2, &mut rr, data, (round, need), &b.walls, &mut out, Some(rings));
        hit::hits(&mut pair, &mut ctx, G98);
        b.rand = rr;
        physics::after_hits_by(&mut pair, res2, rings[0], &mut b.walls, radius);
        bridge::pose_face(&mut self.fighters[o], &pair[1 - me]);
        bridge::publish(&pair[me], &mut self.fighters[k]);
        let [p0, p1] = pair;
        let (mine, ghost) = if me == 0 { (p0, p1) } else { (p1, p0) };
        b.fs[k] = mine;
        if let Some(CrossThrow { ghost: Ghost::Br1(g), .. }) = b.throws.of_mut(k) {
            **g = ghost;
        }
        b.out.extend(out.into_iter().map(|(s, r)| (if s as usize == me { k } else { o } as u8, r)));
    }

    /// The pair integration's move of imported fighter `k` (`done`: the fighters moved this frame): a thrower across the
    /// engines moved with its proxy in its frame (its victim posed from it); an imported throw pair (the victim
    /// attached by F.64 0x60000) through the imported game's pair move, the lower index in P1's place; else alone
    /// (`physics::integrate_one`), its landing, F.54 0x40000000 while held.
    pub(super) fn br1_integrate(&mut self, data: &Loaded, k: usize, done: &mut [bool; N]) {
        done[k] = true;
        if self.br1_cross_thrower(k) {
            if let Some(v) = self.br1.as_ref().and_then(|b| b.throws.of(k)).map(|t| t.victim as usize) {
                done[v] = true;
            }
            return;
        }
        let Some(res) = data.br1_res(k) else { return };
        let t = self.target[k];
        let attached = |f: &Br1Fighter| f.f.f64 & 0x6_0000 != 0;
        let pair = self.br1_body(t) && !done[t] && self.target[t] == k && {
            let b = self.br1.as_deref().expect("Imported fighters");
            attached(&b.fs[k]) || attached(&b.fs[t])
        };
        if pair {
            let (lo, hi) = (k.min(t), k.max(t));
            self.pair_run_at(data, lo, 0, View::Core(hi), |fs, _, ctx, _| frame::pair_move(fs, ctx.res));
            done[t] = true;
            self.br1_publish(lo);
            self.br1_publish(hi);
            return;
        }
        let b = self.br1.as_deref_mut().expect("Imported fighters");
        let f = &mut b.fs[k];
        physics::integrate_one(f, res);
        physics::landing(f);
        if f.f.f50 & 0x1000_0000 != 0 {
            f.f.f54 |= 0x4000_0000;
        }
        self.br1_publish(k);
    }

    /// After the push-apart moved the faces (`before` their roots): each imported core follows its face
    /// (`Match::br1_follow_push`).
    pub(super) fn br1_follow(&mut self, data: &Loaded, before: [[i32; 3]; N]) {
        for k in 0..N {
            if !self.br1_body(k) {
                continue;
            }
            let (Some(b), Some(res)) = (self.br1.as_deref_mut(), data.br1_res(k)) else { continue };
            let f = &self.fighters[k];
            let (dx, dz) = (f.m_t[0] - before[k][0], f.m_t[2] - before[k][2]);
            if dx != 0 || dz != 0 {
                physics::push_out(&mut b.fs[k], res, dx, dz);
            }
        }
    }

    /// The ring walls of fighter `k` (`Match::ring_walls_one`; `opp` its target's status, `bodies` its body spheres):
    /// the main game's, an imported core's (the push-apart's distance on its face taken in
    /// first); none for an imported thrower across the engines and its posed victim (their pair's `after_hits`).
    /// Returns (touched, push x, push z).
    pub(super) fn br1_ring_walls(&mut self, data: &Loaded, k: usize, opp: u8, bodies: &[crate::formats::act_ani::Sphere]) -> (u32, i32, i32) {
        if self.puppeted(k) || (self.br1_body(k) && self.in_cross_throw(k)) {
            return (0, 0, 0);
        }
        if !self.br1_body(k) {
            return crate::fight::physics::ring_walls(&mut self.fighters[k], opp, bodies, &mut self.world.stage);
        }
        let (hips, radius) = (self.br1_rings(data)[0], Self::br1_wall_radius(data));
        let (Some(b), Some(res)) = (self.br1.as_deref_mut(), data.br1_res(k)) else { return (0, 0, 0) };
        bridge::absorb(&mut b.fs[k], &self.fighters[k]);
        let (t, o) = physics::walls_by(&mut b.fs[k], res, opp, hips, &mut b.walls, radius);
        bridge::publish(&b.fs[k], &mut self.fighters[k]);
        (t, o[0] as i32, o[1] as i32)
    }

    /// Main-game attacker `a`'s test on imported fighter `d` (`Match::hits_mixed`): `hit::can_hit`, the
    /// direction bits on `d`'s face, the multiplier, the sphere test against `d`'s core's hurt spheres at its nodes.
    pub(super) fn probe_on_br1(&mut self, data: &Loaded, a: usize, d: usize, nodes: &[Vec<[i32; 3]>]) -> Option<hits::Probe> {
        let (res, cur) = (data.br1_res(d)?, self.br1.as_deref()?.fs[d].cur as usize & 1);
        duel::br1::probe_on_core(&mut self.fighters, a, d, nodes, &res.anm.spheres[2 + cur])
    }

    /// Imported attacker `a`'s test on `d` (`Match::hits_mixed`; its view: `d`'s core, or the main-game body's
    /// shadow with its ANI's hit sphere count): `can_hit` (the attacker's F.54 0x14 cleared, onto its face), the
    /// direction bits, the multiplier, the sphere test; the damage in the main game's units (× 200 / 187) for the trade.
    /// Returns the probe, the direction bits and the contact point.
    pub(super) fn probe_br1(&mut self, data: &Loaded, a: usize, d: usize) -> Option<(hits::Probe, u32, [i16; 3])> {
        let core = self.br1_body(d);
        let hurt = if core {
            let cur = self.br1.as_deref()?.fs[d].cur as usize & 1;
            data.br1_res(d)?.anm.spheres[2 + cur].clone()
        } else {
            data.fighters[d].ani.spheres(2 + self.fighters[d].beast() as usize)
        };
        let n = hurt.len() as u8;
        let view = if core { View::Core(d) } else { View::Shadow(d) };
        let r = self.pair_run(data, a, view, |fs, me, _, _| {
            if !core {
                // F.9: the view's hit sphere count (its ANI's table)
                fs[1 - me].f.hit_spheres = n;
            }
            duel::br1::probe_core(fs, me, &hurt)
        });
        // the attacker's F.54 0x14 (can_hit clears it whatever its result) on its face, so that the absorb after the
        // combo flags keeps it
        let f54 = self.br1.as_deref()?.fs[a].f.f54;
        self.fighters[a].flags[F5C] = f54;
        r
    }

    /// Main-game attacker `a`'s hit on imported fighter `d` (`Match::hits_mixed`): `hit::apply` on the face
    /// (its probe's direction bits `f68`) with the imported game's guard gate (F.54 0x2000 as F5C 1), the launch, the result
    /// into the core (`bridge::take_hit`) and the face again from it; a third party's hit on a throw victim
    /// (`damage_only`) only queues its damage and gauge on the face.
    pub(super) fn land_on_br1(&mut self, data: &Loaded, a: usize, d: usize, p: &hits::Probe, f68: u32, damage_only: bool) -> crate::fight::hit::HitResult {
        let f54 = self.br1.as_deref().map_or(0, |b| b.fs[d].f.f54);
        let saved = damage_only.then(|| self.fighters[d].clone());
        self.fighters[d].flags[F68] = f68;
        let r = duel::br1::land_on_face(&mut self.fighters, a, d, f54, p.mult, p.kind, data);
        if let Some(s) = saved {
            let hurt = std::mem::replace(&mut self.fighters[d], s);
            let v = &mut self.fighters[d];
            (v.o_c8, v.o_ca, v.o_d8) = (hurt.o_c8, hurt.o_ca, hurt.o_d8);
            return r;
        }
        if let Some(b) = self.br1.as_deref_mut() {
            bridge::take_hit(&mut b.fs[d], &self.fighters[d]);
            // the core's masked result word on the face
            bridge::publish(&b.fs[d], &mut self.fighters[d]);
        }
        r
    }

    /// Imported attacker `a`'s hit on `d` (`hit::apply` on its view, its probe's direction bits `dir` and contact
    /// point `pos16`): a core takes it in its engine, a main-game body the result onto its face
    /// (`bridge::give_hit`); a third party's hit on a throw victim (`damage_only`) only its damage and gauge. The
    /// faces published. Returns whether it was guarded.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn land_br1(&mut self, data: &Loaded, a: usize, d: usize, p: &hits::Probe, dir: u32, pos16: [i16; 3], damage_only: bool) -> bool {
        let core = self.br1_body(d);
        let n = if core { 0 } else { data.fighters[d].ani.spheres(2 + self.fighters[d].beast() as usize).len() as u8 };
        let saved = if core && damage_only { self.br1.as_deref().map(|b| b.fs[d].clone()) } else { None };
        let view = if core { View::Core(d) } else { View::Shadow(d) };
        let (mult, kind) = (p.mult, p.kind);
        let (res, hurt) = self.pair_run(data, a, view, |fs, me, ctx, _| {
            if !core {
                fs[1 - me].f.hit_spheres = n;
            }
            let r = hit::apply(fs, me, dir, mult, pos16, kind, ctx, G98);
            (r, (!core).then(|| fs[1 - me].clone()))
        });
        if let Some(h) = hurt {
            let face = &mut self.fighters[d];
            if damage_only {
                face.o_c8 = face.o_c8.wrapping_add(bridge::scale_damage(h.f.queued_damage, bridge::BR1_HP));
                face.o_d8 = face.o_d8.wrapping_add(h.f.gauge_queued);
            } else {
                bridge::give_hit(&h, face);
            }
        } else if let (Some(s), Some(b)) = (saved, self.br1.as_deref_mut()) {
            let hurt = std::mem::replace(&mut b.fs[d], s);
            (b.fs[d].f.queued_damage, b.fs[d].f.gauge_queued) = (hurt.f.queued_damage, hurt.f.gauge_queued);
        }
        if core {
            self.br1_publish(d);
        }
        self.br1_publish(a);
        res == 2
    }

    /// A hit on thrower `t` breaks its throw where the imported game's code holds it: an imported thrower's
    /// throw across the engines ends with its victim lying (`cross_end`), an imported thrower's imported
    /// victim drops out lying (`br1_release`). False where the main game's rule applies (`break_throw`; a
    /// main-game thrower's proxy in its imported victim's slot breaks as a main-game victim, `br1_finish` hands the
    /// victim back).
    pub(super) fn br1_break(&mut self, data: &Loaded, t: usize) -> bool {
        if self.br1_cross_thrower(t) {
            self.events.push(format!("frame {}: P{}'s throw broken", self.frame, t + 1));
            self.cross_end(data, t, 0x67);
            return true;
        }
        if !self.br1_body(t) {
            return false;
        }
        let v = self.target[t];
        let held = self.br1_body(v) && self.br1.as_deref().is_some_and(|b| b.fs[v].f.f50 & 0x10_0000 != 0);
        if v != t && held {
            self.events.push(format!("frame {}: P{}'s throw on P{} broken", self.frame, t + 1, v + 1));
            self.br1_release(data, v, 0x67);
        }
        true
    }

    /// Imported fighter `v` let go by its thrower: out of the throw (F.50 0x100000, F.64 0x860000 cleared), state
    /// `state` started and its script run once (as a match's hand-back, `Match::cross_br2_thrower`).
    fn br1_release(&mut self, data: &Loaded, v: usize, state: i16) {
        let (Some(res), Some(b)) = (data.br1_res(v), self.br1.as_deref_mut()) else { return };
        duel::br1::release(&mut b.fs[v], res, state);
        let view = self.br1_view(v);
        self.pair_run(data, v, view, |fs, me, ctx, _| script::run(fs, me, ctx));
        self.br1_publish(v);
    }

    /// Thrower `k`'s throw across the engines ends now, its victim going on in state `state` in its own engine (the
    /// hand-back of `Match::cross_br1_thrower` / `cross_br2_thrower`; a main-game thrower's proxy is out of its
    /// victim's slot by then): the proxy's damage still queued goes with the victim.
    fn cross_end(&mut self, data: &Loaded, k: usize, state: i16) {
        let Some(t) = self.br1.as_deref_mut().and_then(|b| b.throws.end(k)) else { return };
        let v = t.victim as usize;
        self.events.push(format!("frame {}: P{}'s throw on P{} over after {} frames, state {state:#x}", self.frame, k + 1, v + 1, t.frames));
        match t.ghost {
            Ghost::Br1(mut g) => {
                duel::br1::proxy_damage(&mut g, &mut self.fighters[v]);
                self.fighters[v].o_c0 = g.f.down_timer;
                duel::release(&mut self.fighters[v], &data.fighters[v].ani, state);
                self.run_start_move(data, v);
            }
            Ghost::Br2(g) => {
                let face = &mut self.fighters[v];
                face.o_c8 = face.o_c8.wrapping_add(g.o_c8);
                face.o_d8 = face.o_d8.wrapping_add(g.o_d8);
                if let Some(b) = self.br1.as_deref_mut() {
                    b.fs[v].f.down_timer = g.o_c0;
                }
                (self.target[v], self.target[k]) = (k, v);
                self.br1_release(data, v, state);
            }
        }
    }

    /// After the angles (the end of `Match::cross_br2_thrower` / `cross_br1_thrower`): each main-game throw's
    /// proxy out of its victim's slot, the victim's health by the main game's rules, its core posed from the proxy;
    /// a throw ends when its proxy reaches a state both games share (`CrossThrow::handover`) or its thrower is KO'd
    /// (then lying); the walls back into the stage, the requests of this frame.
    pub(super) fn br1_finish(&mut self, data: &Loaded, walls0: Walls) {
        if self.br1.is_none() {
            return;
        }
        let (charge, recover) = (self.custom.charge as i16, self.custom.recover);
        let throwers = self.br1.as_ref().map(|b| b.throws.throwers()).unwrap_or_default();
        for t in throwers {
            let ko = self.fighters[t].o_00 & 0x80 != 0;
            let b = self.br1.as_deref_mut().expect("Imported fighters");
            let next = match b.throws.of_mut(t) {
                Some(CrossThrow { victim, ghost: Ghost::Br2(g), .. }) => {
                    let v = *victim as usize;
                    std::mem::swap(&mut self.fighters[v], &mut **g);
                    b.swapped[v] = false;
                    let g = (**g).clone();
                    let Some(res) = data.br1_res(v) else { continue };
                    bridge::health(&mut b.fs[v], v, &mut self.fighters[v], &mut b.synced[v], charge, recover, &mut b.out);
                    duel::br1::puppet_from(&mut b.fs[v], res, &g);
                    bridge::publish(&b.fs[v], &mut self.fighters[v]);
                    // (T7, simulation v25) the victim's own body (the proxy's walls saw the proxy's) back inside the
                    // walls and chamfers at once: its core, its face and the proxy
                    if data.gates().posed_inside {
                        let cur = b.fs[v].cur as usize & 1;
                        let (dx, dz) = crate::fight::physics::inside(&self.fighters[v], &res.anm.spheres[cur], &self.world.stage, data.gates());
                        if dx != 0 || dz != 0 {
                            physics::push_out(&mut b.fs[v], res, dx, dz);
                            bridge::publish(&b.fs[v], &mut self.fighters[v]);
                            if let Some(CrossThrow { ghost: Ghost::Br2(p), .. }) = b.throws.of_mut(t) {
                                crate::fight::physics::shift(p, dx, dz);
                            }
                        }
                    }
                    let state = b.throws.of_mut(t).and_then(|c| c.handover(g.move_cur));
                    state.or(ko.then_some(0x67))
                }
                Some(c) => {
                    let state = match &c.ghost {
                        Ghost::Br1(g) => g.s.state_cur,
                        Ghost::Br2(_) => continue,
                    };
                    c.handover(state).or(ko.then_some(0x67))
                }
                None => continue,
            };
            if let Some(state) = next {
                self.cross_end(data, t, state);
            }
        }
        self.br1_walls_out(walls0);
        self.br1_requests(data);
    }

    /// The requests of the imported game's code this frame (`duel::br1::requests`, as `Match::br1_requests`): the sounds
    /// queued (bank 1's fighters' character ids moved up to slots 5 / 6), the stage sound on an
    /// imported stage, op 0x17 sub 2 case 10's 0x5F, the rest logged for the drawing.
    fn br1_requests(&mut self, data: &Loaded) {
        let frame = self.frame;
        let Some(b) = self.br1.as_deref_mut() else { return };
        let out = std::mem::take(&mut b.out);
        duel::br1::requests(&mut b.host(), data, out, frame, &self.world.stage.br1);
    }

    /// An imported stage's sounds of this frame (`duel::br1::stage_sounds`, as `Match::br1_stage_sounds`): into
    /// the imported game's queue, without an imported fighter to `Counters::sounds` for the session; the debris'
    /// splashes logged for the drawing (under the frame number this tick's step started from).
    pub(super) fn br1_stage_sounds(&mut self, g94: u8, stage: Vec<bool>, raw: Vec<i16>, splash: Vec<[i32; 3]>) {
        let frame = self.frame.saturating_sub(1);
        let h = self.br1.as_deref_mut().map(TeamBr1::host);
        duel::br1::stage_sounds(h, &mut self.world.stage.br1, g94, stage, raw, splash, frame);
    }

    /// The imported fighters as the drawing shows them.
    pub(super) fn br1_scene(&self) -> Option<crate::fight::scene::Br1Scene<'_>> {
        self.br1.as_deref().map(|b| crate::fight::scene::Br1Scene { side: &b.side, fs: &b.fs, fx_log: &b.fx_log })
    }

    /// A decoded state's imported part fits `data`: the same fighters, each core's lists and the AI blocks in
    /// range, the throws across the engines each a thrower of one engine on a victim of the other with its proxy
    /// fitting the proxy's data.
    pub(super) fn br1_check(&self, data: &Loaded) -> Result<(), String> {
        let want: [bool; N] = std::array::from_fn(|k| data.br1[k].is_some());
        let err = || Err("Imported fighters".to_string());
        let b = match &self.br1 {
            None if want == [false; N] => return Ok(()),
            Some(b) if b.side == want => b,
            _ => return err(),
        };
        for k in 0..N {
            if let Some(r) = data.br1_res(k) {
                bridge::check(&b.fs[k], r)?;
            }
        }
        if !b.ai.iter().all(|a| a.valid()) {
            return Err("Imported AI".into());
        }
        b.throws.check(N)?;
        for (t, c) in b.throws.iter() {
            let v = c.victim as usize;
            let ok = match &c.ghost {
                Ghost::Br1(g) => b.side[t] && !b.side[v] && data.br1_ghost[v].as_deref().is_some_and(|r| bridge::check(g, r).is_ok()),
                Ghost::Br2(g) => !b.side[t] && b.side[v] && duel::setup::fighter_fits(g, &data.fighters[v].ani),
            };
            if !ok {
                return err();
            }
        }
        Ok(())
    }
}
