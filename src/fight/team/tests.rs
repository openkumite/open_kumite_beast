//! Tests of the team battle (engine addition): the targets and the round flow on made-up fighters, and with the export
//! beside the engine whole matches (no friendly fire, a throw broken by a third party, determinism, the 2 VS 2 and
//! FREE FOR ALL traces of `fixtures/team_v22_hashes.txt`).

use super::round::{Request, TeamRound};
use super::target::{self, HYSTERESIS};
use super::*;
use crate::fight::fighter::{F6C, F58, F60};
use crate::fight::input::pad::{CROSS, START};
use crate::fight::round::Message;
use crate::fight::version::{Gates, v};

fn at(k: usize, x: i32, z: i32) -> Fighter {
    let mut f = Fighter::new((k & 1) as u8, 0);
    f.m_t = [x, 0, z];
    f
}

const VS2: [usize; N] = [0, 1, 0, 1];
const FFA: [usize; N] = [0, 1, 2, 3];

#[test]
fn the_nearest_enemy_with_hysteresis_and_ties_to_the_lower_index() {
    let fs = [at(0, 0, 0), at(1, 1000, 0), at(2, 100, 0), at(3, 0, 1000)];
    let out = [false; N];
    // 2 VS 2: P1's enemies are P2 and P4 (1000 each: the tie goes to P2), never its partner P3 (100 away)
    assert_eq!(target::nearest(&fs, 0, None, &VS2, &out), Some(1));
    // FREE FOR ALL: P3 is the nearest
    assert_eq!(target::nearest(&fs, 0, None, &FFA, &out), Some(2));
    // the current target is kept while no other is nearer by more than the hysteresis
    let fs = [at(0, 0, 0), at(1, 1000, 0), at(2, 0, 0), at(3, 1000 - HYSTERESIS as i32, 0)];
    assert_eq!(target::nearest(&fs, 0, Some(1), &VS2, &out), Some(1));
    let fs = [at(0, 0, 0), at(1, 1000, 0), at(2, 0, 0), at(3, 1000 - HYSTERESIS as i32 - 1, 0)];
    assert_eq!(target::nearest(&fs, 0, Some(1), &VS2, &out), Some(3));
    // a fighter out of the round is no target; none left: None
    assert_eq!(target::nearest(&fs, 0, Some(1), &VS2, &[false, true, false, false]), Some(3));
    assert_eq!(target::nearest(&fs, 0, None, &VS2, &[false, true, false, true]), None);
}

#[test]
fn l2_cycles_and_locks_a_throw_keeps_its_partner() {
    let fs = [at(0, 0, 0), at(1, 500, 0), at(2, 0, 100), at(3, 2000, 0)];
    let (mut tgt, mut locked, out) = ([1, 0, 3, 2], [false; N], [false; N]);
    // P1 presses L2: the next enemy (P4), locked although P2 is nearer
    target::update(&fs, &mut tgt, &mut locked, &VS2, &out, [true, false, false, false], [false; N]);
    assert_eq!((tgt[0], locked[0]), (3, true));
    target::update(&fs, &mut tgt, &mut locked, &VS2, &out, [false; N], [false; N]);
    assert_eq!(tgt[0], 3, "locked");
    // again: back to P2 (the next after P4, wrapping)
    target::update(&fs, &mut tgt, &mut locked, &VS2, &out, [true, false, false, false], [false; N]);
    assert_eq!(tgt[0], 1);
    // the locked target goes out: the lock ends, the nearest rule picks the one left
    tgt[0] = 3;
    target::update(&fs, &mut tgt, &mut locked, &VS2, &[false, false, false, true], [false; N], [false; N]);
    assert_eq!((tgt[0], locked[0]), (1, false));
    // a thrower and its victim keep each other, whoever is nearer
    let mut fs = fs;
    fs[0].flags[F58] |= 0x4000_0000;
    fs[3].flags[F58] |= 0x10_0000;
    let mut tgt = [3, 0, 3, 0];
    target::update(&fs, &mut tgt, &mut [false; N], &VS2, &out, [false; N], [false; N]);
    assert_eq!((tgt[0], tgt[3]), (3, 0));
    // this frame's grab request locks as well
    fs[0].flags[F58] = 0;
    fs[3].flags[F58] = 0;
    fs[0].flags[F6C] |= 0x40_0000;
    fs[3].flags[F6C] |= 0x80_0000;
    target::update(&fs, &mut tgt, &mut [false; N], &VS2, &out, [false; N], [false; N]);
    assert_eq!((tgt[0], tgt[3]), (3, 0));
}

/// A round in state 4 with `fs` (damage `o_ac` of `o_b0` = 200 each), run for `frames` frames or until it leaves 3 / 4.
fn fight(r: &mut TeamRound, fs: &mut [Fighter; N], mode: TeamMode) {
    fight_v(r, fs, mode, v::TEAM);
}

fn fight_v(r: &mut TeamRound, fs: &mut [Fighter; N], mode: TeamMode, sim_version: u32) {
    r.state = 4;
    for _ in 0..3 {
        r.tick(fs, mode, &mut Vec::new(), 0, Gates::of(sim_version), None);
        if r.state != 4 {
            return;
        }
    }
}

fn fighters() -> [Fighter; N] {
    std::array::from_fn(|k| {
        let mut f = at(k, 0, 0);
        f.o_b0 = 200;
        f
    })
}

#[test]
fn the_last_team_standing_wins_the_round_and_two_rounds_the_match() {
    let mut r = TeamRound::new();
    let mut fs = fighters();
    // 2 VS 2: P2 KO'd, team 1 still has P4: the round goes on
    fs[1].o_00 = 0x80;
    fight(&mut r, &mut fs, TeamMode::Vs2);
    assert_eq!((r.state, r.out), (4, [false, true, false, false]));
    // P4 rung out too: team 0 wins the round
    fs[3].o_00 = 0x81;
    fight(&mut r, &mut fs, TeamMode::Vs2);
    assert_eq!((r.state, &r.round_winners[..]), (5, &[0][..]));
    let mut req = Request::None;
    while req == Request::None && r.state != 0xD {
        req = r.tick(&mut fs, TeamMode::Vs2, &mut Vec::new(), 0, Gates::of(v::TEAM), None);
    }
    assert_eq!((req, r.wins[0]), (Request::NextRound(2), 1));
    r.start();
    r.index = 1;
    let mut fs = fighters();
    fs[1].o_00 = 0x80;
    fs[3].o_00 = 0x80;
    fight(&mut r, &mut fs, TeamMode::Vs2);
    let mut req = Request::None;
    while req == Request::None {
        req = r.tick(&mut fs, TeamMode::Vs2, &mut Vec::new(), 0, Gates::of(v::TEAM), None);
    }
    assert_eq!((req, r.winner), (Request::MatchOver(0), Some(0)));
}

#[test]
fn time_over_the_most_health_wins_a_tie_is_a_draw_for_the_tied() {
    // 2 VS 2: the team's sum (team 1: 150 + 150 left beats team 0: 200 + 50)
    let mut r = TeamRound::new();
    let mut fs = fighters();
    (fs[0].o_ac, fs[2].o_ac, fs[1].o_ac, fs[3].o_ac) = (0, 150, 50, 50);
    r.timer = 1;
    fight(&mut r, &mut fs, TeamMode::Vs2);
    assert_eq!((r.state, &r.round_winners[..]), (5, &[1][..]));
    // FREE FOR ALL: the highest single health; P1 and P3 tied: a draw for both, each gets the round
    let mut r = TeamRound::new();
    let mut fs = fighters();
    (fs[0].o_ac, fs[1].o_ac, fs[2].o_ac, fs[3].o_ac) = (20, 100, 20, 30);
    r.timer = 1;
    fight(&mut r, &mut fs, TeamMode::Ffa);
    assert_eq!((r.state, &r.round_winners[..]), (8, &[0, 2][..]));
    r.tick(&mut fs, TeamMode::Ffa, &mut Vec::new(), 0, Gates::of(v::TEAM), None);
    assert_eq!(r.wins, [1, 0, 1, 0]);
    // a fighter out counts nothing
    let mut r = TeamRound::new();
    let mut fs = fighters();
    fs[1].o_00 = 0x81;
    (fs[0].o_ac, fs[2].o_ac, fs[3].o_ac) = (100, 100, 90);
    r.timer = 1;
    fight(&mut r, &mut fs, TeamMode::Ffa);
    assert_eq!((r.state, &r.round_winners[..]), (5, &[3][..]));
}

/// (T6) A SUDDEN DEATH time over (every fighter started it at no health, reset mode 3) is a DRAW for every team still
/// standing from simulation v24, whatever few points some won back; v22 / v23 keep the most health %.
#[test]
fn a_sudden_death_time_over_is_a_draw_from_v24() {
    for (sim, mode, want) in [
        (v::SUDDEN_DEATH_DRAW, TeamMode::Ffa, (8, &[0, 2, 3][..])),
        (v::TEAM, TeamMode::Ffa, (5, &[0][..])),
        (v::SUDDEN_DEATH_DRAW, TeamMode::Vs2, (8, &[0, 1][..])),
        (v::TEAM, TeamMode::Vs2, (5, &[0][..])),
    ] {
        let mut r = TeamRound::new();
        r.index = -1;
        r.wins = match mode {
            TeamMode::Ffa => [2; N],
            TeamMode::Vs2 => [2, 2, 0, 0],
        };
        let mut fs = fighters();
        fs[1].o_00 = 0x80;
        (fs[0].o_ac, fs[1].o_ac, fs[2].o_ac, fs[3].o_ac) = (198, 200, 199, 200);
        r.timer = 1;
        fight_v(&mut r, &mut fs, mode, sim);
        assert_eq!((r.state, &r.round_winners[..]), want, "v{sim} {mode:?}");
        r.check(mode).expect("a state tick makes");
    }
}

/// (T7) Several teams reaching the rounds needed at once (here a D.K.O. in the deciding round) end the match in a DRAW
/// GAME from simulation v25: state 0xD without a winner, the winners' pose for nobody, then `MatchDraw`; before v25 a
/// SUDDEN DEATH round follows.
#[test]
fn teams_reaching_the_rounds_needed_together_draw_the_match_from_v25() {
    for (sim, mode) in [(v::DRAW_GAME, TeamMode::Vs2), (v::DRAW_GAME, TeamMode::Ffa), (v::DRAW_GAME - 1, TeamMode::Vs2)] {
        let mut r = TeamRound::new();
        r.index = 2;
        r.wins = match mode {
            TeamMode::Ffa => [1, 1, 0, 1],
            TeamMode::Vs2 => [1, 1, 0, 0],
        };
        let mut fs = fighters();
        // P3 out before; P1, P2 and P4 go down together
        fs[2].o_00 = 0x80;
        fight_v(&mut r, &mut fs, mode, sim);
        for k in [0, 1, 3] {
            fs[k].o_00 = 0x80;
        }
        fight_v(&mut r, &mut fs, mode, sim);
        assert_eq!(r.state, 7, "v{sim} {mode:?}: D.K.O.");
        let mut req = Request::None;
        let mut messages = Vec::new();
        while req == Request::None {
            req = r.tick(&mut fs, mode, &mut Vec::new(), 0, Gates::of(sim), None);
            messages.push(r.message);
            r.check(mode).expect("a state tick makes");
        }
        if Gates::of(sim).draw_game {
            assert_eq!((req, r.winner, r.state), (Request::MatchDraw, None, 0xD), "v{sim} {mode:?}");
            assert!(messages.contains(&Message::DrawGame), "v{sim} {mode:?}");
            assert_eq!(r.wins.iter().filter(|&&w| w == 2).count(), if mode == TeamMode::Vs2 { 2 } else { 3 });
        } else {
            assert_eq!((req, r.index), (Request::NextRound(3), -1), "v{sim}: the sudden-death round");
        }
    }
}

/// (T7) A kept target stays while it is in the round, whatever is nearer; out, the nearest rule picks again.
#[test]
fn a_kept_target_stays_until_it_is_out() {
    let fs = [at(0, 0, 0), at(1, 3000, 0), at(2, 0, 100), at(3, 100, 0)];
    let mut tgt = [1, 0, 1, 0];
    let keep = [true, false, false, false];
    target::update(&fs, &mut tgt, &mut [false; N], &VS2, &[false; N], [false; N], keep);
    assert_eq!(tgt[0], 1, "kept though P4 is nearer");
    target::update(&fs, &mut tgt, &mut [false; N], &VS2, &[false, true, false, false], [false; N], keep);
    assert_eq!(tgt[0], 3, "out: the nearest");
}

/// (T7) A thrown fighter, a human's as a CPU's (`thrown` does not look at who plays it), keeps its thrower after the
/// release whatever is nearer, until it lies / gets up / stands (`done`), its thrower is out, or for `THROW_FRAMES`.
#[test]
fn a_thrown_fighter_keeps_its_thrower_until_it_is_done() {
    // FREE FOR ALL: P2 throws P1; P4 is next to P1, P2 far off
    let mut fs = [at(0, 0, 0), at(1, 3000, 0), at(2, 0, 3000), at(3, 100, 0)];
    fs[1].flags[F58] |= 0x4000_0000;
    fs[0].flags[F58] |= 0x10_0000;
    let (mut tgt, mut locked, mut by, out) = ([1, 0, 0, 0], [false; N], [None; N], [false; N]);
    let step = |fs: &[Fighter; N], tgt: &mut [usize; N], locked: &mut [bool; N], by: &mut _, done, out: &[bool; N]| {
        let keep = target::thrown(fs, tgt, by, done, out);
        target::update(fs, tgt, locked, &FFA, out, [false; N], keep);
    };
    step(&fs, &mut tgt, &mut locked, &mut by, [false; N], &out);
    assert_eq!((tgt[0], tgt[1], by[0]), (1, 0, Some((1, 0))), "held");
    // the release: P1 keeps P2 though P4 is nearer by far more than the hysteresis
    fs[1].flags[F58] = 0;
    fs[0].flags[F58] = 0;
    for _ in 0..30 {
        step(&fs, &mut tgt, &mut locked, &mut by, [false; N], &out);
        assert_eq!(tgt[0], 1, "kept after the release");
    }
    assert_eq!(by[0], Some((1, 30)));
    // lying (or getting up, standing): the nearest rule again
    step(&fs, &mut tgt, &mut locked, &mut by, [true, false, false, false], &out);
    assert_eq!((tgt[0], by[0]), (3, None), "done");
    // the thrower out, or THROW_FRAMES after the release: the same
    let (mut tgt, mut by) = ([1, 0, 0, 0], [Some((1, 0)), None, None, None]);
    step(&fs, &mut tgt, &mut locked, &mut by, [false; N], &[false, true, false, false]);
    assert_eq!((tgt[0], by[0]), (3, None), "thrower out");
    let (mut tgt, mut by) = ([1, 0, 0, 0], [Some((1, crate::fight::br1::bridge::THROW_FRAMES - 1)), None, None, None]);
    step(&fs, &mut tgt, &mut locked, &mut by, [false; N], &out);
    assert_eq!((tgt[0], by[0]), (3, None), "THROW_FRAMES");
}

/// (T7) `Locks` without a thrown fighter's lock serialises as the `[bool; N]` before it; with one it round-trips, and one out of range is refused.
#[test]
fn locks_keep_the_bytes_of_the_l2_locks() {
    let l2 = [true, false, false, true];
    let locks = target::Locks { l2, thrown_by: [None; N] };
    assert_eq!(postcard::to_allocvec(&locks).unwrap(), postcard::to_allocvec(&l2).unwrap());
    let locks = target::Locks { l2, thrown_by: [None, Some((2, 300)), None, Some((0, 7))] };
    let back: target::Locks = postcard::from_bytes(&postcard::to_allocvec(&locks).unwrap()).unwrap();
    assert_eq!(back, locks);
    // a thrower or a frame count out of range is refused
    for bad in [Some((N as u8, 0)), Some((0, crate::fight::br1::bridge::THROW_FRAMES))] {
        let locks = target::Locks { l2, thrown_by: [None, bad, None, None] };
        assert!(postcard::from_bytes::<target::Locks>(&postcard::to_allocvec(&locks).unwrap()).is_err());
    }
}

#[test]
fn the_last_ones_down_together_is_a_dko_for_their_teams() {
    let mut r = TeamRound::new();
    let mut fs = fighters();
    fs[0].o_00 = 0x80;
    fs[2].o_00 = 0x80;
    fight(&mut r, &mut fs, TeamMode::Ffa);
    assert_eq!(r.state, 4);
    fs[1].o_00 = 0x80;
    fs[3].o_00 = 0x80;
    fight(&mut r, &mut fs, TeamMode::Ffa);
    assert_eq!((r.state, &r.round_winners[..]), (7, &[1, 3][..]));
}

#[test]
fn a_decoded_round_with_states_or_teams_tick_cannot_handle_is_refused() {
    let mut r = TeamRound::new();
    assert!(r.check(TeamMode::Vs2).is_ok());
    // state 5 indexes its first winner
    r.state = 5;
    assert!(r.check(TeamMode::Vs2).is_err());
    r.round_winners = vec![1];
    assert!(r.check(TeamMode::Vs2).is_ok());
    // a team of FREE FOR ALL is none of 2 VS 2
    r.round_winners = vec![3];
    assert!(r.check(TeamMode::Vs2).is_err() && r.check(TeamMode::Ffa).is_ok());
    r.winner = Some(4);
    assert!(r.check(TeamMode::Ffa).is_err());
    r.winner = None;
    r.state = 6;
    assert!(r.check(TeamMode::Ffa).is_err());
}

pub(super) fn export() -> Option<crate::assets::Assets> {
    let root = crate::base::env::var("EXPORT").unwrap_or_else(|_| concat!(env!("CARGO_MANIFEST_DIR"), "/../okb_export").into());
    std::path::Path::new(&root).join("manifest.json").exists().then(|| crate::assets::Assets::new(root))
}

/// The team battles of the trace fixture: name, mode, fighters, stage.
const FIXTURES: [(&str, TeamMode, [&str; N], &str); 2] =
    [("team_2v2_st0", TeamMode::Vs2, ["YUG", "ALI", "BAK", "ULI"], "ST0"), ("team_ffa_st0", TeamMode::Ffa, ["GAD", "JEN", "STU", "LON"], "ST0")];

fn load(a: &crate::assets::Assets, chr: [&str; N], stage: &str) -> Loaded {
    pollster::block_on(crate::fight::fight_data::load_team(a, chr, [0; N], stage, v::TEAM)).expect("the team battle's data")
}

/// A four-CPU match to its end (at most 20000 frames, the HUD stepped as the session steps it), `each` after every
/// frame; its trace: FNV-1a 64 of the state's postcard bytes folded after every frame, at every 500th and the last
/// (`name frame hash` lines).
pub(super) fn play(data: &Loaded, mode: TeamMode, name: &str, mut each: impl FnMut(&TeamFight)) -> (TeamFight, Vec<String>) {
    use crate::base::hash::{FNV_BASIS, fnv1a};
    let mut t = TeamFight::new(data, mode);
    let (mut h, mut out) = (FNV_BASIS, Vec::new());
    while t.result.is_none() && t.frame < 20000 {
        t.tick(data);
        t.hud_step();
        each(&t);
        h = fnv1a(h, &postcard::to_allocvec(&t).expect("serialises"));
        if t.frame.is_multiple_of(500) {
            out.push(format!("{name} {} {h:016x}", t.frame));
        }
    }
    out.push(format!("{name} {} {h:016x} {:?}", t.frame, t.result));
    (t, out)
}

/// Both fixtures play to a winner, with hits but never on a team-mate, and trace as recorded (`team_fixture_hashes_record`).
#[test]
fn team_fixtures() {
    let Some(a) = export() else {
        eprintln!("skipped: no okb_export");
        return;
    };
    let want = include_str!("../../session/fixtures/team_v22_hashes.txt");
    for (name, mode, chr, stage) in FIXTURES {
        let data = load(&a, chr, stage);
        let (t, trace) = play(&data, mode, name, |_| {});
        assert!(matches!(t.result, Some(TeamOutcome::Winner(_))), "{name}: {:?}", t.result);
        let hits: Vec<(usize, usize)> = t
            .events
            .iter()
            .filter_map(|e| {
                let (_, rest) = e.split_once(": P")?;
                let (a, rest) = rest.split_once(' ')?;
                let d = rest.rsplit_once(" P").map(|(_, d)| d.split(' ').next().unwrap_or(""))?;
                Some((a.parse::<usize>().ok()? - 1, d.parse::<usize>().ok()? - 1))
            })
            .collect();
        assert!(hits.len() > 20, "{name}: {} hits", hits.len());
        assert!(hits.iter().all(|&(a, d)| mode.team(a) != mode.team(d)), "{name}: friendly fire");
        let lines: Vec<&str> = want.lines().filter(|l| l.starts_with(&format!("{name} "))).collect();
        assert_eq!(trace, lines, "{name}: the trace");
    }
}

/// The FREE FOR ALL fixture: P2's hit at frame 498 lands on both P1 and P3 (one attack, two enemies), P3 was throwing
/// P1: the throw breaks, P3 goes into a hit reaction and P1 drops out of the throw; P1's damage counts, its throw
/// reaction does not run (damage only).
#[test]
fn a_hit_on_the_thrower_breaks_the_throw() {
    let Some(a) = export() else {
        eprintln!("skipped: no okb_export");
        return;
    };
    let (name, mode, chr, stage) = FIXTURES[1];
    let data = load(&a, chr, stage);
    let mut seen = (false, false);
    let (t, _) = play(&data, mode, name, |t| {
        if t.frame == 498 {
            seen.0 = t.fighters[2].flags[F58] & 0x4000_0000 != 0 && t.fighters[0].flags[F58] & 0x10_0000 != 0 && t.target[0] == 2;
        }
        if t.frame == 500 {
            let (thrower, victim) = (&t.fighters[2], &t.fighters[0]);
            seen.1 = thrower.flags[F58] & 0x4000_0000 == 0 && victim.flags[F58] & 0x10_0000 == 0 && victim.o_ac > 0 && thrower.o_ac > 0;
        }
    });
    assert_eq!(seen, (true, true), "the throw at 498, broken at 500");
    for e in ["frame 498: P2 hits P1", "frame 498: P2 hits P3", "frame 498: P3's throw on P1 broken"] {
        assert!(t.events.iter().any(|x| x.starts_with(e)), "{e}");
    }
}

/// The same match twice gives the same states; a state saved mid-match, decoded and rebound, plays on to the same end.
#[test]
fn team_battles_are_deterministic() {
    let Some(a) = export() else {
        eprintln!("skipped: no okb_export");
        return;
    };
    let (name, mode, chr, stage) = FIXTURES[0];
    let data = load(&a, chr, stage);
    let (t1, trace1) = play(&data, mode, name, |_| {});
    let (t2, trace2) = play(&data, mode, name, |_| {});
    assert_eq!(trace1, trace2);
    assert_eq!(postcard::to_allocvec(&t1).unwrap(), postcard::to_allocvec(&t2).unwrap());
    let mut t = TeamFight::new(&data, mode);
    for _ in 0..1500 {
        t.tick(&data);
        t.hud_step();
    }
    let bytes = postcard::to_allocvec(&t).unwrap();
    // a state with a fighter its own target or last victim, or a round won by no team, is refused
    let mutated = |f: fn(&mut TeamFight)| {
        let mut r: TeamFight = postcard::from_bytes(&bytes).unwrap();
        f(&mut r);
        r.rebind(&data).is_err()
    };
    assert!(mutated(|r| r.target[2] = 2));
    assert!(mutated(|r| r.last_victim[1] = 1));
    assert!(mutated(|r| {
        r.round.state = 5;
        r.round.round_winners.clear();
    }));
    // (T6) a teammate as the target or last victim (2 VS 2), wins of a team the mode does not have or past `need`,
    // `need` out of 1..=4, a round index past the last possible round
    assert_eq!(mode, TeamMode::Vs2);
    assert!(mutated(|r| r.target[0] = 2));
    assert!(mutated(|r| r.last_victim[3] = 1));
    assert!(mutated(|r| r.round.wins[2] = 1));
    assert!(mutated(|r| r.round.wins[0] = 3));
    assert!(mutated(|r| r.round.need = 0));
    assert!(mutated(|r| r.round.need = 9));
    assert!(mutated(|r| r.round.index = 3));
    assert!(mutated(|r| r.round.index = -2));
    // (R1) an effect task naming a fighter past the four (`native_tasks::State::check`)
    use crate::fight::fx::native_tasks::Task;
    assert!(mutated(|r| r.fxn.tasks.push(Task::RecoveryLater { side: 4, delay: 1 })));
    assert!(mutated(|r| r.fxn.tasks.push(Task::Recovery { m: 0, side: usize::MAX, ang: 0, size: 0 })));
    assert!(!mutated(|r| r.fxn.tasks.push(Task::RecoveryLater { side: 3, delay: 1 })));
    let mut r: TeamFight = postcard::from_bytes(&bytes).unwrap();
    r.rebind(&data).expect("rebinds");
    while t.result.is_none() && t.frame < 20000 {
        t.tick(&data);
        t.hud_step();
        r.tick(&data);
        r.hud_step();
    }
    assert_eq!(postcard::to_allocvec(&t).unwrap(), postcard::to_allocvec(&r).unwrap());
    // a match's data is refused
    let m = pollster::block_on(crate::fight::fight_data::load_match(&a, ["YUG", "ALI"], [0, 0], "ST0", false, v::TEAM)).unwrap();
    assert!(r.rebind(&m).is_err());
    // (R1) and a match refuses the team battle's data and an effect task naming a fighter past its two
    let mut mm = crate::fight::sim::Match::new(&m);
    for _ in 0..10 {
        mm.tick(&m);
    }
    let bytes = postcard::to_allocvec(&mm).unwrap();
    let decoded = |f: fn(&mut crate::fight::sim::Match)| {
        let mut x: crate::fight::sim::Match = postcard::from_bytes(&bytes).unwrap();
        f(&mut x);
        x
    };
    assert!(decoded(|_| {}).rebind(&m).is_ok());
    assert!(decoded(|_| {}).rebind(&data).is_err());
    assert!(decoded(|x| x.fxn.tasks.push(Task::RecoveryLater { side: 2, delay: 1 })).rebind(&m).is_err());
    assert!(decoded(|x| x.fx_delayed.push((2, 0, 1))).rebind(&m).is_err());
    assert!(decoded(|x| x.fxn.tasks.push(Task::RecoveryLater { side: 1, delay: 1 })).rebind(&m).is_ok());
}

/// (R1) The battles of the simulation v25 (T7) trace fixture `fixtures/team_v25_hashes.txt`: the v22 fixtures'
/// line-ups, the 2 VS 2 on ST0 inside its walls, the FREE FOR ALL on Q3DM6 against its chamfers (skipped without its
/// Quake III files).
const FIXTURES_V25: [(&str, TeamMode, [&str; N], &str); 2] =
    [("team_2v2_st0_v25", TeamMode::Vs2, ["YUG", "ALI", "BAK", "ULI"], "ST0"), ("team_ffa_q3dm6_v25", TeamMode::Ffa, ["GAD", "JEN", "STU", "LON"], "Q3DM6")];

/// The team battle's data at simulation v25; None (and a note) for Q3DM6 without its Quake III files.
fn load_v25(a: &crate::assets::Assets, chr: [&str; N], stage: &str) -> Option<Loaded> {
    match pollster::block_on(crate::fight::fight_data::load_team(a, chr, [0; N], stage, 25)) {
        Ok(d) => Some(d),
        Err(e) if stage == "Q3DM6" => {
            eprintln!("skipped {stage}: {e}");
            None
        }
        Err(e) => panic!("{e}"),
    }
}

/// (R1) Some fighter in the round has a body sphere (but the first, as `physics::chamfers` pushes them) within 16 units
/// of touching one of the stage's chamfers, or past it.
pub(super) fn at_a_chamfer(t: &TeamFight, data: &Loaded) -> bool {
    let Some(e) = crate::fight::stage::extra::by_stage(t.world.stage.id) else { return false };
    let chamfers: Vec<_> = e.chamfers(data.gates()).collect();
    (0..N).filter(|&k| !t.out(k)).any(|k| {
        let f = &t.fighters[k];
        let near = |p: &[i32; 3], r: i32| chamfers.iter().any(|c| c.n[0] * p[0] + c.n[1] * p[2] - c.c + ((r * 181) >> 7) > -16);
        t.bodies(data, k).iter().skip(1).any(|b| f.parts.get(b.node as usize).is_some_and(|p| near(p, b.radius as i32)))
    })
}

/// (R1) The v25 battles play to their end and trace as recorded (`team_v25_fixture_hashes_record`), so a change of
/// simulation v25's team battle fails here: the 2 VS 2 pushes against ST0's walls, the FREE FOR ALL's fighters reach
/// Q3DM6's chamfers; no hit on a team-mate, `watch` finds nothing.
#[test]
fn team_v25_fixtures() {
    let Some(a) = export() else {
        eprintln!("skipped: no okb_export");
        return;
    };
    let want = include_str!("../../session/fixtures/team_v25_hashes.txt");
    for (name, mode, chr, stage) in FIXTURES_V25 {
        let Some(data) = load_v25(&a, chr, stage) else { continue };
        let (mut walls, mut chamfer, mut found) = (0u32, 0u32, Vec::new());
        let mut watch = super::watch::Watch::default();
        let (t, trace) = play(&data, mode, name, |t| {
            walls += t.world.stage.walls.pow.iter().any(|&p| p > 0) as u32;
            chamfer += at_a_chamfer(t, &data) as u32;
            found.extend(watch.step(t, &data));
        });
        assert!(matches!(t.result, Some(TeamOutcome::Winner(_) | TeamOutcome::Draw)), "{name}: {:?}", t.result);
        assert!(t.events.iter().filter(|e| e.contains(" hits P")).count() > 20, "{name}: hits");
        assert!(found.is_empty(), "{name}: {found:?}");
        if stage == "Q3DM6" {
            assert!(chamfer > 0, "{name}: nobody at a chamfer");
        } else {
            assert!(walls > 0, "{name}: nobody pushed a wall");
        }
        let lines: Vec<&str> = want.lines().filter(|l| l.starts_with(&format!("{name} "))).collect();
        assert_eq!(trace, lines, "{name}: the trace");
    }
}

/// Writes `fixtures/team_v25_hashes.txt` (by hand, after an intended change of simulation v25's team battle: `cargo
/// test --release team_v25_fixture_hashes_record -- --ignored`, with the export and Q3DM6's files beside the engine).
#[test]
#[ignore]
fn team_v25_fixture_hashes_record() {
    let a = export().expect("okb_export");
    let mut lines = Vec::new();
    for (name, mode, chr, stage) in FIXTURES_V25 {
        let data = load_v25(&a, chr, stage).expect("Q3DM6's files");
        lines.extend(play(&data, mode, name, |_| {}).1);
    }
    let path = format!("{}/src/session/fixtures/team_v25_hashes.txt", env!("CARGO_MANIFEST_DIR"));
    std::fs::write(&path, lines.join("\n") + "\n").expect("written");
}

/// Writes `fixtures/team_v22_hashes.txt` (run by hand after an intended change of the team battle's state or logic:
/// `cargo test --release team_fixture_hashes_record -- --ignored`, with the export beside the engine).
#[test]
#[ignore]
fn team_fixture_hashes_record() {
    let a = export().expect("okb_export");
    let mut lines = Vec::new();
    for (name, mode, chr, stage) in FIXTURES {
        lines.extend(play(&load(&a, chr, stage), mode, name, |_| {}).1);
    }
    let path = format!("{}/src/session/fixtures/team_v22_hashes.txt", env!("CARGO_MANIFEST_DIR"));
    std::fs::write(&path, lines.join("\n") + "\n").expect("written");
}

/// VRAM bank 1 (stage T3): the match's VRAM with fighters 2 / 3's `<CHR>EFT.DAT` sheets and CLUTs at P1's / P2's
/// places, the same everywhere else; the effects of fighters 2 / 3 (their scripts' billboards) draw from it (CLUT bit
/// 15) and nobody else's do.
#[test]
fn bank_1_holds_fighters_2_and_3s_effect_pages() {
    let Some(a) = export() else {
        eprintln!("skipped: no okb_export");
        return;
    };
    let (name, mode, chr, stage) = FIXTURES[0];
    let data = load(&a, chr, stage);
    let (v0, v1) = (&data.vram.words, &data.team.as_ref().expect("team data").vram1.words);
    // P1's / P2's sheets (960, 256..384) and CLUTs (96..128, 480..)
    let inside = |i: usize| {
        let (x, y) = (i % 1024, i / 1024);
        (x >= 960 && (256..384).contains(&y)) || ((96..128).contains(&x) && y >= 480)
    };
    assert!((0..v0.len()).filter(|&i| !inside(i)).all(|i| v0[i] == v1[i]), "the same outside P1's / P2's places");
    let differ = |x0: usize, y0: usize| (y0..y0 + 64).any(|y| (x0..x0 + 64).any(|x| v0[y * 1024 + x] != v1[y * 1024 + x]));
    assert!(differ(960, 256) && differ(960, 320), "BAK's sheet over YUG's, ULI's over ALI's");
    let mut tagged = 0;
    play(&data, mode, name, |t| tagged += t.vm.draws.iter().filter(|b| b.clut & 0x8000 != 0).count());
    assert!(tagged > 0, "fighters 2 / 3's billboards in bank 1");
    let m = pollster::block_on(crate::fight::fight_data::load_match(&a, ["YUG", "ALI"], [0, 0], "ST0", false, v::TEAM)).unwrap();
    let mut s = crate::fight::sim::Match::new(&m);
    s.set_cpu(0, true);
    s.set_cpu(1, true);
    for _ in 0..3000 {
        s.tick(&m);
        assert!(s.vm.draws.iter().all(|b| b.clut & 0x8000 == 0), "a match draws from bank 0 only");
    }
}

/// The HUD: P1 / P2's bars at the top, P3 / P4's at the bottom (2 VS 2: the team's round lamps under P1 / P2 only),
/// a fighter out of the round greyed.
#[test]
fn the_team_hud_puts_four_bars_and_greys_the_ko_d() {
    let fs: [Fighter; N] = std::array::from_fn(|k| at(k, 0, 0));
    let mut r = TeamRound::new();
    r.hud_hidden = false;
    r.out[2] = true;
    let mut h = TeamHud::default();
    h.build(fs.each_ref());
    for _ in 0..40 {
        h.frame(fs.each_ref(), &[1, 0, 3, 2], &r, TeamMode::Vs2, [None; N]);
    }
    let ys = |p: &crate::output::prim::Prim| p.xy.map(|v| v[1]);
    // every prim's place in the window: the top rows anchored to the top edge, the bottom ones to the bottom edge
    let (top, bottom): (Vec<_>, Vec<_>) = h.prims.iter().partition(|p| p.at.y == crate::output::layout::Ax::Start);
    assert!(top.iter().all(|p| ys(p).iter().all(|&y| y < 200.0)), "the top bars");
    assert!(!bottom.is_empty() && bottom.iter().all(|p| ys(p).iter().all(|&y| y > 300.0)), "the bottom bars");
    // P3 (left, bottom) greyed: a subtract tile (blend mode 2) over each part of its bar (health and name, gauge), none
    // over P4's
    let sub = |p: &&crate::output::prim::Prim| p.semi && !p.textured && p.tpage & 0x60 == 0x40;
    assert_eq!(bottom.iter().filter(sub).filter(|p| p.xy[0][0] < 256.0).count(), 2, "P3 out: greyed");
    assert_eq!(bottom.iter().filter(sub).filter(|p| p.xy[0][0] >= 256.0).count(), 0, "P4 in the round");
}

/// Fighters 2 / 3's effect prims draw from VRAM bank 1 (CLUT bit 15), fighters 0 / 1's from bank 0 (T3, review of
/// T4): the move effects (the claw trail of task 0x39, Gado's comet), the native tasks and the effect VM's models
/// (Busuzima's tongue: a native task with an emitter model of its fighter's group), each spawned for one fighter on a
/// copy of the same battle.
#[test]
fn effect_prims_of_fighters_2_and_3_carry_the_bank_bit() {
    use crate::fight::fx::{move_effects, native_tasks};
    let Some(a) = export() else {
        eprintln!("skipped: no okb_export");
        return;
    };
    let chr = ["BUS", "GAD", "BUS", "GAD"];
    let data = load(&a, chr, "ST0");
    let mut t = TeamFight::new(&data, TeamMode::Ffa);
    for _ in 0..400 {
        t.tick(&data);
    }
    let gte = crate::output::gte::Gte::new(t.cam.view(&data.trig), glam::Mat4::IDENTITY, [1.0, 1.0], 0);
    let project = |p: [i32; 3]| gte.rtps(p).map(|q| (q.sx, q.sy, q.otz));
    for k in 0..N {
        let mut u = t.clone();
        let before = u.vm.model_prims(&project).len();
        let m = u.fighters[k].m_t;
        if chr[k] == "GAD" {
            move_effects::comet(&mut u.fx_host(), k, [m[0], m[1] - 600, m[2]], 30);
            move_effects::trail(&mut u.fx_host(), &data.tables, k, 7, 30, 0, 0);
        } else {
            native_tasks::tongue(&mut u.fx_host(), k, 2, 30);
        }
        for _ in 0..4 {
            move_effects::tick(&mut u.fx_host(), &data.tables);
            native_tasks::tick(&mut u.fx_host(), &data);
            u.vm.tick();
        }
        // the new models come first (the VM draws its newest model first)
        let models = u.vm.model_prims(&project);
        let textured = |p: &&crate::fight::fx::effect_models::ModelPrim| p.textured;
        let mut prims: Vec<u16> = models[..models.len() - before].iter().filter(textured).map(|p| p.clut).collect();
        prims.extend(move_effects::prims(&u.fxn.mv, &gte, (u.frame & 1) as usize).iter().filter(textured).map(|p| p.clut));
        prims.extend(native_tasks::prims(&u.fxn, &gte).iter().filter(textured).map(|p| p.clut));
        assert!(!prims.is_empty(), "fighter {k} ({}): no prims", chr[k]);
        assert!(prims.iter().all(|&c| (c & 0x8000 != 0) == (k >= 2)), "fighter {k} ({}): CLUTs {prims:x?}", chr[k]);
    }
}

/// (T6) The pause's CONTROLLER CONFIG. asks the session for its screen from simulation v24, with a human on pad 1 or 2
/// (P3 alone: the item is greyed, as a WATCH match's); before v24 the item does nothing.
#[test]
fn the_pause_asks_for_controller_config_from_v24() {
    use crate::fight::input::pad::{CROSS, DOWN, START};
    let Some(a) = export() else {
        eprintln!("skipped: no okb_export");
        return;
    };
    let chr = ["YUG", "ALI", "BAK", "ULI"];
    for (sim, cpu, want) in
        [(v::TEAM_CONFIG, [false, true, true, true], true), (v::TEAM_CONFIG, [true, true, false, true], false), (v::TEAM, [false, true, true, true], false)]
    {
        let data = pollster::block_on(crate::fight::fight_data::load_team(&a, chr, [0; N], "ST0", sim)).expect("the team battle's data");
        let mut t = TeamFight::with_cpu(&data, TeamMode::Vs2, cpu);
        let human = cpu.iter().position(|&c| !c).expect("a human");
        while t.round.state != 4 {
            t.tick(&data);
        }
        let mut asked = false;
        for p in [START, 0, DOWN, 0, CROSS, 0] {
            t.raw_pads[human] = p;
            t.tick(&data);
            asked |= std::mem::take(&mut t.config_request);
        }
        assert!(t.pause.frozen(), "v{sim} {cpu:?}: paused");
        assert_eq!(asked, want, "v{sim} {cpu:?}");
    }
}

/// (T7) From simulation v25 the enlarged rings have the game's walls at their edge (±2304, the models standing, hp 180;
/// before v25 open: hp 0, no models), and they hold: a four-CPU battle on ST8 (the line-up of the user's first
/// video) plays to its end with no ring-out, no wall broken, and `watch` finds no fighter stuck in the air or through a
/// wall.
#[test]
fn the_enlarged_rings_have_unbreakable_walls_from_v25() {
    let Some(a) = export() else {
        eprintln!("skipped: no okb_export");
        return;
    };
    let chr = ["STU", "GAD", "MAR", "BUS"];
    let at = |sim| pollster::block_on(crate::fight::fight_data::load_team(&a, chr, [0; N], "ST8", sim)).expect("the team battle's data");
    let walls = |d: &Loaded| {
        let mut t = TeamFight::new(d, TeamMode::Vs2);
        t.tick(d);
        let s = &t.world.stage;
        (s.ring.hx, s.ring.hz, s.walls.hp, s.walls.model.iter().all(|&m| m >= 0))
    };
    assert_eq!(walls(&at(v::TEAM_WALLS - 1)), (2304, 2304, [0; 4], false));
    let data = at(v::TEAM_WALLS);
    assert_eq!(walls(&data), (2304, 2304, [180; 4], true));
    let mut watch = super::watch::Watch::default();
    let (mut found, mut ring_out, mut pushed) = (Vec::new(), false, 0u32);
    let (t, _) = play(&data, TeamMode::Vs2, "t7_st8", |t| {
        found.extend(watch.step(t, &data));
        ring_out |= t.fighters.iter().any(|f| f.o_00 == 0x81);
        pushed += t.world.stage.walls.pow.iter().filter(|&&p| p > 0).count() as u32;
    });
    assert!(matches!(t.result, Some(TeamOutcome::Winner(_) | TeamOutcome::Draw)), "{:?}", t.result);
    assert!(found.is_empty(), "{found:?}");
    assert!(!ring_out, "a ring-out");
    assert!(pushed > 0, "nobody touched a wall");
    assert!(t.world.stage.walls.model.iter().all(|&m| m >= 0) && !t.events.iter().any(|e| e.contains("wall")), "a wall broke");
}

/// (R1) `watch` tests Q3DM6's chamfers only where they are walls (simulation v25, T7): a body standing in a corner,
/// inside the ring's box but past the chamfer, is a finding at v25 and none at v24 (no false findings on older runs).
#[test]
fn the_watch_tests_the_chamfers_from_v25_only() {
    let Some(a) = export() else {
        eprintln!("skipped: no okb_export");
        return;
    };
    let chr = ["YUG", "ALI", "BAK", "ULI"];
    for (sim, want) in [(v::CHAMFERS - 1, false), (v::CHAMFERS, true)] {
        let data = match pollster::block_on(crate::fight::fight_data::load_team(&a, chr, [0; N], "Q3DM6", sim)) {
            Ok(d) => d,
            Err(e) => {
                eprintln!("skipped Q3DM6: {e}");
                return;
            }
        };
        let mut t = TeamFight::new(&data, TeamMode::Vs2);
        while t.round.state != 4 {
            t.tick(&data);
        }
        let (hx, hz) = (t.world.stage.ring.hx as i32, t.world.stage.ring.hz as i32);
        for (sx, sz) in [(1, 1), (1, -1), (-1, 1), (-1, -1)] {
            let mut u = t.clone();
            for p in u.fighters[0].parts.iter_mut() {
                (p[0], p[2]) = (sx * (hx - 20), sz * (hz - 20));
            }
            let found = super::watch::Watch::default().step(&u, &data);
            assert_eq!(found.iter().any(|f| f.contains("P1 through a wall")), want, "v{sim} corner ({sx}, {sz}): {found:?}");
        }
    }
}

/// (T8) Busuzima's cloak (the palette fade op 0x2B to level 255, BKOUT) is his own: with him fighter 3 his team-mate
/// fighter 1 (the same slot `k & 1`) is drawn with its own fade, not with the World slot's fighter 3 left there.
/// (R1) Asserted on what the drawing takes, `SceneRef::body` (the fade level, the body hidden): Gado whole and unfaded
/// while Busuzima is cloaked, Busuzima hidden. (R2) Every frame each fighter's `SceneRef::fade` is its own record.
#[test]
fn a_fighters_fade_is_drawn_from_its_own_record() {
    let Some(a) = export() else {
        eprintln!("skipped: no okb_export");
        return;
    };
    let data = pollster::block_on(crate::fight::fight_data::load_team(&a, ["STU", "GAD", "MAR", "BUS"], [0; N], "ST8", v::TEAM_WALLS))
        .expect("the team battle's data");
    let mut t = TeamFight::new(&data, TeamMode::Vs2);
    let (mut cloaked, mut hidden) = (0, 0);
    while t.result.is_none() && t.frame < 3000 {
        t.tick(&data);
        let s = t.scene();
        for k in 0..N {
            assert_eq!(s.fade(k), t.ext[k].p4b18, "fighter {k} at frame {}", t.frame);
        }
        if s.body(3) == (255, true) {
            hidden += 1;
        }
        if t.ext[3].p4b18[2] == 255 && t.ext[1].p4b18 == [0; 4] {
            assert_ne!(t.world.p4b18[crate::fight::side::Slot::P2], t.ext[1].p4b18, "the slot holds fighter 3's fade");
            assert_eq!(s.body(1), (0, false), "Gado drawn whole while Busuzima is cloaked, frame {}", t.frame);
            cloaked += 1;
        }
    }
    assert!(cloaked > 0, "Busuzima cloaked while Gado was not");
    assert!(hidden > 0, "Busuzima never hidden");
}

/// (TEAM ARCADE A3) The match's end of an arcade battle (`ArcadeRules`): its winner team's state 0xD from `winner`
/// (None: a DRAW GAME), then ticks with `press` as the humans' raw presses on the frame `at` of the CONTINUE state;
/// the request, the messages, the countdown's ticks (sound 0x11), the music requests and the frames ticked.
fn arcade_end(winner: Option<usize>, press: u16, at: i32) -> (Request, Vec<Message>, usize, Vec<u8>, usize) {
    let mut r = TeamRound::new();
    let mut fs = fighters();
    let mut a = ArcadeRules::new(0, [0x80; N]);
    r.state = 0xD;
    r.winner = winner;
    let (mut req, mut messages, mut ticks, mut music, mut frames) = (Request::None, Vec::new(), 0, Vec::new(), 0);
    while frames < 4000 {
        frames += 1;
        r.raw_press = if r.state == 0xF && r.cnt == at { press } else { 0 };
        let mut sounds = Vec::new();
        req = r.tick(&mut fs, TeamMode::Vs2, &mut sounds, 0, Gates::of(v::TEAM_ARCADE), Some(&mut a));
        ticks += sounds.iter().filter(|s| s.0 == 0x11).count();
        music.extend(r.music);
        r.check(TeamMode::Vs2).expect("a state tick makes");
        a.check().expect("rules tick makes");
        if req != Request::None {
            break;
        }
        if messages.last() != Some(&r.message) {
            messages.push(r.message);
        }
    }
    (req, messages, ticks, music, frames)
}

/// (TEAM ARCADE A3) YOU WIN ends the battle; YOU LOSE and a DRAW GAME go to CONTINUE? 9..0, where START continues and
/// the end of the countdown gives GAME OVER (900 frames, a press skips them).
#[test]
fn arcade_wins_and_losses_and_the_continue_countdown() {
    let (req, messages, _, music, frames) = arcade_end(Some(0), 0, 0);
    assert_eq!((req, &messages[..], &music[..], frames), (Request::MatchOver(0), &[Message::YouWin][..], &[3][..], 361));
    // START at the countdown's frame 250: CONTINUE? 9, 8, 7, then the continue
    let (req, messages, ticks, music, _) = arcade_end(Some(1), START, 250);
    assert_eq!(req, Request::Continue);
    assert_eq!(messages, [Message::YouLose, Message::Continue(9), Message::Continue(8), Message::Continue(7)]);
    assert_eq!((ticks, &music[..]), (3, &[4, 0x2A][..]));
    // nobody presses: ten digits, GAME OVER after its 4 frames' wait for 900 frames
    let (req, messages, ticks, music, frames) = arcade_end(None, 0, 0);
    assert_eq!(req, Request::GameOver);
    let digits: Vec<Message> = (0..10).rev().map(Message::Continue).collect();
    assert_eq!(messages[0], Message::DrawGame);
    assert_eq!(messages[1..11], digits[..]);
    assert_eq!(messages[11..], [Message::None, Message::GameOver]);
    assert_eq!((ticks, &music[..], frames), (10, &[4, 0x2A, 0x27][..], 361 + 999 + 3 + 900));
    // □ / × / ○ / R1 after the countdown's frame 200 jump to the next digit (the match's): 49 frames sooner
    let (req, messages, .., jumped) = arcade_end(Some(1), CROSS, 250);
    assert_eq!((req, &messages[1..11], jumped), (Request::GameOver, &digits[..], frames - 49));
}

/// (TEAM ARCADE A3) The battles of `fixtures/team_arcade_v26_hashes.txt` at simulation v26 (YUGO + BAKURYU, the humans'
/// team P1 + P3, against ALICE + ULI on ST0): name, the four CPU levels (`ArcadeRules::levels`), P1 a human (else a
/// CPU), P1's raw presses by battle frame. A loss continued with START, the retry won (P1 a CPU at the boss's level,
/// the enemies weak), a loss whose countdown runs out with GAME OVER skipped by ×.
#[allow(clippy::type_complexity)]
const ARCADE_FIXTURES: [(&str, [u16; N], bool, &[(u64, u16)]); 3] = [
    ("arcade_lose_continue", [0x80, 0xC0, 0x80, 0xC0], true, &[(6300, START)]),
    ("arcade_win", [0xFF, 0x10, 0xFF, 0x10], false, &[]),
    ("arcade_game_over", [0x80, 0xC0, 0x80, 0xC0], true, &[(7090, CROSS)]),
];

/// `play` of a TEAM ARCADE battle (`ARCADE_FIXTURES`): P1's pad as the session sets a human's (`team_frame`).
fn play_arcade(data: &Loaded, name: &str, levels: [u16; N], human: bool, press: &[(u64, u16)], mut each: impl FnMut(&TeamFight)) -> (TeamFight, Vec<String>) {
    use crate::base::hash::{FNV_BASIS, fnv1a};
    let cpu = [!human, true, true, true];
    let mut t = TeamFight::with_setup(data, TeamMode::Vs2, cpu, 0x80, [crate::assets::game_tables::default_buttons(); N], Some(ArcadeRules::new(0, levels)));
    let (mut h, mut out) = (FNV_BASIS, Vec::new());
    while t.result.is_none() && t.frame < 20000 {
        let p = press.iter().find(|(f, _)| *f == t.frame).map_or(0, |(_, p)| *p);
        t.raw_pads[0] = p;
        t.pads[0] = if t.cpu[0] { 0 } else { p };
        t.tick(data);
        t.hud_step();
        each(&t);
        h = fnv1a(h, &postcard::to_allocvec(&t).expect("serialises"));
        if t.frame.is_multiple_of(500) {
            out.push(format!("{name} {} {h:016x}", t.frame));
        }
    }
    out.push(format!("{name} {} {h:016x} {:?}", t.frame, t.result));
    (t, out)
}

fn load_arcade(a: &crate::assets::Assets) -> Loaded {
    pollster::block_on(crate::fight::fight_data::load_team(a, ["YUG", "ALI", "BAK", "ULI"], [0; N], "ST0", v::TEAM_ARCADE)).expect("the team battle's data")
}

/// (TEAM ARCADE A3) The arcade battles play as recorded (`team_arcade_fixture_hashes_record`): each AI at its fighter's
/// level, YOU LOSE → the countdown → `Continue`, YOU WIN → `Winner(0)`, the countdown's end → GAME OVER → `GameOver`;
/// the winners hidden and the losers shown while it counts; a state saved in the countdown decodes and plays on to the
/// same end, and is refused without its rules or as FREE FOR ALL.
#[test]
fn team_arcade_fixtures() {
    let Some(a) = export() else {
        eprintln!("skipped: no okb_export");
        return;
    };
    let data = load_arcade(&a);
    let want = include_str!("../../session/fixtures/team_arcade_v26_hashes.txt");
    let ends = [(TeamOutcome::Continue, Message::YouLose), (TeamOutcome::Winner(0), Message::YouWin), (TeamOutcome::GameOver, Message::GameOver)];
    for ((name, levels, human, press), (end, message)) in ARCADE_FIXTURES.into_iter().zip(ends) {
        let (mut messages, mut hidden, mut saved) = (Vec::new(), true, None);
        let (t, trace) = play_arcade(&data, name, levels, human, press, |t| {
            if messages.last() != Some(&t.round.message) {
                messages.push(t.round.message);
            }
            if t.round.state == 0xF && t.round.cnt > 1 {
                hidden &= (0..N).all(|k| (t.fighters[k].flags[F60] & 2 != 0) == (k & 1 == 1));
                if t.round.cnt == 50 {
                    saved = Some((postcard::to_allocvec(t).expect("serialises"), t.frame));
                }
            }
        });
        assert_eq!(t.result, Some(end), "{name}");
        assert!(messages.contains(&message), "{name}: {messages:?}");
        assert!(t.ai.iter().zip(levels).all(|(ai, l)| ai.as_ref().is_some_and(|ai| ai.level == l)), "{name}: the levels");
        assert!(hidden, "{name}: the winners hidden in the countdown");
        let lines: Vec<&str> = want.lines().filter(|l| l.starts_with(&format!("{name} "))).collect();
        assert_eq!(trace, lines, "{name}: the trace");
        let Some((bytes, frame)) = saved else { continue };
        let decoded = |f: fn(&mut TeamFight)| {
            let mut d: TeamFight = postcard::from_bytes(&bytes).expect("decodes");
            f(&mut d);
            d.rebind(&data).map(|_| d)
        };
        assert!(decoded(|d| d.arcade = None).is_err(), "{name}: the countdown without its rules");
        assert!(decoded(|d| d.mode = TeamMode::Ffa).is_err(), "{name}: FREE FOR ALL rules");
        let mut d = decoded(|_| {}).expect("rebinds");
        while d.result.is_none() && d.frame < 20000 {
            let p = press.iter().find(|(f, _)| *f == d.frame).map_or(0, |(_, p)| *p);
            d.raw_pads[0] = p;
            d.pads[0] = p;
            d.tick(&data);
            d.hud_step();
        }
        assert!(frame < d.frame && d.frame == t.frame && d.result == t.result, "{name}: played on from frame {frame}");
        assert_eq!(postcard::to_allocvec(&d).expect("serialises"), postcard::to_allocvec(&t).expect("serialises"), "{name}: the same end");
    }
}

/// Writes `fixtures/team_arcade_v26_hashes.txt` (by hand, after an intended change of TEAM ARCADE's battle: `cargo test
/// --release team_arcade_fixture_hashes_record -- --ignored`, with the export beside the engine).
#[test]
#[ignore]
fn team_arcade_fixture_hashes_record() {
    let a = export().expect("okb_export");
    let data = load_arcade(&a);
    let mut lines = Vec::new();
    for (name, levels, human, press) in ARCADE_FIXTURES {
        lines.extend(play_arcade(&data, name, levels, human, press, |_| {}).1);
    }
    let path = format!("{}/src/session/fixtures/team_arcade_v26_hashes.txt", env!("CARGO_MANIFEST_DIR"));
    std::fs::write(&path, lines.join("\n") + "\n").expect("written");
}
