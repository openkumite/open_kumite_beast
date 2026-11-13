//! An imported fighter as a round starts it: the fighter block, the script state and the forms set up again.
//!
//! State: none of its own: builds the serialised `Br1Fighter`
//!
//! The pointer setup of the loader (the state, command, chain and clip tables, the bone maps) is `Br1Res`' static
//! data and the relocations are not needed. Left out: the spring chains' setup (their links are
//! `Br1Res::linked`, their motion is presentation, stage S8) and the AI block's setup (stage S10).

use super::input;
use super::object::{Br1Fighter, Form, FormDesc, Node};
use super::pose::{IDENTITY, Rec};
use super::res::Br1Res;
use super::script::{self, Ctx, model_switch};

/// What the reset reads of the match besides the fighter (`*(G.BC)` + 0x2C, G.99 / G.9E).
#[derive(Clone, Copy, Debug, Default)]
pub struct Setup {
    /// `*(G.BC)` + 0x2C bit 0x80: the red health regenerates 4/3 as fast (F.D1 × 3/4)
    pub fast_regen: bool,
    /// G.99 == 4 and G.9E (practice): a human fighter keeps its damage
    pub practice: bool,
    /// F.D3: the vitality factor, % of the ANM's max HP (75 in the traced VS default, VITALITY 100 %)
    pub vitality: u8,
}

/// The vitality factor of the traced VS default (187 = 250 × 75 / 100).
pub const VITALITY: u8 = 75;

/// A model node reset (local and world matrices identity, translations 0).
fn node_reset() -> Node {
    Node { lm: IDENTITY, lt: [0; 3], wm: IDENTITY, wt: [0; 3] }
}

/// The ANM's values (the part a reset repeats): the form descriptors from the ANM, F.D1 / F.A8 / S.C4, the flag
/// words, the candidates, the per-state fields, the objects' hidden flags; with `mode` & 0xF in 0, 2, 3 also the
/// animation's position fields.
fn load_values(f: &mut Br1Fighter, res: &Br1Res, mode: u8, s: &Setup) {
    let a = &res.anm;
    let p = &a.params;
    let sb = res.anm.header[10] as usize;
    let b = |o: usize| a.bytes.get(sb + o).copied().unwrap_or(0);
    // F.24: +1 nodes, +2 / +3 body / hit spheres, +0x18 weight, +0x1A the op-14 factor (+0 the object count: the
    // form setup's)
    for form in 0..2 {
        let d = &mut f.desc[form];
        d.nodes = p.nodes[form];
        d.body_spheres = b(2 * form);
        d.hit_spheres = b(4 + 2 * form);
        d.weight = p.weight[form] as i16;
        d.factor = p.timer_factor[form] as i16;
    }
    f.f.regen_interval = p.regen;
    f.f.max_hp = p.max_hp as i16;
    f.anim.root_scale = p.root_scale as i16;
    if s.fast_regen {
        let v = 3 * f.f.regen_interval as i32;
        f.f.regen_interval = ((if v < 0 { v + 3 } else { v }) >> 2) as u8;
    }
    (f.f.f50, f.f.f54, f.f.f58, f.f.f5c, f.f.f60, f.f.f64) = (0, 0, 0, 0, 0, 0);
    let st = &mut f.s;
    st.attack_index = 0;
    (st.cand_flags, st.cand_mv, st.cand_life) = (0, -1, 0);
    (st.cancel_flags, st.cancel_mv, st.cancel_life) = (0, -1, 0);
    (st.att_flags, st.att_mv, st.att_life, st.att_prio) = (0, -1, 0, -1);
    (st.chain_pos, st.chain_len, st.chain_sel) = (0, 0, 0xFF);
    st.sleep = 0;
    // S.A6 (s16 over the bytes A6 / A7), S.A8, S.C8 = 1, S.B8 = 0 (no clip)
    st.sa5[1] = 1;
    st.sa5[2] = 0;
    st.clip_cycle = 1;
    f.anim.keep_root = 1;
    f.anim.clip = None;
    f.f.step = f.f.step_normal;
    let st = &mut f.s;
    st.land_state = -1;
    (st.trail_mode, st.trail_time) = (-1, -1);
    f.anim.head_yaw = 0;
    f.anim.head_pitch = 0;
    let st = &mut f.s;
    st.auto_turn = 0;
    f.f.attack = -1;
    f.spheres[0].flag = -1;
    f.f.hittable = 0;
    let st = &mut f.s;
    (st.delayed_sound, st.delayed_timer, st.impact_class, st.impact_own, st.sound_timer) = (-1, -1, -1, -1, 0);
    // objects 0..25 of both forms shown, then 15..18 hidden
    f.parts = Default::default();
    if matches!(mode & 0xF, 0 | 2 | 3) {
        let an = &mut f.anim;
        an.mirror = false;
        an.flip_next = false;
        (an.yaw, an.yaw_base, an.turn) = (0, 0, 0);
        (an.pos, an.root, an.bind1, an.t1, an.bind4, an.t4) = ([0; 3], [0; 3], [0; 3], [0; 3], [0; 3], [0; 3]);
    }
}

/// The per-round fields, health and gauge by `mode` & 0xF (0 the first round: gauge 0xF00, Uriko's
/// 0x3BFF; 1 / 2 a later round: the gauge kept; 3 sudden death: the damage at the max).
fn round_values(f: &mut Br1Fighter, mode: u8, s: &Setup) {
    let x = &mut f.f;
    x.recover_timer = 0x1E0;
    (x.held, x.pressed, x.v_angle, x.v_speed, x.v_decay, x.vy, x.gravity, x.floor) = (0, 0, 0, 0, 0, 0, 0, 0);
    (x.launch_angle, x.launch_speed, x.attack_damage, x.knock_angle, x.knock_strength, x.regen_count) = (0, 0, 0, 0, 0, 0);
    (x.queued_damage, x.down_timer, x.knock_dir, x.push_speed, x.no_throw, x.combo, x.last_damage) = (0, 0, 0, 0, 0, 0, 0);
    x.vel = [0; 3];
    (x.f98[1], x.f98[2], x.f98[3]) = (0, 0, 0);
    x.rounds_lost = x.fda;
    let vital = |hp: i16| ((hp as i32 * x_vit(s)) / 100) as i16;
    match mode & 0xF {
        m @ 0..=2 => {
            if m == 0 {
                x.gauge = if x.chr == 8 { 0x3BFF } else { 0xF00 };
            }
            if s.practice && x.cpu == 0 {
                x.blue = x.damage;
                x.damage_perm = x.damage;
            } else {
                (x.damage, x.blue, x.damage_perm) = (0, 0, 0);
            }
            x.max_hp = vital(x.max_hp);
            x.rave = 0;
            x.gauge_queued = 0;
            if x.gauge < 0 {
                x.gauge = 0;
            }
        }
        3 => {
            x.gauge = 0xF00;
            x.rave = 0;
            x.f54 |= 0x20_0000;
            x.max_hp = vital(x.max_hp);
            (x.damage, x.blue, x.damage_perm) = (x.max_hp, x.max_hp, x.max_hp);
            if x.chr == 8 {
                x.gauge = 0x3BFF;
            }
            x.gauge_queued = 0;
        }
        _ => {}
    }
}

fn x_vit(s: &Setup) -> i32 {
    s.vitality as i32
}

/// The current form's pose records and objects reset, the node-0 angles 0, the start position and facing
/// by `place` (`mode` >> 4: 0 the round start, 1 / 2 / 3 the other layouts) and the side.
fn place(f: &mut Br1Fighter, place: u8) {
    (f.f.pitch, f.f.yaw, f.f.roll) = (0, 0, 0);
    let n = f.f.nodes as usize;
    let form = f.form_mut();
    for k in 0..n {
        if let Some(d) = form.md.get_mut(k) {
            *d = node_reset();
        }
        if let Some(r) = form.rec.get_mut(k) {
            *r = Rec { dir: [0, 0x7FFF, 0], twist: 0 };
        }
    }
    let k = f.cur as usize & 1;
    for o in f.objs[k].iter_mut() {
        *o = [0; 3];
    }
    let p1 = f.f.player == 0;
    let (x, z, yaw) = match place {
        2 if p1 => (-0x200, -0x200, 0x400),
        2 if f.f.player == 1 => {
            // the facing by character (a jump table; URI's entry and past it 0x400)
            let yaw = match f.f.chr {
                0 | 2 | 6 => 0x600,
                1 | 5 => 0x200,
                3 => 0x38E,
                4 => 0x4E3,
                7 => 0x31C,
                _ => 0x400,
            };
            (0x200, 0x200, yaw)
        }
        1 if p1 => (0, 0, 0x400),
        1 if f.f.player == 1 => (0x400, 0, -0x400),
        3 if p1 => (-0x400, 0, 0x400),
        3 if f.f.player == 1 => (-0x200, 0, -0x400),
        1..=3 => (0, 0, 0),
        _ if p1 => (-0x100, 0, 0x400),
        _ if f.f.player == 1 => (0x100, 0, -0x400),
        _ => (0, 0, 0),
    };
    let form = f.form_mut();
    if let Some(d) = form.md.first_mut() {
        d.wt[0] = x;
        d.lt[0] = x;
        d.wt[2] = z;
        d.lt[2] = z;
    }
    f.anim.pos[0] = x as i16;
    f.f.yaw = yaw;
    f.anim.yaw_base = yaw;
}

/// The form setup's per-fighter part for form `form`: the nodes' static local translations, S.EC (node 1's bind
/// offset).
fn form_setup(f: &mut Br1Fighter, res: &Br1Res, form: usize) {
    let skel = &res.skels[form & 1];
    let fm = &mut f.forms[form & 1];
    for (k, d) in fm.md.iter_mut().enumerate().skip(1) {
        if let Some(t) = skel.t16.get(k) {
            d.lt = *t;
        }
    }
    f.anim.bind1 = skel.bind1();
}

/// The spring chains' setup, its pose-record part (by character, for the current form F.5
/// & 1): the modes in the twist words of the chain nodes' records (their links are `Br1Res::linked`, the fields
/// past +8 of a record are not modelled).
fn chain_setup(f: &mut Br1Fighter) {
    let beast = f.f.form & 1 != 0;
    let chr = f.f.chr;
    let rec = &mut f.form_mut().rec;
    let mut tw = |n: usize, v: i16| {
        if let Some(r) = rec.get_mut(n) {
            r.twist = v;
        }
    };
    match chr {
        0 if beast => {
            tw(19, 2);
            tw(20, 1);
        }
        4..=7 if beast => tw(19, 1),
        8 if beast => {
            tw(19, 2);
            tw(20, 4);
            tw(21, 4);
            tw(22, 8);
            tw(23, 8);
        }
        8 => {
            if let Some(r) = rec.get_mut(25) {
                r.dir[0] = 0x200;
                r.twist = 0xFA;
            }
        }
        _ => {}
    }
}

/// The round start's input reset for one side (before the fighter resets): the
/// button counters (held 0, since a press 100), the command records and the chain slots cleared.
pub fn reset_input(f: &mut Br1Fighter) {
    f.input.hold = [0; 16];
    f.input.since = [100; 16];
    input::reset_commands(f);
    input::reset_chain_slots(f);
}

/// A fighter block as the loader leaves it before its first reset: side, character, the step, the CPU flag, the
/// forms' lists sized from the ANM, the VS default buttons.
pub fn new(res: &Br1Res, player: u8, chr: u8, cpu: u8) -> Br1Fighter {
    let mut f = Br1Fighter::default();
    f.f.player = player;
    f.f.chr = chr;
    f.f.cpu = cpu;
    f.f.step_normal = 16;
    f.f.step = 16;
    f.f.vitality = VITALITY;
    f.s.state_req = 0x28;
    f.s.state_cur = -1;
    f.forms = std::array::from_fn(|k| {
        let n = res.anm.params.nodes[k] as usize;
        Form { rec: vec![Rec::default(); n], md: vec![node_reset(); n] }
    });
    // F.24 + 0: the object counts of the forms' models (`XX20.DAT` / `XX1n.DAT`, read by the model loader; only the
    // drawing uses them), as the traced fights hold them; the node count for the fighters not traced
    let objects = match chr {
        1 => [20, 20],
        4 => [17, 22],
        6 => [21, 22],
        8 => [23, 24],
        _ => res.anm.params.nodes,
    };
    f.desc = std::array::from_fn(|k| FormDesc { objects: objects[k], ..Default::default() });
    f.input.buttons = input::default_buttons(&res.tables);
    f
}

/// The round-start reset of `f` (`mode` & 0xF: 0 the first round, 1 the match-end reset
/// (health only), 2 a later round, 3 sudden death; `mode` >> 4 the start layout), then the requested state started
/// and its script run once.
pub fn reset(fs: &mut [Br1Fighter; 2], me: usize, mode: u8, s: &Setup, ctx: &mut Ctx) {
    let res = ctx.res[me];
    let f = &mut fs[me];
    if mode & 0xF == 1 {
        f.f.status = 1;
        (f.f.opp_angle, f.f.centre_angle, f.f.opp_dist, f.f.opp_dy) = (0, 0, 0x200, 0);
        load_values(f, res, mode, s);
        round_values(f, mode, s);
    } else if mode & 0xF <= 3 {
        if mode & 0xF == 0 {
            f.f.form = 0;
        }
        let form = f.f.form;
        f.f.status = 1;
        f.f.form = 0;
        (f.f.opp_angle, f.f.centre_angle, f.f.opp_dist, f.f.opp_dy) = (0, 0, 0x200, 0);
        load_values(f, res, mode, s);
        round_values(f, mode, s);
        model_switch(f, 0);
        place(f, mode >> 4);
        form_setup(f, res, 0);
        chain_setup(f);
        f.f.form = 0x11;
        model_switch(f, 1);
        place(f, mode >> 4);
        form_setup(f, res, 1);
        chain_setup(f);
        f.f.form = form;
        model_switch(f, form as usize & 1);
        f.f.f0a = 1;
    } else {
        return;
    }
    // the requested state, its script once
    let f = &mut fs[me];
    f.s.state_cur = f.s.state_req;
    let n = f.s.state_req;
    script::start_state(f, res, n);
    script::run(fs, me, ctx);
}
