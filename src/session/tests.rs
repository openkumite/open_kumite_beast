//! The session's tests: portable math, round trips, decoders, fixtures, the rollback test.
//!
//! The session's tests: the portable skeleton math end to end, steps and save states round trips, the snapshot and
//! file decoders against bad bytes, the save-state v1 fixture, the roster / stage defaults and the rollback test.

use super::snapshot::DECODE_BUDGET;
use super::*;
use crate::assets::roster;
use crate::base::decode_limit;
use std::sync::Arc;

/// Stage `s`'s name as the select shows it (`frontend::stage_name`).
fn st(s: u8) -> &'static str {
    crate::frontend::stage_name(s)
}

/// The select's RANDOM (the text list `stages`, entry 9).
fn random() -> &'static str {
    crate::assets::text::item("stages", 9)
}

/// The converted export next to the engine (OKB_EXPORT overrides); None skips the test.
fn export() -> Option<crate::assets::Assets> {
    let root = crate::base::env::var("EXPORT").unwrap_or_else(|_| concat!(env!("CARGO_MANIFEST_DIR"), "/../okb_export").into());
    std::path::Path::new(&root).join("manifest.json").exists().then(|| crate::assets::Assets::new(root))
}

/// Simulation version 3's (and later ones') same bits on every target, end to end: a direct fight (P1 scripted,
/// P2 the CPU, both into the beast) through `step`, FNV-1a over only the skeleton's outputs (`Fighter::parts`,
/// `part_rots`, `spring_aims`), so a state-layout change leaves it be. A glam / libm / compiler change that alters
/// the portable math fails here (and needs a SIM_VERSION bump); the determinism matrix compares
/// targets.
#[test]
fn v3_skeleton_bits() {
    let Some(assets) = export() else {
        eprintln!("skipped: no okb_export");
        return;
    };
    // v12: the skeleton before the head turn; v13 on: node 2 and its children turned (`move_script::head_turn`)
    assert_eq!(skeleton_bits(&assets, 12), 0x6FA9D28EE88B139A, "the v3 skeleton changed");
    assert_eq!(skeleton_bits(&assets, SIM_VERSION), 0x72F8DBD4BCC8A214, "the v13 skeleton changed");
}

/// The hash of `v3_skeleton_bits`'s fight at simulation version `v`.
fn skeleton_bits(assets: &crate::assets::Assets, v: u32) -> u64 {
    let (p1, p2) = ("ALI", "LON");
    let data = pollster::block_on(crate::fight::fight_data::load_match(assets, [p1, p2], crate::fight::fight_data::direct_costume(), "ST0", false, v))
        .expect("the fight");
    assert!(v >= 3 && data.portable());
    let start = SessionStart { seed: 1, sim_version: v, entry: Entry::DirectFight { p1: p1.into(), p2: p2.into(), stage: "ST0".into() }, ..Default::default() };
    let store = AssetStore { fight: Some(Arc::new(data)), ..Default::default() };
    let mut s = new_state(&start, &store.assets(Default::default()), &mut Out::default());
    let script = crate::fight::input_script::parse_script("120:,30:R,1:J,12:,1:K,12:,20:L,1:DK,30:,40:R,1:RJK,30:,1:C,60:,30:L,1:J,12:,1:K,200:");
    let mut h = 0xCBF2_9CE4_8422_2325u64;
    let mut fnv = |w: u32| h = w.to_le_bytes().iter().fold(h, |h, &b| (h ^ b as u64).wrapping_mul(0x0000_0100_0000_01B3));
    let (mut springs, mut beasts) = (0usize, [false; 2]);
    for k in 0..600 {
        let cheats = match k {
            150 => vec![Cheat::Gauges],
            160 => vec![Cheat::Beast(0), Cheat::Beast(1)],
            _ => Vec::new(),
        };
        let i = FrameInput { pad: [script.get(k).copied().unwrap_or(0), 0], ..Default::default() };
        step(&mut s, &store.assets(Default::default()), i, &cheats, &mut Out::default());
        let m = &s.fight.as_ref().expect("the fight").m;
        for (f, b) in m.fighters.iter().zip(&mut beasts) {
            f.parts.iter().flatten().for_each(|&v| fnv(v as u32));
            f.part_rots.iter().flatten().flatten().for_each(|&v| fnv(v as u32));
            for (n, d) in &f.spring_aims {
                fnv(*n as u32);
                d.to_array().iter().for_each(|v| fnv(v.to_bits()));
            }
            springs += f.spring_aims.len();
            *b |= f.beast();
        }
    }
    assert!(springs > 0 && beasts == [true; 2], "springs {springs} beasts {beasts:?}");
    eprintln!("skeleton bits v{v}: {h:#018X}");
    h
}

/// The cheap observables of a step: the session's frame, the front end's screen, the fight's frame, round,
/// positions, damage and gauges.
fn observe(s: &GameState) -> String {
    let mut o = format!("{} fe {}", s.frame, s.front.as_ref().map(|f| f.describe()).unwrap_or_default());
    if let Some(g) = &s.fight {
        let r = &g.m.round;
        o += &format!(" fight {} round {} {:#x} {}", g.m.frame, r.index, r.state, r.timer);
        for f in &g.m.fighters {
            o += &format!(" | {:?} mv {:#x} dmg {} gauge {}", f.m_t, f.move_cur, f.o_ac, f.gauge);
        }
    }
    o
}

/// `n` steps from `start` with `script` on pad 1; the match a step asks for is loaded at once and reported
/// ready to the next step (as the app does once it is in the store).
fn run(assets: &crate::assets::Assets, mut store: AssetStore, start: &SessionStart, script: &[u16], n: usize) -> Vec<String> {
    let prefs = crate::frontend::options::Prefs::default();
    let mut out = Out::default();
    let mut s = new_state(start, &store.assets(prefs), &mut out);
    let mut flags = 0;
    let mut seen = Vec::new();
    for k in 0..n {
        let i = FrameInput { pad: [script.get(k).copied().unwrap_or(0), 0], forced: [0; 2], flags: std::mem::take(&mut flags), ..Default::default() };
        let mut out = Out::default();
        step(&mut s, &store.assets(prefs), i, &[], &mut out);
        for r in out.requests {
            match r {
                Request::Load(spec) => {
                    let chr = spec.chr.map(crate::assets::roster::code);
                    let l = pollster::block_on(crate::fight::fight_data::load_match(
                        assets,
                        chr,
                        spec.costume,
                        crate::fight::fight_data::stage_code(spec.stage),
                        false,
                        crate::session::SIM_VERSION,
                    ));
                    store.fight = Some(Arc::new(l.expect("the match")));
                    flags |= FrameInput::LOADED;
                }
                Request::FightOver { .. } => store.fight = None,
                _ => {}
            }
        }
        seen.push(observe(&s));
    }
    seen
}

/// `session::step` is deterministic: the same start and pads give the same states, for a direct fight (P2 the
/// CPU) and for the front end idling into the attract demo (a match loaded through `Request::Load` /
/// `FrameInput::LOADED`, CPU vs CPU).
#[test]
fn step_twice_same_states() {
    let Some(assets) = export() else {
        eprintln!("skipped: no okb_export");
        return;
    };
    let script = crate::fight::input_script::parse_script("340:,20:R,1:J,12:,1:K,12:,1:DK,20:,1:R,1:RJK,30:,1:C,90:,1:J,12:,1:K,200:");
    let data = || pollster::block_on(crate::session::start::load(&assets, "YUG", "ALI", "ST0")).expect("the fight");
    let start = SessionStart { seed: 1, entry: Entry::DirectFight { p1: "YUG".into(), p2: "ALI".into(), stage: "ST0".into() }, ..Default::default() };
    let a = run(&assets, AssetStore { fight: Some(Arc::new(data())), ..Default::default() }, &start, &script, 900);
    let b = run(&assets, AssetStore { fight: Some(Arc::new(data())), ..Default::default() }, &start, &script, 900);
    assert!(a.last().is_some_and(|l| l.contains("fight 900")), "{:?}", a.last());
    assert_eq!(a, b);

    let front = Arc::new(pollster::block_on(crate::frontend::load(&assets)).expect("front data"));
    let start = SessionStart { seed: 1, entry: Entry::Title, ..Default::default() };
    let store = || AssetStore { front: Some(front.clone()), ..Default::default() };
    let a = run(&assets, store(), &start, &[], 1200);
    let b = run(&assets, store(), &start, &[], 1200);
    assert!(a.iter().any(|l| l.contains(" fight ")), "the attract demo did not start");
    assert_eq!(a, b);
}

/// What a test does with a step's request (as the app): a match loaded, files fetched, a fight's data dropped
/// or kept as the backdrop; the completion goes into the next input's flags.
fn serve(assets: &crate::assets::Assets, store: &mut AssetStore, r: Request, flags: &mut u8) {
    match r {
        Request::Load(spec) => {
            let chr = spec.chr.map(crate::assets::roster::code);
            let l = pollster::block_on(crate::fight::fight_data::load_match(
                assets,
                chr,
                spec.costume,
                crate::fight::fight_data::stage_code(spec.stage),
                false,
                crate::session::SIM_VERSION,
            ));
            store.fight = Some(Arc::new(l.expect("the match")));
            *flags |= FrameInput::LOADED;
        }
        Request::Fetch(names) => {
            for n in names {
                if let Ok(b) = pollster::block_on(assets.fetch(&n)) {
                    store.files.insert(n, b.into());
                }
            }
            *flags |= FrameInput::FETCHED;
        }
        Request::FightOver { backdrop, .. } => {
            let data = store.fight.take();
            if backdrop {
                store.backdrop = data;
            }
        }
        _ => {}
    }
}

/// `n` steps from `start` (pad 1 and the cheats by step); after the steps in `checks` the state is saved and
/// loaded again (the bytes of the loaded state must be the same), and the copy then runs `m` steps beside the
/// original with the same inputs and the same store: same hash and requests after every step. Returns per check
/// (step, state bytes, save / load / hash ms, a backdrop or a movie up).
fn round_trips(
    assets: &crate::assets::Assets,
    mut store: AssetStore,
    start: &SessionStart,
    n: usize,
    input: impl Fn(usize) -> (u16, Vec<Cheat>),
    checks: &[usize],
    m: usize,
) -> Vec<(usize, usize, f64, f64, f64, String)> {
    let prefs = crate::frontend::options::Prefs::default();
    let mut out = Out::default();
    let mut s = new_state(start, &store.assets(prefs), &mut out);
    let mut flags = 0;
    let mut copies: Vec<(usize, GameState, usize)> = Vec::new();
    let mut report = Vec::new();
    let mut file_sizes = Vec::new();
    for k in 0..n {
        let (pad, cheats) = input(k);
        let i = FrameInput { pad: [pad, 0], forced: [0; 2], flags: std::mem::take(&mut flags), ..Default::default() };
        let mut out = Out::default();
        step(&mut s, &store.assets(prefs), i, &cheats, &mut out);
        let (h, reqs) = (hash(&s), format!("{:?}", out.requests));
        for (at, c, left) in copies.iter_mut() {
            let mut o = Out::default();
            step(c, &store.assets(prefs), i, &cheats, &mut o);
            if hash(c) != h {
                let (a, b) = (section_hashes(&s), section_hashes(c));
                let diff: Vec<_> = a.iter().zip(&b).filter(|(x, y)| x != y).map(|(x, _)| x.0).collect();
                panic!("step {k}: the state loaded after step {at} differs in {diff:?}");
            }
            assert_eq!(format!("{:?}", o.requests), reqs, "step {k}: requests of the state loaded after step {at}");
            assert_eq!(o.card_dirty, out.card_dirty);
            *left -= 1;
        }
        copies.retain(|c| c.2 > 0);
        for r in out.requests {
            serve(assets, &mut store, r, &mut flags);
        }
        if s.backdrop.is_none() {
            store.backdrop = None;
        }
        store.prune(&s);
        if checks.contains(&k) {
            assert_eq!(hash(&s.clone()), hash(&s), "a clone hashes the same");
            let t = std::time::Instant::now();
            let bytes = save_state(&s);
            let t_save = t.elapsed().as_secs_f64() * 1e3;
            let t = std::time::Instant::now();
            let c = load_state(&bytes, &store.assets(prefs)).expect("the state loads");
            let t_load = t.elapsed().as_secs_f64() * 1e3;
            assert!(save_state(&c) == bytes, "step {k}: the loaded state serialises differently");
            let file = save_file(&s);
            let f = load_file(&file, &store.assets(prefs)).expect("the file loads");
            assert!(save_state(&f) == bytes, "step {k}: the state from the file differs");
            file_sizes.push(file.len());
            // the fetched files the state reads: without them in the store it does not load
            let needs = s.front.as_ref().is_some_and(|f| !f.needed_files().is_empty());
            if needs {
                let none = FileCache::default();
                let bare = Assets { files: &none, ..store.assets(prefs) };
                assert!(load_state(&bytes, &bare).is_err_and(|e| e.contains("not in the store")), "step {k}: loaded without its files");
                assert!(load_file(&file, &bare).is_err(), "step {k}: the file loaded without its files");
            }
            let t = std::time::Instant::now();
            let _ = hash(&s);
            let t_hash = t.elapsed().as_secs_f64() * 1e3;
            let what = format!(
                "{}{}{}{}{}",
                s.front.as_ref().map(|f| f.describe()).unwrap_or_default(),
                if needs { " +files" } else { "" },
                if s.fight.is_some() { " +fight" } else { "" },
                if s.backdrop.is_some() { " +backdrop" } else { "" },
                s.music.map(|m| format!(" +music {:#x} at {}", m.track, m.frames)).unwrap_or_default()
            );
            report.push((k, bytes.len(), t_save, t_load, t_hash, what));
            copies.push((k, c, m));
        }
    }
    assert!(copies.is_empty(), "a loaded copy did not run its {m} steps (a check less than {m} steps before {n})");
    for ((k, len, a, b, c, what), file) in report.iter().zip(&file_sizes) {
        eprintln!("step {k:5}: {len:6} bytes (file {file:6}), save {a:5.2} ms, load {b:5.2} ms, hash {c:5.2} ms  {what}");
    }
    report
}

/// (TEAM ARCADE A4) A runtime snapshot of state version 24 (the released A2 build's: no `TeamFight::arcade`, no
/// `TeamOutcome::Continue` / `GameOver`) is refused by its version, also with this build's id, before its bytes decode.
#[test]
fn state_24_refused_by_version() {
    const { assert!(STATE_VERSION > 24) };
    let files = FileCache::default();
    let none = Assets { front: None, fight: None, backdrop: None, files: &files, prefs: Default::default() };
    let old = postcard::to_allocvec(&(24u32, crate::base::build::VERSION, GameState::default())).unwrap();
    let e = load_state(&old, &none).err().unwrap_or_default();
    assert!(e.contains("version 24"), "{e}");
}

/// Bytes that are not a state of this build give Err, never a panic or a large allocation: empty, truncated,
/// garbage, another version or build, too large, length bombs (a huge length prefix; a long run of one-byte
/// `None`s of a large type), a file of a newer or unknown format; with the export, every truncation and many
/// single-byte changes of a title-screen state and of a fight state load or give Err.
#[test]
fn load_state_rejects_bad_bytes() {
    const STEPS_AFTER_LOAD: usize = 120;
    let files = FileCache::default();
    let none = Assets { front: None, fight: None, backdrop: None, files: &files, prefs: Default::default() };
    let good = save_state(&GameState::default());
    assert!(load_state(&good, &none).is_ok());
    assert!(load_state(&[], &none).is_err());
    assert!(load_state(&good[..good.len() - 1], &none).is_err());
    assert!(load_state(&[0xFF; 64], &none).is_err());
    assert!(load_state(&vec![0; MAX_STATE_BYTES + 1], &none).is_err());
    let other = postcard::to_allocvec(&(STATE_VERSION + 1, crate::base::build::VERSION, GameState::default())).unwrap();
    assert!(load_state(&other, &none).is_err());
    let other = postcard::to_allocvec(&(STATE_VERSION, "0.0.0 (none)", GameState::default())).unwrap();
    assert!(load_state(&other, &none).is_err());
    // length bombs: a 2^32 length with no data; a million 1-byte `None`s that would decode into 260 MB
    type Big = Vec<Option<[u64; 32]>>;
    let huge = [0xFF, 0xFF, 0xFF, 0xFF, 0x0F];
    assert!(postcard::from_bytes::<decode_limit::Budgeted<Big, DECODE_BUDGET>>(&huge).is_err());
    let mut nones = postcard::to_allocvec(&1_000_000u32).unwrap();
    nones.resize(nones.len() + 1_000_000, 0);
    assert!(postcard::from_bytes::<decode_limit::Budgeted<Big, DECODE_BUDGET>>(&nones).is_err());
    let mut cbor = Vec::new();
    ciborium::into_writer(&vec![None::<[u64; 32]>; 1_000_000], &mut cbor).unwrap();
    assert!(ciborium::from_reader::<decode_limit::Budgeted<Big, DECODE_BUDGET>, _>(cbor.as_slice()).is_err());
    assert!(ciborium::from_reader::<decode_limit::Budgeted<Big, DECODE_BUDGET>, _>([0x9B, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF].as_slice()).is_err());
    // files
    let file = save_file(&GameState::default());
    assert!(load_file(&file, &none).is_ok());
    assert!(load_file(&file[..file.len() - 1], &none).is_err());
    assert!(load_file(b"BR2", &none).is_err());
    assert!(load_file(&[0xFF; 64], &none).is_err());
    let mut newer = file.clone();
    newer[4..6].copy_from_slice(&(FILE_FORMAT + 1).to_le_bytes());
    assert!(load_file(&newer, &none).is_err_and(|e| e.contains("newer")));
    newer[4..6].copy_from_slice(&0u16.to_le_bytes());
    assert!(load_file(&newer, &none).is_err_and(|e| e.contains("unknown")));
    let mut bomb = file[..6].to_vec();
    bomb.extend_from_slice(&[0x82, 0x60, 0x9B, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF]);
    assert!(load_file(&bomb, &none).is_err());

    let Some(assets) = export() else {
        eprintln!("skipped the title and fight states: no okb_export");
        return;
    };
    let front = Arc::new(pollster::block_on(crate::frontend::load(&assets)).expect("front data"));
    let data = pollster::block_on(crate::session::start::load(&assets, "YUG", "ALI", "ST0")).expect("the fight");
    let starts = [
        (SessionStart { seed: 1, entry: Entry::Title, ..Default::default() }, AssetStore { front: Some(front), ..Default::default() }, 30),
        (
            SessionStart { seed: 1, entry: Entry::DirectFight { p1: "YUG".into(), p2: "ALI".into(), stage: "ST0".into() }, ..Default::default() },
            AssetStore { fight: Some(Arc::new(data)), ..Default::default() },
            400,
        ),
    ];
    for (start, store, n) in starts {
        let a = store.assets(Default::default());
        let mut s = new_state(&start, &a, &mut Out::default());
        for _ in 0..n {
            step(&mut s, &a, FrameInput::default(), &[], &mut Out::default());
        }
        for bytes in [save_state(&s), save_file(&s)] {
            let load = |b: &[u8]| if b.starts_with(FILE_MAGIC) { load_file(b, &a) } else { load_state(b, &a) };
            assert!(load(&bytes).is_ok());
            let mut tail = bytes.clone();
            tail.push(0);
            assert!(load(&tail).is_err(), "trailing byte");
            let step_by = (bytes.len() / 3000).max(1);
            for k in (0..bytes.len()).step_by(step_by) {
                assert!(load(&bytes[..k]).is_err(), "truncated at {k}");
                for x in [0xA5, 0xFF, 0x01] {
                    let mut b = bytes.clone();
                    b[k] ^= x;
                    // a state that loads runs on without a panic
                    if let Ok(mut s) = load(&b) {
                        for _ in 0..STEPS_AFTER_LOAD {
                            step(&mut s, &a, FrameInput::default(), &[], &mut Out::default());
                        }
                    }
                }
            }
        }
    }
}

/// The v1 save-state file (`fixtures/state_v1.okbs`: the character select with its music, written before
/// `GameState::music_volume` existed) still loads, the field at its default, and runs on.
#[test]
fn old_state_file_loads() {
    let file = include_bytes!("fixtures/state_v1.okbs");
    let (s, engine) = decode_file(file).expect("the v1 file decodes");
    assert!(!engine.is_empty());
    assert_eq!(s.frame, 150);
    assert!(s.front.as_ref().is_some_and(|f| f.describe().starts_with("select")));
    assert!(s.music.is_some_and(|m| m.track == 0x1F && m.looped && m.frames > 0), "{:?}", s.music);
    assert_eq!(s.music_volume, 0x7F);
    let Some(assets) = export() else { return };
    let front = Arc::new(pollster::block_on(crate::frontend::load(&assets)).expect("front data"));
    let store = AssetStore { front: Some(front), ..Default::default() };
    let a = store.assets(Default::default());
    let mut s = load_file(file, &a).expect("the v1 file loads");
    let frames = s.music.map(|m| m.frames);
    step(&mut s, &a, FrameInput::default(), &[], &mut Out::default());
    assert_eq!((s.frame, s.music.map(|m| m.frames)), (151, frames.map(|f| f + 1)));
}

/// The state of `fixtures/state_v1.okbs`: the title, START, ARCADE, then the character select with its music
/// (150 steps).
fn select_state(a: &Assets) -> GameState {
    let start = SessionStart { seed: 1, entry: Entry::Title, ..Default::default() };
    let mut s = new_state(&start, a, &mut Out::default());
    for k in 0..150 {
        let pad = match k {
            40 => crate::fight::input::pad::START,
            70 => crate::fight::input::pad::CROSS,
            _ => 0,
        };
        step(&mut s, a, FrameInput { pad: [pad, 0], ..Default::default() }, &[], &mut Out::default());
    }
    s
}

/// Writes `fixtures/state_v1.okbs` (run once, with the export, when the fixture is made).
#[test]
#[ignore]
fn write_state_v1_fixture() {
    let assets = export().expect("the export");
    let front = Arc::new(pollster::block_on(crate::frontend::load(&assets)).expect("front data"));
    let store = AssetStore { front: Some(front), ..Default::default() };
    let s = select_state(&store.assets(Default::default()));
    assert!(s.front.as_ref().is_some_and(|f| f.describe().starts_with("select")) && s.music.is_some(), "{:?}", s.music);
    std::fs::write(concat!(env!("CARGO_MANIFEST_DIR"), "/src/session/fixtures/state_v1.okbs"), save_file(&s)).unwrap();
}

/// The roster by default (v5): every character and fourth costume selectable, the card untouched (its
/// progression still the defaults' after the select); `locked` and simulation v4 sessions: the save's.
#[test]
fn roster_open_by_default_locked_opt_out() {
    let Some(assets) = export() else {
        eprintln!("skipped: no okb_export");
        return;
    };
    let front = Arc::new(pollster::block_on(crate::frontend::load(&assets)).expect("front data"));
    let store = AssetStore { front: Some(front), ..Default::default() };
    let a = store.assets(Default::default());
    let at_select = |start: SessionStart| {
        let mut s = new_state(&start, &a, &mut Out::default());
        for k in 0..150 {
            let pad = match k {
                40 => crate::fight::input::pad::START,
                70 => crate::fight::input::pad::CROSS,
                _ => 0,
            };
            step(&mut s, &a, FrameInput { pad: [pad, 0], ..Default::default() }, &[], &mut Out::default());
        }
        let f = s.front.expect("front end");
        assert!(f.describe().starts_with("select"), "{}", f.describe());
        let card = f.save.memory_card.clone();
        (f.selectable(), (f.save.settings.characters, f.save.settings.costume4), f.save.dirty, card)
    };
    let base = SessionStart { seed: 1, entry: Entry::Title, ..Default::default() };
    let (open, kept, dirty, card) = at_select(base.clone());
    assert_eq!(open, (0x7FF, 0x7FF));
    // the save's progression is the defaults' (9 characters, no fourth costume) and nothing is to be written
    assert_eq!((kept, dirty), ((0x1FF, 0), false));
    assert_eq!(card, crate::frontend::options::Card::new(None, None));
    assert_eq!(at_select(SessionStart { locked: true, ..base.clone() }).0, (0x1FF, 0));
    assert_eq!(at_select(SessionStart { sim_version: 4, ..base.clone() }).0, (0x1FF, 0));
    // the debug unlock opens everything either way
    assert_eq!(at_select(SessionStart { locked: true, unlock: true, ..base }).0, (0x7FF, 0x7FF));
}

/// The VS stage select (L1 / R1 from RANDOM): by default (v5) ST9..STC follow the game's nine as EXTRA1..4
/// before RANDOM; `locked` and v4 sessions: the game's nine and RANDOM. RANDOM draws from all the rows'
/// stages.
#[test]
fn stage_select_open_by_default_locked_opt_out() {
    let Some(assets) = export() else {
        eprintln!("skipped: no okb_export");
        return;
    };
    let front = Arc::new(pollster::block_on(crate::frontend::load(&assets)).expect("front data"));
    let store = AssetStore { front: Some(front), ..Default::default() };
    let a = store.assets(Default::default());
    use crate::fight::input::pad::{CROSS, DOWN, L1, R1, START};
    // title → START → DOWN (VS) → × → the select; then L1 / R1 presses every 10 frames
    let walk = |start: SessionStart, keys: &[u16]| {
        let mut s = new_state(&start, &a, &mut Out::default());
        let mut seen = Vec::new();
        for k in 0..170 + 10 * keys.len() {
            let pad = match k {
                40 => START,
                60 => DOWN,
                80 => CROSS,
                k if k >= 170 && k % 10 == 0 => keys[(k - 170) / 10],
                _ => 0,
            };
            step(&mut s, &a, FrameInput { pad: [pad, 0], ..Default::default() }, &[], &mut Out::default());
            let d = s.front.as_ref().expect("front end").describe();
            if let Some(t) = d.strip_prefix("select ").and_then(|d| d.split(" stage ").nth(1)) {
                seen.push(t.split(" [").next().unwrap_or("").to_string());
            }
        }
        seen.dedup();
        seen
    };
    let base = SessionStart { seed: 1, entry: Entry::Title, ..Default::default() };
    let l1 = [L1; 5];
    assert_eq!(walk(base.clone(), &l1), [random(), "EXTRA4", "EXTRA3", "EXTRA2", "EXTRA1", st(8)]);
    assert_eq!(walk(base.clone(), &[R1, L1, L1]), [random(), st(0), random(), "EXTRA4"]);
    let original = [random(), st(8), st(7), st(6), st(5), st(4)];
    assert_eq!(walk(SessionStart { locked: true, ..base.clone() }, &l1), original);
    assert_eq!(walk(SessionStart { sim_version: 4, ..base.clone() }, &l1), original);
    // the debug unlock does not change the select's stages (a v4 session may have used it)
    assert_eq!(walk(SessionStart { locked: true, unlock: true, ..base }, &l1), original);
}

/// The extra stage Q3DM17 (simulation v10, `SessionStart::extra_stage`): Q3DM17 after EXTRA4 before RANDOM
/// only when the session has its files, from v10, and not `locked`; chosen, its match is stage 13; RANDOM never
/// draws it (its rows' stages 0..12 as before).
#[test]
fn stage_select_extra_stage() {
    let Some(assets) = export() else {
        eprintln!("skipped: no okb_export");
        return;
    };
    let front = Arc::new(pollster::block_on(crate::frontend::load(&assets)).expect("front data"));
    let store = AssetStore { front: Some(front), ..Default::default() };
    let a = store.assets(Default::default());
    use crate::fight::input::pad::{CROSS, DOWN, L1, R1, START};
    // title → START → DOWN (VS) → × → the select; then the keys every 10 frames, then × for both players twice
    // (character, costume); the stages the select showed and the match asked for
    let walk = |start: SessionStart, keys: &[u16]| {
        let mut s = new_state(&start, &a, &mut Out::default());
        let (mut seen, mut spec) = (Vec::new(), None);
        let end = 170 + 10 * keys.len();
        for k in 0..end + 400 {
            let pad = match k {
                40 => START,
                60 => DOWN,
                80 => CROSS,
                k if (170..end).contains(&k) && k % 10 == 0 => keys[(k - 170) / 10],
                k if k >= end && k < end + 200 && k % 40 == 0 => CROSS,
                _ => 0,
            };
            let mut out = Out::default();
            step(&mut s, &a, FrameInput { pad: [pad, pad], ..Default::default() }, &[], &mut out);
            if let Some(m) = out.requests.iter().find_map(|r| if let Request::Load(m) = r { Some(*m) } else { None }) {
                spec = Some(m.stage);
                break;
            }
            let d = s.front.as_ref().expect("front end").describe();
            if let Some(t) = d.strip_prefix("select ").and_then(|d| d.split(" stage ").nth(1)) {
                seen.push(t.split(" [").next().unwrap_or("").to_string());
            }
        }
        seen.dedup();
        (seen, spec)
    };
    let base = SessionStart { seed: 1, entry: Entry::Title, ..Default::default() };
    let with = SessionStart { extra_stage: true, ..base.clone() };
    let (seen, spec) = walk(with.clone(), &[L1, L1]);
    assert_eq!(seen, [random(), "Q3DM17", "EXTRA4"]);
    assert_eq!(spec, Some(12));
    let (seen, spec) = walk(with.clone(), &[L1]);
    assert_eq!(seen, [random(), "Q3DM17"]);
    assert_eq!(spec, Some(crate::fight::stage::extra::Q3DM17.stage));
    assert_eq!(walk(with.clone(), &[R1, L1, L1]).0, [random(), st(0), random(), "Q3DM17"]);
    // without the files, before v10, locked: no extra row
    for start in [base.clone(), SessionStart { sim_version: 9, ..with.clone() }] {
        assert_eq!(walk(start, &[L1]).0, [random(), "EXTRA4"]);
    }
    assert_eq!(walk(SessionStart { locked: true, ..with.clone() }, &[L1]).0, [random(), st(8)]);
    // RANDOM: one of the 13 (the draw as without the files), for a range of seeds
    let both = SessionStart { extra_q3dm6: true, ..with.clone() };
    for seed in 0..24 {
        let (_, a) = walk(SessionStart { seed, ..with.clone() }, &[]);
        let (_, b) = walk(SessionStart { seed, ..base.clone() }, &[]);
        let (_, c) = walk(SessionStart { seed, ..both.clone() }, &[]);
        assert!(a.is_some_and(|s| s < 13) && a == b && c == b, "seed {seed}: {a:?} {b:?} {c:?}");
    }
    // Q3DM6 (simulation v12, `SessionStart::extra_q3dm6`): after Q3DM17, or in its row without Q3DM17's files; its
    // match is stage 14
    let q6 = crate::fight::stage::extra::Q3DM6.stage;
    let (seen, spec) = walk(both.clone(), &[L1, L1]);
    assert_eq!(seen, [random(), "Q3DM6", "Q3DM17"]);
    assert_eq!(spec, Some(13));
    let (seen, spec) = walk(both.clone(), &[L1]);
    assert_eq!(seen, [random(), "Q3DM6"]);
    assert_eq!(spec, Some(q6));
    assert_eq!(walk(both.clone(), &[R1, L1, L1, L1]).0, [random(), st(0), random(), "Q3DM6", "Q3DM17"]);
    let only = SessionStart { extra_q3dm6: true, ..base.clone() };
    let (seen, spec) = walk(only.clone(), &[L1, L1]);
    assert_eq!(seen, [random(), "Q3DM6", "EXTRA4"]);
    assert_eq!(spec, Some(12));
    let (seen, spec) = walk(only.clone(), &[L1]);
    assert_eq!(seen, [random(), "Q3DM6"]);
    assert_eq!(spec, Some(q6));
    // before v12 no Q3DM6 row (a v11 session with both runs as recorded: Q3DM17 alone); locked: none
    assert_eq!(walk(SessionStart { sim_version: 11, ..only.clone() }, &[L1]).0, [random(), "EXTRA4"]);
    assert_eq!(walk(SessionStart { sim_version: 11, ..both.clone() }, &[L1, L1]).0, [random(), "Q3DM17", "EXTRA4"]);
    assert_eq!(walk(SessionStart { locked: true, ..both.clone() }, &[L1]).0, [random(), st(8)]);
}

/// The select's imported row and stages (simulation v19, the import S9; `SessionStart::br1`): in VS the
/// bottom row's DOWN leads to MITSUKO FOX GREG "URIKO (BR1)" (wrapping as the main game's rows); the stage select has
/// the imported game's eleven stages after the extra stages, the extra stages passed over while a cursor is on an
/// imported fighter; the match asks for characters 11..14 and stages 15..25. Without the data, before v19 or locked:
/// neither (the walk as before). The front end's own data only (its portraits blank without the install).
/// Data-gated on both games: their texts are installed before the walk (the names it reads).
#[test]
fn select_br1_row_and_stages() {
    let (Some(assets), Some(_)) = (export(), crate::formats::tables::br1_export_tables()) else {
        eprintln!("skipped: no okb_export (both games)");
        return;
    };
    crate::formats::tables::export_tables().expect("tables.okt");
    let front = Arc::new(pollster::block_on(crate::frontend::load(&assets)).expect("front data"));
    let store = AssetStore { front: Some(front), ..Default::default() };
    let a = store.assets(Default::default());
    use crate::fight::input::pad::{CROSS, DOWN, L1, LEFT, START};
    // title → START → DOWN (VS) → × → the select; P1's keys then the stage keys every 10 frames, then × for both
    // players; the cursors and stages the select showed and the match asked for
    let walk = |start: SessionStart, keys: &[u16]| {
        let mut s = new_state(&start, &a, &mut Out::default());
        let (mut seen, mut spec) = (Vec::new(), None);
        let end = 170 + 10 * keys.len();
        for k in 0..end + 400 {
            let pad = match k {
                40 => [START; 2],
                60 => [DOWN; 2],
                80 => [CROSS; 2],
                k if (170..end).contains(&k) && k % 10 == 0 => [keys[(k - 170) / 10], 0],
                k if k >= end && k < end + 200 && k % 40 == 0 => [CROSS; 2],
                _ => [0; 2],
            };
            let mut out = Out::default();
            step(&mut s, &a, FrameInput { pad, ..Default::default() }, &[], &mut out);
            if let Some(m) = out.requests.iter().find_map(|r| if let Request::Load(m) = r { Some(*m) } else { None }) {
                spec = Some((m.chr, m.stage));
                break;
            }
            let d = s.front.as_ref().expect("front end").describe();
            if let Some(t) = d.strip_prefix("select ") {
                seen.push(t.split(" [").next().unwrap_or("").to_string());
            }
        }
        seen.dedup();
        (seen, spec)
    };
    let base = SessionStart { seed: 1, entry: Entry::Title, ..Default::default() };
    let with = SessionStart { br1: Some(7), ..base.clone() };
    // P1 on character `p1`, P2 on 2, the stage
    let sel = |p1: u8, stage: &str| format!("P1 {} P2 {} stage {stage}", roster::name(p1), roster::name(2));
    // characters 0 → 10 → 11 → (left, wrapping) 14; stage 25 before RANDOM
    let (seen, spec) = walk(with.clone(), &[DOWN, DOWN, LEFT, L1]);
    assert_eq!(seen, [sel(0, random()), sel(10, random()), sel(11, random()), sel(14, random()), sel(14, st(25))]);
    assert_eq!(spec, Some(([14, 2], 25)));
    // eleven imported stages after EXTRA4: L1 × 12 from RANDOM
    let (seen, spec) = walk(with.clone(), &[L1; 12]);
    let stages: Vec<&str> = seen.iter().map(|s| s.split(" stage ").nth(1).unwrap_or("")).collect();
    let want: Vec<&str> = std::iter::once(random()).chain((15..=25).rev().map(st)).chain(["EXTRA4"]).collect();
    assert_eq!(stages, want);
    assert_eq!(spec, Some(([0, 2], 12)));
    // with the extra stages: their rows come before the imported game's and are passed over with an imported cursor
    let both = SessionStart { extra_stage: true, extra_q3dm6: true, ..with.clone() };
    let tail = |seen: &[String]| seen.iter().rev().take(2).map(|s| s.split(" stage ").nth(1).unwrap_or("").to_string()).collect::<Vec<_>>();
    assert_eq!(tail(&walk(both.clone(), &[L1; 12]).0), ["Q3DM6", st(15)]);
    assert_eq!(tail(&walk(both.clone(), &[&[DOWN, DOWN][..], &[L1; 12]].concat()).0), ["EXTRA4", st(15)]);
    // an extra stage chosen before an imported cursor turns RANDOM
    let (seen, spec) = walk(both.clone(), &[&[L1; 13][..], &[DOWN, DOWN]].concat());
    assert_eq!(seen.iter().rev().nth(2).map(String::as_str), Some(sel(0, "Q3DM17").as_str()));
    assert_eq!(seen.last().map(String::as_str), Some(sel(11, random()).as_str()));
    assert!(spec.is_some_and(|(c, s)| c == [11, 2] && s < 13));
    // without the data, before v19, locked: no imported row and no imported stages
    for start in [base.clone(), SessionStart { sim_version: 18, ..with.clone() }, SessionStart { locked: true, ..with.clone() }] {
        let (seen, spec) = walk(start.clone(), &[DOWN, DOWN, L1]);
        let last = seen.last().cloned().unwrap_or_default();
        let stage = if start.locked { st(8) } else { "EXTRA4" };
        assert!(last.ends_with(&format!("stage {stage}")) && !last.contains(roster::name(11)), "{last}");
        assert!(spec.is_some_and(|(c, s)| c.iter().all(|&c| c < 11) && s < 13));
    }
}

/// A decoded state with a select cursor or a last choice on the imported row (the import S9) loads only where
/// the select has that row (VS with `SessionStart::br1`, v19): an ARCADE or STORY select's (the NORMAL and STORY
/// selects' 11-cell tables, the ladder's MatchSpec) gives Err, as one without the imported game's data. From v20 (S10)
/// ARCADE's last choices and fights may hold imported fighters (its QUICK select's row, the ladder's
/// imported boss and opponents), its NORMAL select's cursors still not; v19 as before.
#[test]
fn br1_cursor_only_on_br1_select() {
    let Some(assets) = export() else {
        eprintln!("skipped: no okb_export");
        return;
    };
    let front = Arc::new(pollster::block_on(crate::frontend::load(&assets)).expect("front data"));
    let store = AssetStore { front: Some(front), ..Default::default() };
    let a = store.assets(Default::default());
    use crate::fight::input::pad::{CROSS, DOWN, START};
    // title → START → (`downs` × DOWN: 1 VS, 2 STORY) → × → (STORY: its movie) → the select
    let select_v = |br1: Option<u32>, downs: u32, sim_version: u32| {
        let start = SessionStart { seed: 1, entry: Entry::Title, br1, sim_version, ..Default::default() };
        let mut s = new_state(&start, &a, &mut Out::default());
        let on_select = |s: &GameState| s.front.as_ref().is_some_and(|f| f.describe().starts_with("select"));
        for k in 0..600 {
            if k > 80 && on_select(&s) {
                break;
            }
            let pad = match k {
                40 => START,
                60 | 70 if (k - 60) / 10 < downs => DOWN,
                80 => CROSS,
                _ => 0,
            };
            // STORY's movie: its files "arrive" missing, so it ends
            let movie = s.front.as_ref().is_some_and(|f| f.describe().starts_with("movie"));
            let flags = if movie { FrameInput::FETCHED } else { 0 };
            step(&mut s, &a, FrameInput { pad: [pad; 2], flags, ..Default::default() }, &[], &mut Out::default());
        }
        assert!(on_select(&s), "{downs}: {}", s.front.as_ref().map(|f| f.describe()).unwrap_or_default());
        s
    };
    let select = |br1: Option<u32>, downs: u32| select_v(br1, downs, crate::session::SIM_VERSION);
    let loads = |mut s: GameState, cur: [u8; 2], last: [u8; 2]| {
        s.front.as_mut().expect("front end").set_select_cursors(cur, last);
        load_state(&save_state(&s), &a).is_ok()
    };
    let vs = select(Some(7), 1);
    assert!(loads(vs.clone(), [14, 11], [11, 14]));
    assert!(!loads(vs.clone(), [15, 0], [0, 2]));
    let arcade = select(Some(7), 0);
    assert!(loads(arcade.clone(), [10, 0], [10, 2]));
    // v20: the NORMAL select has the imported row too (its third row)
    assert!(loads(arcade.clone(), [11, 0], [0, 2]));
    // v20: the last choice may be an imported fighter (the QUICK select's row, a ladder's); v19 not
    assert!(loads(arcade.clone(), [0, 2], [11, 2]));
    let arcade19 = select_v(Some(7), 0, 19);
    assert!(!loads(arcade19.clone(), [0, 2], [11, 2]));
    assert!(!loads(arcade19.clone(), [11, 0], [0, 2]));
    let story = select(Some(7), 2);
    assert!(loads(story.clone(), [10, 0], [0, 2]));
    assert!(!loads(story.clone(), [11, 0], [0, 2]));
    assert!(!loads(story.clone(), [0, 11], [0, 2]));
    let plain = select(None, 1);
    assert!(!loads(plain.clone(), [11, 0], [0, 2]));
    assert!(!loads(plain.clone(), [0, 2], [0, 14]));
    // a fight's spec (in progress or its load pending): an imported side only in VS, an imported stage in any
    // mode with the imported game's data
    let fight = |mut s: GameState, chr: [u8; 2], stage: u8, pending: bool| {
        let spec = crate::frontend::MatchSpec { chr, costume: [0, 0], stage, cpu: [false, false] };
        s.front.as_mut().expect("front end").set_fight(spec, pending);
        load_state(&save_state(&s), &a).is_ok()
    };
    let br1_stage = crate::fight::stage::br1::STAGES[0].stage;
    for pending in [false, true] {
        assert!(fight(vs.clone(), [11, 3], 0, pending));
        assert!(fight(arcade.clone(), [11, 3], 0, pending));
        assert!(fight(arcade.clone(), [3, 14], 0, pending));
        assert!(!fight(arcade19.clone(), [11, 3], 0, pending));
        assert!(!fight(arcade19.clone(), [3, 14], 0, pending));
        assert!(!fight(story.clone(), [3, 14], 0, pending));
        assert!(fight(arcade.clone(), [3, 0], br1_stage, pending));
        assert!(!fight(plain.clone(), [3, 0], br1_stage, pending));
        assert!(!fight(plain.clone(), [11, 3], 0, pending));
    }
}

/// A fight on the extra stage Q3DM6 (skipped without its files in the export, `assets::extras`): the ring is the
/// courtyard's walled box (x ±2240, z ±1680, invisible indestructible walls on all four sides, as ST4's). YUG backs
/// towards the west wall and stays there; the CPU (ALI) attacks him into it: the wall stops his body (its contact
/// bits, the wall's push power; his hips at most 45 short of it), nobody is rung out and nobody's hips leave the
/// courtyard.
#[test]
fn extra_stage_q3dm6_walls() {
    let Some(assets) = export().filter(|a| crate::assets::extras::present(a) & 2 != 0) else {
        eprintln!("skipped: no okb_export with Q3DM6's files");
        return;
    };
    let e = crate::fight::stage::extra::Q3DM6;
    let half = e.ring_half(crate::fight::version::Gates::of(SIM_VERSION));
    let data = pollster::block_on(crate::fight::fight_data::load_match(&assets, ["YUG", "ALI"], [0, 1], e.code, false, SIM_VERSION)).expect("load");
    assert_eq!((data.stage_no, data.stage_id, data.arena.is_some(), data.stage_name.as_str()), (e.stand_in, e.stage as usize, true, e.name));
    let start = SessionStart { seed: 1, entry: Entry::DirectFight { p1: "YUG".into(), p2: "ALI".into(), stage: e.code.into() }, ..Default::default() };
    let store = AssetStore { fight: Some(Arc::new(data)), ..Default::default() };
    let a = store.assets(Default::default());
    let mut s = new_state(&start, &a, &mut Out::default());
    let script = crate::fight::input_script::parse_script("300:,150:L,2550:");
    let (mut far, mut west, mut touch, mut pushed, mut outs, mut hits) = ([0i32; 2], 0, 0, 0, 0, 0);
    for k in 0..3000 {
        step(&mut s, &a, FrameInput { pad: [script.get(k).copied().unwrap_or(0), 0], ..Default::default() }, &[], &mut Out::default());
        let m = &s.fight.as_ref().expect("the fight").m;
        let st = &m.world.stage;
        assert_eq!((st.id, st.ring.kind, st.ring.hx, st.ring.hz, st.ring.drop, st.outer.is_none()), (e.stage as usize, 0, half[0], half[1], 1, true));
        assert_eq!((st.walls.hp, st.walls.model, m.fx.arena.half), ([-1; 4], [-1; 4], (half[0] as i32, half[1] as i32)));
        assert_ne!(m.round.message, crate::fight::round::Message::OutOfRing, "frame {k}");
        for f in &m.fighters {
            far = [far[0].max(f.hips[0].abs()), far[1].max(f.hips[2].abs())];
            outs += (f.o_00 == 0x81) as u32;
        }
        let f = &m.fighters[0];
        west = west.min(f.hips[0]);
        touch += (f.flags[crate::fight::fighter::F6C] & 2 != 0) as u32;
        pushed += (st.walls.pow[1] > 0) as u32;
        hits += (m.fighters[0].o_ac > 0) as u32;
    }
    assert_eq!((far, west), ([2195, 1615], -2195));
    assert!(touch > 0 && pushed > 0 && hits > 0 && outs == 0, "touch {touch} pushed {pushed} hits {hits} outs {outs}");
}

/// A fight on the extra stage Q3DM17 (skipped without its files in the export, `assets::extras`): the ring is the
/// engine's open box (no walls), YUG backs up to the platform's edge and the CPU rings him out: OUT OF RING, his hips
/// past the edge, and he falls to the deck below (`stage::extra::ring_drop`).
#[test]
fn extra_stage_ring_out() {
    let Some(assets) = export().filter(|a| crate::assets::extras::present(a) & 1 != 0) else {
        eprintln!("skipped: no okb_export with the extra stage's files");
        return;
    };
    let e = crate::fight::stage::extra::Q3DM17;
    let (half, depth, stage_n, stand_in) =
        (e.ring_half(crate::fight::version::Gates::of(SIM_VERSION)), e.ring_drop(crate::fight::version::Gates::of(SIM_VERSION)), e.stage, e.stand_in);
    let data = pollster::block_on(crate::fight::fight_data::load_match(&assets, ["YUG", "ALI"], [0, 1], "Q3DM17", false, SIM_VERSION)).expect("load");
    assert_eq!((data.stage_no, data.stage_id, data.arena.is_some()), (stand_in, stage_n as usize, true));
    let start = SessionStart { seed: 1, entry: Entry::DirectFight { p1: "YUG".into(), p2: "ALI".into(), stage: "Q3DM17".into() }, ..Default::default() };
    let store = AssetStore { fight: Some(Arc::new(data)), ..Default::default() };
    let a = store.assets(Default::default());
    let mut s = new_state(&start, &a, &mut Out::default());
    let script = crate::fight::input_script::parse_script("300:,95:L,2700:");
    let (mut out_at, mut hips_out, mut lowest) = (None, 0, 0);
    for k in 0..3000 {
        step(&mut s, &a, FrameInput { pad: [script.get(k).copied().unwrap_or(0), 0], ..Default::default() }, &[], &mut Out::default());
        let m = &s.fight.as_ref().expect("the fight").m;
        if k == 0 {
            let st = &m.world.stage;
            assert_eq!((st.id, st.ring.kind, st.ring.hx, st.ring.hz, st.ring.drop, st.outer.is_none()), (stage_n as usize, 0, half[0], half[1], depth, true));
            assert_eq!((st.walls.hp, st.walls.model), ([0; 4], [-1; 4]));
        }
        let f = &m.fighters[0];
        if out_at.is_none() && m.round.message == crate::fight::round::Message::OutOfRing {
            out_at = Some(k);
            hips_out = f.parts.get(4).map_or(0, |p| p[0]);
        }
        if out_at.is_some() && m.round.index == 0 {
            lowest = lowest.max(f.m_t[1]);
        }
    }
    let at = out_at.expect("a ring-out");
    assert!(hips_out < -(half[0] as i32), "hips x {hips_out} at the ring-out (frame {at})");
    assert_eq!(lowest, depth as i32, "YUG's lowest point after the ring-out");
}

/// A temporary copy of the export without the extra stages' files (`extras`, `q3`; its other folders linked, from the
/// canonical root so a relative OKB_EXPORT or a linked root still resolves from the temp folder), removed when
/// dropped, also when the test panics.
#[cfg(unix)]
pub(crate) struct BareExport(pub std::path::PathBuf);

/// Debug OKB_Q3 names a Quake III install outside the export (`Assets::q3`): an export without the extra stages'
/// files cannot be made, so the tests that need one are skipped (said so).
#[cfg(unix)]
pub(crate) fn q3_elsewhere() -> bool {
    let set = crate::base::env::var_os("Q3").is_some_and(|v| !v.is_empty());
    if set {
        eprintln!("skipped: OKB_Q3 names a Quake III install outside the export");
    }
    set
}

#[cfg(unix)]
impl BareExport {
    pub(crate) fn new(assets: &crate::assets::Assets, name: &str) -> Self {
        let root = std::fs::canonicalize(assets.root()).expect("the export's root");
        let bare = Self(std::env::temp_dir().join(format!("{name}_{}", std::process::id())));
        let _ = std::fs::remove_dir_all(&bare.0);
        std::fs::create_dir_all(&bare.0).unwrap();
        for e in std::fs::read_dir(&root).unwrap().flatten().filter(|e| e.file_name() != "extras" && e.file_name() != crate::assets::extras::Q3_DIR) {
            std::os::unix::fs::symlink(e.path(), bare.0.join(e.file_name())).unwrap();
        }
        bare
    }
}

#[cfg(unix)]
impl Drop for BareExport {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// The extra stages' maps are drawing only: an export without their files (the export's folders linked, `extras` and
/// `q3` left out) loads each stage's fight all the same (no map) and plays it to the same states as with the files
/// (each stage whose files the export has).
#[cfg(unix)]
#[test]
fn extra_stage_without_files_same_fight() {
    let Some(assets) = export().filter(|a| crate::assets::extras::present(a) != 0) else {
        eprintln!("skipped: no okb_export with an extra stage's files");
        return;
    };
    if q3_elsewhere() {
        return;
    }
    let bare = BareExport::new(&assets, "br2_no_extras");
    let without = crate::assets::Assets::new(&bare.0);
    assert_eq!(crate::assets::extras::present(&without), 0);
    let has = crate::assets::extras::present(&assets);
    for e in crate::fight::stage::extra::STAGES.iter().filter(|e| has & 1 << e.index() != 0) {
        let load = |a: &crate::assets::Assets| pollster::block_on(crate::fight::fight_data::load_match(a, ["YUG", "ALI"], [0, 1], e.code, false, SIM_VERSION));
        let (with, bare_data) = (load(&assets).expect("load with the files"), load(&without).expect("load without the files"));
        assert_eq!((bare_data.stage_no, bare_data.stage_id, bare_data.arena.is_some(), with.arena.is_some()), (e.stand_in, e.stage as usize, false, true));
        let start = SessionStart { seed: 1, entry: Entry::DirectFight { p1: "YUG".into(), p2: "ALI".into(), stage: e.code.into() }, ..Default::default() };
        let script = crate::fight::input_script::parse_script("120:,40:L,30:RJ,60:K,200:");
        let hashes = |data: crate::fight::fight_data::Loaded| {
            let store = AssetStore { fight: Some(Arc::new(data)), ..Default::default() };
            let a = store.assets(Default::default());
            let mut s = new_state(&start, &a, &mut Out::default());
            (0..600)
                .map(|k| {
                    step(&mut s, &a, FrameInput { pad: [script.get(k).copied().unwrap_or(0), 0], ..Default::default() }, &[], &mut Out::default());
                    hash::hash(&s)
                })
                .collect::<Vec<_>>()
        };
        assert_eq!(hashes(with), hashes(bare_data), "{}", e.code);
    }
}

/// A start written before `locked` existed (a v4 replay's header, a v4 netplay blob) reads it as false and keeps
/// its `unlock`; a locked start round-trips.
#[test]
fn start_without_locked_reads_false() {
    let start = SessionStart { unlock: true, locked: true, ..Default::default() };
    let mut v: ciborium::Value = ciborium::Value::serialized(&start).unwrap();
    let back: SessionStart = v.deserialized().unwrap();
    assert!(back.locked && back.unlock);
    if let ciborium::Value::Map(m) = &mut v {
        m.retain(|(k, _)| k.as_text() != Some("locked"));
    }
    let old: SessionStart = v.deserialized().unwrap();
    assert!(!old.locked && old.unlock);
}

/// The rollback test (N = 8) over what the baseline routes do not reach: every debug cheat in a direct fight
/// (P2 the CPU, then human, a KO), and the front end with F11's unlock from the title into the character select.
#[test]
fn rollback_with_cheats() {
    use rollback::{Held, Rollback};
    let Some(assets) = export() else {
        eprintln!("skipped: no okb_export");
        return;
    };
    let resim = |s: &mut GameState, a: &Assets, (i, c): &(FrameInput, Vec<Cheat>)| step(s, a, *i, c, &mut Out::default());
    let run = |store: &AssetStore, start: &SessionStart, n: usize, input: &dyn Fn(usize) -> (u16, Vec<Cheat>)| {
        let mut rb = Rollback::new(8, Box::new(std::io::stderr()), std::env::temp_dir());
        let mut s = new_state(start, &store.assets(Default::default()), &mut Out::default());
        rb.start(&s);
        for k in 0..n {
            let (pad, cheats) = input(k);
            let i = FrameInput { pad: [pad, 0], ..Default::default() };
            let held = Held::of(store, Default::default());
            step(&mut s, &held.assets(), i, &cheats, &mut Out::default());
            rb.check(&s, held, (i, cheats), resim);
        }
        (rb.finish(), s)
    };
    let data = Arc::new(pollster::block_on(crate::session::start::load(&assets, "LON", "BAK", "ST5")).expect("the fight"));
    let start = SessionStart { seed: 1, entry: Entry::DirectFight { p1: "LON".into(), p2: "BAK".into(), stage: "ST5".into() }, ..Default::default() };
    let script = crate::fight::input_script::parse_script("340:,20:R,1:J,12:,1:K,12:,1:DK,20:,1:R,1:RJK,30:,1:C,90:,1:J,12:,1:K,200:");
    let cheats = |k: usize| match k {
        350 => vec![Cheat::Gauges],
        360 => vec![Cheat::Beast(0), Cheat::Beast(1)],
        420 => vec![Cheat::NextMove],
        421 => vec![Cheat::NextMove, Cheat::PrevMove],
        450 => vec![Cheat::ForceMove(3)],
        500 => vec![Cheat::Break(0)],
        520 => vec![Cheat::Timer],
        600 => vec![Cheat::Cpu],
        700 => vec![Cheat::Cpu, Cheat::OneHp(0)],
        760 => vec![Cheat::OneHp(1), Cheat::P2Dmg(199)],
        _ => Vec::new(),
    };
    let store = AssetStore { fight: Some(data), ..Default::default() };
    let (bad, s) = run(&store, &start, 1200, &|k| (script.get(k).copied().unwrap_or(0), cheats(k)));
    assert_eq!(bad, 0, "rollback mismatches in the fight");
    assert!(s.fight.is_some_and(|g| g.m.frame == 1200));

    let front = Arc::new(pollster::block_on(crate::frontend::load(&assets)).expect("front data"));
    let store = AssetStore { front: Some(front), ..Default::default() };
    let start = SessionStart { seed: 1, entry: Entry::Title, ..Default::default() };
    let input = |k: usize| match k {
        20 => (0, vec![Cheat::Unlock]),
        40 => (crate::fight::input::pad::START, Vec::new()),
        70 => (crate::fight::input::pad::CROSS, Vec::new()),
        k if k > 100 && k % 15 == 0 => (crate::fight::input::pad::RIGHT, Vec::new()),
        _ => (0, Vec::new()),
    };
    let (bad, s) = run(&store, &start, 400, &input);
    assert_eq!(bad, 0, "rollback mismatches in the front end");
    assert!(s.front.is_some_and(|f| f.describe().starts_with("select")));
}

/// Save → load (with `rebind`) → save gives the same bytes, and the loaded state runs on exactly as the
/// original (same hashes every step), in a direct fight with a KO and its replay, the front end idling through
/// the attract demo into the ranking with the demo's stage behind it, and the intro's logos and opening movie.
#[test]
fn save_load_round_trip() {
    let Some(assets) = export() else {
        eprintln!("skipped: no okb_export");
        return;
    };
    // a direct fight: P1 scripted, P2 the CPU, a KO at fight frame 345 (its replay), round 2
    let script = crate::fight::input_script::parse_script("340:,20:R,1:J,12:,1:K,12:,1:DK,20:,1:R,1:RJK,30:,1:C,90:,1:J,12:,1:K,200:");
    let data = pollster::block_on(crate::session::start::load(&assets, "YUG", "ALI", "ST0")).expect("the fight");
    let start = SessionStart { seed: 1, entry: Entry::DirectFight { p1: "YUG".into(), p2: "ALI".into(), stage: "ST0".into() }, ..Default::default() };
    let input = |k: usize| (script.get(k).copied().unwrap_or(0), if k == 344 { vec![Cheat::P2Dmg(199)] } else { Vec::new() });
    let r =
        round_trips(&assets, AssetStore { fight: Some(Arc::new(data)), ..Default::default() }, &start, 1400, input, &[0, 200, 344, 400, 520, 700, 1000], 120);
    assert_eq!(r.len(), 7);
    assert!(r.iter().any(|c| c.5.contains("+music")), "no stage music: {r:?}");

    // the front end from the title: the attract demo (from ~390), then the ranking over the demo's stage
    let front = Arc::new(pollster::block_on(crate::frontend::load(&assets)).expect("front data"));
    let start = SessionStart { seed: 1, entry: Entry::Title, ..Default::default() };
    let store = || AssetStore { front: Some(front.clone()), ..Default::default() };
    let r = round_trips(&assets, store(), &start, 2700, |_| (0, Vec::new()), &[0, 100, 395, 420, 1000, 2000, 2300, 2500], 90);
    assert!(r.iter().any(|c| c.5.contains("+fight")), "the attract demo did not start");
    assert!(r.iter().any(|c| c.5.contains("attract ranking") && c.5.contains("+backdrop")), "no ranking over the backdrop");

    // the character select with its music (title, START, ARCADE): loaded mid-track
    let pads = |k: usize| match k {
        40 => crate::fight::input::pad::START,
        70 => crate::fight::input::pad::CROSS,
        _ => 0,
    };
    let r = round_trips(&assets, store(), &start, 400, |k| (pads(k), Vec::new()), &[100, 200, 300], 60);
    assert!(r.iter().all(|c| c.5.starts_with("select") && c.5.contains("+music 0x1f")), "{r:?}");

    // the boot: the logos, then the opening movie, loaded mid-movie (its frame counter; the stream is fetched)
    let start = SessionStart { seed: 1, entry: Entry::Intro, ..Default::default() };
    let r = round_trips(&assets, store(), &start, 760, |_| (0, Vec::new()), &[300, 600], 60);
    assert!(r.iter().any(|c| c.5.starts_with("movie") && !c.5.contains("frame None") && c.5.contains("+files")), "the movie did not play: {r:?}");
}

/// Q3DM17's teleporter (simulation v14, `fight::physics::solids`; the fight needs no extras files): YUG, put on the
/// game's z axis facing it with ALI behind it by the platform's far edge, walks forward into it. At v14 his body
/// spheres stop at the near face of the pad's step (game z 259) and then slide round the teleporter, never more than
/// 2 units inside it (the disc's push: `hypot` and a truncating division); at v13 he walks into it,
/// through the step and the pad up to ALI.
#[test]
fn q3dm17_teleporter_blocks_from_v14() {
    let Some(assets) = export() else {
        eprintln!("skipped: no okb_export");
        return;
    };
    let e = crate::fight::stage::extra::Q3DM17;
    let solids: Vec<_> = e.solids(crate::fight::version::Gates::of(14)).collect();
    let walk = |v: u32| {
        let data = pollster::block_on(crate::fight::fight_data::load_match(&assets, ["YUG", "ALI"], [0, 1], e.code, false, v)).expect("load");
        let bodies = data.fighters[0].ani.spheres(0).to_vec();
        let start = SessionStart {
            seed: 1,
            sim_version: v,
            entry: Entry::DirectFight { p1: "YUG".into(), p2: "ALI".into(), stage: e.code.into() },
            ..Default::default()
        };
        let store = AssetStore { fight: Some(Arc::new(data)), ..Default::default() };
        let a = store.assets(Default::default());
        let mut s = new_state(&start, &a, &mut Out::default());
        // the front of the body spheres in front of the step (within its x ±77), the deepest overlap of a body sphere
        // and a solid part
        let (mut front, mut inside) = (i32::MIN, 0);
        for k in 0..400 {
            if k == 100 {
                // YUG at z −300 facing +z (yaw 2048), ALI at z 1060 facing −z
                let m = &mut s.fight.as_mut().expect("the fight").m;
                for (f, z, yaw) in [(0, -300, 2048), (1, 1060, 0)] {
                    let f = &mut m.fighters[f];
                    (f.pos, f.yaw_base, f.yaw) = ([0, 0, z], yaw, yaw);
                }
            }
            let pad = if k >= 150 { crate::fight::input::pad::RIGHT } else { 0 };
            step(&mut s, &a, FrameInput { pad: [pad, 0], ..Default::default() }, &[], &mut Out::default());
            let f = &s.fight.as_ref().expect("the fight").m.fighters[0];
            if k <= 110 {
                continue;
            }
            for b in bodies.iter().skip(1) {
                let (Some(n), r) = (f.parts.get(b.node as usize), b.radius as i32) else { continue };
                if n[0].abs() + r < 77 {
                    front = front.max(n[2] + r);
                }
                for p in &solids {
                    let (dx, dz) = (n[0] - p.centre[0], n[2] - p.centre[1]);
                    let depth =
                        if p.disc { p.half[0] + r - ((dx as f64).hypot(dz as f64) as i32) } else { (p.half[0] + r - dx.abs()).min(p.half[1] + r - dz.abs()) };
                    inside = inside.max(depth);
                }
            }
        }
        (front, inside)
    };
    let (front, inside) = walk(14);
    assert!(front == 259 && inside <= 2, "v14: the body's front at z {front}, {inside} inside");
    let (front, inside) = walk(13);
    assert!(front > 600 && inside > 300, "v13: the body's front at z {front}, {inside} inside");
}

/// ARCADE with the imported game's data (simulation v20, the import S10), every fight won (`end_match`, as `fetest`
/// WINS): a main-game fighter (the NORMAL select) or an imported fighter (its third row) meets "URIKO (BR1)" on her stage after the main game's last
/// fight, then its ending as before (the epilogue's CON file); MITSUKO (the QUICK select's imported row) meets
/// GREG and FOX in fights 3 and 6, the same boss, then her ending movie from the imported game and STAFF ROLL B; URIKO (BR1)
/// meets the boss as a mirror, then her gallery picture (here missing: none) and STAFF ROLL A. At v19, or without the
/// data: no boss.
#[test]
fn br1_arcade_boss_and_endings() {
    let Some(assets) = export() else {
        eprintln!("skipped: no okb_export");
        return;
    };
    let front = Arc::new(pollster::block_on(crate::frontend::load(&assets)).expect("front data"));
    let store = AssetStore { front: Some(front), ..Default::default() };
    let a = store.assets(Default::default());
    use crate::fight::input::pad::{CROSS, DOWN, RIGHT, START};
    // title → START → × (ARCADE) → the select; P1's `keys` every 20 frames, then ×; every fight won; the matches asked
    // for and the files fetched until NAME ENTRY (or 20000 frames)
    let run = |start: SessionStart, keys: &[u16]| {
        let mut s = new_state(&start, &a, &mut Out::default());
        let (mut specs, mut fetched) = (Vec::new(), Vec::new());
        let mut flags = 0;
        let mut on_select = None;
        for k in 0..20000 {
            let d = s.front.as_ref().map(|f| f.describe()).unwrap_or_default();
            if d.starts_with("name entry") {
                break;
            }
            if d.starts_with("select") && on_select.is_none() {
                on_select = Some(k);
            }
            let pad = match (k, on_select) {
                (40, _) => START,
                (80, _) => CROSS,
                (_, Some(t)) if k > t + 40 && (k - t - 40) % 20 == 0 && specs.is_empty() => {
                    keys.get((k - t - 40) / 20).copied().unwrap_or(if (k - t) % 40 == 0 { CROSS } else { 0 })
                }
                _ => 0,
            };
            let mut out = Out::default();
            step(&mut s, &a, FrameInput { pad: [pad, 0], flags, ..Default::default() }, &[], &mut out);
            flags = 0;
            for r in out.requests {
                match r {
                    Request::Load(m) => {
                        specs.push((m.chr, m.stage));
                        flags |= FrameInput::LOADED;
                    }
                    Request::Fight => {
                        let mut over = Out::default();
                        end_match(&mut s, &a, crate::fight::sim::Outcome::Winner(0), [0, 0], false, &mut over);
                        for r in over.requests {
                            if let Request::Load(m) = r {
                                specs.push((m.chr, m.stage));
                                flags |= FrameInput::LOADED;
                            }
                        }
                    }
                    // the files "arrive" missing: a movie ends at once, the epilogue waits
                    Request::Fetch(names) => {
                        fetched.extend(names);
                        flags |= FrameInput::FETCHED;
                    }
                    _ => {}
                }
            }
        }
        (specs, fetched)
    };
    let uriko = (crate::assets::roster::BR1_FIRST + 3, crate::fight::stage::br1::STAGES[8].stage);
    let base = SessionStart { seed: 1, entry: Entry::Title, ..Default::default() };
    let with = SessionStart { br1: Some(7), ..base.clone() };
    // a main-game fighter (YUGO, the NORMAL select): 9 fights, the boss, the epilogue
    let (specs, fetched) = run(with.clone(), &[]);
    assert_eq!(specs.len(), 10, "{specs:?}");
    assert!(specs[..9].iter().all(|(c, s)| c[0] == 0 && c[1] < 11 && *s < 13), "{specs:?}");
    assert_eq!(specs[9], ([0, uriko.0], uriko.1));
    assert_eq!(fetched.first().map(String::as_str), Some("engine/con/YUGCON.DAT"));
    // without the data, or at v19: no boss
    for start in [base.clone(), SessionStart { sim_version: 19, ..with.clone() }] {
        let (specs, fetched) = run(start, &[]);
        assert_eq!(specs.len(), 9, "{specs:?}");
        assert_eq!(fetched.first().map(String::as_str), Some("engine/con/YUGCON.DAT"));
    }
    // the NORMAL select's third row: DOWN × 3 is MITSUKO (the first presses fall in its intro)
    let (specs, _) = run(with.clone(), &[0, 0, DOWN, DOWN, DOWN]);
    assert_eq!(specs.first().map(|s| s.0[0]), Some(crate::assets::roster::BR1_FIRST), "{specs:?}");
    // the QUICK select (CHARACTER SELECT), its imported row: DOWN × 3 is MITSUKO, RIGHT × 3 more URIKO (BR1)
    let mut card = crate::frontend::options::Card::new(None, None);
    assert!(crate::frontend::options::Settings { char_select: 1, ..Default::default() }.save(&mut card));
    let quick = SessionStart { card: card.file().map(|b| b.to_vec()), ..with.clone() };
    let (specs, fetched) = run(quick.clone(), &[DOWN, DOWN, DOWN]);
    assert_eq!(specs.len(), 10, "{specs:?}");
    let first = crate::assets::roster::BR1_FIRST;
    assert!(specs.iter().all(|(c, _)| c[0] == first), "{specs:?}");
    let mut br1_opps = [specs[2].0[1], specs[5].0[1]];
    br1_opps.sort();
    assert_eq!(br1_opps, [first + 1, first + 2], "{specs:?}");
    assert!(specs.iter().enumerate().all(|(k, (c, _))| [2, 5, 9].contains(&k) || c[1] < 11), "{specs:?}");
    assert_eq!(specs[9], ([first, uriko.0], uriko.1));
    let br1 = crate::assets::BR1_PREFIX;
    assert_eq!(
        fetched[..4],
        [format!("{br1}movies/MITUKO.bsf"), format!("{br1}movies/MITUKO.mp3"), format!("{br1}movies/MITUKOST.bsf"), format!("{br1}movies/MITUKOST.mp3")]
    );
    let (specs, fetched) = run(quick.clone(), &[DOWN, DOWN, DOWN, RIGHT, RIGHT, RIGHT]);
    assert_eq!(specs[9], ([uriko.0, uriko.0], uriko.1));
    // (the picture never arrives here: no epilogue, STAFF ROLL A next; `Epilogue::missing`)
    assert_eq!(fetched, [format!("{br1}pictures/ART1/ART124.TIM"), format!("{br1}movies/ALICESTF.bsf"), format!("{br1}movies/ALICESTF.mp3")]);
}

/// An imported ending's length from simulation v21 (`frontend::movie::BR1_MOVIE_END`, the import S11): an
/// ARCADE run (the QUICK select, every fight won) reaches NAME ENTRY on the same frame whether the install has the
/// staff roll (MITSUKO's STAFF ROLL B) or the epilogue's picture (URIKO (BR1)'s) or not; at v20 a missing one makes the
/// ending shorter. The files from the imported game's install beside the engine (data-gated).
#[test]
fn br1_ending_length_without_its_files() {
    let br1 = std::path::PathBuf::from(std::env::var("BR1_EXPORT").unwrap_or_else(|_| concat!(env!("CARGO_MANIFEST_DIR"), "/../okb_export/br1").into()));
    let (Some(assets), true) = (export(), br1.join("manifest.json").exists()) else {
        eprintln!("skipped: no okb_export / okb_export/br1");
        return;
    };
    let front = Arc::new(pollster::block_on(crate::frontend::load(&assets)).expect("front data"));
    use crate::fight::input::pad::{CROSS, DOWN, RIGHT, START};
    let mut card = crate::frontend::options::Card::new(None, None);
    assert!(crate::frontend::options::Settings { char_select: 1, ..Default::default() }.save(&mut card));
    // the frame of NAME ENTRY: title → START → × (ARCADE) → the QUICK select, `keys` every 20 frames, then ×; every
    // fight won; the fetched files of the imported game from the install but `without`
    let run = |sim_version: u32, keys: &[u16], without: &str| -> u32 {
        let start = SessionStart { seed: 1, entry: Entry::Title, br1: Some(7), sim_version, card: card.file().map(|b| b.to_vec()), ..Default::default() };
        let mut store = AssetStore { front: Some(front.clone()), ..Default::default() };
        let mut s = new_state(&start, &store.assets(Default::default()), &mut Out::default());
        let (mut flags, mut on_select, mut fighting) = (0, None, false);
        for k in 0..60000 {
            let d = s.front.as_ref().map(|f| f.describe()).unwrap_or_default();
            if d.starts_with("name entry") {
                return s.frame;
            }
            if d.starts_with("select") && on_select.is_none() {
                on_select = Some(k);
            }
            let pad = match (k, on_select) {
                (40, _) => START,
                (80, _) => CROSS,
                (_, Some(t)) if !fighting && k > t + 40 && (k - t - 40) % 20 == 0 => {
                    keys.get((k - t - 40) / 20).copied().unwrap_or(if (k - t) % 40 == 0 { CROSS } else { 0 })
                }
                _ => 0,
            };
            let mut out = Out::default();
            step(&mut s, &store.assets(Default::default()), FrameInput { pad: [pad, 0], flags, ..Default::default() }, &[], &mut out);
            flags = 0;
            for r in out.requests {
                match r {
                    Request::Load(_) => flags |= FrameInput::LOADED,
                    Request::Fight => {
                        fighting = true;
                        let mut over = Out::default();
                        end_match(&mut s, &store.assets(Default::default()), crate::fight::sim::Outcome::Winner(0), [0, 0], false, &mut over);
                        flags |= over.requests.iter().any(|r| matches!(r, Request::Load(_))) as u8 * FrameInput::LOADED;
                    }
                    Request::Fetch(names) => {
                        for n in names {
                            let rel = n.strip_prefix(crate::assets::BR1_PREFIX).filter(|r| !r.ends_with(".mp3") && !r.ends_with(without));
                            if let Some(b) = rel.and_then(|r| std::fs::read(br1.join(r)).ok()) {
                                store.files.insert(n, b.into());
                            }
                        }
                        flags |= FrameInput::FETCHED;
                    }
                    _ => {}
                }
            }
        }
        panic!("no name entry")
    };
    let mitsuko = [DOWN, DOWN, DOWN];
    let uriko = [DOWN, DOWN, DOWN, RIGHT, RIGHT, RIGHT];
    let v = crate::fight::version::v::BR1_FIXED_MOVIES;
    for (keys, file) in [(&mitsuko[..], "MITUKOST.bsf"), (&uriko[..], "ART124.TIM")] {
        let (with, without) = (run(v, keys, "-"), run(v, keys, file));
        assert_eq!(with, without, "v{v}: {file} missing changes the ending's length");
        let (with, without) = (run(v - 1, keys, "-"), run(v - 1, keys, file));
        assert!(without < with, "v{}: without {file} the ending is shorter ({without} / {with})", v - 1);
    }
}

/// An ARCADE run left at its imported boss (QUIT in the boss fight) or after it, back at the title: the attract
/// demo ends the run's boss mark, so the demo's states still load (the import S10, simulation v20).
#[test]
fn br1_boss_mark_ends_with_the_demo() {
    let Some(assets) = export() else {
        eprintln!("skipped: no okb_export");
        return;
    };
    let front = Arc::new(pollster::block_on(crate::frontend::load(&assets)).expect("front data"));
    let store = AssetStore { front: Some(front), ..Default::default() };
    let a = store.assets(Default::default());
    let start = SessionStart { seed: 1, entry: Entry::Title, br1: Some(7), ..Default::default() };
    for mark in [1, 2] {
        let mut s = new_state(&start, &a, &mut Out::default());
        s.front.as_mut().expect("front end").set_br1_boss(mark);
        let mut demo = false;
        for _ in 0..600 {
            step(&mut s, &a, FrameInput::default(), &[], &mut Out::default());
            if s.front.as_ref().is_some_and(|f| f.sync_key()[9] == 6) {
                demo = true;
                break;
            }
        }
        assert!(demo, "the attract demo did not start");
        let bytes = save_state(&s);
        if let Err(e) = load_state(&bytes, &a) {
            panic!("mark {mark}: the demo's state does not load: {e}");
        }
    }
}

/// The team battle through the session (simulation v22, `GameState::team`): stepped by `step`, a snapshot taken
/// mid-match loads (`load_state`, its `TeamFight` rebound to the four fighters' data) and plays on to the same state;
/// it does not load over a match's data.
#[test]
fn team_battle_snapshot_round_trip() {
    let Some(assets) = export() else {
        eprintln!("skipped: no okb_export");
        return;
    };
    let chr = ["YUG", "ALI", "BAK", "ULI"];
    let data = pollster::block_on(crate::fight::fight_data::load_team(&assets, chr, [0; 4], "ST5", SIM_VERSION)).expect("the team battle's data");
    let team = crate::fight::team::TeamFight::new(&data, crate::fight::team::TeamMode::Ffa);
    let mut s = GameState { team: Some(Box::new(team)), ..Default::default() };
    let files = FileCache::default();
    let a = Assets { front: None, fight: Some(&data), backdrop: None, files: &files, prefs: Default::default() };
    for _ in 0..900 {
        step(&mut s, &a, FrameInput::default(), &[], &mut Out::default());
    }
    let snap = save_state(&s);
    let mut r = load_state(&snap, &a).expect("loads");
    for _ in 0..600 {
        step(&mut s, &a, FrameInput::default(), &[], &mut Out::default());
        step(&mut r, &a, FrameInput::default(), &[], &mut Out::default());
    }
    assert_eq!(hash(&s), hash(&r));
    assert!(s.team.as_ref().is_some_and(|t| t.frame == 1500));
    // the pause's CONTROLLER CONFIG. (v24) loads with its rows and cursor in range, not out of them nor as another menu
    use crate::frontend::menus::{Kind, Menu};
    let config = |kind: Kind, f: fn(&mut Menu)| {
        let mut c = s.clone();
        let mut menu = Menu::new(kind, Default::default());
        f(&mut menu);
        c.team_config = Some(super::TeamConfig { menu, pads: Default::default() });
        load_state(&save_state(&c), &a).map(|_| ())
    };
    assert_eq!(config(Kind::Controller, |_| {}), Ok(()));
    assert!(config(Kind::Controller, |m| m.cc_row[0] = 8).is_err());
    assert!(config(Kind::Controller, |m| m.cursor = 0).is_err());
    assert_eq!(config(Kind::Options, |_| {}), Err("team config".into()));
    let mut c = s.clone();
    c.team_config = Some(super::TeamConfig { menu: Menu::new(Kind::Controller, Default::default()), pads: Default::default() });
    c.sim_version = crate::fight::version::v::TEAM_BR1;
    assert_eq!(load_state(&save_state(&c), &a).err().as_deref(), Some("team config"), "a v23 team battle has no config screen");
    let m = pollster::block_on(crate::fight::fight_data::load_match(&assets, ["YUG", "ALI"], [0, 0], "ST5", false, SIM_VERSION)).unwrap();
    let am = Assets { fight: Some(&m), ..a };
    assert!(load_state(&snap, &am).is_err());
}

/// TEAM ARCADE's pads by slot (simulation v26): the partner's pad (pad 2, or pad 3 joining on the loading screen) drives
/// P3 with its own button map, the starter's pad P1, the other pads nothing; a partner who joined on the loading screen
/// starts the battle as a human (the session's spec too).
#[test]
fn team_arcade_pads_by_slot() {
    use crate::fight::input::pad::{CIRCLE, CROSS, SQUARE, START, UP};
    let Some(assets) = export() else {
        eprintln!("skipped: no okb_export");
        return;
    };
    let front = Arc::new(pollster::block_on(crate::frontend::load(&assets)).expect("front data"));
    for partner in [None, Some(1)] {
        let mut store = AssetStore { front: Some(front.clone()), ..Default::default() };
        let start = SessionStart { seed: 1, entry: Entry::Title, ..Default::default() };
        let mut s = new_state(&start, &store.assets(Default::default()), &mut Out::default());
        let f = s.front.as_mut().expect("front end");
        f.save.settings.buttons[1].reverse();
        let maps = f.buttons();
        let spec = f.set_team_arcade_load(partner);
        s.team_spec = Some(spec);
        let chr = spec.chr.map(crate::assets::roster::code);
        let stage = crate::fight::fight_data::stage_code(spec.stage);
        let data = pollster::block_on(crate::fight::fight_data::load_team(&assets, chr, spec.costume, stage, SIM_VERSION)).expect("the battle's data");
        store.fight = Some(Arc::new(data));
        let a = store.assets(Default::default());
        // pad 3's START on the loading screen: the partner when there is none yet
        step(&mut s, &a, FrameInput { ext: [START, 0], ..Default::default() }, &[], &mut Out::default());
        inputs(&mut s, &a, FrameInput::LOADED, &mut Out::default());
        for _ in 0..120 {
            if s.team.is_some() {
                break;
            }
            step(&mut s, &a, FrameInput::default(), &[], &mut Out::default());
        }
        let humans = [false, true, false, true];
        let t = s.team.as_ref().expect("the battle started");
        assert_eq!((t.cpu, s.team_spec.map(|x| x.cpu)), (humans, Some(humans)), "partner {partner:?}");
        let partner_map = if partner == Some(1) { maps[1] } else { crate::assets::game_tables::default_buttons() };
        assert_eq!((t.buttons[0], t.buttons[2]), (maps[0], partner_map), "partner {partner:?}");
        step(&mut s, &a, FrameInput { pad: [CROSS, UP], ext: [SQUARE, CIRCLE], ..Default::default() }, &[], &mut Out::default());
        let t = s.team.as_ref().expect("the battle runs");
        let p3 = if partner == Some(1) { UP } else { SQUARE };
        assert_eq!(t.raw_pads, [CROSS, 0, p3, 0], "partner {partner:?}");
    }
}

/// (TEAM ARCADE, the A2 review) A pad-1 run with a CPU partner: pad 2's START held through a won battle's last frame is
/// no join on the next stage's loading screen (the front end takes the pads held at the end, `end_team`), so the
/// partner stays a CPU; pressed anew there, pad 2 joins.
#[test]
fn team_arcade_start_held_across_a_win_is_no_join() {
    use crate::fight::input::pad::START;
    let Some(assets) = export() else {
        eprintln!("skipped: no okb_export");
        return;
    };
    let front = Arc::new(pollster::block_on(crate::frontend::load(&assets)).expect("front data"));
    let mut store = AssetStore { front: Some(front), ..Default::default() };
    let start = SessionStart { seed: 1, entry: Entry::Title, ..Default::default() };
    let mut s = new_state(&start, &store.assets(Default::default()), &mut Out::default());
    let spec = s.front.as_mut().expect("front end").set_team_arcade_load(None);
    s.team_spec = Some(spec);
    let chr = spec.chr.map(crate::assets::roster::code);
    let stage = crate::fight::fight_data::stage_code(spec.stage);
    let data = pollster::block_on(crate::fight::fight_data::load_team(&assets, chr, spec.costume, stage, SIM_VERSION)).expect("the battle's data");
    store.fight = Some(Arc::new(data));
    let a = store.assets(Default::default());
    inputs(&mut s, &a, FrameInput::LOADED, &mut Out::default());
    for _ in 0..120 {
        if s.team.is_some() {
            break;
        }
        step(&mut s, &a, FrameInput::default(), &[], &mut Out::default());
    }
    assert!(s.team.as_ref().is_some_and(|t| t.arcade.is_some()), "the arcade battle started");
    let held = FrameInput { pad: [0, START], ..Default::default() };
    for _ in 0..3 {
        step(&mut s, &a, held, &[], &mut Out::default());
    }
    // the humans' team wins (as the KO's last frame): the next stage's loading screen
    s.team.as_mut().expect("the battle").result = Some(crate::fight::team::TeamOutcome::Winner(0));
    step(&mut s, &a, held, &[], &mut Out::default());
    assert!(s.team.is_none() && s.team_spec.is_some_and(|t| t != spec), "the next stage loads");
    let partner = |s: &GameState| s.front.as_ref().and_then(|f| f.team_slot_ports()).map(|p| p[2]);
    step(&mut s, &a, held, &[], &mut Out::default());
    assert_eq!(partner(&s), Some(None), "START held from the battle is no join");
    step(&mut s, &a, FrameInput::default(), &[], &mut Out::default());
    step(&mut s, &a, held, &[], &mut Out::default());
    assert_eq!(partner(&s), Some(Some(1)), "pressed anew it is");
}
