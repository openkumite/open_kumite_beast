//! Tests of the team battle's imported fighters (T5), with the export and the imported game's install beside the
//! engine (`OKB_EXPORT` / `BR1_EXPORT`, else `../okb_export` / `../okb_export/br1`; skipped without them): whole battles of
//! imported fighters alone and mixed with the main game's, on a team stage and on BIG STAGE, with throws across the
//! engines and broken ones; their state-hash traces (`fixtures/team_br1_v23_hashes.txt`); a state saved in the middle of
//! a throw across the engines; the version gate.

use super::tests::{export, play};
use super::*;
use crate::fight::br1::bridge::{CrossThrow, Ghost};
use crate::fight::version::v;

/// The imported game's install beside the engine.
fn br1() -> Option<crate::assets::Assets> {
    let root = std::env::var("BR1_EXPORT").unwrap_or_else(|_| concat!(env!("CARGO_MANIFEST_DIR"), "/../okb_export/br1").into());
    std::path::Path::new(&root).join("manifest.json").exists().then(|| crate::assets::Assets::new(root))
}

/// The team battle's data of `chr` on `stage` at `sim_version` (a duplicate in the next costume, as `sim`'s).
fn load_at(chr: [&str; N], stage: &str, sim_version: u32) -> Option<Result<Loaded, String>> {
    let (a, b) = (export()?, br1()?);
    let costume = std::array::from_fn(|k| (chr[..k].iter().filter(|c| **c == chr[k]).count() % 4) as u8);
    Some(pollster::block_on(crate::fight::fight_data::load_team_with(&a, Some(&b), chr, costume, stage, sim_version)))
}

fn load(chr: [&str; N], stage: &str) -> Option<Loaded> {
    load_at(chr, stage, v::TEAM_BR1).map(|d| d.expect("the team battle's data"))
}

/// The battles of the trace fixture: name, mode, fighters, stage.
const FIXTURES: [(&str, TeamMode, [&str; N], &str); 3] = [
    ("team_br1_2v2_st0", TeamMode::Vs2, ["MIT", "YUG", "FOX", "ALI"], "ST0"),
    ("team_br1_ffa_st0", TeamMode::Ffa, ["YUG", "MIT", "GAD", "URI"], "ST0"),
    ("team_br1_ffa_big00", TeamMode::Ffa, ["MIT", "FOX", "GRE", "URI"], "BIG00"),
];

/// The (attacker, defender) pairs of a battle's hit log.
fn hits(t: &TeamFight) -> Vec<(usize, usize)> {
    t.events
        .iter()
        .filter_map(|e| {
            let (_, rest) = e.split_once(": P")?;
            let (a, rest) = rest.split_once(' ')?;
            let d = rest.rsplit_once(" P").map(|(_, d)| d.split(' ').next().unwrap_or(""))?;
            Some((a.parse::<usize>().ok()? - 1, d.parse::<usize>().ok()? - 1))
        })
        .collect()
}

/// Each fixture plays to a winner with hits by both engines, never on a team-mate, and traces as recorded
/// (`team_br1_fixture_hashes_record`); together they have throws across the engines both ways, the imported game's
/// own, and broken ones.
#[test]
fn br1_team_fixtures() {
    let want = include_str!("../../session/fixtures/team_br1_v23_hashes.txt");
    let mut events = Vec::new();
    for (name, mode, chr, stage) in FIXTURES {
        let Some(data) = load(chr, stage) else {
            eprintln!("skipped: no okb_export / okb_export/br1");
            return;
        };
        let (t, trace) = play(&data, mode, name, |_| {});
        assert!(matches!(t.result, Some(TeamOutcome::Winner(_))), "{name}: {:?}", t.result);
        let h = hits(&t);
        assert!(h.len() > 20, "{name}: {} hits", h.len());
        assert!(h.iter().all(|&(a, d)| mode.team(a) != mode.team(d)), "{name}: friendly fire");
        assert!(t.events.iter().any(|e| e.ends_with("(the imported game)")), "{name}: no imported hit");
        let lines: Vec<&str> = want.lines().filter(|l| l.starts_with(&format!("{name} "))).collect();
        assert_eq!(trace, lines, "{name}: the trace");
        events.extend(t.events);
    }
    for what in
        ["(across the engines: the imported fighter throws)", "(across the engines: the imported fighter is thrown)", "throw broken", "throw on", "grab on"]
    {
        assert!(events.iter().any(|e| e.contains(what)), "no '{what}'");
    }
}

/// A state saved in the middle of a main-game throw on an imported fighter (its proxy, the list keyed by
/// thrower), decoded and rebound, plays on to the same end; a decoded state whose imported part does not fit the
/// data is refused.
#[test]
fn br1_team_state_round_trip() {
    let (_, mode, chr, stage) = FIXTURES[0];
    let Some(data) = load(chr, stage) else {
        eprintln!("skipped: no okb_export / okb_export/br1");
        return;
    };
    let mut t = TeamFight::new(&data, mode);
    let cross = |t: &TeamFight| t.br1.as_ref().is_some_and(|b| b.throws.iter().any(|(_, c)| matches!(c.ghost, Ghost::Br2(_))));
    while !cross(&t) && t.frame < 20000 {
        t.tick(&data);
        t.hud_step();
    }
    assert!(cross(&t), "a main-game throw on an imported fighter");
    for _ in 0..30 {
        t.tick(&data);
        t.hud_step();
    }
    let bytes = postcard::to_allocvec(&t).unwrap();
    let mut r: TeamFight = postcard::from_bytes(&bytes).unwrap();
    r.rebind(&data).expect("rebinds");
    while t.result.is_none() && t.frame < 30000 {
        t.tick(&data);
        t.hud_step();
        r.tick(&data);
        r.hud_step();
    }
    assert!(t.result.is_some());
    assert_eq!(postcard::to_allocvec(&t).unwrap(), postcard::to_allocvec(&r).unwrap());
    let mutated = |f: fn(&mut TeamFight)| {
        let mut r: TeamFight = postcard::from_bytes(&bytes).unwrap();
        f(&mut r);
        r.rebind(&data).is_err()
    };
    // no imported part for imported fighters; an imported throw's proxy on an imported victim
    assert!(mutated(|r| r.br1 = None));
    assert!(mutated(|r| {
        let g = Box::new(crate::fight::br1::object::Br1Fighter::default());
        r.br1.as_mut().unwrap().throws.start(1, CrossThrow { victim: 0, ghost: Ghost::Br1(g), frames: 0 });
    }));
    // the data of another battle
    let other = load(["YUG", "MIT", "GAD", "URI"], stage).expect("data");
    assert!(r.rebind(&other).is_err());
}

/// Imported fighters and the big stages need simulation v23; the big stages load at their own size.
#[test]
fn br1_team_battles_from_v23() {
    let Some(Err(e)) = load_at(["MIT", "YUG", "FOX", "ALI"], "ST0", v::TEAM) else {
        eprintln!("skipped (or loaded): no okb_export / okb_export/br1");
        return;
    };
    assert!(e.contains("simulation version 23"), "{e}");
    for stage in ["BIG00", "Q3DM6"] {
        assert!(load_at(["YUG", "ALI", "BAK", "ULI"], stage, v::TEAM).is_some_and(|d| d.is_err()), "{stage} at v22");
    }
    let d = load_at(["YUG", "ALI", "BAK", "ULI"], "BIG00", v::TEAM_BR1).expect("the exports").expect("BIG00 loads");
    let t = TeamFight::new(&d, TeamMode::Ffa);
    assert_eq!((t.world.stage.ring.hx, t.world.stage.ring.hz, d.team.as_ref().map(|t| t.stage_scale)), (3520, 3520, Some(100)));
    assert!(t.br1.is_none() && d.br1_stage.is_some());
}

/// Writes `fixtures/team_br1_v23_hashes.txt` (run by hand after an intended change of the team battle's imported
/// part: `cargo test --release team_br1_fixture_hashes_record -- --ignored`, with both exports beside the engine).
#[test]
#[ignore]
fn team_br1_fixture_hashes_record() {
    let mut lines = Vec::new();
    for (name, mode, chr, stage) in FIXTURES {
        lines.extend(play(&load(chr, stage).expect("okb_export / okb_export/br1"), mode, name, |_| {}).1);
    }
    let path = format!("{}/src/session/fixtures/team_br1_v23_hashes.txt", env!("CARGO_MANIFEST_DIR"));
    std::fs::write(&path, lines.join("\n") + "\n").expect("written");
}

/// (R1) The battle of the simulation v25 (T7) trace fixture `fixtures/team_br1_v25_hashes.txt`: the v23
/// fixture's 2 VS 2 inside ST0's walls.
const FIXTURE_V25: (&str, TeamMode, [&str; N], &str) = ("team_br1_2v2_st0_v25", TeamMode::Vs2, ["MIT", "YUG", "FOX", "ALI"], "ST0");

/// (R1) The v25 imported battle plays to its end and traces as recorded (`team_br1_v25_fixture_hashes_record`), so
/// a change of simulation v25's team battle with imported fighters fails here; it has throws across the engines and
/// a thrown fighter whose target stays its thrower after the release (`target::Locks::thrown_by`, T7), hits by both
/// engines, none on a team-mate, and `watch` finds nothing.
#[test]
fn br1_team_v25_fixture() {
    let (name, mode, chr, stage) = FIXTURE_V25;
    let Some(data) = load_at(chr, stage, 25).map(|d| d.expect("the team battle's data")) else {
        eprintln!("skipped: no okb_export / okb_export/br1");
        return;
    };
    let (mut kept, mut found) = (0u32, Vec::new());
    let mut watch = super::watch::Watch::default();
    let (t, trace) = play(&data, mode, name, |t| {
        kept += t.locked.thrown_by.iter().any(|l| l.is_some_and(|(_, n)| n > 0)) as u32;
        found.extend(watch.step(t, &data));
    });
    assert!(matches!(t.result, Some(TeamOutcome::Winner(_) | TeamOutcome::Draw)), "{:?}", t.result);
    let h = hits(&t);
    assert!(h.len() > 20 && h.iter().all(|&(a, d)| mode.team(a) != mode.team(d)), "{} hits, or friendly fire", h.len());
    assert!(t.events.iter().any(|e| e.ends_with("(the imported game)")), "no imported hit");
    assert!(t.events.iter().any(|e| e.contains("(across the engines:")), "no throw across the engines");
    assert!(kept > 0, "no thrown fighter kept its thrower after the release");
    assert!(found.is_empty(), "{found:?}");
    let want: Vec<&str> = include_str!("../../session/fixtures/team_br1_v25_hashes.txt").lines().collect();
    assert_eq!(trace, want, "the trace");
}

/// Writes `fixtures/team_br1_v25_hashes.txt` (by hand, after an intended change of simulation v25's team battle with
/// imported fighters: `cargo test --release team_br1_v25_fixture_hashes_record -- --ignored`, with both exports
/// beside the engine).
#[test]
#[ignore]
fn team_br1_v25_fixture_hashes_record() {
    let (name, mode, chr, stage) = FIXTURE_V25;
    let data = load_at(chr, stage, 25).expect("okb_export / okb_export/br1").expect("the team battle's data");
    let lines = play(&data, mode, name, |_| {}).1;
    let path = format!("{}/src/session/fixtures/team_br1_v25_hashes.txt", env!("CARGO_MANIFEST_DIR"));
    std::fs::write(&path, lines.join("\n") + "\n").expect("written");
}

/// (T7) The line-ups of the user's videos of the imported fighters (FREE FOR ALL MIT LON FOX JEN on BIG STAGE, 2 VS
/// 2 GRE YUG URI ALI on Q3DM6) and an imported throw on an imported fighter in a corner (2 VS 2 FOX MIT GRE URI
/// on Q3DM6) play with nobody stuck in the air or through a wall from simulation v25 (`watch`), for as long as the
/// videos ran; at v24 the BIG STAGE battle has its imported fighters hanging in the air after a main-game
/// knock-down (the check sees the old bug). Q3DM6's battles are skipped without its Quake III files.
#[test]
fn no_imported_fighter_hangs_in_the_air_or_goes_through_a_wall_from_v25() {
    let battles: [(TeamMode, [&str; N], &str, u64); 3] = [
        (TeamMode::Ffa, ["MIT", "LON", "FOX", "JEN"], "BIG00", 8359),
        (TeamMode::Vs2, ["GRE", "YUG", "URI", "ALI"], "Q3DM6", 7995),
        (TeamMode::Vs2, ["FOX", "MIT", "GRE", "URI"], "Q3DM6", 9000),
    ];
    let run = |mode, chr, stage, frames, sim| {
        let data = match load_at(chr, stage, sim)? {
            Ok(d) => d,
            Err(e) if stage == "Q3DM6" => {
                eprintln!("skipped {stage}: {e}");
                return None;
            }
            Err(e) => panic!("{e}"),
        };
        let mut t = TeamFight::new(&data, mode);
        let mut watch = super::watch::Watch::default();
        let mut found = Vec::new();
        while t.result.is_none() && t.frame < frames {
            t.tick(&data);
            found.extend(watch.step(&t, &data));
        }
        Some(found)
    };
    for (mode, chr, stage, frames) in battles {
        let Some(found) = run(mode, chr, stage, frames, v::BR1_LAUNCH) else { continue };
        assert!(found.is_empty(), "{chr:?} on {stage}: {found:?}");
    }
    let (mode, chr, stage, frames) = battles[0];
    if let Some(found) = run(mode, chr, stage, frames, v::BR1_LAUNCH - 1) {
        assert!(found.iter().any(|f| f.contains("in the air")), "v24: {found:?}");
    }
}
