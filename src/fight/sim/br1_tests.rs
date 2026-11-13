//! Tests of imported fighters inside the main game's rules (the import S7): each move of the move sheets
//! against an imported and a main-game dummy, and the hit exchange's damage scale.
//!
//! Data-gated: the main-game export (`OKB_EXPORT`, else `../okb_export`) and the imported game's install
//! (`BR1_EXPORT`, else `../okb_export/br1`); skipped without them.

use super::Match;
use crate::assets::Assets;
use crate::fight::br1::script::{Call, Request};
use crate::fight::fight_data::{Loaded, load_match_with};
use crate::fight::input::pad::{LEFT, RIGHT};

/// The move sheets' numbers (`fixtures/move_sheets.txt`, from the S6
/// sheets of recorded runs): code, state, the attack's first active frame after the state starts, its active
/// frames, its damage F.CE, the defender's hit stun F.C2 of a clean hit.
const SHEETS: &str = include_str!("../br1/fixtures/move_sheets.txt");

fn data(p1: &str, p2: &str) -> Option<Loaded> {
    data_on(p1, p2, "ST0", crate::fight::version::v::BR1_FIGHTERS)
}

/// `data` on stage `stage` in simulation version `sim_version`.
pub(super) fn data_on(p1: &str, p2: &str, stage: &str, sim_version: u32) -> Option<Loaded> {
    let br2 = crate::base::env::var("EXPORT").unwrap_or_else(|_| concat!(env!("CARGO_MANIFEST_DIR"), "/../okb_export").into());
    let br1 = std::env::var("BR1_EXPORT").unwrap_or_else(|_| concat!(env!("CARGO_MANIFEST_DIR"), "/../okb_export/br1").into());
    if !std::path::Path::new(&br2).join("manifest.json").exists() || !std::path::Path::new(&br1).join("manifest.json").exists() {
        return None;
    }
    let (a2, a1) = (Assets::new(br2), Assets::new(br1));
    Some(pollster::block_on(load_match_with(&a2, Some(&a1), [p1, p2], [0, 1], stage, false, sim_version)).expect("load"))
}

/// What a forced move did: its first active frame after the state's start (the core's F.22 0), the frame of its
/// first hit on the dummy (P2), the dummy's face damage (the main game's units) and hit stun (the core's for a
/// imported dummy, the face's else).
#[derive(Debug, PartialEq, Eq)]
struct Hit {
    active: Option<u64>,
    hit: u64,
    damage: i16,
    stun: i16,
}

/// P1 and P2 walk into each other from the round's start (P1 until frame `walk`; P2 keeps walking: no guard), P1's
/// `state` forced at frame 380; the first hit within 90 frames.
fn run(data: &Loaded, state: i16, walk: u64) -> Option<Hit> {
    let mut m = Match::new(data);
    let (mut start, mut active) = (None, None);
    for frame in 0..470u64 {
        m.pads = [if (300..walk).contains(&frame) { RIGHT } else { 0 }, if frame >= 300 { LEFT } else { 0 }];
        if frame == 380 {
            m.force_move = Some((0, state));
        }
        m.tick(data);
        let b = m.br1.as_deref().expect("an imported match");
        if start.is_none() && frame >= 380 && b.fs[0].s.state_cur == state {
            start = Some(frame);
        }
        let Some(s) = start else { continue };
        let d = &m.fighters[1];
        // the attack's F.22 0 (a hit sets it back to −1 on its frame)
        if active.is_none() && (b.fs[0].f.attack == 0 || d.o_cc != 0) {
            active = Some(frame - s);
        }
        if d.o_cc != 0 {
            let stun = if b.side[1] { b.fs[1].f.hit_stun } else { d.o_cc };
            m.tick(data);
            return Some(Hit { active, hit: frame - s, damage: m.fighters[1].o_ac, stun });
        }
    }
    None
}

/// Every move of the sheets inside a main-game match, against an imported and a main-game dummy pressed
/// against the attacker (or, for a move that misses there, 36 frames' walk apart): its first active frame the
/// sheet's, its first hit within its active frames with the sheet's stun, and a hit on the first active frame doing
/// the sheet's damage × 200 / 187 on the dummy's HP bar (later in the window a move's damage may be less, as the
/// recorded contacts show).
#[test]
fn moves_inside_br2_rules() {
    let mut checked = 0;
    let mut bad = Vec::new();
    let mut loaded: Vec<(String, String, Loaded)> = Vec::new();
    for line in SHEETS.lines().filter(|l| !l.starts_with('#') && !l.trim().is_empty()) {
        let w: Vec<&str> = line.split_whitespace().collect();
        let n = |k: usize| w[k].parse::<i32>().expect("a number");
        let (code, state, startup, act, damage, stun) = (w[0], n(1) as i16, n(2) as u64, n(3) as u64, n(4), n(5) as i16);
        let br1_dummy = if code == "FOX" { "MIT" } else { "FOX" };
        for dummy in [br1_dummy, "YUG"] {
            if !loaded.iter().any(|(a, b, _)| a == code && b == dummy) {
                let Some(d) = data(code, dummy) else {
                    eprintln!("skipped: no okb_export / okb_export/br1");
                    return;
                };
                loaded.push((code.into(), dummy.into(), d));
            }
            let d = &loaded.iter().find(|(a, b, _)| a == code && b == dummy).expect("loaded").2;
            let scaled = (damage * crate::fight::br1::bridge::HP / crate::fight::br1::bridge::BR1_HP as i32) as i16;
            checked += 1;
            let got = run(d, state, 376).or_else(|| run(d, state, 340));
            let ok = match &got {
                Some(h) => {
                    h.active == Some(startup)
                        && (startup..=startup + act).contains(&h.hit)
                        && h.stun == stun
                        && (damage == 0 || h.hit != startup || h.damage == scaled)
                        && (damage == 0 || (1..=scaled).contains(&h.damage))
                }
                None => false,
            };
            if !ok {
                bad.push(format!("{code} state {state} vs {dummy}: {got:?}; the sheet: active {startup} for {act}, damage {scaled}, stun {stun}"));
            }
        }
    }
    assert!(bad.is_empty(), "{} of {checked} moves differ:\n{}", bad.len(), bad.join("\n"));
    eprintln!("{checked} moves as their sheets");
}

/// The damage scale of the hit exchange: × 200 / the defender's imported max HP, truncated, the sign kept.
#[test]
fn damage_scale() {
    use crate::fight::br1::bridge::scale_damage;
    assert_eq!([scale_damage(12, 187), scale_damage(30, 187), scale_damage(30, 150), scale_damage(-3, 187)], [12, 32, 40, -3]);
}

/// The red health's rate of an imported fighter (`bridge::health`): RECOVER SPEED << 8 / (F.D1 + 1) + 1, none
/// at RECOVER SPEED 0, growing with it.
#[test]
fn br1_regen_rate() {
    use crate::fight::br1::bridge::regen_rate;
    assert_eq!([0, 1, 2, 4, 8].map(|r| regen_rate(r, 9)), [0, 26, 52, 103, 205]);
    assert_eq!([regen_rate(0, 0), regen_rate(1, 0), regen_rate(1, 255)], [0, 257, 2]);
}

/// A match with imported fighters (both, and mixed both ways) is a pure function of its state: from its bytes
/// (postcard, as the snapshots) and `rebind` the same frames follow as from the recording, through hits, a KO and the
/// next round.
#[test]
fn br1_match_state_round_trip() {
    for (p1, p2) in [("MIT", "URI"), ("GRE", "LON"), ("ALI", "FOX")] {
        let Some(d) = data(p1, p2) else {
            eprintln!("skipped: no okb_export / okb_export/br1");
            return;
        };
        let mut m = Match::new(&d);
        m.hp_pct = 20;
        let pads = |f: u64| {
            [
                if f >= 300 && f % 14 < 3 { crate::fight::input::pad::SQUARE } else { 0 } | if (300..340).contains(&f) { RIGHT } else { 0 },
                if f >= 300 { LEFT } else { 0 },
            ]
        };
        for f in 0..700 {
            m.pads = pads(f);
            m.tick(&d);
        }
        let bytes = postcard::to_allocvec(&m).expect("serialises");
        let mut n: Match = postcard::from_bytes(&bytes).expect("decodes");
        n.rebind(&d).expect("rebinds");
        for f in 700..1400 {
            for x in [&mut m, &mut n] {
                x.pads = pads(f);
                x.tick(&d);
            }
        }
        assert_eq!(postcard::to_allocvec(&m).unwrap(), postcard::to_allocvec(&n).unwrap(), "{p1} vs {p2}");
        assert!(m.fighters.iter().any(|f| f.o_ac != 0) || m.round.index > 0, "{p1} vs {p2}: nothing happened");
    }
}

/// What a throw from frame 340 (P1 walks in, □ + ×) did to P2 by frame 740: whether a throw across the engines ran,
/// the frame P2 first lay down (state 0x67), P2's damage; and the match's bytes at the end, the run snapshotted at
/// frame `snap` (postcard, `rebind`) and continued from there.
fn throw_run(d: &Loaded, snap: u64) -> (bool, Option<u64>, i16, Vec<u8>, Vec<u8>) {
    use crate::fight::input::pad::{CROSS, SQUARE};
    let pads = |f: u64| {
        [
            if (300..340).contains(&f) {
                RIGHT
            } else if (340..343).contains(&f) {
                SQUARE | CROSS
            } else {
                0
            },
            0,
        ]
    };
    let mut m = Match::new(d);
    let (mut cross, mut lying, mut copy) = (false, None, None);
    for f in 0..740u64 {
        if f == snap {
            let mut c: Match = postcard::from_bytes(&postcard::to_allocvec(&m).expect("serialises")).expect("decodes");
            c.rebind(d).expect("rebinds");
            copy = Some(c);
        }
        for x in std::iter::once(&mut m).chain(copy.as_mut()) {
            x.pads = pads(f);
            x.tick(d);
        }
        cross |= m.br1.as_ref().is_some_and(|b| b.throw.is_some());
        if lying.is_none() && m.fighters[1].move_cur == 0x67 {
            lying = Some(f);
        }
    }
    let bytes = postcard::to_allocvec(&m).expect("serialises");
    let copy = copy.map(|c| postcard::to_allocvec(&c).expect("serialises")).unwrap_or_default();
    (cross, lying, m.fighters[1].o_ac, bytes, copy)
}

/// The throws across the engines (engine-made choreography from the game's clips): an imported thrower on a main-game victim
/// (MIT on JEN, FOX its imported proxy) and a main-game thrower on an imported victim (YUG / GAD on MIT, Shina its main-game proxy; LON on
/// GRE, Gado its proxy: a throw ending in a strike) land, the victim lies down on the frame (± 1) the same throw puts
/// the proxy down in its own game and ends with the same damage; a snapshot in the middle of the throw goes on to
/// the same match.
#[test]
fn throws_across_the_engines() {
    for (thrower, victim, proxy) in [("MIT", "JEN", "FOX"), ("YUG", "MIT", "MAR"), ("GAD", "MIT", "MAR"), ("LON", "GRE", "GAD")] {
        let (Some(mixed), Some(same)) = (data(thrower, victim), data(thrower, proxy)) else {
            eprintln!("skipped: no okb_export / okb_export/br1");
            return;
        };
        let (cross, lying, damage, end, from_snap) = throw_run(&mixed, 380);
        let (_, want_lying, want_damage, ..) = throw_run(&same, 0);
        assert!(cross, "{thrower} on {victim}: no throw across the engines");
        let (l, w) = (lying.expect("lay down"), want_lying.expect("the reference lay down"));
        assert!(l.abs_diff(w) <= 1, "{thrower} on {victim}: lay down at {l}, {thrower} on {proxy} at {w}");
        assert_eq!(damage, want_damage, "{thrower} on {victim} vs {thrower} on {proxy}");
        assert!(damage > 0);
        assert_eq!(end, from_snap, "{thrower} on {victim}: from the snapshot at frame 380");
    }
}

/// A main-game attacker's hits on an imported fighter (P1 mashing □, P2 attacking now and then: some land as
/// counter hits, whose
/// F68 0x8000000 the imported game does not have) leave only the result word's common bits in the core (`take_hit`'s
/// mask, not undone by the frame's later absorb).
#[test]
fn br2_hits_masked_in_the_core() {
    use crate::fight::br1::bridge::COMMON;
    let Some(d) = data("YUG", "MIT") else {
        eprintln!("skipped: no okb_export / okb_export/br1");
        return;
    };
    let mut m = Match::new(&d);
    let (mut hits, mut counters) = (0, 0);
    for f in 0..1500u64 {
        let sq = crate::fight::input::pad::SQUARE;
        m.pads = [
            if f >= 300 && f % 5 < 2 { sq } else { 0 } | if (300..330).contains(&f) { RIGHT } else { 0 },
            if f >= 300 && f % 37 < 2 { sq } else { 0 } | if (300..330).contains(&f) { LEFT } else { 0 },
        ];
        // the multiplier's counter hit: the defender attacking (F58 0x80000000, a startup or active count)
        let v = &m.fighters[1];
        let counter = (v.flags[crate::fight::fighter::F58] as i32) < 0 && (v.o_ba != 0 || v.o_b8 != 0);
        let n = m.events.len();
        m.tick(&d);
        let b = m.br1.as_deref().expect("an imported match");
        assert_eq!(b.fs[1].f.f60 & !COMMON, 0, "frame {f}: core F.60 {:08x}", b.fs[1].f.f60);
        if m.events[n..].iter().any(|e| e.contains("P1 hits P2")) {
            hits += 1;
            counters += counter as u32;
        }
    }
    assert!(hits > 0 && counters > 0, "{hits} hits, {counters} on an attacking defender");
}

/// A decoded state with a broken cross-engine throw is refused by `rebind`: a victim out of range or of the
/// thrower's engine, an imported proxy whose lists do not fit its data, a main-game proxy whose move-script
/// position is outside its ANI.
#[test]
fn br1_throw_state_checked() {
    use crate::fight::br1::bridge::{CrossThrow, Ghost};
    let Some(d) = data("MIT", "JEN") else {
        eprintln!("skipped: no okb_export / okb_export/br1");
        return;
    };
    let mut m = Match::new(&d);
    for _ in 0..10 {
        m.tick(&d);
    }
    let rebind = |t: Option<CrossThrow>| {
        let mut c = m.clone();
        c.br1.as_deref_mut().expect("an imported match").throw = t;
        let mut n: Match = postcard::from_bytes(&postcard::to_allocvec(&c).expect("serialises")).expect("decodes");
        n.rebind(&d)
    };
    let br2_proxy = |pc: usize| {
        let mut g = m.fighters[0].clone();
        (g.pc, g.move_cur, g.move_req) = (pc, -1, -1);
        Ghost::Br2(Box::new(g))
    };
    let br1_proxy = |full: bool| {
        let b = m.br1.as_deref().expect("an imported match");
        let mut g = b.fs[0].clone();
        if !full {
            g.forms[0].md.truncate(1);
        }
        Ghost::Br1(Box::new(g))
    };
    assert!(rebind(None).is_ok());
    // an imported thrower's proxy of JEN (P2) fits FOX's data, the MIT core's lists do not
    let fox = crate::fight::br1::init::new(d.br1_ghost[1].as_deref().expect("JEN's BR1 proxy"), 1, 0, 0);
    let ok = |g: Ghost, victim: u8| rebind(Some(CrossThrow { victim, ghost: g, frames: 0 }));
    assert!(ok(Ghost::Br1(Box::new(fox.clone())), 1).is_ok());
    assert!(ok(Ghost::Br1(Box::new(fox)), 0).is_err());
    assert!(ok(br1_proxy(false), 1).is_err());
    assert!(ok(br1_proxy(true), 7).is_err());
    // a main-game thrower's proxy of MIT (P1) in its ANI's range, and beyond it
    assert!(ok(br2_proxy(0), 0).is_ok());
    assert!(ok(br2_proxy(0), 1).is_err());
    assert!(ok(br2_proxy(d.fighters[0].ani.bytes.len() + 1), 0).is_err());
}

/// A decoded state's imported-game AI blocks are checked: a return stack deeper than its 8 entries or a driver state
/// beyond 21 is refused (`br1_check`), so the VM's Ret never reads past the stack.
#[test]
fn br1_ai_state_checked() {
    let Some(d) = data("MIT", "JEN") else {
        eprintln!("skipped: no okb_export / okb_export/br1");
        return;
    };
    let mut m = Match::new(&d);
    for _ in 0..10 {
        m.tick(&d);
    }
    let rebind = |depth: u16, state: i16| {
        let mut c = m.clone();
        let a = &mut c.br1.as_deref_mut().expect("an imported match").ai[0];
        (a.depth, a.state) = (depth, state);
        let mut n: Match = postcard::from_bytes(&postcard::to_allocvec(&c).expect("serialises")).expect("decodes");
        n.rebind(&d)
    };
    assert!(rebind(8, 21).is_ok());
    assert!(rebind(9, 0).is_err());
    assert!(rebind(u16::MAX, 0).is_err());
    assert!(rebind(0, 22).is_err());
    assert!(rebind(0, -1).is_err());
}

/// A trade of equal damage (both candidates kept: the lower damage's dropped, none here) with the imported side
/// P1: the imported game's hit is applied first and publishes its attacker's face, which keeps the direction and counter
/// bits the main game's candidate wrote on it, so the main-game hit's result word has its direction bits (0x4 /
/// 0x8). MIT jabs and YUG jabs back, YUG's damage F.DA set to each value until the two match.
#[test]
fn br1_p1_trade_keeps_direction_bits() {
    use crate::fight::input::pad::SQUARE;
    let Some(d) = data("MIT", "YUG") else {
        eprintln!("skipped: no okb_export / okb_export/br1");
        return;
    };
    let mut trades = 0;
    for off in 0..=8u64 {
        for dmg in 1..=40u8 {
            let mut m = Match::new(&d);
            for f in 0..420u64 {
                m.pads = [
                    if (300..330).contains(&f) { RIGHT } else { 0 } | if (360..362).contains(&f) { SQUARE } else { 0 },
                    if (300..330).contains(&f) { LEFT } else { 0 } | if (360 + off..362 + off).contains(&f) { SQUARE } else { 0 },
                ];
                if f > 360 + off && m.fighters[1].o_da != 0 {
                    m.fighters[1].o_da = dmg;
                }
                let n = m.events.len();
                m.tick(&d);
                let e = &m.events[n..];
                if e.iter().any(|e| e.contains("P1 hits P2 (the imported game")) && e.iter().any(|e| e.contains("P2 hits P1")) {
                    let f60 = m.br1.as_deref().expect("an imported match").fs[0].f.f60;
                    assert_ne!(f60 & 0xC, 0, "offset {off} damage {dmg} frame {f}: core F.60 {f60:08x}");
                    trades += 1;
                }
            }
        }
    }
    assert!(trades > 0, "no trade of equal damage");
    eprintln!("{trades} trades");
}

/// Uriko's stage's counter G.4A in the frame (`stage_anim_step`, simulation v17): it watches P2 if P2 is the imported game's
/// Uriko, else P1; the watched fighter's beast form bit (F.05 bit 0, poked into the core and the face every frame from
/// frame 400) starts it at 2 on that frame, +1 per frame; the other fighter's form does nothing, and v16 never counts.
#[test]
fn uriko_stage_counter_in_the_frame() {
    use crate::fight::version::v::BR1_STAGE_SOUND;
    for (p1, p2, poked, sim, want) in [
        ("MIT", "URI", 1, BR1_STAGE_SOUND, 11),
        ("MIT", "URI", 0, BR1_STAGE_SOUND, 0),
        ("URI", "MIT", 0, BR1_STAGE_SOUND, 11),
        ("URI", "MIT", 1, BR1_STAGE_SOUND, 0),
        ("MIT", "URI", 1, BR1_STAGE_SOUND - 1, 0),
    ] {
        let Some(d) = data_on(p1, p2, "URI00", sim) else {
            eprintln!("skipped: no okb_export / okb_export/br1");
            return;
        };
        assert_eq!(d.br1_stage.map(|s| s.g94), Some(8));
        let mut m = Match::new(&d);
        for f in 0..410u64 {
            if f >= 400 {
                m.br1.as_deref_mut().expect("an imported match").fs[poked].f.form |= 1;
                m.fighters[poked].form |= 1;
            }
            m.tick(&d);
            if f == 399 {
                assert_eq!(m.world.stage.br1.g4a, 0, "{p1} vs {p2}: before the form");
            }
        }
        assert_eq!(m.world.stage.br1.g4a, want, "{p1} vs {p2}, P{} in beast form, v{sim}", poked + 1);
    }
}

/// Uriko's stage's scene split (`load_br1_stage`): its eight walls' objects (`tmd_023_obj5..12`) each a model of its
/// own, none of them left in the stage's model.
#[test]
fn uriko_stage_walls_split_out() {
    let Some(d) = data_on("MIT", "URI", "URI00", crate::fight::version::v::BR1_DEBRIS) else {
        eprintln!("skipped: no okb_export / okb_export/br1");
        return;
    };
    assert_eq!(d.br1_walls.len(), 8);
    assert!(d.br1_walls.iter().all(|m| !m.prims.is_empty() && !m.indices.is_empty()));
    let wall = |n: &str| (5..=12).any(|k| n == format!("tmd_023_obj{k}"));
    assert!(!d.stage.prims.is_empty() && !d.stage.prims.iter().any(|p| wall(d.stage.prim_node(p))));
}

/// Uriko's stage's walls in the frame (simulation v18): once P2 (Uriko) takes the beast form, the walls on
/// the camera's far side shatter at G.4A 15 / 30 / 45 / 60 into the imported game's debris (at most 8 pieces, its sound
/// countdown), all four rise again from G.4A 0x4B / 0x55 / 0x5F / 0x69 and the stage sound (1) (the stage bank's 0x81
/// once G.4A ≥ 0x4B) at 16; the collision walls stay. v17 has none of it.
#[test]
fn uriko_stage_walls_in_the_frame() {
    use crate::fight::stage::br1::Counters;
    use crate::fight::version::v::{BR1_DEBRIS, BR1_STAGE_SOUND};
    for sim in [BR1_DEBRIS, BR1_STAGE_SOUND] {
        let Some(d) = data_on("MIT", "URI", "URI00", sim) else {
            eprintln!("skipped: no okb_export / okb_export/br1");
            return;
        };
        let mut m = Match::new(&d);
        let (mut debris, mut thuds) = (0, 0);
        for f in 0..560u64 {
            if f >= 400 {
                m.br1.as_deref_mut().expect("an imported match").fs[1].f.form |= 1;
                m.fighters[1].form |= 1;
            }
            m.tick(&d);
            debris = debris.max(m.world.stage.debris.iter().filter(|p| p.alive != 0).count());
            thuds += m.br1.as_deref().expect("an imported match").sounds_started.iter().filter(|&&id| id == 0x81).count();
        }
        let c = &m.world.stage.br1;
        if sim == BR1_STAGE_SOUND {
            assert_eq!((c.g50, debris, thuds), ([0; 4], 0, 0), "v17");
            assert_eq!(*c, Counters { g4a: c.g4a, ..Default::default() }, "v17: only G.4A leaves its default");
            continue;
        }
        assert_eq!(c.g50, [16; 4], "every wall risen");
        assert!(c.init && c.g4a > 0x69 + 16);
        assert!((1..=8).contains(&debris), "the shattered walls' debris: {debris}");
        assert_eq!(thuds, 4, "a thud a wall");
        assert!(m.world.stage.walls.hp.iter().all(|&h| h > 0) && m.world.stage.walls.model.iter().all(|&k| k >= 0), "the collision walls stay");
    }
}

/// Mitsuko's stage's water (simulation v18): a broken wall's imported debris splashes into it, each
/// splash logged under the frame the tick started from, so the effects take it on the drawn frame its
/// piece is gone on (`Br1Fx::advance`, BR1FXDBG: `lf + 1 == frame`).
#[test]
fn mitsuko_stage_splash_on_its_frame() {
    use crate::fight::version::v::BR1_DEBRIS;
    let Some(d) = data_on("MIT", "URI", "MIT00", BR1_DEBRIS) else {
        eprintln!("skipped: no okb_export / okb_export/br1");
        return;
    };
    let s = d.br1_stage.as_ref().expect("an imported stage");
    let mut m = Match::new(&d);
    let mut splashes = 0;
    for f in 0..400u64 {
        if f == 100 {
            m.world.stage.spawn_debris_br1(0, s);
        }
        let alive = m.world.stage.debris.iter().filter(|p| p.alive != 0).count();
        m.tick(&d);
        let gone = alive - m.world.stage.debris.iter().filter(|p| p.alive != 0).count();
        let b = m.br1.as_deref().expect("an imported match");
        let logged = b.fx_log.iter().filter(|(lf, _, r)| *lf + 1 == m.frame && matches!(r, Request::Fx { call: Call::StillParticle, .. })).count();
        assert_eq!(logged, gone, "frame {f}: a splash a piece gone, on its frame");
        splashes += gone;
    }
    assert!(splashes > 0, "the debris reaches the water");
}

/// The imported game's CPU AI (simulation v20, `version::v::BR1_AI`): a CPU-controlled imported fighter against a
/// imported and a main-game opponent (both sides CPU) presses its own pads and lands hits; its random table
/// is shuffled at the round start by a stream-G draw; at v19 the same match's imported pads stay 0.
#[test]
fn br1_cpu_ai_fights() {
    for (p1, p2) in [("MIT", "FOX"), ("GRE", "URI"), ("URI", "YUG"), ("ALI", "FOX")] {
        for v in [crate::fight::version::v::BR1_AI, crate::fight::version::v::BR1_SELECT] {
            let Some(d) = data_on(p1, p2, "ST0", v) else {
                eprintln!("skipped: no okb_export / okb_export/br1");
                return;
            };
            let mut m = Match::new(&d);
            m.set_cpu(0, true);
            m.set_cpu(1, true);
            let b = m.br1.as_deref().expect("an imported match");
            let boot = crate::fight::br1::ai::Table::boot(&d.br1_res(if b.side[0] { 0 } else { 1 }).expect("data").tables);
            let none = crate::fight::br1::ai::Table::default();
            assert!(if v >= 20 { b.ai_rnd != boot && b.ai_rnd != none } else { b.ai_rnd == none }, "{p1} {p2} v{v}: the round start's shuffle");
            let (mut pads, mut hurt) = ([0u32; 2], [false; 2]);
            for _ in 0..1500 {
                m.tick(&d);
                let b = m.br1.as_deref().expect("an imported match");
                for i in 0..2 {
                    // the pad without the facing swap 0x200 and the may-guard bit 0x400
                    if b.side[i] && b.fs[i].f.held & !0x600 != 0 {
                        pads[i] += 1;
                    }
                    hurt[i] |= m.fighters[i].o_ac > 0;
                }
            }
            let b = m.br1.as_deref().expect("an imported match");
            for i in (0..2).filter(|&i| b.side[i]) {
                if v >= 20 {
                    assert!(pads[i] > 100, "{p1} {p2} v{v}: P{} pressed on {} frames", i + 1, pads[i]);
                    assert!(hurt[1 - i], "{p1} {p2} v{v}: P{}'s opponent never hit", i + 1);
                } else {
                    assert_eq!(pads[i], 0, "{p1} {p2} v{v}: P{} pressed", i + 1);
                }
            }
        }
    }
}

/// PRACTICE with an imported dummy (simulation v20): the main game's PRACTICE settings (M.D4) give the imported game's practice
/// dummy states (`practice_mode`): STANDING 9 (no input), with COM. GUARD 10, with DEFENSIVE FALL 21, BLOCKS AFTER
/// TAKING DAMAGE 13, CROUCHING 19 (crouched all along) / with a guard 20, JUMPING 12, DODGING 14.
#[test]
fn br1_practice_dummy() {
    let Some(d) = data_on("YUG", "MIT", "ST0", crate::fight::version::v::BR1_AI) else {
        eprintln!("skipped: no okb_export / okb_export/br1");
        return;
    };
    for (d4, state) in [(0x00, 9), (0x08, 10), (0x10, 10), (0x20, 21), (0x01, 13), (0x02, 19), (0x0A, 20), (0x03, 12), (0x04, 14)] {
        let mut m = Match::new(&d);
        let custom = crate::fight::rules::Custom::normal();
        m.configure(&d, super::Rules { need: 2, time_limit: 0xE10, hp_pct: 100, cpu_level: 0x80, custom, practice: true, demo: false });
        m.practice_d4 = d4;
        m.set_cpu(1, true);
        m.br1_ai_init(&d, 1);
        let mut crouched = true;
        for _ in 0..400 {
            m.tick(&d);
            crouched &= m.br1.as_deref().expect("an imported match").fs[1].f.held & 0x4000 != 0;
        }
        let b = m.br1.as_deref().expect("an imported match");
        assert_eq!(b.ai[1].state, state, "d4 {d4:#x}");
        assert_eq!(crouched, d4 == 0x02, "d4 {d4:#x}: crouched");
    }
}

/// (R1) The simulation v25 1v1 trace `fixtures/br1_mixed_v25_hashes.txt`: JENNY (the main game) against MITSUKO
/// (the imported game) on ST0, CPU against CPU for `MIXED_V25_FRAMES` frames; the match's postcard bytes folded into an
/// FNV-1a 64 after every frame, at every 500th and the last (`name frame hash` lines).
const MIXED_V25: &str = "br1_mixed_jen_mit_v25";
const MIXED_V25_FRAMES: u32 = 6000;

/// The trace of `MIXED_V25` and the frames MITSUKO spent in a launched reaction in the air (0x5B, 0x54, 0x56, 0x57,
/// 0x60 above y −64): all of them, and the longest stay; None without the exports.
fn mixed_v25_trace() -> Option<(Vec<String>, u32, u32)> {
    use crate::base::hash::{FNV_BASIS, fnv1a};
    let d = data_on("JEN", "MIT", "ST0", crate::fight::version::v::BR1_LAUNCH)?;
    let mut m = Match::new(&d);
    m.set_cpu(0, true);
    m.set_cpu(1, true);
    let (mut h, mut out, mut air, mut run, mut max) = (FNV_BASIS, Vec::new(), 0u32, 0u32, 0u32);
    for frame in 1..=MIXED_V25_FRAMES {
        m.tick(&d);
        h = fnv1a(h, &postcard::to_allocvec(&m).expect("serialises"));
        if frame % 500 == 0 {
            out.push(format!("{MIXED_V25} {frame} {h:016x}"));
        }
        let c = &m.br1.as_deref().expect("an imported match").fs[1];
        if matches!(c.s.state_cur, 0x5B | 0x54 | 0x56 | 0x57 | 0x60) && c.anim.pos[1] < -64 {
            (air, run) = (air + 1, run + 1);
            max = max.max(run);
        } else {
            run = 0;
        }
    }
    out.push(format!("{MIXED_V25} {MIXED_V25_FRAMES} {h:016x} end"));
    Some((out, air, max))
}

/// (R1) A 1v1 mixed match at simulation v25 traces as recorded (`br1_mixed_v25_fixture_hashes_record`), so a change of
/// v25's mixed pair fails here; it covers `version::v::BR1_LAUNCH`: JENNY's knock-downs launch MITSUKO (frames in a
/// launched reaction in the air, none longer than 90).
#[test]
fn br1_mixed_v25_fixture() {
    let Some((trace, air, max)) = mixed_v25_trace() else {
        eprintln!("skipped: no okb_export / okb_export/br1");
        return;
    };
    assert!(air > 0 && max <= 90, "launched {air} frames, the longest {max}");
    let want: Vec<&str> = include_str!("../../session/fixtures/br1_mixed_v25_hashes.txt").lines().collect();
    assert_eq!(trace, want, "the trace");
}

/// Writes `fixtures/br1_mixed_v25_hashes.txt` (by hand, after an intended change of simulation v25's mixed match:
/// `cargo test --release br1_mixed_v25_fixture_hashes_record -- --ignored`, with both exports beside the engine).
#[test]
#[ignore]
fn br1_mixed_v25_fixture_hashes_record() {
    let (lines, ..) = mixed_v25_trace().expect("okb_export / okb_export/br1");
    let path = format!("{}/src/session/fixtures/br1_mixed_v25_hashes.txt", env!("CARGO_MANIFEST_DIR"));
    std::fs::write(&path, lines.join("\n") + "\n").expect("written");
}

/// (T7, simulation v25 `version::v::BR1_LAUNCH`) A main-game knock-down without a launch of its own launches the
/// imported fighter it hits (`bridge::launch`): CPU vs CPU, the imported side's launched reactions (0x5B, 0x54,
/// 0x56, 0x57, 0x60) stay in the air at most about a second (the longest seen: 62 frames); at v24 JENNY's knock-down
/// leaves MITSUKO hanging in 0x5B for hundreds of frames.
#[test]
fn a_br2_knock_down_launches_a_br1_fighter_from_v25() {
    let longest = |p1: &str, p2: &str, v: u32, frames: u32| {
        let d = data_on(p1, p2, "ST0", v)?;
        let mut m = Match::new(&d);
        m.set_cpu(0, true);
        m.set_cpu(1, true);
        let (mut run, mut max) = (0u32, 0u32);
        for _ in 0..frames {
            m.tick(&d);
            let c = &m.br1.as_deref().expect("an imported match").fs[1];
            if matches!(c.s.state_cur, 0x5B | 0x54 | 0x56 | 0x57 | 0x60) && c.anim.pos[1] < -64 {
                run += 1;
                max = max.max(run);
            } else {
                run = 0;
            }
        }
        Some(max)
    };
    for (p1, p2) in [("JEN", "MIT"), ("GAD", "FOX")] {
        let Some(max) = longest(p1, p2, crate::fight::version::v::BR1_LAUNCH, 8000) else {
            eprintln!("skipped: no okb_export / okb_export/br1");
            return;
        };
        assert!(max <= 90, "{p1} {p2}: {max} frames in the air");
    }
    let old = longest("JEN", "MIT", crate::fight::version::v::BR1_LAUNCH - 1, 6000).expect("the exports");
    assert!(old > 300, "v24: {old}");
}
