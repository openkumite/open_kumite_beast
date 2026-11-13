//! Tests of the imported interpreter: synthetic scripts (no game data) and the reference runs against the
//! original (data-gated: the imported game's install and `.research/br1/harness/out/s4*`).

use super::*;
use crate::fight::br1::object::{Form, FormDesc, Node, Sphere};
use crate::fight::br1::pose::{Rec, Src};
use crate::formats::br1_anm::tests::synthetic;

/// The imported game's tables good enough for `Br1Res::new`: the random bytes 3·k + 1 and stage 2's hips ring record (hx 1472,
/// hz 1600); the rest missing (read as 0).
fn tables() -> crate::formats::tables::Tables {
    use crate::formats::tables::{Elem, Tables};
    let mut hips = vec![0u8; 8 * 15];
    hips[8 * 2 + 4..8 * 2 + 8].copy_from_slice(&[1472u16.to_le_bytes(), 1600u16.to_le_bytes()].concat());
    Tables::build([
        ("math.random_bytes".into(), Elem::U8, 0x100, 0x8012_0448, (0..256).map(|k| (3 * k + 1) as u8).collect()),
        (crate::fight::br1::physics::RING_HIPS.into(), Elem::Rec(8), 15, 0x801A_3AE8, hips),
        ("image_len".into(), Elem::U32, 1, 0, 0x8B000u32.to_le_bytes().to_vec()),
    ])
    .unwrap()
}

/// The synthetic fighter with `script` appended to its ANM, and the script's offset.
fn res(script: &[u8]) -> (Br1Res, u32) {
    let mut anm = synthetic();
    let pc = anm.len() as u32;
    anm.extend_from_slice(script);
    // CMN.ACT: a block of one empty clip
    let cmn = [12u8, 0, 0, 0, 1, 0, 0, 0, 0, 0, 0, 0].to_vec();
    (Br1Res::new("MIT", anm, cmn, &crate::fight::br1::res::Br1Tables::new(tables()).unwrap()).unwrap(), pc)
}

/// Two fighters of 19 nodes in both forms, MIT (chr 4) as P1 and FOX (6) as P2, at the normal step.
fn fighters() -> [Br1Fighter; 2] {
    std::array::from_fn(|k| {
        let mut f = Br1Fighter::default();
        f.f.player = k as u8;
        f.f.chr = [4, 6][k];
        f.f.step = 16;
        f.f.step_normal = 16;
        f.f.status = 1;
        f.forms = std::array::from_fn(|_| Form { rec: vec![Rec::default(); 19], md: vec![Node::default(); 19] });
        f.desc = [
            FormDesc { objects: 17, nodes: 19, body_spheres: 9, hit_spheres: 8, weight: 820, factor: 153 },
            FormDesc { objects: 17, nodes: 19, body_spheres: 9, hit_spheres: 8, weight: 820, factor: 166 },
        ];
        f.spheres[0] = Sphere { flag: -1, node: 0, radius: 0 };
        f
    })
}

struct World {
    rand: u8,
    out: Vec<(u8, Request)>,
}

impl World {
    fn new() -> Self {
        Self { rand: 0, out: Vec::new() }
    }

    /// Fighter 0's script from `pc` until it stops.
    fn run(&mut self, r: &Br1Res, fs: &mut [Br1Fighter; 2], pc: u32) {
        fs[0].pc = pc;
        let mut ctx = Ctx {
            res: [r, r],
            rand: &mut self.rand,
            stage: 2,
            round: 1,
            walls: [true, false, false, false],
            need: 2,
            g_ad: 0,
            g_e8: 0,
            out: &mut self.out,
            trace: None,
            rings: None,
        };
        run(fs, 0, &mut ctx);
    }
}

/// Little-endian op bytes: opcode, sub, then 16-bit operands.
fn op(code: u8, sub: u8, args: &[i32]) -> Vec<u8> {
    let mut v = vec![code, sub];
    for a in args {
        v.extend_from_slice(&(*a as u16).to_le_bytes());
    }
    v
}

fn script(ops: &[Vec<u8>]) -> Vec<u8> {
    ops.concat()
}

#[test]
fn op_sizes_follow_the_census() {
    // the imported sizes that differ from the main game's (script_ops.md §1)
    assert_eq!((OPLEN[0x11], OPLEN[0x25], OPLEN[0x28], OPLEN[0x23]), (2, 6, 2, 4));
    assert_eq!(OPLEN.iter().map(|&n| n as u32).sum::<u32>(), 158);
}

#[test]
fn stop_sleep_and_loop() {
    // 01.01 mark, 03.02 sleep 2 frames, 00.07 stop at next, 01.00 back to the mark
    let (r, pc) = res(&script(&[op(1, 1, &[]), op(3, 2, &[]), op(0, 7, &[]), op(1, 0, &[])]));
    let mut fs = fighters();
    let mut w = World::new();
    w.run(&r, &mut fs, pc);
    assert_eq!((fs[0].pc, fs[0].loop_mark, fs[0].s.sleep), (pc + 4, pc + 2, 32));
    // sleep counts down by S.D6 with the pc held
    fs[0].anim.speed = 16;
    let mut ctx =
        Ctx { res: [&r, &r], rand: &mut w.rand, stage: 0, round: 0, walls: [false; 4], need: 2, g_ad: 0, g_e8: 0, out: &mut w.out, trace: None, rings: None };
    run(&mut fs, 0, &mut ctx);
    run(&mut fs, 0, &mut ctx);
    assert_eq!((fs[0].pc, fs[0].s.sleep), (pc + 4, 0));
    // then 00.07 stops at the next instruction
    run(&mut fs, 0, &mut ctx);
    assert_eq!(fs[0].pc, pc + 6);
    // 01.00 jumps to the mark: 03.02 sleeps again
    run(&mut fs, 0, &mut ctx);
    assert_eq!((fs[0].pc, fs[0].s.sleep), (pc + 4, 32));
}

#[test]
fn jumps_calls_and_conditions() {
    let (mut r, pc) = res(&[]);
    let t = pc + 0x40;
    // 04.05 call t; at t: 0A.14 test F.50 & 1 → res; 00.03 return if res == 0
    let body = script(&[op(4, 5, &[t as i32]), op(0, 0, &[])]);
    r.anm.bytes.extend_from_slice(&body);
    r.anm.bytes.resize(t as usize, 0);
    r.anm.bytes.extend_from_slice(&script(&[op(0x0A, 0x14, &[1, 0]), op(0, 3, &[])]));
    let mut fs = fighters();
    let mut w = World::new();
    w.run(&r, &mut fs, pc);
    assert_eq!((fs[0].pc, fs[0].ret, fs[0].s.res), (pc + 4, pc + 4, 0));
    // F.50 bit 1 set: the test is non-zero, 00.03 goes on (to the zeros after it: 00.00 stops)
    fs[0].f.f50 = 1;
    w.run(&r, &mut fs, pc);
    assert_eq!((fs[0].pc, fs[0].s.res), (t + 8, 1));
    // the opponent's character: 04.D4 (target opp, chr == 4) does not jump, 04.D6 (chr == 6) does: to the end
    let end = synthetic().len() as i32 + 12;
    let (r, pc) = res(&script(&[op(4, 0xD4, &[0x99]), op(4, 0xD6, &[end]), op(0, 0, &[]), op(0, 0, &[])]));
    w.run(&r, &mut fs, pc);
    assert_eq!(fs[0].pc, end as u32);
    // a human-only jump (gate 0x40) is skipped in beast form
    let (r, pc) = res(&script(&[op(4, 0x40, &[0x20]), op(0, 0, &[])]));
    fs[0].f.form = 1;
    w.run(&r, &mut fs, pc);
    assert_eq!(fs[0].pc, pc + 4);
}

#[test]
fn flag_words_and_the_step_reset() {
    // 0A.00 sets F.50 and the step back to normal; ×3/2 attacking under rave
    let (r, pc) = res(&script(&[op(0x0A, 0x00, &[0, 0x8000]), op(0x0A, 0xD9, &[0x10, 0]), op(0x0A, 0x1E, &[7, 0])]));
    let mut fs = fighters();
    fs[0].f.step = 4;
    fs[0].f.rave = 1;
    let mut w = World::new();
    w.run(&r, &mut fs, pc);
    assert_eq!((fs[0].f.f50, fs[0].f.step), (0x8000_0000, 24));
    // 0xC0 | 0x19: the opponent's F.64 set; 0x1E: own F.4C set
    assert_eq!((fs[1].f.f64, fs[0].f.f4c), (0x10, 7));
}

#[test]
fn attack_ops() {
    // 23.00 trail mode 3, 0D sphere at key slot 6 (node 7), 0E damage 25, 0F knockback, 10 phases (startup 2, active 3)
    let (r, pc) = res(&script(&[op(0x23, 0, &[3]), vec![0x0D, 0, 7, 0x40], op(0x0E, 0, &[25]), vec![0x0F, 0, 0x34, 0x12], op(0x10, 1, &[]), vec![2, 3]]));
    let mut fs = fighters();
    fs[0].f.f5c = 0x400 | 0x20;
    fs[0].anim.mirror = true;
    let mut w = World::new();
    w.run(&r, &mut fs, pc);
    let f = &fs[0];
    // the synthetic bone maps read slot n − 1 for node n: map[7] = 6, stored + 1 = node 7; trail mode 3: 7 → (6, 7)
    // (op 0x10 then ends the list at its head again)
    assert_eq!((f.spheres[0], f.spheres[1].flag), (Sphere { flag: -1, node: 7, radius: 0x40 }, -1));
    assert_eq!(w.out, vec![(0, Request::Call { call: Call::Task, args: [0x1000F, 0, 0, 0, 6, 7] })]);
    assert_eq!((f.f.attack_damage, f.f.gauge_queued, f.s.damage_sum), (25, 180, 25));
    assert_eq!((f.f.knock_strength, f.f.knock_angle), (0x34, -0x123));
    assert_eq!((fs[1].s.impact_class, f.s.impact_own, f.s.trail_mode, f.s.trail_time), (0x43, 0x58, -1, -1));
    assert_eq!((f.f.attack, f.f.recovery, f.f.startup, f.f.active, f.s.sound_timer), (-1, -1, 32, 48, 32));
}

#[test]
fn timers_damage_and_revert() {
    // 14.00 down timer = a·K >> 3; 19.02 throw damage from the attacker's damage byte
    let (r, pc) = res(&script(&[op(0x14, 0, &[10]), op(0x19, 2, &[0x100])]));
    let mut fs = fighters();
    fs[1].f.attack_damage = 30;
    let mut w = World::new();
    w.run(&r, &mut fs, pc);
    // 30·0x100 >> 7 = 60: B8 = 60·153 >> 3, gauges: attacker + 30·60, own + 20·60
    assert_eq!((fs[0].f.down_timer, fs[0].f.queued_damage), (((60 * 153) >> 3) as i16, 60));
    assert_eq!((fs[1].f.gauge_queued, fs[0].f.gauge_queued), (1800, 1200));
    // transformed (F.5 & 0x10): the own gauge loses (3·60/2)·60; the gauge below 0 reverts the beast
    let mut fs = fighters();
    fs[1].f.attack_damage = 30;
    fs[0].f.form = 0x11;
    fs[0].cur = 1;
    fs[0].f.gauge = -1;
    fs[0].f.damage = 40;
    fs[0].forms[1].md[0].wt = [1, 2, 3];
    w.out.clear();
    w.run(&r, &mut fs, pc + 4);
    let f = &fs[0];
    assert_eq!((f.f.gauge_queued, f.f.gauge, f.f.form, f.cur, f.f.damage_perm), (0, 0, 0, 0, 40));
    assert_eq!(f.forms[0].md[0].wt, [1, 2, 3]);
    assert_eq!(w.out, vec![(0, Request::Sound { id: 4, delay: 0 }), (0, Request::Call { call: Call::EffectScript, args: [1, 0, 0, 0, 0, 0] })]);
}

#[test]
fn random_branch_and_voice() {
    // the table is 3k + 1: 29.05 at counter 0 (1 < 5) goes on, at counter 2 (7) jumps
    let (r, pc) = res(&script(&[op(0x29, 5, &[0x30]), op(0x29, 5, &[0x30])]));
    let mut fs = fighters();
    let mut w = World::new();
    w.rand = 1;
    w.run(&r, &mut fs, pc);
    assert_eq!((fs[0].pc, w.rand), (0x30, 3));
    // 26.05: voice rand % 3 + 0x61 (human); P2 remap only for ids ≥ 0xC0
    let (r, pc) = res(&script(&[op(0x26, 5, &[0]), op(0x26, 0, &[0xC1])]));
    w.rand = 4;
    w.out.clear();
    fs[0].f.player = 1;
    w.run(&r, &mut fs, pc);
    assert_eq!(w.out, vec![(0, Request::Sound { id: 13 % 3 + 0x61, delay: 0 }), (0, Request::Sound { id: 0x101, delay: 0 })]);
}

#[test]
fn velocity_position_and_wall_ops() {
    // 09.04 with b = 0: the R3000's quotient −1 (no guard); 07.06 the ring walls of stage 2, bits in order
    let (r, pc) = res(&script(&[op(9, 4, &[100, 0]), op(7, 6, &[50, 0]), op(0x24, 0x10, &[]), op(0x24, 0, &[])]));
    let mut fs = fighters();
    fs[0].f.opp_dist = 300;
    fs[0].f.f64 = 1 | 2 | 8;
    let mut w = World::new();
    w.run(&r, &mut fs, pc);
    assert_eq!(fs[0].f.launch_speed, -1);
    assert_eq!((fs[0].anim.pos[0], fs[0].anim.pos[2]), ((50 - 1472) as i16, (50 - 1600) as i16));
    // 24 gated (human only, beast): pc + 4, so the 24.00 after it is skipped — with F.54 bit 24 it finds wall 0
    fs[0].f.form = 1;
    fs[0].f.f54 = 0x100_0000;
    fs[0].s.res = 7;
    w.run(&r, &mut fs, pc + 12);
    assert_eq!((fs[0].pc, fs[0].s.res), (pc + 16, 7));
    w.run(&r, &mut fs, pc + 14);
    assert_eq!(fs[0].s.res, 1);
}

/// Ops the reference runs do not reach: 28 / 11 (the auto-turn request), 22 (a target's value), 27 (sounds with a
/// delay, the delayed slot), 0C.04 (blend from the push speed), 1F.03.
#[test]
fn rarer_ops() {
    let (r, pc) =
        res(&script(&[op(0x28, 8, &[]), op(0x22, 0x42, &[100]), op(0x27, 0, &[0xC4, 2]), op(0x27, 1, &[0x33, 3]), op(0x0C, 4, &[2]), op(0x1F, 3, &[0x70])]));
    let mut fs = fighters();
    fs[0].f.player = 1;
    fs[0].f.step = 8;
    fs[0].f.push_speed = 0x1000;
    fs[0].f.launch_speed = -0x200;
    fs[1].s.state_req = 130;
    let mut w = World::new();
    w.run(&r, &mut fs, pc);
    let f = &fs[0];
    assert_eq!((f.s.auto_turn, f.f.f58, f.f.f54), (8, 8, 0x2_0000));
    assert_eq!(f.s.res, 30);
    // 27.00: delay (2 << 4) / F.80 = 4, P2's 0xC4 → 0x104
    assert_eq!(w.out, vec![(0, Request::Sound { id: 0x104, delay: 4 })]);
    assert_eq!((f.s.delayed_sound, f.s.delayed_timer), (0x33, 48));
    // 0C.04: 0x1000 / 512 + 2 = 10 frames
    assert_eq!((f.anim.d0, f.anim.left), (160, 160));
    // 1F.03: decay a, the launch speed's size added to the push speed, then 0
    assert_eq!((f.f.push_decay, f.f.push_speed, f.f.launch_speed), (0x70, 0x1200, 0));
    let (r, pc) = res(&op(0x11, 0, &[]));
    w.run(&r, &mut fs, pc);
    assert_eq!((fs[0].s.auto_turn, fs[0].f.f58, fs[0].f.f54), (0, 0, 0));
}

/// Op 0x05 sub 0 on a clip that neither the fighter nor CMN.ACT has: the null clip from low RAM, which never ends.
#[test]
fn null_clip() {
    let (r, pc) = res(&script(&[op(5, 0, &[0]), op(2, 0xFF, &[])]));
    let mut fs = fighters();
    let mut w = World::new();
    w.run(&r, &mut fs, pc);
    let a = &fs[0].anim;
    assert_eq!((a.clip, a.src, a.total, a.key, a.d0, a.left, a.speed, a.target, a.next_dur), (None, Src::Cmn, 0xFF0, 1, 0x75A0, 0x75A0, 16, 8, 13312));
    assert_eq!(fs[0].pc, pc + 4);
    let mut ctx =
        Ctx { res: [&r, &r], rand: &mut w.rand, stage: 0, round: 0, walls: [false; 4], need: 2, g_ad: 0, g_e8: 0, out: &mut w.out, trace: None, rings: None };
    for _ in 0..3 {
        step(&mut fs, 0, &mut ctx);
    }
    assert_eq!((fs[0].pc, fs[0].anim.left), (pc + 4, 0x75A0));
}

#[test]
fn start_state_and_model_switch() {
    let (r, _) = res(&[]);
    let mut fs = fighters();
    let f = &mut fs[0];
    f.f.f54 = !0;
    f.f.f58 = !0;
    f.f.v_speed = 9;
    f.f.move_time = 5;
    start_state(f, &r, 3);
    assert_eq!(
        (f.pc, f.loop_mark, f.f.f54, f.f.f58, f.f.v_speed, f.f.attack, f.s.delayed_timer),
        (r.anm.states[3] as u32, r.anm.states[3] as u32, 0xFFFC_7FD7, 0xA9DF_FFEF, 0, -1, -1)
    );
    assert!(f.parts.is_hidden(0, 15) && !f.parts.is_hidden(0, 11));
    f.forms[0].rec[5].twist = 77;
    f.forms[0].md[4].wt = [4, 5, 6];
    model_switch(f, 1);
    assert_eq!((f.cur, f.f.weight, f.forms[1].rec[5].twist, f.forms[1].md[4].wt, f.forms[1].md[3].wt), (1, 820, 77, [4, 5, 6], [0; 3]));
}

#[test]
fn unused_state_entry_stops() {
    // an entry of 0xFFFF gives pc 0xFFFF_FFFF: `run` stops there on every target and emits nothing
    let (mut r, _) = res(&[]);
    let o = r.anm.header[1] as usize + 2 * 3;
    r.anm.bytes[o..o + 2].copy_from_slice(&0xFFFFu16.to_le_bytes());
    let mut fs = fighters();
    start_state(&mut fs[0], &r, 3);
    assert_eq!(fs[0].pc, 0xFFFF_FFFF);
    let mut w = World::new();
    for pc in [0xFFFF_FFFF, 0xFFFF_FFFE] {
        w.run(&r, &mut fs, pc);
        assert_eq!(fs[0].pc, pc);
    }
    assert!(w.out.is_empty());
}

/// The reference runs (`.research/br1/specs/s4/runs.sh`): the port's script step against every step of the original
/// in them, from each step's own entry state; every field, the random counter and the requests must agree.
#[test]
fn reference_runs() {
    use crate::fight::br1::reference;
    use crate::formats::br1_anm::{CHARACTERS, tests::br1_file};
    let (Some(cmn), Some(tables)) = (br1_file("engine/CMN/CMN.ACT"), crate::formats::tables::br1_export_tables()) else {
        eprintln!("skipped: no okb_export/br1");
        return;
    };
    let tables = crate::fight::br1::res::Br1Tables::new(tables).unwrap();
    let res: Vec<Option<Br1Res>> =
        CHARACTERS.iter().map(|c| br1_file(&format!("engine/{c}/{c}31.DAT")).and_then(|a| Br1Res::new(c, a, cmn.clone(), &tables).ok())).collect();
    let out = concat!(env!("CARGO_MANIFEST_DIR"), "/../.research/br1/harness/out");
    let (mut steps, mut runs) = (0, 0);
    for run in ["s4a", "s4b", "s4c", "s4d", "s4e", "s4f"] {
        let Ok(bytes) = std::fs::read(format!("{out}/{run}/steps.bin")) else { continue };
        runs += 1;
        let snaps = reference::parse(&bytes).unwrap();
        let events = reference::parse_events(&std::fs::read_to_string(format!("{out}/{run}/events.txt")).unwrap_or_default());
        for p in snaps.windows(2).filter(|p| p[0].tag == 1 && p[1].tag == 2 && p[0].side == p[1].side) {
            let (pre, post, me) = (&p[0], &p[1], p[0].side);
            let rs = [0, 1].map(|k| res[pre.chr(k)].as_ref().unwrap());
            let mut fs = [reference::load(pre, 0, rs), reference::load(pre, 1, rs)];
            let mut trace = Vec::new();
            let (reqs, rand) = reference::run_step(&mut fs, me, rs, &reference::globals(pre), pre.rand, &mut trace);
            for k in 0..2 {
                let d = reference::diff(&fs[k], &reference::load(post, k, rs));
                assert!(d.is_empty(), "{run} frame {} P{} step, fighter {k}: {d:?}", pre.frame, me + 1);
            }
            assert_eq!(rand, post.rand, "{run} frame {}", pre.frame);
            let md = [0, 1].map(|k| u32::from_le_bytes(pre.f(k)[0x1C..0x20].try_into().unwrap()));
            let theirs: Vec<Request> = events
                .iter()
                .filter(|e| e.frame == pre.frame && e.fighter == reference::FIGHTERS[me])
                .filter_map(|e| {
                    reference::event_request(e, md)
                        .or_else(|| reference::event_request(e, [0, 1].map(|k| u32::from_le_bytes(post.f(k)[0x1C..0x20].try_into().unwrap()))))
                })
                .collect();
            let ours: Vec<Request> =
                reqs.into_iter().map(|r| r.1).filter(|r| !matches!(r, Request::Cam { .. } | Request::Call { call: Call::Idle, .. })).collect();
            assert_eq!(ours, theirs, "{run} frame {} P{}", pre.frame, me + 1);
            steps += 1;
        }
    }
    if runs == 0 {
        eprintln!("skipped: no reference runs in {out}");
    } else {
        eprintln!("{runs} runs, {steps} steps bit-exact");
    }
}

/// The census of script_ops.md §14 on the install's nine fighters: a linear decode with `OPLEN` from the first state's
/// script to the per-opponent table lands on every state start and finds no opcode ≥ 0x2A; and every op 0x05 sub 0
/// clip that is neither the fighter's nor CMN.ACT's is one of the known null clips.
#[test]
fn script_census() {
    use crate::formats::br1_anm::{Anm, CHARACTERS, Clips, tests::br1_file};
    let (Some(cmn), Some(tables)) = (br1_file("engine/CMN/CMN.ACT"), crate::formats::tables::br1_export_tables()) else {
        eprintln!("skipped: no okb_export/br1");
        return;
    };
    let tables = crate::fight::br1::res::Br1Tables::new(tables).unwrap();
    let cmn = Clips::parse(&cmn, 0).unwrap();
    let (mut nulls, mut past) = (std::collections::BTreeSet::new(), std::collections::BTreeSet::new());
    for c in CHARACTERS {
        let Some(b) = br1_file(&format!("engine/{c}/{c}31.DAT")) else { continue };
        let anm = match Anm::parse(b) {
            Ok(a) => a,
            Err(e) => {
                eprintln!("{c}: {e}");
                continue;
            }
        };
        let starts: std::collections::BTreeSet<usize> = (0..anm.states.len()).filter_map(|n| anm.state(n)).filter(|&s| s > 0).collect();
        let (mut p, end) = (*starts.first().unwrap(), anm.header[2] as usize);
        let mut seen = std::collections::BTreeSet::new();
        while p < end {
            seen.insert(p);
            let (op, sub) = (anm.u8(p), anm.u8(p + 1));
            assert!((op as usize) < 0x2A, "{c}: op {op:#x} at {p:#x}");
            if op == 5 && sub & 0xF == 0 {
                let a = anm.i16(p + 2) as i32;
                // an empty own slot below 0x100 takes CMN.ACT's entry as it is: an empty one (0: the null clip,
                // `ops::null_clip`) is modelled, one past CMN.ACT's table (its −1 end, then data) is not
                if !(a == -1 || a >= anm.clips.count() as i32 || anm.clips.get(a as usize).is_some() || a >= 0x100 || cmn.get(a as usize).is_some()) {
                    if (0..cmn.count() as i32).contains(&a) {
                        nulls.insert(a)
                    } else {
                        past.insert(a)
                    };
                }
            }
            p += OPLEN[op as usize] as usize;
        }
        assert!(starts.iter().all(|s| seen.contains(s) || *s >= end), "{c}: a state start off the linear decode");
    }
    // the null clips the scripts name (every fighter), and clip 143 of state 47 (every fighter) past CMN.ACT's table:
    // bytes of the EXE image (CMN.ACT's word 143 = 0x720DE6 → RAM 0x80120DE6, `ops::image_clip`)
    if !nulls.is_empty() {
        assert_eq!((nulls, past), ([54, 55].into(), [143].into()));
        let r = Br1Res::new("MIT", br1_file("engine/MIT/MIT31.DAT").unwrap(), br1_file("engine/CMN/CMN.ACT").unwrap(), &tables).unwrap();
        assert_eq!(super::ops::image_clip(&r, 143), Some(0x15E6));
        // played: the header and keys of the EXE's bytes there
        let mut r = r;
        let pc = r.anm.bytes.len() as u32;
        r.anm.bytes.extend_from_slice(&[5, 0, 143, 0]);
        let mut fs = fighters();
        let mut w = World::new();
        w.run(&r, &mut fs, pc);
        let a = &fs[0].anim;
        assert_eq!((a.src, a.clip, a.total, a.key), (Src::Image, Some(0x15E6), 18241i16.wrapping_shl(4), 1));
        let mut ctx = Ctx {
            res: [&r, &r],
            rand: &mut w.rand,
            stage: 0,
            round: 0,
            walls: [false; 4],
            need: 2,
            g_ad: 0,
            g_e8: 0,
            out: &mut w.out,
            trace: None,
            rings: None,
        };
        for _ in 0..4 {
            step(&mut fs, 0, &mut ctx);
        }
    }
}

/// Every clip of the executable a fighter's op 0x05 sub 0 can start (an empty own slot below its count, CMN.ACT's
/// word past its table: `ops::image_clip`), played from key 1 and after a jump of op 0x0C sub 1 to each key the scripts
/// jump to, in both facings, for 1200 frames: every read (`pose::image_reads`) lies whole inside one of the clips' ranges
/// (`tables::BR1_CLIP_RANGES`), so none lands in the zeros `tables::BR1_CLIPS` holds between them; the first range one
/// byte shorter, a read falls outside (data-gated: the install's tables).
#[test]
fn image_clips_read_inside_the_clip_ranges() {
    use crate::fight::br1::pose::image_reads;
    use crate::formats::br1_anm::{CHARACTERS, tests::br1_file};
    let (Some(cmn), Some(tables)) = (br1_file("engine/CMN/CMN.ACT"), crate::formats::tables::br1_export_tables()) else {
        eprintln!("skipped: no okb_export/br1");
        return;
    };
    let ranges = tables.clip_ranges();
    let tables = crate::fight::br1::res::Br1Tables::new(tables).unwrap();
    let (mut clips, mut reads) = (std::collections::BTreeSet::new(), Vec::new());
    for c in CHARACTERS {
        let Some(anm) = br1_file(&format!("engine/{c}/{c}31.DAT")) else { continue };
        let r = Br1Res::new(c, anm, cmn.clone(), &tables).unwrap();
        let ks: Vec<u8> = (0..r.anm.clips.count().min(0x100))
            .filter(|&k| r.anm.clips.get(k).is_none() && super::ops::image_clip(&r, k as i32).is_some())
            .map(|k| k as u8)
            .collect();
        for k in ks {
            clips.insert(super::ops::image_clip(&r, k as i32).unwrap());
            for jump in [None, Some(2u8), Some(3), Some(6), Some(7), Some(8)] {
                for mirror in [false, true] {
                    let mut r2 = Br1Res::new(c, r.anm.bytes.clone(), cmn.clone(), &tables).unwrap();
                    let pc = r2.anm.bytes.len() as u32;
                    r2.anm.bytes.extend_from_slice(&[5, 0, k, 0]);
                    if let Some(a) = jump {
                        r2.anm.bytes.extend_from_slice(&[0x0C, 1, a, 0]);
                    }
                    image_reads::watch(&[&r2.image_clips[..]]);
                    let mut fs = fighters();
                    fs[0].anim.mirror = mirror;
                    let mut w = World::new();
                    w.run(&r2, &mut fs, pc);
                    let mut ctx = Ctx {
                        res: [&r2, &r2],
                        rand: &mut w.rand,
                        stage: 0,
                        round: 0,
                        walls: [false; 4],
                        need: 2,
                        g_ad: 0,
                        g_e8: 0,
                        out: &mut w.out,
                        trace: None,
                        rings: None,
                    };
                    for _ in 0..1200 {
                        step(&mut fs, 0, &mut ctx);
                    }
                    reads.extend(image_reads::take());
                }
            }
        }
    }
    image_reads::watch(&[]);
    eprintln!("{} clips of the executable, {} reads", clips.len(), reads.len());
    assert!(clips.len() >= 3 && !reads.is_empty());
    let outside = |ranges: &[(usize, usize)]| reads.iter().find(|&&(o, n)| !ranges.iter().any(|&(a, b)| a <= o && o + n <= b)).copied();
    assert_eq!(outside(&ranges), None, "a read outside {ranges:x?}");
    let mut shorter = ranges.clone();
    shorter[0].0 += 1;
    assert!(outside(&shorter).is_some(), "the first range's first byte is read");
}
