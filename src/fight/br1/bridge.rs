//! The bridge between imported fighters and a main-game match (engine addition, the import S7): the
//! fighters' match state, the main-game face every main-game system reads (`publish`), the face's writes
//! taken back (`absorb`), the health and gauge by the main game's rules (`health`), the proxy characters.
//!
//! State: serialised (`Br1Match`, part of `fight::sim::Match`)
//!
//! A side whose fighter is an imported one runs its `Br1Fighter` (`fs`) with the imported game's code; its
//! `fight::fighter::Fighter` in `Match::fighters` is the face: written from the core after the core's part of the frame
//! (`publish`), read by the round flow, the camera, the HUD, the effects and the session; what those write into it
//! (the round flow's flags and moves, the debug cheats) goes back into the core at the next frame's start (`absorb`).
//! The health, the gauge and the KO are the face's (the main game's rules, `hit::health`): the core's queued damage
//! and gauge changes are moved into the face in the main game's units (the damage × 200 / the defender's
//! imported-game max HP) and the face's health is written back into the core in the imported game's units.

use super::object::Br1Fighter;
use super::physics::Walls;
use super::res::Br1Res;
use super::script::{Call, Request};
use crate::fight::fighter::{F5C, F6C, F54, F58, F60, F64, F68, Fighter};
use serde::{Deserialize, Serialize};

/// The launch of the main game's hit result `r` on an imported face whose result word is `f68` (engine addition,
/// simulation v25 `version::v::BR1_LAUNCH`): a knock-down main-game leaves to its victim's reaction move (F68 0x4000 /
/// 0x10000 / 0x2000 / 0x40000 without the "big" 0x800000: `hit::launch_kind` None) is a launched reaction in the
/// imported game (0x5B, 0x55 / 0x56 / 0x60, 0x54 / 0x57), which falls by F.88 / F.8A alone (the imported game's apply always sets them):
/// it takes the launch of its big form, else the core hangs in the air with both 0. Before
/// v25, and for a guarded hit, the main game's own.
pub fn launch(r: &crate::fight::hit::HitResult, f68: u32, g: crate::fight::version::Gates) -> Option<u8> {
    match r.launch {
        None if g.br1_launch && !r.guarded => crate::fight::hit::launch_kind(f68 | 0x80_0000),
        l => l,
    }
}

/// An imported fighter's gauge `g` as the main game's HUD shows it (engine addition, simulation v16): in human
/// form (`form` without 0x10) the imported game can transform from 0xF00 where the main game's bar marks 0x1E00, so 0 ..
/// 0xF00 fills the bar's human range 0 .. 0x1E00 and 0xF00 .. 0x3C00 its full range 0x1E00 .. 0x3C00 (`(g − 0xF00) ·
/// 0x1E00 / 0x2D00`, toward zero); the beast form's gauge (its life, the same 0x3BFF cap) as it is.
pub fn hud_gauge(g: i16, form: u8) -> i16 {
    if form & 0x10 != 0 {
        return g;
    }
    let g = g as i32;
    let v = if g < 0xF00 { g * 2 } else { 0x1E00 + (g - 0xF00) * 0x1E00 / 0x2D00 };
    v as i16
}

/// The main game's standard max HP, the scale of the damage an imported fighter takes or deals (a user
/// decision: the imported game's damage × 200 / 187, Uriko's × 200 / 150).
pub const HP: i32 = 200;

/// The face's F6C bits an imported fighter may show: never the Beast Drive (0x8000; a user decision, no Beast
/// Drive for imported fighters).
pub const F6C_SHOWN: u32 = !0x8000;

/// The main-game character standing in for an imported one in the main game's per-opponent tables (the
/// face's `chr`; engine-made, by build and kind: Mitsuko → Shina, Fox → Jenny, Greg → Gado, Uriko → Uriko).
pub fn br2_proxy(br1_chr: u8) -> u8 {
    match br1_chr {
        4 => 4,
        6 => 7,
        1 => 9,
        8 => 3,
        // the fighters not imported: their main-game selves (ZIN Yugo, EVE Alice, VAK Bakuryu, GAD Gado, RON Long)
        0 => 0,
        2 => 1,
        3 => 6,
        5 => 9,
        7 => 2,
        _ => 0,
    }
}

/// The imported character standing in for a main-game one in the imported game's per-opponent tables (engine-made:
/// the same fighter where the imported game has one, else by build and kind).
pub fn br1_proxy(br2_chr: u8) -> u8 {
    match br2_chr {
        0 => 0,  // YUG → ZIN
        1 => 2,  // ALI → EVE
        2 => 7,  // LON → RON
        3 => 8,  // ULI → URI
        4 => 4,  // MAR → MIT
        5 => 3,  // BUS → VAK
        6 => 3,  // BAK → VAK
        7 => 6,  // JEN → FOX
        8 => 1,  // STU → GRE
        9 => 5,  // GAD → GAD
        10 => 7, // RON → RON
        _ => 0,
    }
}

/// The imported fighters of a match and what the imported game's frame keeps besides them.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Br1Match {
    /// the sides whose fighter is an imported one
    pub side: [bool; 2],
    /// the cores by side (a main-game side's entry is unused)
    pub fs: [Br1Fighter; 2],
    /// the ring walls' state of the imported game's code (the stage block + 0x44)
    pub walls: Walls,
    /// the gameplay random stream G
    pub rand: u8,
    /// the sound request queue (id, delay) and the sounds started this frame (≤ 1, the queue's 0x400
    /// kind): the imported game's sound ids
    pub sound_queue: Vec<(i16, i16)>,
    pub sounds_started: Vec<i16>,
    /// per side, the core's damage, permanent damage and blue bar as `health` last wrote them (a change since is the
    /// core's own: a revert's F.A6 = F.A4, a ring-out's −1)
    pub synced: [[i16; 3]; 2],
    /// a throw across the engines running (engine addition: the victim puppeted by a proxy of the thrower's engine)
    #[serde(default)]
    pub throw: Option<CrossThrow>,
    /// engine addition (the import S10, simulation v20): the AI blocks by side (`*(F.34)`; a main-game side's
    /// entry is unused) and their random table
    #[serde(default)]
    pub ai: [super::ai::Br1Ai; 2],
    #[serde(default)]
    pub ai_rnd: super::ai::Table,
    /// the side whose face slot holds a main-game throw's proxy for the moment (within one frame's
    /// `fight::sim::br1` step only, never between frames): its body is the main game's there
    #[serde(skip)]
    pub swapped: Option<u8>,
    /// engine addition (the import S8): the effect, camera and presentation requests of the imported game's code with
    /// their frame and side, the last `FX_FRAMES` frames' (`fight::sim::br1`), for the presentation to draw
    /// (`present::br1_fx`); not part of the state (a decoded or restored state has none)
    #[serde(skip)]
    pub fx_log: Vec<(u64, u8, super::script::Request)>,
}

/// The frames of requests `Br1Match::fx_log` keeps (more than a drawn frame ever steps).
pub const FX_FRAMES: u64 = 16;

/// A throw across the engines (engine-made choreography, a user decision): the thrower's engine plays the throw on a
/// proxy of the victim (`ghost`: the victim's imported proxy for an imported thrower, its main-game
/// proxy for a main-game thrower), whose own script for that throw (its victim row for the thrower's character)
/// plays the thrower's clips, takes the damage and makes the sounds; each frame its pose (the nodes' direction and
/// twist records, nodes 1 / 4's translations) and position are the real victim's, until it reaches a state both
/// games share after a throw (`victim_done`) and the victim goes on in that state in its own engine.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CrossThrow {
    /// the victim's side
    pub victim: u8,
    pub ghost: Ghost,
    /// frames puppeted
    pub frames: u16,
}

impl CrossThrow {
    /// The state the victim goes on in after this frame (the proxy's `state`), None while the throw runs.
    pub fn handover(&mut self, state: i16) -> Option<i16> {
        self.frames = self.frames.saturating_add(1);
        victim_done(state).or((self.frames >= THROW_FRAMES).then_some(0x67))
    }
}

/// Engine addition (the team battle, T5): the throws across the engines of a team battle,
/// keyed by thrower, in thrower order. A fighter is in at most one of them (as the thrower or the victim).
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct CrossThrows(Vec<(u8, CrossThrow)>);

impl CrossThrows {
    /// Thrower `k`'s throw starts (a throw of `k` or on its victim before is dropped: a fighter is in one throw).
    pub fn start(&mut self, k: usize, t: CrossThrow) {
        let v = t.victim as usize;
        self.0.retain(|(a, x)| ![*a as usize, x.victim as usize].iter().any(|&j| j == k || j == v));
        let at = self.0.partition_point(|(a, _)| (*a as usize) < k);
        self.0.insert(at, (k as u8, t));
    }

    /// Thrower `k`'s throw.
    pub fn of(&self, k: usize) -> Option<&CrossThrow> {
        self.0.iter().find(|(a, _)| *a as usize == k).map(|(_, t)| t)
    }

    pub fn of_mut(&mut self, k: usize) -> Option<&mut CrossThrow> {
        self.0.iter_mut().find(|(a, _)| *a as usize == k).map(|(_, t)| t)
    }

    /// The thrower whose throw has `v` as its victim.
    pub fn thrower_of(&self, v: usize) -> Option<usize> {
        self.0.iter().find(|(_, t)| t.victim as usize == v).map(|(a, _)| *a as usize)
    }

    /// Fighter `k` is in one of the throws (the thrower or the victim).
    pub fn involves(&self, k: usize) -> bool {
        self.of(k).is_some() || self.thrower_of(k).is_some()
    }

    /// Thrower `k`'s throw ends (None: it has none).
    pub fn end(&mut self, k: usize) -> Option<CrossThrow> {
        let i = self.0.iter().position(|(a, _)| *a as usize == k)?;
        Some(self.0.remove(i).1)
    }

    pub fn clear(&mut self) {
        self.0.clear();
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// The throwers, in order.
    pub fn throwers(&self) -> Vec<usize> {
        self.0.iter().map(|(a, _)| *a as usize).collect()
    }

    /// The (thrower, throw) pairs, in thrower order.
    pub fn iter(&self) -> impl Iterator<Item = (usize, &CrossThrow)> {
        self.0.iter().map(|(a, t)| (*a as usize, t))
    }

    /// A decoded list fits `n` fighters: in thrower order, every thrower and victim below `n`, no fighter in two throws
    /// or throwing itself.
    pub fn check(&self, n: usize) -> Result<(), String> {
        let mut seen = vec![false; n];
        let mut last = None;
        for (a, t) in self.iter() {
            let v = t.victim as usize;
            if a >= n || v >= n || a == v || seen[a] || seen[v] || last.is_some_and(|l| l >= a) {
                return Err("cross-engine throws".into());
            }
            (seen[a], seen[v], last) = (true, true, Some(a));
        }
        Ok(())
    }
}

/// The proxy victim of a `CrossThrow`, of the thrower's engine.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum Ghost {
    Br1(Box<Br1Fighter>),
    Br2(Box<Fighter>),
}

/// The states both games share after a throw (imported and main-game victims go on through 0x67 lying, 0x71 getting up, 1
/// standing; the imported game's 0x66 fall and every other state of a victim script, whose numbers differ between the engines,
/// stay the proxy's): the throw is over when the proxy is in one of them, or after `THROW_FRAMES` (then lying).
pub fn victim_done(state: i16) -> Option<i16> {
    matches!(state, 0x67 | 0x71 | 1).then_some(state)
}

/// The longest a `CrossThrow` puppets its victim (a proxy that never reaches a shared state hands it over lying).
pub const THROW_FRAMES: u16 = 600;

impl Br1Match {
    pub fn both(&self) -> bool {
        self.side[0] && self.side[1]
    }
}

/// The imported game's units of a face value `v` (the main game's units of max `max`): × the core's max HP / `max`; −1 stays.
fn to_br1(v: i16, max: i16, br1_max: i16) -> i16 {
    if v < 0 {
        return -1;
    }
    (v as i32 * br1_max as i32 / (max as i32).max(1)) as i16
}

/// The main game's units of a core value `v`: × `max` / the core's max HP; −1 stays.
fn to_br2(v: i16, max: i16, br1_max: i16) -> i16 {
    if v < 0 {
        return -1;
    }
    (v as i32 * max as i32 / (br1_max as i32).max(1)) as i16
}

/// The damage `d` of an imported attack in the main game's units against a defender of imported max HP
/// `br1_max` (C division, the sign kept: a negative `d` is chip damage).
pub fn scale_damage(d: i16, br1_max: i16) -> i16 {
    (d as i32 * HP / (br1_max as i32).max(1)) as i16
}

/// The face's fields of core `f`: what the round flow, the camera, the HUD, the effects and the session read. The
/// health (`o_ac`, `o_ae`, `o_aa`, `o_b0`, `o_c8`, `o_ca`), the gauge's queue (`o_d8`, `o_fa`) and `o_e5` are the
/// face's own and stay.
pub fn publish(f: &Br1Fighter, face: &mut Fighter) {
    let x = &f.f;
    face.o_00 = x.status;
    face.o_01 = x.cpu;
    face.player = x.player;
    face.form = x.form;
    face.flags[F54] = x.f4c;
    face.flags[F58] = x.f50;
    face.flags[F5C] = x.f54;
    face.flags[F60] = x.f58;
    face.flags[F64] = x.f5c;
    face.flags[F68] = x.f60;
    face.flags[F6C] = x.f64 & F6C_SHOWN;
    face.gauge = x.gauge;
    face.o_24 = x.weight;
    face.o_26 = x.attack;
    face.o_27 = x.hittable;
    (face.o_4c, face.yaw, face.o_50) = (x.pitch, x.yaw, x.roll);
    (face.o_70, face.o_72, face.o_74, face.o_76, face.o_78) = (x.centre_angle, x.centre_dist, x.opp_angle, x.opp_dist, x.opp_dy);
    (face.pressed, face.held, face.prev, face.recent, face.dbl, face.long) = (x.pressed, x.held, x.prev, x.recent, x.dbl, x.long);
    face.o_86 = x.step_normal;
    face.step = x.step;
    face.o_fc = x.step_normal;
    (face.o_8a, face.o_8c, face.o_8e) = (x.v_angle, x.v_speed, x.v_decay);
    (face.o_90, face.o_92, face.o_94, face.o_96, face.o_98) = (x.vy, x.gravity, x.floor, x.launch_angle, x.launch_speed);
    face.vel = x.vel;
    (face.o_b2, face.o_b4, face.o_b8, face.o_ba, face.o_bc) = (x.push_speed, x.push_decay, x.active, x.startup, x.recovery);
    (face.o_be, face.o_c0, face.o_c4, face.o_c6) = (x.guard_stun, x.down_timer, x.knock_dir, x.knock_angle);
    face.o_cc = x.hit_stun;
    (face.o_ce, face.o_d0, face.o_d2) = (x.recover_timer, x.no_throw, x.fc8);
    (face.o_da, face.o_db, face.o_dd) = (x.attack_damage, x.knock_strength, x.combo);
    // the attack and defence factors: the imported game has none (identity)
    (face.o_fe, face.o_100, face.o_102, face.o_104) = (0x80, 0x80, 0x80, 0x80);
    let s = &f.s;
    face.s_14 = s.attack_index;
    face.s_2c = s.thrown_by;
    face.s_3a = s.damage_sum;
    (face.chain_pos, face.chain_len, face.chain_sel) = (s.chain_pos, s.chain_len, s.chain_sel as i8);
    face.move_req = s.state_req;
    face.move_cur = s.state_cur;
    face.sleep = s.sleep as i32;
    face.res = s.res;
    face.pc = f.pc as usize;
    face.loop_mark = f.loop_mark as usize;
    face.ret = f.ret as usize;
    let a = &f.anim;
    face.yaw_base = a.yaw_base;
    face.s_d2 = a.yaw;
    face.pos = a.pos;
    face.m_pos = a.pos;
    [face.s_dc, face.s_de, face.s_e0] = a.root;
    face.s_e4 = a.turn;
    face.anim.mirror = a.mirror;
    // no main-game clip (the drawing's bind pose until S8 poses the model from the core)
    face.anim.id = -1;
    face.anim.left = a.left;
    face.anim.speed = a.speed;
    face.head_yaw = a.head_yaw;
    face.head_pitch = a.head_pitch;
    // the skeleton: node n's world translation and rotation (the camera's bones, the effects' bone matrices)
    let md = &f.form().md;
    face.parts = md.iter().map(|d| d.wt).collect();
    face.part_rots = md.iter().map(|d| d.wm).collect();
    face.m_t = f.world(0);
    face.head = f.world(2);
    face.head_w = f.world(2);
    face.chest = f.world(3);
    face.hips = f.world(4);
    face.prev_root = face.m_pos;
    face.pose_set = false;
}

/// The face's writes since the last `publish` (the round flow's flags, moves, facing and CPU flag, the debug
/// cheats; in a mixed pair the main game's combo count and angles), taken into core `f`: every field `publish`
/// writes that now differs from what it would write.
pub fn absorb(f: &mut Br1Fighter, face: &Fighter) {
    let mut p = face.clone();
    publish(f, &mut p);
    let x = &mut f.f;
    if face.o_00 != p.o_00 {
        x.status = face.o_00;
    }
    if face.o_01 != p.o_01 {
        x.cpu = face.o_01;
    }
    for (k, w) in [(F54, &mut x.f4c), (F58, &mut x.f50), (F5C, &mut x.f54), (F60, &mut x.f58), (F64, &mut x.f5c), (F68, &mut x.f60)] {
        if face.flags[k] != p.flags[k] {
            *w = face.flags[k];
        }
    }
    if face.flags[F6C] != p.flags[F6C] {
        x.f64 = (x.f64 & !F6C_SHOWN) | (face.flags[F6C] & F6C_SHOWN);
    }
    if face.gauge != p.gauge {
        x.gauge = face.gauge;
    }
    if face.o_c0 != p.o_c0 {
        x.down_timer = face.o_c0;
    }
    if face.o_dd != p.o_dd {
        x.combo = face.o_dd;
    }
    for (a, b, w) in [
        (face.o_70, p.o_70, &mut x.centre_angle),
        (face.o_72, p.o_72, &mut x.centre_dist),
        (face.o_74, p.o_74, &mut x.opp_angle),
        (face.o_76, p.o_76, &mut x.opp_dist),
        (face.o_78, p.o_78, &mut x.opp_dy),
    ] {
        if a != b {
            *w = a;
        }
    }
    if face.yaw != p.yaw {
        x.yaw = face.yaw;
    }
    if face.yaw_base != p.yaw_base {
        f.anim.yaw_base = face.yaw_base;
    }
    if face.move_req != p.move_req {
        f.s.state_req = face.move_req;
    }
    if face.move_cur != p.move_cur {
        f.s.state_cur = face.move_cur;
    }
    if face.form != p.form {
        // the debug transform cheat: the form's lists follow
        f.f.form = face.form;
        super::script::model_switch(f, face.form as usize & 1);
    }
}

/// An imported fighter's health step by the main game's rules: the core's queued damage (× 200 / the core's max HP) and gauge
/// change moved into the face, the main game's `hit::health` run on it (gauge, red health, blue bar, KO), the
/// result written back into the core in its units; on a KO the core's own tail (F.54 0x200000, the KO
/// flash, the down timer, the rounds lost). `synced` = the side's `Br1Match::synced`. Returns whether this frame
/// KO'd the fighter. `charge` / `recover` = the custom options' (`rules::Custom`).
pub fn health(f: &mut Br1Fighter, me: usize, face: &mut Fighter, synced: &mut [i16; 3], charge: i16, recover: u8, out: &mut Vec<(u8, Request)>) -> bool {
    let x = &mut f.f;
    let max = face.o_b0;
    // the core's own changes since the last write-back (a revert's F.A6 = F.A4, a ring-out's −1)
    if [x.damage, x.damage_perm, x.blue] != *synced {
        if x.damage != synced[0] {
            face.o_ac = to_br2(x.damage, max, x.max_hp);
        }
        if x.damage_perm != synced[1] {
            face.o_ae = if x.damage_perm == x.damage { face.o_ac } else { to_br2(x.damage_perm, max, x.max_hp) };
        }
        if x.blue != synced[2] {
            face.o_aa = to_br2(x.blue, max, x.max_hp);
        }
    }
    face.o_c8 = face.o_c8.wrapping_add(scale_damage(x.queued_damage, x.max_hp));
    x.queued_damage = 0;
    face.o_d8 = face.o_d8.wrapping_add(x.gauge_queued);
    x.gauge_queued = 0;
    (face.gauge, face.form, face.o_00) = (x.gauge, x.form, x.status);
    (face.flags[F54], face.flags[F58], face.flags[F60]) = (x.f4c, x.f50, x.f58);
    face.flags[F6C] = x.f64 & F6C_SHOWN;
    let rate = regen_rate(recover, x.regen_interval);
    let was = face.o_00;
    crate::fight::hit::health(face, charge, rate);
    x.gauge = face.gauge;
    x.damage = to_br1(face.o_ac, max, x.max_hp);
    x.damage_perm = to_br1(face.o_ae, max, x.max_hp);
    x.blue = to_br1(face.o_aa, max, x.max_hp);
    x.last_damage = to_br1(face.o_ca, max, x.max_hp);
    x.status = face.o_00;
    *synced = [x.damage, x.damage_perm, x.blue];
    if !(was == 1 && face.o_00 & 0x80 != 0) {
        return false;
    }
    // the KO (the face's health KO'd it; the main game's KO rule, no guts save)
    x.f54 |= 0x20_0000;
    out.push((me as u8, Request::Call { call: Call::BackgroundFlash, args: [4, 0xFF, 0xFF, 0xFF, 0, 0] }));
    x.down_timer = -1;
    x.rounds_lost = x.rounds_lost.wrapping_add(1);
    true
}

/// The red health's rate of an imported fighter at RECOVER SPEED `recover` (`health`): the main game's
/// (the start of a fighter's step, RECOVER SPEED << 8 / ANI param 4 + 1, none at 0) with ANI param 4 the core's period
/// `interval` (F.D1: F.D1 + 1 frames a point, the health step counting it down to 0, then a point).
pub fn regen_rate(recover: u8, interval: u8) -> u16 {
    let r = recover as i32;
    if r == 0 { 0 } else { ((r << 8) / (interval as i32 + 1) + 1) as u16 }
}

/// The imported game's standard max HP (250 × the vitality factor 75 %): a main-game fighter's in the imported
/// view (`shadow`).
pub const BR1_HP: i16 = 187;

/// The imported view of a main-game fighter `face` (engine addition): the fields the imported game's code reads
/// of its opponent (the flag words by their shared meanings, the stance, the facing and position, the timers, the
/// skeleton's nodes from `face.parts`), in the slot of a mixed pair's main-game side. The character is the
/// imported proxy (`br1_proxy`), the max HP the imported game's standard.
pub fn shadow(face: &Fighter) -> Br1Fighter {
    let mut f = Br1Fighter::default();
    let x = &mut f.f;
    (x.status, x.cpu, x.player, x.chr, x.form) = (face.o_00, face.o_01, face.player, br1_proxy(face.chr), face.form);
    (x.f4c, x.f50, x.f58, x.f5c, x.f60) = (face.flags[F54], face.flags[F58], face.flags[F60], face.flags[F64], face.flags[F68]);
    // The imported game's guard gate is F.54 0x2000 where the main game's is F5C 1
    x.f54 = (face.flags[F5C] & !0x2000) | if face.flags[F5C] & 1 != 0 { 0x2000 } else { 0 };
    x.f64 = face.flags[F6C];
    (x.weight, x.attack, x.hittable) = (face.o_24, face.o_26, face.o_27);
    (x.pitch, x.yaw, x.roll) = (face.o_4c, face.yaw, face.o_50);
    (x.centre_angle, x.centre_dist, x.opp_angle, x.opp_dist, x.opp_dy) = (face.o_70, face.o_72, face.o_74, face.o_76, face.o_78);
    (x.pressed, x.held, x.prev, x.recent, x.dbl, x.long) = (face.pressed, face.held, face.prev, face.recent, face.dbl, face.long);
    (x.step_normal, x.step) = (face.o_86, face.step);
    (x.v_angle, x.v_speed, x.v_decay) = (face.o_8a, face.o_8c, face.o_8e);
    (x.vy, x.gravity, x.floor, x.launch_angle, x.launch_speed) = (face.o_90, face.o_92, face.o_94, face.o_96, face.o_98);
    x.vel = face.vel;
    (x.push_speed, x.push_decay, x.active, x.startup, x.recovery) = (face.o_b2, face.o_b4, face.o_b8, face.o_ba, face.o_bc);
    (x.guard_stun, x.down_timer, x.knock_dir, x.knock_angle, x.hit_stun) = (face.o_be, face.o_c0, face.o_c4, face.o_c6, face.o_cc);
    (x.recover_timer, x.no_throw, x.fc8) = (face.o_ce, face.o_d0, face.o_d2);
    (x.gauge, x.attack_damage, x.knock_strength, x.combo) = (face.gauge, face.o_da, face.o_db, face.o_dd);
    x.max_hp = BR1_HP;
    x.damage = to_br1(face.o_ac, face.o_b0, BR1_HP);
    (f.s.state_req, f.s.state_cur, f.s.attack_index) = (face.move_req, face.move_cur, face.s_14);
    f.anim.pos = face.pos;
    f.anim.yaw_base = face.yaw_base;
    f.anim.mirror = face.anim.mirror;
    f.cur = face.form & 1;
    let md = face.parts.iter().zip(face.part_rots.iter().chain(std::iter::repeat(&super::pose::IDENTITY))).map(|(&wt, &wm)| super::object::Node {
        lm: super::pose::IDENTITY,
        lt: [0; 3],
        wm,
        wt,
    });
    let form = super::object::Form { rec: Vec::new(), md: md.collect() };
    f.objs[f.cur as usize] = [face.head, face.hips];
    f.forms[f.cur as usize] = form;
    f
}

/// The hit the imported game's `hit::apply` gave the main-game defender's view `d` (`shadow`), onto its face (engine
/// addition, the hit exchange's "imported attacker → main-game defender"): the hit result word without the
/// bits only the imported game has, the damage × 200 / 187, the stun and guard stun, the push, the launch, the down
/// timer, the hit flags, the gauge change; the hit-strength class `o_c2` from the stun as main-game derives it.
pub fn give_hit(d: &Br1Fighter, face: &mut Fighter) {
    let x = &d.f;
    face.flags[F68] = x.f60 & !(0x10 | 0x20 | 0x80 | 0x2_0000 | 0x80_0000);
    face.o_c8 = face.o_c8.wrapping_add(scale_damage(x.queued_damage, BR1_HP));
    face.o_cc = x.hit_stun;
    // the stun's class as the main game's hit reactions derive it (o_cc = (o_c2 − 5) · 16, then the class steps)
    let c2 = (x.hit_stun >> 4) + 5;
    face.o_c2 = if c2 >= 0x1B {
        c2 - 0x1A
    } else if c2 >= 0x16 {
        c2 - 0x15
    } else {
        c2 - 0x10
    };
    face.o_be = x.guard_stun;
    (face.o_c4, face.o_b2, face.o_b4) = (x.knock_dir, x.push_speed, x.push_decay);
    (face.o_90, face.o_92, face.o_98) = (x.vy, x.gravity, x.launch_speed);
    face.o_c0 = x.down_timer;
    face.flags[F5C] = (face.flags[F5C] & 0xF9FF_FFFF) | (x.f54 & 0x0600_0000);
    face.flags[F5C] |= x.f54 & 0x6000_0000;
    face.o_d8 = face.o_d8.wrapping_add(x.gauge_queued);
}

/// The bits of the main game's hit result word (F68) that the imported game's F.60 means the same.
pub const COMMON: u32 = 0xF | 0x70_0000 | 0x340 | 0x400 | 0x800 | 0x1000 | 0x2000 | 0x4000 | 0x8000 | 0x1_0000 | 0x4_0000 | 0x8_0000 | 0x80_0000 | 0x200_0000;

/// The hit the main game's `hit::apply` gave an imported fighter's face, into its core `f` (engine addition, the
/// hit exchange's "main-game attacker → imported defender"): the result word's common bits, the stun and
/// guard stun, the push, the launch, the down timer, the hit flags (the damage and the gauge stay on the face, whose
/// health is the main game's).
pub fn take_hit(f: &mut Br1Fighter, face: &Fighter) {
    let x = &mut f.f;
    x.f60 = face.flags[F68] & COMMON;
    x.hit_stun = face.o_cc;
    x.guard_stun = face.o_be;
    (x.knock_dir, x.push_speed, x.push_decay) = (face.o_c4, face.o_b2, face.o_b4);
    (x.vy, x.gravity, x.launch_speed) = (face.o_90, face.o_92, face.o_98);
    x.down_timer = face.o_c0;
    x.f54 = face.flags[F5C];
}

/// A main-game face's pose as the imported game's records (the same direction / twist encoding; nodes 1..18).
pub fn recs_of(face: &Fighter) -> Vec<super::pose::Rec> {
    face.pose.iter().map(|p| super::pose::Rec { dir: p.dir, twist: p.twist }).collect()
}

/// The proxy victim of a main-game throw on imported fighter `f` (`face` its face, `ani` the
/// main-game proxy's script file, `fight_data::Loaded::fighters`): the face with the proxy's values as a round start loads
/// them (weight, attack / defence factors, the beast's step, HP), full health, its pose the core's,
/// the grab's request (F6C 0x800000, `s_2c`) kept so that its decision takes its victim row for the thrower.
pub fn br2_ghost(f: &Br1Fighter, face: &Fighter, ani: &crate::formats::act_ani::Ani) -> Fighter {
    let mut g = face.clone();
    g.o_24 = ani.param(2 + g.beast() as usize);
    (g.o_fc, g.o_fe, g.o_100, g.o_102, g.o_104) = (ani.param(7), ani.param(8), ani.param(9), ani.param(10), ani.param(11));
    (g.o_b0, g.o_ac, g.o_ae, g.o_aa, g.o_c8, g.o_d8) = (ani.param(5), 0, 0, 0, 0, 0);
    g.hit_spheres.clear();
    g.anim = Default::default();
    g.anim.id = -1;
    g.move_cur = -1;
    let rec = &f.form().rec;
    for (n, p) in g.pose.iter_mut().enumerate() {
        if let Some(r) = rec.get(n) {
            (p.dir, p.twist) = (r.dir, r.twist);
        }
        p.stretch = match n {
            1 => f.anim.t1,
            4 => f.anim.t4,
            _ => [0; 3],
        };
    }
    g.pose_set = true;
    g
}

/// Main-game victim `face` posed by the proxy `g` of an imported throw: its records (nodes 1..18, nodes 1 /
/// 4's translations as the stretch), its position, model translation and facing; F58 0x100000 (being thrown).
pub fn pose_face(face: &mut Fighter, g: &Br1Fighter) {
    let rec = &g.form().rec;
    for (n, p) in face.pose.iter_mut().enumerate() {
        if let Some(r) = rec.get(n) {
            (p.dir, p.twist) = (r.dir, r.twist);
        }
        p.stretch = match n {
            1 => g.anim.t1,
            4 => g.anim.t4,
            _ => [0; 3],
        };
    }
    face.pose_set = true;
    face.pos = g.anim.pos;
    face.m_t = g.world(0);
    face.m_pos = face.m_t.map(|v| v as i16);
    face.prev_root = face.m_pos;
    (face.yaw, face.yaw_base, face.s_d2, face.s_e4) = (g.f.yaw, g.anim.yaw_base, 0, 0);
    [face.s_dc, face.s_de, face.s_e0] = [0; 3];
    face.vel = g.f.vel;
    face.flags[F58] |= 0x10_0000;
}

/// The face's health written into core `f` in its units (`synced` as `health` keeps it): after a round start.
pub fn write_health(f: &mut Br1Fighter, face: &Fighter, synced: &mut [i16; 3]) {
    let x = &mut f.f;
    let max = face.o_b0;
    x.damage = to_br1(face.o_ac, max, x.max_hp);
    x.damage_perm = to_br1(face.o_ae, max, x.max_hp);
    x.blue = to_br1(face.o_aa, max, x.max_hp);
    *synced = [x.damage, x.damage_perm, x.blue];
}

/// A decoded core fits its data: the forms' lists as long as the ANM's node counts, the current form 0 / 1, its player
/// (F.02) P1 or P2 (a team battle's core plays as its slot), its clip in its files (the interpreter and the clip code
/// guard their offsets themselves: a pc outside the file stops the script).
pub fn check(f: &Br1Fighter, res: &Br1Res) -> Result<(), String> {
    let forms = (0..2).all(|k| f.forms[k].rec.len() == res.anm.params.nodes[k] as usize && f.forms[k].md.len() == res.anm.params.nodes[k] as usize);
    if !forms || f.cur > 1 || f.f.player > 1 || f.anim.clip.is_some_and(|c| c > res.anm.bytes.len().max(res.cmn.len()).max(res.image_len)) {
        return Err("Imported fighter".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{CrossThrow, CrossThrows, Ghost, hud_gauge};

    fn throw(victim: u8) -> CrossThrow {
        CrossThrow { victim, ghost: Ghost::Br1(Box::default()), frames: 0 }
    }

    #[test]
    fn cross_throws_are_keyed_by_thrower() {
        let mut l = CrossThrows::default();
        assert!(l.is_empty() && l.of(0).is_none() && l.thrower_of(1).is_none());
        // kept in thrower order, whatever the order they start in
        l.start(3, throw(2));
        l.start(0, throw(1));
        assert_eq!(l.throwers(), [0, 3]);
        assert_eq!((l.of(0).map(|t| t.victim), l.of(3).map(|t| t.victim)), (Some(1), Some(2)));
        assert_eq!((l.thrower_of(1), l.thrower_of(2), l.thrower_of(0)), (Some(0), Some(3), None));
        assert!((0..4).all(|k| l.involves(k)));
        l.check(4).expect("a valid list");
        // a frame of thrower 3's throw
        l.of_mut(3).expect("thrower 3").frames += 1;
        assert_eq!(l.of(3).map(|t| t.frames), Some(1));
        // a fighter is in one throw: a new throw on 1 (thrower 0's victim) drops thrower 0's, and one by 2 drops 3's
        l.start(2, throw(1));
        assert_eq!(l.throwers(), [2]);
        assert_eq!(l.thrower_of(1), Some(2));
        assert!(!l.involves(0) && !l.involves(3));
        // the end of a throw hands it back; a fighter without one has none to end
        assert_eq!(l.end(2).map(|t| t.victim), Some(1));
        assert!(l.end(2).is_none() && l.is_empty());
        l.start(1, throw(0));
        l.clear();
        assert!(l.is_empty());
    }

    #[test]
    fn a_decoded_cross_throw_list_is_checked() {
        let list = |v: Vec<(u8, u8)>| CrossThrows(v.into_iter().map(|(a, b)| (a, throw(b))).collect());
        assert!(list(vec![(0, 1), (2, 3)]).check(4).is_ok());
        // out of range, throwing itself, a fighter in two throws, out of order
        for bad in [vec![(0, 4)], vec![(4, 0)], vec![(1, 1)], vec![(0, 1), (2, 1)], vec![(0, 1), (1, 2)], vec![(2, 3), (0, 1)]] {
            assert!(list(bad.clone()).check(4).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn hud_gauge_maps_the_human_range() {
        assert_eq!(hud_gauge(0, 0), 0);
        assert_eq!(hud_gauge(0xEFF, 0), 0x1DFE);
        assert_eq!(hud_gauge(0xF00, 0), 0x1E00);
        assert_eq!(hud_gauge(0x3BFF, 0), 0x3BFF);
        assert_eq!(hud_gauge(0x3C00, 0), 0x3C00);
        // the beast form's gauge as it is
        assert_eq!(hud_gauge(0x1234, 0x10), 0x1234);
        assert_eq!(hud_gauge(0x3BFF, 0x11), 0x3BFF);
    }
}
