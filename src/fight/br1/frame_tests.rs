//! Tests of the imported game's fight frame (S5): synthetic cases of the input, the throws, the decision and the physics
//! (no game data), and the frame runs against recorded reference runs (data-gated: the imported game's install and
//! the recorded runs).

use super::decision::{self, Opts};
use super::input::{self, Pad};
use super::object::{Br1Fighter, Form, FormDesc, Node};
use super::physics::{self, RING_EDGE, RING_HIPS, Ring, Walls};
use super::pose::{IDENTITY, Rec};
use super::res::Br1Res;
use super::script::{Call, Ctx, Request};
use super::throw;
use crate::formats::br1_anm::tests::synthetic;
use crate::formats::tables::{Elem, RAM, Tables};

/// The imported game's tables good enough for `Br1Res::new`: a sine table, stage 0's ring records (hips ±1472, edge ±1536,
/// drop 256) and stage 1's circle (r² = 1000², drop 64); the rest missing (read as 0).
pub(super) fn tables() -> Tables {
    let sin = (0..0x401).flat_map(|k| (((k as f64 * std::f64::consts::PI / 2048.0).sin() * 4096.0).round() as i16).to_le_bytes()).collect();
    let rec = |w0: u32, w1: u32| [w0.to_le_bytes(), w1.to_le_bytes()].concat();
    let (mut edge, mut hips) = (vec![0u8; 8 * 15], vec![0u8; 8 * 15]);
    edge[..8].copy_from_slice(&rec(256 << 16, 1536 | 1536 << 16));
    hips[..8].copy_from_slice(&rec(256 << 16, 1472 | 1472 << 16));
    hips[8..16].copy_from_slice(&rec(1 | 64 << 16, 1_000_000));
    Tables::build([
        ("math.sin".into(), Elem::I16, 0x401, RAM + 0x1000, sin),
        (RING_EDGE.into(), Elem::Rec(8), 15, RAM + 0x2000, edge),
        (RING_HIPS.into(), Elem::Rec(8), 15, RAM + 0x3000, hips),
        ("image_len".into(), Elem::U32, 1, 0, 0x8B000u32.to_le_bytes().to_vec()),
    ])
    .unwrap()
}

/// The synthetic fighter (one command, one chain program), its command table replaced by `cmds` (16-byte records with
/// their 10-byte steps after them) when given.
fn res(cmds: Option<&[(u32, i16, i16, i16, i16, &[[i16; 5]])]>) -> Br1Res {
    let cmn = [12u8, 0, 0, 0, 1, 0, 0, 0, 0, 0, 0, 0].to_vec();
    let mut r = Br1Res::new("MIT", synthetic(), cmn, &crate::fight::br1::res::Br1Tables::new(tables()).unwrap()).unwrap();
    if let Some(cmds) = cmds {
        let b = &mut r.anm.bytes;
        let table = b.len();
        b.resize(table + 16 * (cmds.len() + 1), 0);
        for (i, (flags, mv, life, window, param, steps)) in cmds.iter().enumerate() {
            let seq = b.len() as u32;
            for s in steps.iter() {
                for v in s {
                    b.extend_from_slice(&v.to_le_bytes());
                }
            }
            b.extend_from_slice(&[0; 10]);
            let o = table + 16 * i;
            b[o..o + 4].copy_from_slice(&flags.to_le_bytes());
            for (k, v) in [*mv, *life, *window, *param].into_iter().enumerate() {
                b[o + 4 + 2 * k..o + 6 + 2 * k].copy_from_slice(&v.to_le_bytes());
            }
            b[o + 12..o + 16].copy_from_slice(&seq.to_le_bytes());
        }
        r.anm.header[3] = table as u32;
    }
    r
}

/// The tests' button map (the VS one's layout).
const BUTTONS: [u16; 8] = [0x80, 0x40, 0x20, 0x10, 0x08, 0x04, 0x800, 0];

/// Two fighters of 19 nodes, MIT (chr 4) as P1 and FOX (6) as P2, standing at the normal step with the VS buttons.
fn fighters() -> [Br1Fighter; 2] {
    std::array::from_fn(|k| {
        let mut f = Br1Fighter::default();
        f.f.player = k as u8;
        f.f.chr = [4, 6][k];
        f.f.step = 16;
        f.f.step_normal = 16;
        f.f.status = 1;
        f.f.weight = 820;
        f.f.max_hp = 250;
        f.f.body_spheres = 1;
        f.s.cand_mv = -1;
        f.s.att_mv = -1;
        f.s.att_prio = -1;
        f.s.cancel_mv = -1;
        f.s.chain_sel = 0xFF;
        f.input.buttons = BUTTONS;
        f.input.since = [100; 16];
        f.forms = std::array::from_fn(|_| Form { rec: vec![Rec::default(); 19], md: vec![Node { lm: IDENTITY, wm: IDENTITY, ..Default::default() }; 19] });
        f.desc = [FormDesc { objects: 17, nodes: 19, body_spheres: 1, hit_spheres: 1, weight: 820, factor: 153 }; 2];
        f
    })
}

/// A frame's context on `r` (stage 0, two rounds to win).
fn with_ctx<T>(r: &Br1Res, f: impl FnOnce(&mut Ctx) -> T) -> (T, Vec<(u8, Request)>) {
    let (mut rand, mut out) = (0u8, Vec::new());
    let mut ctx =
        Ctx { res: [r, r], rand: &mut rand, stage: 0, round: 1, walls: [true; 4], need: 2, g_ad: 0, g_e8: 0, out: &mut out, trace: None, rings: None };
    let v = f(&mut ctx);
    (v, out)
}

/// One input frame of fighter 0 with raw pad `raw`.
fn press(fs: &mut [Br1Fighter; 2], r: &Br1Res, raw: u16) {
    input::process(fs, 0, &r.anm, Pad { raw, cam: 0, ai: 0, replay: None });
}

#[test]
fn button_timing_recent_long_double() {
    let r = res(None);
    let mut fs = fighters();
    press(&mut fs, &r, 0x80);
    assert_eq!((fs[0].f.held, fs[0].f.pressed, fs[0].f.recent), (0x80, 0x80, 0x80));
    // held: long from the third frame; released: recent for 9 frames after the press
    press(&mut fs, &r, 0x80);
    press(&mut fs, &r, 0x80);
    press(&mut fs, &r, 0x80);
    assert_eq!(fs[0].f.long & 0x80, 0x80);
    // the press 3 frames ago: recent until 8 frames after it
    for _ in 0..5 {
        press(&mut fs, &r, 0);
    }
    assert_eq!(fs[0].f.recent & 0x80, 0x80);
    press(&mut fs, &r, 0);
    assert_eq!(fs[0].f.recent & 0x80, 0);
    // a second press within 12 frames of the first: double tap, kept while held (17 frames)
    let mut fs = fighters();
    press(&mut fs, &r, 0x2000);
    press(&mut fs, &r, 0);
    press(&mut fs, &r, 0x2000);
    assert_eq!(fs[0].f.dbl, 0x2000);
    press(&mut fs, &r, 0x2000);
    assert_eq!(fs[0].f.dbl, 0x2000);
}

#[test]
fn pad_word_swap_guard_and_masks() {
    let r = res(None);
    let mut fs = fighters();
    fs[0].f.f4c = 1;
    // facing left on screen (the camera angle + yaw negative as 12 bits): left / right exchanged, 0x200 set
    fs[0].f.yaw = -0x100;
    press(&mut fs, &r, 0x2000 | 0x40);
    assert_eq!(fs[0].f.held, 0x8000 | 0x40 | 0x400 | 0x200);
    // F.58 < 0: buttons only; F.58 0x20000: directions and the guard bit only
    let mut fs = fighters();
    fs[0].f.f58 = 0x8000_0000;
    press(&mut fs, &r, 0x1000 | 0x80 | 0x02);
    assert_eq!(fs[0].f.held, 0x80);
    fs[0].f.f58 = 0x2_0000;
    fs[0].f.f4c = 1;
    press(&mut fs, &r, 0x1000 | 0x80);
    assert_eq!(fs[0].f.held, 0x1400);
    // the config: R1 (raw 0x08) → 0x800, R2 (0x02) → nothing; a KO'd fighter reads nothing; a CPU takes the AI's pad
    let mut fs = fighters();
    press(&mut fs, &r, 0x08 | 0x02);
    assert_eq!(fs[0].f.held, 0x800);
    fs[0].f.status = 0x80;
    press(&mut fs, &r, 0x80);
    assert_eq!(fs[0].f.held, 0);
    let mut fs = fighters();
    fs[0].f.cpu = 1;
    input::process(&mut fs, 0, &r.anm, Pad { raw: 0x80, cam: 0, ai: 0x4040, replay: None });
    assert_eq!(fs[0].f.held, 0x4040);
}

/// 236P: down, down-forward, forward (held, 1 frame each, 10 misses), then P pressed; the generic candidate block.
const QCF: &[[i16; 5]] =
    &[[1, 10, 1, 0xF000u16 as i16, 0x4000], [1, 10, 1, 0xF000u16 as i16, 0x6000], [1, 10, 1, 0xF000u16 as i16, 0x2000], [1, 15, 2, 0xE0, 0x80]];

#[test]
fn commands_register_by_their_flags() {
    let r = res(Some(&[(0x23, 128, 15, 40, 0, QCF), (0x63, 130, 15, 40, 0, &[[1, 15, 2, 0xE0, 0x20]]), (0x5280, 176, 5, 20, 294, &[[1, 5, 3, 0xE0, 0xC0]])]));
    let mut fs = fighters();
    for p in [0x4000, 0x6000, 0x2000, 0x2080] {
        press(&mut fs, &r, p);
    }
    let s = &fs[0].s;
    assert_eq!((s.cand_flags, s.cand_mv, s.cand_life), (0x23, 128, 14), "the life counts down after the matching");
    assert_eq!(fs[0].input.cmd[0][0], 0);
    // ○ (0x40: beast only) is ignored in human form; the throw (PK) goes to the attack block with its priority
    let mut fs = fighters();
    fs[0].f.form = 0x11;
    press(&mut fs, &r, 0x20);
    assert_eq!(fs[0].s.cand_mv, 130);
    let mut fs = fighters();
    press(&mut fs, &r, 0x20);
    assert_eq!(fs[0].s.cand_mv, -1);
    press(&mut fs, &r, 0xC0);
    let s = &fs[0].s;
    assert_eq!((s.att_flags, s.att_mv, s.att_param, s.att_prio), (0x5280, 176, 294, 2));
    // a miss limit resets a sequence; nothing matches in hit-stun
    let mut fs = fighters();
    press(&mut fs, &r, 0x4000);
    for _ in 0..10 {
        press(&mut fs, &r, 0x1000);
    }
    assert_eq!(fs[0].input.cmd[0][0], 0);
    let mut fs = fighters();
    fs[0].f.f50 = 0x8000;
    press(&mut fs, &r, 0x4000);
    assert_eq!(fs[0].input.cmd[0], [0; 4]);
}

#[test]
fn chain_program_queues_its_follow_up() {
    // the synthetic program: 1 (a list of 1: attack 0x91), 4 window 2..9 frames, 0 end
    let r = res(None);
    let mut fs = fighters();
    fs[0].f.f50 = 0x8000_0000;
    fs[0].s.attack_index = 0x91;
    press(&mut fs, &r, 0);
    assert_eq!(fs[0].input.chain[0], [3, 0, 0x91, -1, 0]);
    // the window next frame, then the end op: every slot reset, nothing queued
    press(&mut fs, &r, 0);
    assert_eq!(fs[0].input.chain[0], [6, 0, 0x91, 2 << 4, 9 << 4]);
    press(&mut fs, &r, 0);
    assert_eq!(fs[0].input.chain[0][0], 0);
    assert_eq!((fs[0].s.chain_len, fs[0].s.chain_sel), (0, 0xFF));
}

#[test]
fn throws_connect_miss_and_resolve() {
    let r = res(None);
    let mut fs = fighters();
    let s = &mut fs[0].s;
    (s.att_flags, s.att_mv, s.att_param) = (0x1280 | 0x4000, 176, 294);
    fs[0].f.f54 = 0x4000;
    fs[0].f.opp_dist = 200;
    fs[1].f.f50 = 1;
    let (v, _) = with_ctx(&r, |ctx| throw::test(&fs, 0, ctx));
    assert_eq!(v, 1);
    // out of reach: a miss; the resolution whiffs it (F.64 0x600000) and clears the candidate
    fs[0].f.opp_dist = 300;
    let (v, _) = with_ctx(&r, |ctx| throw::test(&fs, 0, ctx));
    assert_eq!(v, -1);
    let mut fs2 = fs.clone();
    with_ctx(&r, |ctx| throw::resolve(&mut fs2, ctx));
    assert_eq!((fs2[0].f.f64, fs2[0].s.att_flags), (0x60_0000, 0));
    // in reach: the grab, the victim's S.34 = the thrower's move
    fs[0].f.opp_dist = 200;
    with_ctx(&r, |ctx| throw::resolve(&mut fs, ctx));
    assert_eq!((fs[0].f.f64, fs[1].f.f64, fs[1].s.thrown_by), (0x40_0000, 0x80_0000, 176));
    // a forced throw (op 0x20) needs no test
    let mut fs = fighters();
    fs[1].s.att_flags = 0x8000;
    fs[1].s.att_mv = 0xB2;
    with_ctx(&r, |ctx| throw::resolve(&mut fs, ctx));
    assert_eq!((fs[1].f.f64, fs[0].f.f64, fs[0].s.thrown_by, fs[0].f.f58), (0x40_0000, 0x80_0000, 0xB2, 0x1000_0000));
}

fn decide(r: &Br1Res, fs: &mut [Br1Fighter; 2], o: Opts) -> i32 {
    let mut w = Walls::default();
    with_ctx(r, |ctx| decision::decide(fs, 0, ctx, o, &mut w)).0
}

#[test]
fn decision_movement_attacks_candidates() {
    let r = res(None);
    let o = Opts { g98: 1, need: 2, opt38: 0, rave: true };
    let mut fs = fighters();
    assert_eq!(decide(&r, &mut fs, o), 1, "stance");
    fs[0].f.held = 0x2000;
    assert_eq!(decide(&r, &mut fs, o), 2, "walk forward");
    fs[0].f.held = 0x3000;
    assert_eq!((decide(&r, &mut fs, o), fs[0].s.next_state), (0x13, 0x18), "a forward jump");
    fs[0].f.held = 0;
    fs[0].f.dbl = 0x8000;
    assert_eq!(decide(&r, &mut fs, o), 7, "back dash");
    // the attack table: P, crouching K, the beast button (transform with gauge ≥ 0xF00)
    let mut fs = fighters();
    fs[0].f.recent = 0x80;
    assert_eq!(decide(&r, &mut fs, o), 0x80);
    fs[0].f.recent = 0x40;
    fs[0].f.held = 0x4000;
    assert_eq!(decide(&r, &mut fs, o), 0x80 + 0x13);
    fs[0].f.held = 0;
    fs[0].f.recent = 0x20;
    fs[0].f.gauge = 0xF00;
    assert_eq!(decide(&r, &mut fs, o), 0x80 + 0x2E);
    fs[0].f.gauge = 0;
    assert_eq!(decide(&r, &mut fs, o), 0x80 + 0x2D);
    // a generic command candidate wins over the table
    let mut fs = fighters();
    fs[0].f.recent = 0x80;
    (fs[0].s.cand_flags, fs[0].s.cand_mv) = (0x23, 128);
    assert_eq!((decide(&r, &mut fs, o), fs[0].s.cand_mv), (128 + 0x80, -1));
    // in an attack: the queued chain follow-up once the recovery reached its wait
    let mut fs = fighters();
    fs[0].f.f50 = 0x8000_0000;
    fs[0].f.recovery = 0x20;
    fs[0].s.chain_len = 1;
    fs[0].s.chain_queue[0] = 0x92;
    fs[0].s.chain_queue[16] = 0x10;
    assert_eq!((decide(&r, &mut fs, o), fs[0].s.chain_pos, fs[0].s.state_cur), (0x1080 + 0x92, 1, -1));
}

#[test]
fn decision_pending_requests() {
    let r = res(None);
    let o = Opts { rave: true, ..Opts::default() };
    // the thrower: its attack candidate's move
    let mut fs = fighters();
    fs[0].f.f64 = 0x40_0000;
    fs[0].s.att_mv = 176;
    assert_eq!((decide(&r, &mut fs, o), fs[0].s.att_mv, fs[0].f.f64), (176 + 0x80, -1, 0));
    // the victim: its per-opponent row for the thrower's character, entry S.34 − 0xB0
    let mut fs = fighters();
    fs[0].f.f64 = 0x80_0000;
    fs[0].s.thrown_by = 0xB0;
    assert_eq!(decide(&r, &mut fs, o), 224 + 0x80);
    // the knock-down request: by the side of the hit and the centre angle
    let mut fs = fighters();
    fs[0].f.f64 = 0x10_0000;
    fs[0].f.centre_angle = 0x500;
    fs[0].f.f50 = 0x2_0000;
    assert_eq!((decide(&r, &mut fs, o), fs[0].f.push_speed, fs[0].f.gravity), (0x6E, -0x1400, 0xC0));
    // a new hit: the reaction from the hit result F.60, the state restarted, the command slots cleared
    let mut fs = fighters();
    fs[0].f.f54 = 0x400_0000;
    fs[0].f.f60 = 0x20_0000;
    fs[0].input.cmd[3] = [1, 1, 0, 1];
    let mut w = Walls::default();
    let (v, out) = with_ctx(&r, |ctx| decision::decide(&mut fs, 0, ctx, o, &mut w));
    assert_eq!((v, fs[0].s.state_cur, fs[0].input.cmd[3]), (0x41, -1, [0; 4]));
    assert_eq!(out, vec![(0, Request::Call { call: Call::MoveCancel, args: [0; 6] })]);
}

#[test]
fn ground_ring_and_circle() {
    let r = res(None);
    let hips = Ring::of(&r.tables, RING_HIPS, 0);
    assert_eq!((physics::ground(1472, -1472, hips, 0), physics::ground(1473, 0, hips, 0), physics::ground(0, -1473, hips, 0)), (0, 256, 256));
    assert_eq!(physics::ground(0, 0, hips, 10), 256);
    let circle = Ring::of(&r.tables, RING_HIPS, 1);
    assert_eq!((physics::ground(600, 800, circle, 0), physics::ground(601, 800, circle, 0)), (0, 64));
}

#[test]
fn velocity_keeps_remainders_and_decays() {
    let r = res(None);
    let mut f = fighters()[0].clone();
    // 100 units of 1/128 a frame along the facing 0 (−z): −1 each frame with the remainder carried
    f.f.v_speed = 100;
    f.f.v_decay = -60;
    physics::velocity(&mut f, &r.trig);
    assert_eq!((f.f.vel, f.f.f98[3], f.f.v_speed, f.f.v_decay), ([0, 0, -1], 28, 40, -60));
    physics::velocity(&mut f, &r.trig);
    assert_eq!((f.f.vel[2], f.f.v_speed, f.f.v_decay), (0, 0, 0));
}

#[test]
fn gravity_and_landing() {
    let r = res(None);
    let mut f = fighters()[0].clone();
    // airborne: node 0 above the floor, falling (y down)
    f.forms[0].md[0].wt = [0, -40, 0];
    f.f.vy = 0x500;
    f.f.gravity = 0xC0;
    physics::vertical(&mut f, &r.trig);
    assert_eq!((f.f.vel[1], f.f.vy), (10, 0x500 + 0xC0));
    physics::landing(&mut f);
    assert_eq!(f.f.f54 & 0x180, 0x180);
    // at or below the floor: landed (F.64 0x80000), node 0 and S.DE on the floor
    f.forms[0].md[0].wt = [0, 3, 0];
    physics::landing(&mut f);
    assert_eq!((f.f.vy, f.f.gravity, f.f.f64, f.forms[0].md[0].wt[1], f.anim.pos[1], f.f.f54 & 0x80), (0, 0, 0x8_0000, 0, 0, 0));
}

#[test]
fn walk_with_fewer_nodes_than_the_skeleton() {
    // a node list shorter than the skeleton (19 nodes), or none: the walk places what is there, no panic
    let r = res(None);
    let mut fs = fighters();
    fs[0].forms[0].md.truncate(5);
    fs[1] = Br1Fighter::default();
    fs[0].forms[0].md[1].lt = [0, 100, 0];
    physics::integrate(&mut fs, [&r, &r]);
    assert_eq!(fs[0].forms[0].md.len(), 5);
    assert_eq!(fs[0].forms[0].md[1].wt, [0, 100, 0]);
    assert!(fs[1].forms[0].md.is_empty());
}

#[test]
fn linked_nodes_per_fighter() {
    use super::res::linked_nodes;
    // the base skeleton 0..18; URI's beast walks its extra nodes 19..23, every other chain node is unlinked
    assert_eq!(linked_nodes("MIT", 1, 23), 0x7_FFFF);
    assert_eq!(linked_nodes("URI", 0, 26), 0x7_FFFF);
    assert_eq!(linked_nodes("URI", 1, 24), 0xFF_FFFF);
    assert_eq!(linked_nodes("GRE", 0, 19), 0x7_FFFF);
    assert_eq!(linked_nodes("FOX", 0, 5), 0x1F);
}

#[test]
fn auto_turn_steps_toward_the_opponent() {
    let mut f = fighters()[0].clone();
    f.f.f54 = 0x2_0000;
    f.f.opp_angle = 0x300;
    physics::auto_turn(&mut f, 0);
    assert_eq!((f.f.opp_angle, f.f.yaw, f.anim.turn, f.f.f54 & 0x40_0400), (0x300 - 0x5B, 0x5B, 0x5B, 0));
    f.f.opp_angle = 0x50;
    physics::auto_turn(&mut f, 0);
    assert_eq!((f.f.opp_angle, f.f.yaw), (0, 0x5B + 0x50));
    // behind (|angle| ≥ 0x400): the flag, no turn unless armed
    let mut f = fighters()[0].clone();
    f.f.opp_angle = -0x600;
    physics::auto_turn(&mut f, 0);
    assert_eq!((f.f.f54, f.f.opp_angle), (0x50_0400, -0x600));
}

#[test]
fn walls_push_back_inside() {
    let mut r = res(None);
    // two body spheres (the walls test from the second)
    let s = r.anm.spheres[0][0];
    r.anm.spheres[0] = vec![s, s];
    let mut f = fighters()[0].clone();
    f.f.body_spheres = 2;
    // sphere 1 of the synthetic file: node 4; past +x by 10 (the margin is the node number)
    f.forms[0].md[4].wt = [1472 - 4 + 10, 0, 0];
    f.f.vel = [5, 0, 0];
    let mut w = Walls::default();
    let (t1, out) = physics::walls(&mut f, &r, 1, Ring::of(&r.tables, RING_HIPS, 0), &mut w);
    assert_eq!((t1, out), (1, [-10, 0]));
    assert_eq!((f.f.f64, f.f.f54 & 0x100_0000), (1 | 1 << 4, 0x100_0000));
    assert_eq!(w.hit[0], (820 * 10 / 1280) as i16);
}

/// The frame runs: every stage of the engine's frame against the recorded run's, from the
/// recorded state before it; then the engine's own state carried through each run (the random counter taken from the
/// recording: its other consumers are presentation); the requests against the call log. Excluded: the KO replay's playback (the recorder
/// returns the recorded pads; the main game's round flow replaces it) and the imported game's guts save (dropped).
#[test]
fn frame_runs() {
    use super::reference::diff;
    use super::reference_frame as rf;
    use crate::formats::br1_anm::{CHARACTERS, tests::br1_file};
    let (Some(cmn), Some(tables)) = (br1_file("engine/CMN/CMN.ACT"), crate::formats::tables::br1_export_tables()) else {
        eprintln!("skipped: no okb_export/br1");
        return;
    };
    let tables = crate::fight::br1::res::Br1Tables::new(tables).unwrap();
    let res: Vec<Option<Br1Res>> =
        CHARACTERS.iter().map(|c| br1_file(&format!("engine/{c}/{c}31.DAT")).and_then(|a| Br1Res::new(c, a, cmn.clone(), &tables).ok())).collect();
    // the recorded runs live outside the repository: BR1_RUNS names their folder
    let Ok(out) = std::env::var("BR1_RUNS") else {
        eprintln!("skipped: BR1_RUNS (the recorded runs' folder) is not set");
        return;
    };
    let (mut steps, mut runs) = (0, 0);
    for run in ["s5fa", "s5fb", "s5fc", "s5fd", "s5fe", "s5ff", "s5fg", "s5fh", "s6fa", "s6fb", "s6fc", "s6fd", "s6fe", "s6fg", "s6fh", "s6fi"] {
        let Ok(bytes) = std::fs::read(format!("{out}/{run}/frames.bin")) else { continue };
        // a run of another record version is skipped (re-run it); any other parse failure is an error
        let magic = bytes.get(..4).map_or(0, |m| u32::from_le_bytes([m[0], m[1], m[2], m[3]]));
        if magic & 0xFF_FFFF == rf::MAGIC & 0xFF_FFFF && magic != rf::MAGIC {
            eprintln!("{run}: skipped (a frames.bin of another record version)");
            continue;
        }
        let recs = rf::parse(&bytes).unwrap_or_else(|e| panic!("{run}/frames.bin: {e}"));
        runs += 1;
        let pick = |r: &rf::Rec| -> Option<[&Br1Res; 2]> { Some([res.get(r.snap.chr(0))?.as_ref()?, res.get(r.snap.chr(1))?.as_ref()?]) };
        let calls = std::fs::read_to_string(format!("{out}/{run}/calls.txt")).map(|t| rf::parse_calls(&t)).unwrap_or_default();
        let mut by_rec: std::collections::BTreeMap<usize, Vec<&rf::Call>> = Default::default();
        for c in &calls {
            by_rec.entry(c.rec).or_default().push(c);
        }
        for carried in [false, true] {
            let mut port: Option<([Br1Fighter; 2], Walls, u8)> = None;
            let mut ko = [false; 2];
            for (i, p) in recs.windows(2).enumerate() {
                let (a, b) = (&p[0], &p[1]);
                let (Some(st), Some(rs)) = (rf::stage(a, b), pick(a)) else {
                    port = None;
                    continue;
                };
                let (oa, ob) = (rf::load2(a, rs), rf::load2(b, rs));
                for k in 0..2 {
                    assert_eq!(rf::linked(a, k), rs[k].linked, "{run} frame {}: P{}'s linked nodes", a.snap.frame, k + 1);
                }
                let (mut fs, mut walls, _) = match (&port, carried) {
                    (Some(p), true) => p.clone(),
                    _ => (oa.clone(), rf::walls(a), a.snap.rand),
                };
                // the random counter is the recording's (its other consumers are presentation)
                let mut rand = a.snap.rand;
                let known = st == rf::Stage::Inputs && rf::playback(a) || matches!(st, rf::Stage::Decide(k) if rf::guts(&oa[k], &ob[k]));
                if st.implemented() && !known {
                    let before = [fs[0].f.status, fs[1].f.status];
                    let ko_k = matches!(st, rf::Stage::Velocity(k) if ko[k]);
                    let reqs = rf::run(st, &mut fs, &mut walls, &mut rand, a, (&oa, &ob, b), rs, None, ko_k);
                    if let rf::Stage::Decide(k) = st {
                        ko[k] = before[k] == 1 && fs[k].f.status == 0x80;
                    }
                    assert_eq!(rf::cam_after(a, &reqs), b.snap.cam, "{run} frame {} {st:?}: camera words", a.snap.frame);
                    let stage_calls = by_rec.get(&(i + 1)).map_or(&[][..], |v| &v[..]);
                    if !calls.is_empty() {
                        let md = |r: &rf::Rec| [0, 1].map(|k| u32::from_le_bytes(r.snap.f(k)[0x1C..0x20].try_into().unwrap()));
                        let theirs: Vec<_> = stage_calls.iter().filter_map(|c| rf::call_request(c, md(a)).or_else(|| rf::call_request(c, md(b)))).collect();
                        assert_eq!(rf::comparable(&reqs), theirs, "{run} frame {} {st:?}: requests", a.snap.frame);
                    }
                    for k in 0..2 {
                        if fs[k] != ob[k] {
                            let d = diff(&fs[k], &ob[k]);
                            panic!("{run} frame {} {st:?} (carried {carried}), P{}: {d:?}", a.snap.frame, k + 1);
                        }
                    }
                    // the counter advanced by the engine's own draws (the effects' draws, logged, are presentation)
                    let want = if calls.is_empty() { b.snap.rand } else { a.snap.rand.wrapping_add(rf::logic_draws(stage_calls)) };
                    assert_eq!(rand, want, "{run} frame {} {st:?}: random counter", a.snap.frame);
                    assert_eq!(walls, rf::walls(b), "{run} frame {} {st:?}: walls", a.snap.frame);
                    steps += 1;
                } else {
                    rf::feed(&mut fs, &mut walls, &mut rand, (&oa, &ob), a, b);
                }
                port = Some((fs, walls, rand));
            }
        }
    }
    if runs == 0 {
        eprintln!("skipped: no frame runs in {out}");
    } else {
        eprintln!("{runs} runs, {steps} stage steps bit-exact");
    }
}

#[test]
fn hit_heights_direction_and_multiplier() {
    use super::hit;
    let mut fs = fighters();
    fs[0].f.attack = 0;
    // a low misses a jumping defender, a high a crouching one, a mid both
    fs[0].f.f5c = 0x40;
    fs[1].f.f50 = 0x4000;
    assert!(!hit::can_hit(&mut fs, 0));
    fs[0].f.f5c = 0x80;
    fs[1].f.f50 = 0x20;
    assert!(!hit::can_hit(&mut fs, 0));
    fs[0].f.f5c = 0x100;
    fs[1].f.f50 = 0x10;
    assert!(!hit::can_hit(&mut fs, 0));
    fs[1].f.f50 = 0;
    fs[0].f.f54 = 0x14;
    assert!(hit::can_hit(&mut fs, 0));
    assert_eq!(fs[0].f.f54, 0);
    // facing each other (P2 yaw 0x800): from the front, a weak knockback (CF 0)
    fs[1].f.yaw = 0x800;
    assert_eq!(hit::direction_bits(&fs, 0, 0), 0x4 | 0x1 | 0x10);
    fs[1].f.yaw = 0;
    assert_eq!(hit::direction_bits(&fs, 0, 0) & 3, 2);
    // counters: startup ×0x99, active ×0xA6, recovery ×0x73; a low on a crouching defender ×2; from behind ×166/128
    fs[1].f.f50 = 0x8000_0000;
    fs[1].f.startup = 16;
    assert_eq!(hit::multiplier(&fs, 0, 1), 0x99);
    fs[1].f.startup = 0;
    assert_eq!(hit::multiplier(&fs, 0, 1), 0x73);
    fs[1].f.active = 16;
    assert_eq!(hit::multiplier(&fs, 0, 1), 0xA6);
    fs[1].f.f50 = 0x20;
    fs[0].f.f5c = 0x40;
    assert_eq!(hit::multiplier(&fs, 0, 1), 0x100);
    fs[1].f.f50 = 0;
    assert_eq!(hit::multiplier(&fs, 0, 2), 0x80 * 166 / 128);
    fs[1].f.f54 = 0x2000_0000;
    assert_eq!(hit::multiplier(&fs, 0, 2), 0x80);
}

#[test]
fn hit_apply_guard_and_hit() {
    use super::hit;
    let r = res(None);
    // a standing, guarding defender (F.54 0x2000, F.50 1) blocks a mid: guard stun from the damage, the guard push
    let mut fs = fighters();
    (fs[0].f.attack, fs[0].f.f5c, fs[0].f.attack_damage, fs[0].f.knock_strength) = (0, 0x100, 20, 0x10);
    (fs[1].f.f54, fs[1].f.f50) = (0x2000, 1);
    let (v, out) = with_ctx(&r, |ctx| hit::apply(&mut fs, 0, 1, 0x80, [1, 2, 3], 1, ctx, 1));
    assert_eq!(v, 2);
    assert_eq!((fs[1].f.f54 & 0x4200_0000, fs[1].f.guard_stun, fs[1].f.hit_stun, fs[0].f.combo, fs[0].f.attack), (0x4200_0000, 10, 0, 1, -1));
    assert_eq!(fs[1].f.f60 & 0x40_03C0, 0x40_0080, "mid, class 0x80 (damage 20)");
    assert_eq!(fs[1].f.gauge_queued, 2 * 60, "a human guard gains (CE >> 3)·60");
    assert!(out.contains(&(0, Request::Cam { word: hit::SHAKE, value: 750 })));
    // not guarding: a hit, 15 frames of stun, the damage queued, the attacker's combo 1; the second hit of a combo ×9/10
    let mut fs = fighters();
    (fs[0].f.attack, fs[0].f.f5c, fs[0].f.attack_damage, fs[0].f.knock_strength) = (0, 0x100, 20, 0x10);
    let (v, _) = with_ctx(&r, |ctx| hit::apply(&mut fs, 0, 1, 0x80, [0; 3], 1, ctx, 1));
    assert_eq!(v, 3);
    assert_eq!((fs[1].f.queued_damage, fs[1].f.hit_stun, fs[1].f.f54 & 0x2400_0000, fs[0].f.combo), (20, 0xF0, 0x2400_0000, 1));
    assert_eq!((fs[0].f.gauge_queued, fs[1].f.gauge_queued), (10 * 60, (20 / 3) * 60));
    fs[0].f.attack = 0;
    with_ctx(&r, |ctx| hit::apply(&mut fs, 0, 1, 0x80, [0; 3], 1, ctx, 1));
    assert_eq!((fs[0].f.combo, fs[1].f.queued_damage), (2, 18));
    // heavy damage (≥ 39) knocks down
    let mut fs = fighters();
    (fs[0].f.attack, fs[0].f.f5c, fs[0].f.attack_damage) = (0, 0x100, 40);
    with_ctx(&r, |ctx| hit::apply(&mut fs, 0, 1, 0x80, [0; 3], 1, ctx, 1));
    assert_eq!(fs[1].f.f60 & 0x4600, 0x4600, "knockdown 0x4000 with the reaction bit 0x400 and class 0x200");
}

#[test]
fn hit_step_trades_by_damage() {
    use super::hit;
    let mut r = res(None);
    // a hurt sphere at node 4 and an attack sphere at node 4 of each fighter, both 100 apart: they overlap
    r.anm.spheres[2] = vec![crate::formats::act_ani::Sphere { group: 2, node: 4, radius: 60 }];
    let mut fs = fighters();
    for (k, x) in [(0usize, 0), (1, 100)] {
        let f = &mut fs[k];
        f.f.attack = 0;
        f.f.f5c = 0x100;
        f.f.hit_spheres = 1;
        f.spheres[0] = super::object::Sphere { flag: 0, node: 4, radius: 60 };
        f.spheres[1].flag = -1;
        f.forms[0].md[4].wt = [x, -200, 0];
    }
    fs[0].f.attack_damage = 10;
    fs[1].f.attack_damage = 30;
    // facing each other (the defender's facing S.D8 with the option byte set): front hits, ×1
    fs[0].anim.yaw_base = 0x800;
    with_ctx(&r, |ctx| hit::hits(&mut fs, ctx, 1));
    // P2's stronger hit lands, P1's is dropped (its attack not spent)
    assert_eq!((fs[0].f.queued_damage, fs[1].f.queued_damage, fs[0].f.attack, fs[1].f.attack), (30, 0, 0, -1));
}

#[test]
fn rave_starts_only_when_allowed() {
    let r = res(None);
    let mut fs = fighters();
    (fs[0].f.form, fs[0].f.recent, fs[0].f.gauge) = (0x11, 0x10, 100);
    let on = Opts { g98: 1, need: 2, opt38: 0, rave: true };
    assert_eq!((decide(&r, &mut fs, on), fs[0].f.rave), (0x80 + 0x2D, 1));
    let mut fs = fighters();
    (fs[0].f.form, fs[0].f.recent, fs[0].f.gauge) = (0x11, 0x10, 100);
    assert_eq!((decide(&r, &mut fs, Opts { rave: false, ..on }), fs[0].f.rave), (1, 0));
}

#[test]
fn decision_wall_break_and_uriko() {
    let r = res(None);
    let o = Opts { g98: 1, need: 2, opt38: 0, rave: true };
    // the match's last KO (rounds lost ≥ needed), flying into wall 0 (F.64 bit 0, F.54 0x1000000): the wall breaks
    let mut fs = fighters();
    (fs[0].f.f50, fs[0].f.f54, fs[0].f.f64, fs[0].f.status, fs[0].f.rounds_lost) = (0x0400_0800, 0x0100_0000, 1, 0x80, 2);
    let mut w = Walls::default();
    with_ctx(&r, |ctx| decision::decide(&mut fs, 0, ctx, o, &mut w));
    assert_eq!((w.w44, w.hit[0], w.lock[0], w.hit[1]), (0, 10000, -1, 0));
    // not the last round: the hit strength from the weight (/ 20) while the hips are high
    let mut fs = fighters();
    (fs[0].f.f50, fs[0].f.f54, fs[0].f.f64, fs[0].f.status) = (0x0400_0800, 0x0100_0000, 2, 0x80);
    fs[0].forms[0].md[4].wt = [0, -0x200, 0];
    let mut w = Walls { w44: 1, ..Default::default() };
    with_ctx(&r, |ctx| decision::decide(&mut fs, 0, ctx, o, &mut w));
    assert_eq!((w.w44, w.hit[1], w.lock[1]), (1, 820 / 20, -1));
    // a beast with its gauge spent reverts on a hit; Uriko (chr 8) never does
    for (chr, form) in [(4u8, 0), (8, 0x11)] {
        let mut fs = fighters();
        (fs[0].f.chr, fs[0].f.form, fs[0].f.gauge, fs[0].f.f54, fs[0].f.f60) = (chr, 0x11, -1, 0x400_0000, 0x4000);
        let mut w = Walls::default();
        with_ctx(&r, |ctx| decision::decide(&mut fs, 0, ctx, o, &mut w));
        assert_eq!(fs[0].f.form, form, "chr {chr}");
    }
    // a human Uriko with her gauge below 0 transforms at once
    let mut fs = fighters();
    (fs[0].f.chr, fs[0].f.gauge) = (8, -1);
    assert_eq!(decide(&r, &mut fs, o), 0xAE);
}
