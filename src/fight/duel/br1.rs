//! The imported parts a fight of either kind shares (engine addition, the import): the option byte G.98, the
//! AI's pad, the stage byte, a throw proxy's damage onto its face, the requests of its code and an imported
//! stage's sounds (`Br1Host`).
//!
//! State: none of its own
//!
//! Free functions shared by the match's imported fighters (`fight::sim::br1`) and the team battle's
//! (`fight::team::br1`).

use crate::fight::br1::ai::{self, Br1Ai, Table};
use crate::fight::br1::bridge::{self, Br1Match};
use crate::fight::br1::hit;
use crate::fight::br1::object::{Block, Br1Fighter};
use crate::fight::br1::physics::{self, Ring, Walls};
use crate::fight::br1::res::Br1Res;
use crate::fight::br1::script::{self, Call, Ctx, Request};
use crate::fight::duel::hits::Probe;
use crate::fight::fight_data::Loaded;
use crate::fight::fighter::{F5C, F68, Fighter, pair_mut};
use crate::fight::hit as h2;
use crate::fight::scene::Br1Scene;
use crate::fight::side::Fid;
use crate::fight::stage::AreaRec;
use crate::fight::stage::br1::Counters;
use crate::formats::act_ani::{Ani, Sphere};

/// The option byte G.98 of every traced imported match (its other effects kept as there).
pub(crate) const G98: u8 = 1;

/// The AI's pad for fighter `me` (only a CPU fighter, F.01, neither KO'd nor frozen, F.00 & 0x8080, runs
/// simulation v20, `version::v::BR1_AI`), `opp` its opponent's block (an imported core's, or a
/// main-game fighter's view) with its hips' height, `timer` the round timer (G.90).
#[allow(clippy::too_many_arguments)]
pub(crate) fn think(on: bool, ai: &mut Br1Ai, rnd: &Table, me: &Br1Fighter, opp: &Block, opp_hips: i32, res: &Br1Res, timer: i16) -> u16 {
    if !on || me.f.cpu == 0 || (me.f.status as u16 | (me.f.cpu as u16) << 8) & 0x8080 != 0 {
        return 0;
    }
    ai.think(&ai::Ctx { me, opp, opp_hips, anm: &res.anm.bytes, tables: &res.tables, rnd, timer })
}

/// The imported game's stage byte G.94 for its code (op 0x22 sub 5, the ring tables): an imported stage's own
/// (simulation v16), else the main-game stage's number.
pub(crate) fn br1_stage_byte(data: &Loaded) -> i8 {
    data.br1_stage.map_or(data.stage_no as i8, |b| b.g94 as i8)
}

/// An imported throw's proxy `g`: its queued damage (× 200 / 187) and gauge change moved onto the
/// main-game victim's face (the proxy is never KO'd), its last damage kept as the imported game's health step keeps it (op 0x19 subs 0 / 1
/// take 3/10, 1/10 of it), in the imported game's units of the face's as `bridge::health` gives a core it.
pub(crate) fn proxy_damage(g: &mut Br1Fighter, face: &mut Fighter) {
    let d = bridge::scale_damage(g.f.queued_damage, bridge::BR1_HP);
    face.o_c8 = face.o_c8.wrapping_add(d);
    face.o_d8 = face.o_d8.wrapping_add(g.f.gauge_queued);
    if d != 0 {
        g.f.last_damage = (d.wrapping_abs() as i32 * bridge::BR1_HP as i32 / bridge::HP) as i16;
    }
    (g.f.queued_damage, g.f.gauge_queued) = (0, 0);
}

/// The parts of a fight's imported state that its requests and its stage's sounds write (`Br1Match`, the team
/// battle's `TeamBr1`), lent by it.
pub(crate) struct Br1Host<'a> {
    /// the imported sides and their cores (`Br1Match::side` / `fs`)
    pub side: &'a [bool],
    pub fs: &'a [Br1Fighter],
    /// the sound request queue
    pub sound_queue: &'a mut Vec<(i16, i16)>,
    /// the requests logged for the drawing (`Br1Match::fx_log`)
    pub fx_log: &'a mut Vec<(u64, u8, Request)>,
}

impl Br1Host<'_> {
    /// The player (F.02) of the first imported Uriko (F.03 8) in index order (`Br1Scene::uriko`;
    /// P1's block, then P2's).
    fn uriko(&self) -> Option<u8> {
        Br1Scene { side: self.side, fs: self.fs, fx_log: self.fx_log }.uriko().map(|k| self.fs[k].f.player)
    }

    /// Sound `id` into the queue (32 slots, a full queue drops it).
    fn queue(&mut self, id: i16, delay: i16) {
        if self.sound_queue.len() < 32 {
            self.sound_queue.push((id, delay));
        }
    }
}

impl Br1Match {
    /// The match's imported state as its requests write it.
    pub(crate) fn host(&mut self) -> Br1Host<'_> {
        Br1Host { side: &self.side, fs: &self.fs, sound_queue: &mut self.sound_queue, fx_log: &mut self.fx_log }
    }
}

/// The requests `out` (by fighter) of the imported game's code this frame (`frame`): the sounds queued (32
/// slots; bank 1's fighters' character ids, slot 3 / 4's 0xC0.., moved 0x80 up to slots 5 / 6, `Bank::br1_sound`: a
/// match's sides are bank 0); the effects, the camera words and the presentation calls logged for the drawing (engine
/// addition: drawing only, not part of the state; the last `FX_FRAMES` frames kept). On an imported stage
/// (simulation v17) the stage sound (n) is queued too (`counters`' `stage_sound`: the stage bank's 0x80 + n,
/// on Uriko's stage while its G.4A < 0x4B the first imported Uriko's voice).
pub(crate) fn requests(h: &mut Br1Host, data: &Loaded, out: Vec<(u8, Request)>, frame: u64, counters: &Counters) {
    let stage = data.br1_stage.filter(|_| data.gates().br1_stage_sound).map(|s| s.g94);
    h.fx_log.retain(|(f, _, _)| f + bridge::FX_FRAMES > frame);
    let uriko = h.uriko();
    for (side, r) in out {
        match r {
            Request::Sound { id, delay } => {
                let id = Fid(side).bank().br1_sound(id);
                if id >= 0 {
                    h.queue(id, delay);
                }
            }
            Request::Call { call: Call::StageSound, args } if stage.is_some() => {
                // P1's block, then P2's
                h.queue(counters.stage_sound(stage.unwrap_or_default(), args[0] != 0, uriko), 0);
                h.fx_log.push((frame, side, r));
            }
            // op 0x17 sub 2's case 10 (a's top nibble 10) queues its sound 0x5F, from
            // simulation v18 (its effects are the presentation's)
            Request::Call { call: Call::NodeEffect, args } if (args[1] >> 12) & 0xF == 10 && data.gates().br1_call_sound => {
                h.queue(0x5F, 0);
                h.fx_log.push((frame, side, r));
            }
            r => h.fx_log.push((frame, side, r)),
        }
    }
}

/// Simulation v18: an imported stage's sounds of this frame into the imported game's queue (`h`; 32 slots):
/// its stage sound calls (`stage`: their argument, `stage::br1::Counters::stage_sound`) and its own ids (`raw`); without
/// an imported fighter (no queue, `h` None) they go to `Counters::sounds` for the session to play. The debris'
/// splashes are logged for the effects (`Br1Match::fx_log`) under `frame`.
pub(crate) fn stage_sounds(h: Option<Br1Host>, st: &mut Counters, g94: u8, stage: Vec<bool>, raw: Vec<i16>, splash: Vec<[i32; 3]>, frame: u64) {
    match h {
        Some(mut h) => {
            let uriko = h.uriko();
            for id in stage.into_iter().map(|n| st.stage_sound(g94, n, uriko)).chain(raw) {
                h.queue(id, 0);
            }
            for p in splash {
                h.fx_log.push((frame, 0, Request::Fx { call: Call::StillParticle, pos: p.map(|v| v as i16), args: [0x10, 0xE, 4, 0] }));
            }
        }
        None => {
            let ids: Vec<i16> = stage.into_iter().map(|n| st.stage_sound(g94, n, None)).chain(raw).collect();
            st.sounds.extend(ids);
        }
    }
}

/// The stage's ring as the imported game's records {hips, edge}: an imported stage's own two (`Loaded::br1_stage`,
/// simulation v16), else the main-game stage's box `r` for both (the main game has one table).
pub(crate) fn rings(data: &Loaded, r: &AreaRec) -> [Ring; 2] {
    if let Some(b) = &data.br1_stage {
        return [b.hips, b.edge];
    }
    let ring = Ring { w0: r.kind as u32 | (r.drop as u16 as u32) << 16, w1: r.w4 as u32 };
    [ring, ring]
}

/// The stage's walls `sw` into the imported game's walls state `w` (absent −1, indestructible 0x7FFF; the hit strength and
/// its lock); returns them as they came in (`walls_out`).
pub(crate) fn walls_in(sw: &crate::fight::stage::Walls, w: &mut Walls) -> Walls {
    for k in 0..4 {
        w.hp[k] = if !sw.present(k) {
            -1
        } else if sw.hp[k] < 0 {
            0x7FFF
        } else {
            sw.hp[k]
        };
        (w.hit[k], w.lock[k]) = (sw.pow[k], sw.st[k]);
    }
    w.w44 = sw.lock as i16;
    *w
}

/// What the imported game's code changed of its walls `w` since `walls_in` (`before`) back into the stage's `sw` (its
/// update breaks and shakes them).
pub(crate) fn walls_out(w: &Walls, before: Walls, sw: &mut crate::fight::stage::Walls) {
    for k in 0..4 {
        if w.hit[k] != before.hit[k] {
            sw.pow[k] = w.hit[k];
        }
        if w.lock[k] != before.lock[k] {
            sw.st[k] = w.lock[k];
        }
    }
    if w.w44 != before.w44 {
        sw.lock = w.w44 as u8;
    }
}

/// The imported game's code's context in a fight of either kind: the data `res` by place, the stream G `rand` (a copy the
/// caller writes back), the stage byte (`br1_stage_byte`), the round and the rounds needed (`round`), the walls
/// present by `walls`' hp, the requests `out`, the ring records `rings`.
pub(crate) fn ctx<'a>(
    res: [&'a Br1Res; 2],
    rand: &'a mut u8,
    data: &Loaded,
    round: (i8, u8),
    walls: &Walls,
    out: &'a mut Vec<(u8, Request)>,
    rings: Option<[Ring; 2]>,
) -> Ctx<'a> {
    Ctx { res, rand, stage: br1_stage_byte(data), round: round.0, walls: walls.hp.map(|h| h >= 0), need: round.1, g_ad: 0, g_e8: 0, out, trace: None, rings }
}

/// An imported throw's proxy `g`, just reset in the victim's place, made main-game victim `face`: its form
/// (the beast's model), posed and placed as the face (`physics::puppet` with the face's records), held (F.64
/// 0x800000) by `thrown_by` with its state as requested, the grab's request taken over.
pub(crate) fn pose_br1_ghost(g: &mut Br1Fighter, gres: &Br1Res, face: &Fighter, thrown_by: i16) {
    if face.form & 1 != 0 {
        g.f.form = face.form;
        script::model_switch(g, 1);
    }
    let recs = bridge::recs_of(face);
    physics::puppet(g, gres, &recs, face.pose[1].stretch, face.pose[4].stretch, face.pos, face.m_t, face.yaw, face.yaw_base);
    g.f.f64 |= 0x80_0000;
    g.s.thrown_by = thrown_by;
    g.s.state_req = g.s.state_cur;
}

/// A main-game throw grabbed imported core `f`'s face `face` (F6C 0x800000, `s_2c`): the victim's
/// main-game proxy made from the face (`bridge::br2_ghost`, `ani` the victim's script file), the core's grab flag
/// (F.64 0x800000) cleared and published.
pub(crate) fn br2_ghost(f: &mut Br1Fighter, face: &mut Fighter, ani: &Ani) -> Fighter {
    let g = bridge::br2_ghost(f, face, ani);
    f.f.f64 &= !0x80_0000;
    bridge::publish(f, face);
    g
}

/// A main-game throw's proxy `g` sees its victim's skeleton (`face`'s nodes and parts: the thrower's scripts read
/// them) before it goes into the victim's slot.
pub(crate) fn proxy_sees(g: &mut Fighter, face: &Fighter) {
    (g.parts, g.part_rots, g.head, g.chest, g.hips, g.head_w) = (face.parts.clone(), face.part_rots.clone(), face.head, face.chest, face.hips, face.head_w);
}

/// A main-game throw's proxy `g` (in the victim's slot): its queued damage and gauge change moved onto the
/// victim's face `face` kept in the throw (the proxy is never KO'd), its last damage kept as `hit::health` keeps it
/// (op 19 subs 0 / 1 take 3/10, 1/10 of it).
pub(crate) fn br2_proxy_damage(g: &mut Fighter, face: &mut Fighter) {
    face.o_c8 = face.o_c8.wrapping_add(g.o_c8);
    face.o_d8 = face.o_d8.wrapping_add(g.o_d8);
    if g.o_c8 != 0 {
        g.o_ca = g.o_c8.wrapping_abs();
    }
    (g.o_c8, g.o_d8, g.o_ac) = (0, 0, 0);
}

/// Imported core `f` (data `res`) posed and placed from the main-game throw's proxy `g` (`physics::puppet`
/// with the proxy's records).
pub(crate) fn puppet_from(f: &mut Br1Fighter, res: &Br1Res, g: &Fighter) {
    physics::puppet(f, res, &bridge::recs_of(g), g.pose[1].stretch, g.pose[4].stretch, g.pos, g.m_t, g.yaw, g.yaw_base);
}

/// Imported core `f` (data `res`) let go out of a throw: F.64 0x860000 and F.50 0x100000 cleared and state
/// `state` requested and started (its script is the caller's to run once, as the round-start reset does).
pub(crate) fn release(f: &mut Br1Fighter, res: &Br1Res, state: i16) {
    f.f.f64 &= !0x86_0000;
    f.f.f50 &= !0x10_0000;
    (f.s.state_req, f.s.state_cur) = (state, state);
    script::start_state(f, res, state);
}

/// Main-game attacker `a`'s test on imported fighter `d`'s face: `hit::can_hit`, the
/// direction bits onto `d`'s face (F68), the multiplier, the sphere test against its core's hurt spheres `hurt` at
/// `d`'s nodes.
pub(crate) fn probe_on_core(fs: &mut [Fighter], a: usize, d: usize, nodes: &[Vec<[i32; 3]>], hurt: &[Sphere]) -> Option<Probe> {
    if !h2::can_hit(&fs[a], &fs[d]) {
        return None;
    }
    let dir = h2::direction_bits(&fs[a], &fs[d]);
    fs[d].flags[F68] = dir;
    let (fa, fd) = pair_mut(fs, a, d);
    let mult = h2::multiplier(fa, fd);
    let dmg = (fa.o_da as i32 * mult) >> 7;
    let (kind, pos) = h2::sphere_test(fa, fd, &nodes[a], &nodes[d], hurt);
    Some(Probe { kind, dmg, mult, pos })
}

/// Imported attacker `me`'s test in its pair `fs` (the other place its view): `can_hit` (it clears
/// the attacker's F.54 0x14 whatever its result), the direction bits, the multiplier, the sphere test against the
/// view's hurt spheres `hurt`; the damage in the main game's units (× 200 / 187) for the trade. Returns the probe
/// (no main-game contact point), the direction bits and the imported game's contact point.
pub(crate) fn probe_core(fs: &mut [Br1Fighter; 2], me: usize, hurt: &[Sphere]) -> Option<(Probe, u32, [i16; 3])> {
    if !hit::can_hit(fs, me) {
        return None;
    }
    let dir = hit::direction_bits(fs, me, G98);
    let mult = hit::multiplier(fs, me, dir);
    let p = fs[me].f.attack_damage as i32 * mult;
    let dmg = (if p < 0 { p + 0x7F } else { p }) >> 7;
    let (kind, pos) = hit::sphere_test(fs, me, hurt, G98);
    Some((Probe { kind, dmg: bridge::scale_damage(dmg as i16, bridge::BR1_HP) as i32, mult, pos: [0; 3] }, dir, pos))
}

/// Main-game attacker `a`'s hit on imported fighter `d`'s face (`hit::apply`, its multiplier and kind) with
/// the imported game's guard gate (its core's F.54 `f54`: 0x2000 as F5C 1, F5C bit 0 back to F.54's after); from
/// simulation v25 a knock-down without a launch of its own launches the core (`bridge::launch`).
pub(crate) fn land_on_face(fs: &mut [Fighter], a: usize, d: usize, f54: u32, mult: i32, kind: i32, data: &Loaded) -> h2::HitResult {
    fs[d].flags[F5C] = (f54 & !1) | (f54 & 0x2000 != 0) as u32;
    let (fa, fd) = pair_mut(fs, a, d);
    let r = h2::apply(fa, fd, mult, kind);
    if let Some(l) = bridge::launch(&r, fd.flags[F68], data.gates()) {
        h2::launch(fa, fd, l, &data.trig);
    }
    fs[d].flags[F5C] = (fs[d].flags[F5C] & !1) | (f54 & 1);
    r
}
