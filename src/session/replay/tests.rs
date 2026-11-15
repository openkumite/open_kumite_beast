//! The replay tests, including the fixtures of older simulation versions.
//!
//! The replay tests: the input stream, the file's envelope and checks against bad bytes, recording and playback,
//! the headless check and its seeks, the fixtures of older versions.

use super::headless::Headless;
use super::inputs::InputWriter;
use super::*;
use crate::session::{Assets, FileCache, GameState, Out, hash, step};

/// A recording of `n` steps of the empty session (no front end, no fight: only the frame advances) with these
/// inputs, a cheat and a preference change.
fn record(n: u32, input: impl Fn(u32) -> FrameInput) -> (Replay, GameState) {
    let files = FileCache::default();
    let a = Assets { front: None, fight: None, backdrop: None, files: &files, prefs: Default::default() };
    let start = SessionStart { seed: 7, card: Some(b"{\"card\": 1}".to_vec()), entry: Entry::Title, ..Default::default() };
    let mut rec = Recorder::new(&start, Prefs::default(), [0, 1]);
    let mut s = GameState::default();
    for k in 0..n {
        let i = input(k);
        let cheats = if k == 5 { vec![Cheat::Gauges, Cheat::OneHp(1)] } else { Vec::new() };
        let prefs = Prefs { hud_size: if k >= 30 { 2 } else { 1 }, ..Default::default() };
        step(&mut s, &Assets { prefs, ..a }, i, &cheats, &mut Out::default());
        rec.step(i, &cheats, prefs, &s).expect("recorded");
    }
    (rec.replay(&s, 1_700_000_000), s)
}

fn pads(k: u32) -> FrameInput {
    let pad = [(k / 7 % 3) as u16 * 0x1010, if k % 50 < 3 { 0xFFFF } else { 0 }];
    let flags = if k == 40 { FrameInput::LOADED | FrameInput::FORCED[0] } else { 0 };
    FrameInput { pad, forced: [if flags != 0 { 0x40 } else { 0 }, 0], flags, ..Default::default() }
}

/// Encode → decode gives the same replay; the player gives back every step's input, cheats and preferences.
#[test]
fn round_trip() {
    let (r, s) = record(1000, pads);
    let b = r.encode();
    let d = Replay::decode(&b).expect("decodes");
    assert_eq!(d.encode(), b);
    assert_eq!((d.sim_version, d.header.frames, d.header.start.seed, d.header.time), (SIM_VERSION, 1000, 7, 1_700_000_000));
    assert_eq!(d.header.start.card.as_deref(), Some(&b"{\"card\": 1}"[..]));
    assert_eq!(d.start().sim_version, SIM_VERSION);
    assert_eq!(d.body.checks.len(), 1000 / 60 + 1, "a checkpoint every 60 steps and one at the end");
    let mut p = Player::new(d);
    assert!(p.strict());
    let mut g = GameState::default();
    let files = FileCache::default();
    for k in 0..1000 {
        let (i, cheats, prefs) = p.next_step().expect("an input");
        assert_eq!(i, pads(k), "step {k}");
        assert_eq!(cheats, if k == 5 { vec![Cheat::Gauges, Cheat::OneHp(1)] } else { Vec::new() });
        assert_eq!(prefs.hud_size, if k >= 30 { 2 } else { 1 });
        let a = Assets { front: None, fight: None, backdrop: None, files: &files, prefs };
        step(&mut g, &a, i, &cheats, &mut Out::default());
        p.check(&g).expect("in sync");
    }
    assert!(p.next_step().is_none());
    assert_eq!((p.checked(), hash(&g)), (1000 / 60 + 1, hash(&s)));
    // runs of every length code, and a stream of one long run
    for n in [1, 7, 8, 9, 300, 70_000] {
        let (r, _) = record(n, |_| FrameInput { pad: [3, 0], ..Default::default() });
        let mut p = Player::new(Replay::decode(&r.encode()).expect("decodes"));
        assert_eq!((0..n).filter(|_| p.next_step().is_some_and(|x| x.0.pad == [3, 0])).count(), n as usize);
        assert!(p.next_step().is_none());
    }
    let (r, _) = record(70_000, |_| FrameInput::default());
    assert!(r.body.inputs.len() < 8, "{} bytes", r.body.inputs.len());
}

/// Pads 3 / 4 (the team battle, engine addition): a recording that used them is FORMAT 2 and gives every step's nine
/// fields back; one that did not stays FORMAT 1, its bytes those of the five-field stream; a FORMAT 2 file is refused
/// before simulation v22, and its run masks' spare bits.
#[test]
fn format_2_round_trip() {
    let ext = |k: u32| {
        let mut i = pads(k);
        i.ext = [(k / 5 % 4) as u16 * 0x0808, if k % 90 < 4 { 0x800 } else { 0 }];
        if k.is_multiple_of(33) {
            i.forced_ext = [0x20, 0x40];
            i.flags |= FrameInput::FORCED_EXT[0] | FrameInput::FORCED_EXT[1];
        }
        i
    };
    let (r, s) = record(1000, ext);
    assert_eq!(r.format, FORMAT_TEAM);
    let b = r.encode();
    assert_eq!(u16::from_le_bytes([b[4], b[5]]), FORMAT_TEAM);
    let d = Replay::decode(&b).expect("decodes");
    assert_eq!(d.encode(), b);
    let mut p = Player::new(d);
    let mut g = GameState::default();
    let files = FileCache::default();
    for k in 0..1000 {
        let (i, cheats, prefs) = p.next_step().expect("an input");
        assert_eq!(i, ext(k), "step {k}");
        assert_eq!((i.pad4(2), i.pad4(3)), if k.is_multiple_of(33) { (0x20, 0x40) } else { (i.ext[0], i.ext[1]) });
        let a = Assets { front: None, fight: None, backdrop: None, files: &files, prefs };
        step(&mut g, &a, i, &cheats, &mut Out::default());
        p.check(&g).expect("in sync");
    }
    assert_eq!(hash(&g), hash(&s));
    // no pads 3 / 4: FORMAT 1, the stream as before
    let (r1, _) = record(1000, pads);
    assert_eq!((r1.format, u16::from_le_bytes([r1.encode()[4], r1.encode()[5]])), (1, 1));
    let mut w = InputWriter::default();
    (0..1000).for_each(|k| w.push(pads(k)));
    assert!(!w.uses_ext);
    assert_eq!(r1.body.inputs, w.bytes(false));
    // and its header without the team battle's fields (nor the netplay flag when false): the bytes of the builds before
    let mut hb = Vec::new();
    ciborium::into_writer(&r1.header, &mut hb).expect("serialises");
    assert!(![&b"team"[..], b"netplay"].iter().any(|k| hb.windows(k.len()).any(|w| w == *k)));
    let net = SessionStart { netplay: true, ..r1.header.start.clone() };
    let mut nb = Vec::new();
    ciborium::into_writer(&net, &mut nb).expect("serialises");
    assert!(ciborium::from_reader::<SessionStart, _>(nb.as_slice()).is_ok_and(|s| s.netplay));
    // FORMAT 2 is refused before the team battle's simulation version
    let mut old = r.clone();
    old.sim_version = crate::fight::version::v::TEAM - 1;
    assert!(Replay::decode(&old.encode()).is_err_and(|e| e.contains("format 2")));
    // a run mask with a spare bit (12..15) set, and FORCED_EXT flags in a FORMAT 1 stream, are refused
    let mut bad = r.clone();
    bad.body.inputs[1] |= 0x10;
    assert!(Replay::decode(&bad.encode()).is_err());
    let mut w2 = InputWriter::default();
    (0..1000).for_each(|k| w2.push(ext(k)));
    let mut narrow = r.clone();
    narrow.format = 1;
    narrow.body.inputs = w2.bytes(false);
    assert!(Replay::decode(&narrow.encode()).is_err_and(|e| e.contains("input flags")));
    // a team battle's header: a TeamSpec of v22 with a winner among its teams
    let spec =
        crate::frontend::TeamSpec { mode: crate::fight::team::TeamMode::Ffa, chr: [0, 1, 2, 3], costume: [0; 4], stage: 2, cpu: [false, true, true, true] };
    let mut t = r.clone();
    (t.header.team, t.header.outcome, t.header.team_rounds) = (Some(spec), Some(Outcome::Winner(3)), [0, 1, 0, 2]);
    assert!(Replay::decode(&t.encode()).is_ok_and(|d| d.header.team == Some(spec)));
    for bad in [
        crate::frontend::TeamSpec { stage: 1, ..spec },
        crate::frontend::TeamSpec { chr: [0, 1, 2, crate::assets::roster::BR1_FIRST], ..spec },
        crate::frontend::TeamSpec { costume: [0, 0, 0, 4], ..spec },
    ] {
        t.header.team = Some(bad);
        assert!(Replay::decode(&t.encode()).is_err(), "{bad:?}");
    }
    t.header.team = Some(crate::frontend::TeamSpec { mode: crate::fight::team::TeamMode::Vs2, ..spec });
    assert!(Replay::decode(&t.encode()).is_err(), "team 3 of 2 VS 2");
}

/// A replay with an imported content id plays only over the same data (`br1_fits`); one without plays anywhere.
#[test]
fn br1_data_needed() {
    let mut start = crate::session::SessionStart::default();
    assert!(super::br1_fits(&start, None).is_ok() && super::br1_fits(&start, Some(3)).is_ok());
    start.br1 = Some(3);
    assert!(super::br1_fits(&start, Some(3)).is_ok());
    for other in [None, Some(4)] {
        assert!(super::br1_fits(&start, other).is_err_and(|e| e.contains("Bloody Roar 1")), "{other:?}");
    }
}

/// A replay records the game data's content id; playing it over other data (or data without one) gives a warning, over
/// the same data or when it has none (recorded before the id) none. A start without it writes no `content` key: the
/// header's bytes are those of the builds before.
#[test]
fn content_id_recorded() {
    let (mut r, _) = record(10, pads);
    let without = r.encode();
    assert!(Replay::decode(&without).is_ok_and(|d| d.header.start.content.is_none()));
    let mut cbor = Vec::new();
    ciborium::into_writer(&r.header.start, &mut cbor).unwrap();
    assert!(!cbor.windows(7).any(|w| w == b"content"), "no content key when None");
    assert!(content_warning(&r.header.start, Some(9)).is_none() && content_warning(&r.header.start, None).is_none());
    r.header.start.content = Some(0x5970_AF0C);
    let d = Replay::decode(&r.encode()).expect("decodes");
    assert_eq!(d.start().content, Some(0x5970_AF0C));
    assert!(content_warning(&d.start(), Some(0x5970_AF0C)).is_none());
    for other in [None, Some(1)] {
        assert!(content_warning(&d.start(), other).is_some_and(|w| w.contains("5970af0c")), "{other:?}");
    }
}

/// A replay of a newer simulation version (or file format) is refused with a message saying so; older and
/// equal versions load.
#[test]
fn newer_version_refused() {
    let (mut r, _) = record(10, pads);
    r.sim_version = SIM_VERSION + 1;
    let e = Replay::decode(&r.encode()).unwrap_err();
    assert_eq!(e, format!("recorded with a newer version (v{}), update the game", SIM_VERSION + 1));
    assert!(Replay::decode_header(&r.encode()).is_err_and(|e| e.contains("newer version")));
    r.sim_version = 0;
    assert!(Replay::decode(&r.encode()).is_err());
    r.sim_version = 1;
    assert!(Replay::decode(&r.encode()).is_ok_and(|d| d.start().sim_version == 1));
    let mut b = r.encode();
    b[4..6].copy_from_slice(&(FORMAT + 1).to_le_bytes());
    assert!(Replay::decode(&b).is_err_and(|e| e.contains("newer")));
}

/// Damaged files give Err, never a panic or a large allocation: every truncation, byte changes (the checksum
/// catches them; with the checksum fixed up the decoder and the checks must), length bombs, bad ids.
#[test]
fn corrupt_files_rejected() {
    let (r, _) = record(400, pads);
    let b = r.encode();
    assert!(Replay::decode(&[]).is_err() && Replay::decode(b"BR2R").is_err() && Replay::decode(&[0xFF; 64]).is_err());
    assert!(Replay::decode(&vec![0; MAX_BYTES + 1]).is_err());
    for k in 0..b.len() {
        assert!(Replay::decode(&b[..k]).is_err(), "truncated at {k}");
        for x in [0x01, 0x80, 0xFF] {
            let mut c = b.clone();
            c[k] ^= x;
            assert!(Replay::decode(&c).is_err(), "byte {k} ^ {x:#x}");
            // the same change with a valid checksum: decodes into a checked replay or gives Err
            let n = c.len() - 8;
            let sum = fnv1a(FNV_BASIS, &c[..n]);
            c[n..].copy_from_slice(&sum.to_le_bytes());
            if let Ok(d) = Replay::decode(&c) {
                let mut p = Player::new(d);
                while p.next_step().is_some() {}
            }
        }
    }
    // crafted parts with a valid envelope
    let with = |f: &dyn Fn(&mut Replay)| {
        let mut x = r.clone();
        f(&mut x);
        Replay::decode(&x.encode())
    };
    assert!(with(&|x| x.header.frames += 1).is_err_and(|e| e.contains("input stream")));
    assert!(with(&|x| x.body.inputs.push(0xE0)).is_err());
    assert!(with(&|x| x.body.inputs.extend([0x1F, 0xFF, 0xFF, 0xFF, 0xFF, 0x7F])).is_err());
    assert!(with(&|x| x.body.cheats.push((399, Cheat::Beast(2)))).is_err_and(|e| e.contains("cheat")));
    assert!(with(&|x| x.body.cheats.push((400, Cheat::Timer))).is_err());
    assert!(with(&|x| x.body.prefs.push((399, Prefs { window: 9, ..Default::default() }))).is_err());
    assert!(with(&|x| x.body.checks.insert(0, (0, 0, 0))).is_err());
    assert!(with(&|x| x.header.start.entry = Entry::DirectFight { p1: "../x".into(), p2: "ALI".into(), stage: "ST0".into() }).is_err());
    // the extra stages from their simulation versions on (Q3DM17 v10, Q3DM6 v12)
    for e in &crate::fight::stage::extra::STAGES {
        let q3 = Entry::DirectFight { p1: "YUG".into(), p2: "ALI".into(), stage: e.code.into() };
        for v in [e.sim - 1, e.sim] {
            let r = with(&|x| (x.sim_version, x.header.start.entry) = (v, q3.clone()));
            assert_eq!(r.is_ok(), v >= e.sim, "{} v{v}: {:?}", e.code, r.err());
        }
    }
    assert!(with(&|x| x.header.start.entry = Entry::Epilogue { intro: false, chr: 11 }).is_err());
    assert!(with(&|x| x.header.spec = Some(MatchSpec { chr: [11, 0], costume: [0; 2], stage: 0, cpu: [false; 2] })).is_err());
    // the select's imported cells (11..14) and stages (15..25) from simulation v19 with the imported game's data (the imported
    // import S9)
    let sel = |chr: [u8; 2], stage: u8, v: u32, br1: Option<u32>| {
        with(&|x| {
            (x.sim_version, x.header.start.br1) = (v, br1);
            x.header.spec = Some(MatchSpec { chr, costume: [0; 2], stage, cpu: [false; 2] });
        })
        .is_ok()
    };
    let v = crate::fight::version::v::BR1_SELECT;
    assert!(sel([11, 14], 25, v, Some(7)) && sel([0, 13], 0, v, Some(7)) && sel([0, 1], 15, v, Some(7)));
    assert!(!sel([11, 0], 0, v - 1, Some(7)) && !sel([0, 1], 15, v - 1, Some(7)) && !sel([12, 0], 0, v, None) && !sel([0, 1], 20, v, None));
    assert!(!sel([15, 0], 0, v, Some(7)) && !sel([0, 1], 26, v, Some(7)));
    assert!(with(&|x| x.header.outcome = Some(Outcome::Winner(2))).is_err());
    assert!(with(&|x| x.header.from = 401).is_err());
    assert!(with(&|x| x.header.name = Some("x".repeat(MAX_NAME + 1))).is_err());
    assert!(with(&|x| x.header.name = Some("é".repeat(MAX_NAME))).is_ok());
    // a body of a huge byte-string length, and of a million one-byte `None`s
    let bomb = |body: &[u8]| {
        let mut x = r.encode();
        let hl = u32::from_le_bytes(x[11 + r.engine.len()..][..4].try_into().unwrap()) as usize;
        x.truncate(11 + r.engine.len() + 4 + hl);
        x.extend_from_slice(&(body.len() as u32).to_le_bytes());
        x.extend_from_slice(body);
        let sum = fnv1a(FNV_BASIS, &x);
        x.extend_from_slice(&sum.to_le_bytes());
        Replay::decode(&x)
    };
    assert!(bomb(&[0xA1, 0x66, b'i', b'n', b'p', b'u', b't', b's', 0x5B, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF]).is_err());
    // half a million cheats (4 MB): decoded under the budget, then refused by the length cap
    let many = Body { cheats: vec![(0, Cheat::Timer); 500_000], ..r.body.clone() };
    assert!(bomb(&cbor(&many)).is_err_and(|e| e.contains("too many")));
    assert!(bomb(&cbor(&r.body)).is_ok());
}

/// A checkpoint that differs stops the playback at its frame (sync hash; with a file of this build the full
/// hash too), and so does a changed input.
#[test]
fn checkpoint_mismatch_detected() {
    let files = FileCache::default();
    let play = |r: Replay| {
        let mut p = Player::new(r);
        let mut g = GameState::default();
        while let Some((i, c, prefs)) = p.next_step() {
            step(&mut g, &Assets { front: None, fight: None, backdrop: None, files: &files, prefs }, i, &c, &mut Out::default());
            p.check(&g)?;
        }
        Ok::<_, String>(p.checked())
    };
    let (r, _) = record(200, pads);
    assert_eq!(play(r.clone()), Ok(4));
    let mut x = r.clone();
    x.body.checks[1].2 ^= 1;
    assert_eq!(play(x), Err("out of sync at frame 120".into()));
    let mut x = r.clone();
    x.body.checks[2].1 ^= 1;
    assert_eq!(play(x.clone()), Err("out of sync at frame 180 (the full state)".into()));
    // another build compares the sync hashes only
    x.engine = "0.0.0 (other)".into();
    assert_eq!(play(x), Ok(4));
    // one step's preferences changed: the GameState hashes them only on the GRAPHICS screen, so the empty
    // session stays in sync; a changed step count does not
    let mut x = r;
    x.body.checks[0].0 = 59;
    assert!(play(x).is_err_and(|e| e.contains("frame 59")));
}

/// A seek back to a position gives the same steps again from there.
#[test]
fn seek_repeats_steps() {
    let (r, _) = record(500, pads);
    let mut p = Player::new(r);
    for _ in 0..123 {
        p.next_step();
    }
    let at = p.pos();
    let a: Vec<_> = (0..200).filter_map(|_| p.next_step()).collect();
    p.seek(at);
    assert_eq!(p.frame(), 123);
    let b: Vec<_> = (0..200).filter_map(|_| p.next_step()).collect();
    assert_eq!(a.len(), 200);
    assert!(a.iter().zip(&b).all(|(x, y)| x.0 == y.0 && x.1 == y.1 && x.2 == y.2));
}

/// A recording stops at a step that does not follow it (a state put back).
#[test]
fn recorder_needs_following_steps() {
    let mut rec = Recorder::new(&SessionStart::default(), Prefs::default(), [0, 1]);
    let s = GameState { frame: 5, ..Default::default() };
    assert!(rec.step(FrameInput::default(), &[], Prefs::default(), &s).is_err());
    let s = GameState { frame: 1, ..Default::default() };
    assert!(rec.step(FrameInput::default(), &[], Prefs::default(), &s).is_ok());
    assert_eq!(rec.frames(), 1);
}

fn export() -> Option<crate::assets::Assets> {
    let root = crate::base::env::var("EXPORT").unwrap_or_else(|_| concat!(env!("CARGO_MANIFEST_DIR"), "/../okb_export").into());
    std::path::Path::new(&root).join("manifest.json").exists().then(|| crate::assets::Assets::new(root))
}

/// A replay recorded by the 6a build (017a617, simulation v1: DOWN ×6 from ARCADE to OPTIONS, which is REPLAY
/// in v2, OPTIONS opened and left) still plays in sync on this build; v2 sessions get the REPLAY row there.
#[test]
fn v1_menu_replay_still_plays() {
    let Some(assets) = export() else {
        eprintln!("skipped: no okb_export");
        return;
    };
    let r = Replay::decode(include_bytes!("../fixtures/menu_v1.br2r")).expect("decodes");
    assert_eq!((r.sim_version, r.engine.as_str()), (1, "0.1.0 (017a617)"));
    let mut menus = Vec::new();
    let sum = simulate(&assets, r.clone(), |s| {
        if let Some(f) = &s.front {
            let d = f.describe();
            if d.starts_with("mode menu") || d.starts_with("menu ") {
                menus.push(d.split(" [").next().unwrap_or("").to_string());
            }
        }
    })
    .expect("in sync");
    assert_eq!(sum.frames, 507);
    menus.dedup();
    let (options, replay) = (crate::assets::text::item("modes", 6), crate::assets::text::get("replay"));
    assert!(menus.iter().any(|m| *m == format!("mode menu ({options})")) && !menus.iter().any(|m| m.contains(replay)), "{menus:?}");
    assert!(menus.iter().any(|m| m.starts_with("menu Options")), "{menus:?}");
    // the same inputs in a v2 session: the sixth DOWN lands on REPLAY, × asks for the replay list
    let mut v2 = r;
    // (v2..v21: from v22 TEAM BATTLE takes the seventh row, REPLAY one down)
    v2.sim_version = crate::fight::version::v::TEAM - 1;
    let start = v2.start();
    let front = std::sync::Arc::new(pollster::block_on(crate::frontend::load(&assets)).expect("front data"));
    let store = crate::session::AssetStore { front: Some(front), ..Default::default() };
    let mut s = crate::session::new_state(&start, &store.assets(Prefs::default()), &mut Out::default());
    let mut p = Player::new(v2);
    let mut asked = None;
    while let Some((i, c, prefs)) = p.next_step() {
        let mut out = Out::default();
        step(&mut s, &store.assets(prefs), i, &c, &mut out);
        if out.requests.iter().any(|r| matches!(r, crate::session::Request::Replays)) {
            asked = Some(s.frame);
            break;
        }
    }
    let f = s.front.as_ref().expect("front end");
    let replay = format!("mode menu ({})", crate::assets::text::get("replay"));
    assert!(asked.is_some() && f.describe().starts_with(&replay), "{asked:?} {}", f.describe());
}

/// Replays recorded by 98ed1f1 (simulation v4, before every character and costume was open by default) play in
/// sync with the progression they were recorded with: `select_v4.okbr` (ARCADE, the cursor walked over the
/// grid with GADO and SHENLONG locked, the first fight), `versus_v4.okbr` (VS: both sides walk, R1 twice on the
/// game's 9-stage select, the fight), `movies_v4_unlock.okbr` (`--unlock`: MOVIES AND PICTURES cycled to
/// ENDING1, the movies the save has not unlocked). The same inputs at v5 open the roster.
#[test]
fn v4_replays_keep_their_progression() {
    let Some(assets) = export() else {
        eprintln!("skipped: no okb_export");
        return;
    };
    let at_select = |r: Replay| {
        let mut seen = None;
        let sum = simulate(&assets, r, |s| {
            if let Some(f) = s.front.as_ref().filter(|f| f.describe().starts_with("select")) {
                seen.get_or_insert(f.selectable());
            }
        });
        (sum.map(|s| s.checks), seen)
    };
    let r = Replay::decode(include_bytes!("../fixtures/select_v4.okbr")).expect("decodes");
    assert_eq!((r.sim_version, r.engine.as_str(), r.header.start.unlock), (4, "0.1.0 (98ed1f1)", false));
    let mut v5 = r.clone();
    let (checks, seen) = at_select(r);
    assert!(checks.expect("in sync") > 0);
    assert_eq!(seen, Some((0x1FF, 0)));
    // the same start at v5: all characters and fourth costumes (the recording's checks no longer apply)
    v5.sim_version = SIM_VERSION;
    v5.body.checks.clear();
    assert_eq!(at_select(v5).1, Some((0x7FF, 0x7FF)));

    let r = Replay::decode(include_bytes!("../fixtures/versus_v4.okbr")).expect("decodes");
    assert_eq!((r.sim_version, r.header.start.unlock), (4, false));
    let mut stages = Vec::new();
    let sum = simulate(&assets, r, |s| {
        if let Some(d) = s.front.as_ref().map(|f| f.describe()).filter(|d| d.starts_with("select")) {
            stages.push(d.split(" stage ").nth(1).and_then(|t| t.split(" [").next()).unwrap_or("").to_string());
        }
    })
    .expect("in sync");
    assert!(sum.checks > 0);
    stages.dedup();
    assert_eq!(stages, [crate::assets::text::item("stages", 9), crate::frontend::stage_name(0), crate::frontend::stage_name(1)]);

    let r = Replay::decode(include_bytes!("../fixtures/movies_v4_unlock.okbr")).expect("decodes");
    assert_eq!((r.sim_version, r.header.start.unlock, r.header.start.locked), (4, true, false));
    let mut menus = Vec::new();
    let sum = simulate(&assets, r, |s| {
        if let Some(f) = &s.front {
            menus.push(f.describe());
        }
    })
    .expect("in sync");
    assert!(sum.checks > 0);
    assert!(menus.iter().any(|m| m.starts_with("menu Movies") && m.ends_with("unlocked 0x7ff]")), "{:?}", menus.last());
}

/// The OPTIONS walk of `movies_v4_unlock.okbr` (UP from EXIT to MOVIES AND PICTURES, ×) at the current version:
/// from v9 OPTIONS has CHANGE GAME DATA after GRAPHICS (engine addition), so it opens on EXIT at item 16 and MOVIES AND
/// PICTURES is item 12; the walk up from EXIT still reaches it. At v4 OPTIONS has its 15 items.
#[test]
fn v9_options_gain_change_game_data() {
    let Some(assets) = export() else {
        eprintln!("skipped: no okb_export");
        return;
    };
    let run = |r: Replay| {
        let mut items = Vec::new();
        simulate(&assets, r, |s| {
            if let Some(d) = s.front.as_ref().map(|f| f.describe()).filter(|d| d.starts_with("menu ")) {
                items.push(d);
            }
        })
        .expect("runs");
        items.dedup();
        items
    };
    let r = Replay::decode(include_bytes!("../fixtures/movies_v4_unlock.okbr")).expect("decodes");
    let mut v9 = r.clone();
    (v9.sim_version, v9.body.checks) = (SIM_VERSION, Vec::new());
    let items = |v: Vec<String>| v.iter().map(|d| d.split(" [").next().unwrap_or("").to_string()).collect::<Vec<_>>();
    let v4 = items(run(r));
    let v9 = items(run(v9));
    let options = |from: u32| (11..=from).rev().map(|k| format!("menu Options item {k}"));
    let movies = ["menu Movies item 3", "menu Movies item 1"].map(String::from);
    assert_eq!(v4, options(15).chain(movies.clone()).collect::<Vec<_>>());
    // one item more, the walk up from EXIT one item shorter: the same MOVIES AND PICTURES
    assert_eq!(v9, options(16).take(5).chain(movies).collect::<Vec<_>>());
}

/// Fights recorded by c1c9639 (simulation v5, direct fights): LON v BAK on ST3 (the CPU, two Beast Drive
/// darkenings, two KOs, the KO replay, the match end and the next match's start), LON v BAK on ST5 (a Beast
/// Drive, a time over, a KO with its replay, the match end) and YUG v ALI on ST0 (a beast knocked back to human:
/// the revert burst) still play in sync.
#[test]
fn v5_fights_still_play() {
    let Some(assets) = export() else {
        eprintln!("skipped: no okb_export");
        return;
    };
    for (file, steps) in [
        (&include_bytes!("../fixtures/fight_st3_v5.okbr")[..], 3417),
        (&include_bytes!("../fixtures/fight_st5_v5.okbr")[..], 5459),
        (&include_bytes!("../fixtures/revert_v5.okbr")[..], 2193),
    ] {
        let r = Replay::decode(file).expect("decodes");
        assert_eq!((r.sim_version, r.engine.as_str()), (5, "0.1.0 (c1c9639)"));
        let sum = simulate(&assets, r, |_| {}).expect("in sync");
        assert_eq!(sum.frames, steps);
        assert!(sum.checks > 30);
    }
}

/// A v26 fight of the two the spring chains' rest frame (v27) changes most, Alice (her beast form's ears) and Shina
/// (her hair, human and beast), still plays in sync, and its chains still move as the earlier model moves them:
/// FNV-1a over every frame's `spring_aims` (the sync hash a later build compares holds no spring state). The same
/// inputs at v27 move them differently, so the pin covers the chains.
#[test]
fn v26_springs_still_play() {
    let Some(assets) = export() else {
        eprintln!("skipped: no okb_export");
        return;
    };
    let run = |v: u32| {
        let mut r = Replay::decode(include_bytes!("../fixtures/springs_v26.okbr")).expect("decodes");
        assert_eq!(r.sim_version, 26);
        if v != r.sim_version {
            (r.sim_version, r.body.checks) = (v, Vec::new());
        }
        let (mut h, mut springs) = (0xCBF2_9CE4_8422_2325u64, [0usize; 2]);
        let sum = simulate(&assets, r, |s| {
            let Some(g) = &s.fight else { return };
            for (f, n) in g.m.fighters.iter().zip(&mut springs) {
                for (node, d) in &f.spring_aims {
                    let words = [*node as u32].into_iter().chain(d.to_array().map(f32::to_bits));
                    h = words.flat_map(u32::to_le_bytes).fold(h, |h, b| (h ^ b as u64).wrapping_mul(0x0000_0100_0000_01B3));
                }
                *n += f.spring_aims.len();
            }
        })
        .expect("in sync");
        assert_eq!(sum.frames, 906);
        assert!(springs.iter().all(|&n| n > 0), "springs {springs:?}");
        eprintln!("springs v{v}: {h:#018X}");
        (sum, h)
    };
    let (sum, v26) = run(26);
    assert!(sum.checks >= 15);
    assert_eq!(v26, 0xA525A7B9F30DCC35, "the v26 spring chains changed");
    assert_ne!(run(SIM_VERSION).1, v26);
}

/// The ST5 fight of `v5_fights_still_play` run at the current version: the stage routine's counter W.28
/// (+0x14 per drawn frame) restarts at the second round, stands still while the Beast Drive darkening is at 255
/// and on the round-start and match-end frames, goes back with the KO replay's snapshot, and the match end
/// (round state 0xD) puts both fighters back on their start marks with their damage kept (recorded, ST5: the
/// same per-frame values, recorded run). At v5 the counter is never stepped and the fighters stay where
/// the KO left them.
#[test]
fn v6_round_and_match_end_reset() {
    let Some(assets) = export() else {
        eprintln!("skipped: no okb_export");
        return;
    };
    let run = |v: u32| {
        let mut r = Replay::decode(include_bytes!("../fixtures/fight_st5_v5.okbr")).expect("decodes");
        r.sim_version = v;
        r.body.checks.clear();
        let (mut prev, mut resets, mut rewinds, mut frozen, mut win, mut before) = (None::<u16>, 0, 0, 0, None, None);
        simulate(&assets, r, |s| {
            let Some(m) = s.fight.as_ref().map(|g| &g.m) else { return };
            let c = m.world.stage.anim.0[1];
            if v < 6 {
                assert_eq!(c, 0);
            } else if let Some(p) = prev {
                if m.world.g4a58[2] == 255 || m.m9e() != 0 {
                    assert_eq!(c, p);
                    frozen += 1;
                } else if (m.round.state, m.round.cnt) == (0xD, 1) && c == p {
                    frozen += 1; // the match-end reset's frame draws no stage
                } else if c != p.wrapping_add(0x14) && matches!(m.round.state, 9 | 10) {
                    rewinds += 1; // the KO replay's snapshot holds the wall block
                } else if c != p.wrapping_add(0x14) {
                    // the round-start frame draws no stage: 0, then 0x14
                    assert_eq!((c, m.round.state, m.round.cnt), (0, 0, 0), "frame {}", m.frame);
                    resets += 1;
                }
            }
            prev = Some(c);
            if m.round.state == 0xB {
                before = Some(m.fighters.each_ref().map(|f| (f.o_aa, f.o_ac, f.o_ae)));
            }
            if m.round.state == 0xD && m.round.cnt == 1 {
                win = Some((m.fighters.each_ref().map(|f| (f.pos, f.o_00)), m.fighters.each_ref().map(|f| (f.o_aa, f.o_ac, f.o_ae))));
            }
        })
        .expect("plays");
        let (win, damage) = win.expect("the match end");
        assert_eq!(Some(damage), before, "the damage kept");
        (resets, rewinds, frozen, win)
    };
    let (resets, rewinds, frozen, win) = run(SIM_VERSION);
    assert_eq!((resets, rewinds), (1, 1), "the second round, the KO replay");
    assert!(frozen > 200, "{frozen} frames at 255");
    assert_eq!((win[0].0, win[1].0), ([-0x100, 0, 0], [0x100, 0, 0]));
    assert_eq!((win[0].1, win[1].1), (1, 1), "F.0 = 1 (the fighter reset)");
    // v5: the counter stays at 0 (the frame count draws the stage), the loser where it fell, still KO'd
    let (resets, rewinds, frozen, win) = run(5);
    assert_eq!((resets, rewinds, frozen), (0, 0, 0));
    assert_ne!(win[1], ([0x100, 0, 0], 1));
}

/// A v6 fight on the extra stage ST9 (EXTRA1; recorded by c8fb23e: YUG walks back from the start, the CPU's
/// BAKURYU follows) still plays in sync, and shows the extra stages' bounds as the game has them (recorded
/// with M.98 forced to 9..12): the ring box is ±4096, so
/// the walk back stops near −4040 (on the normal stages near −1470); there are no walls (hp 0), so
/// the CPU's hit carries YUG over the edge: a ring-out, landing on the drop 256 below the ring.
#[test]
fn v6_extra_stage_bounds() {
    let Some(assets) = export() else {
        eprintln!("skipped: no okb_export");
        return;
    };
    let r = Replay::decode(include_bytes!("../fixtures/fight_st9_v6.okbr")).expect("decodes");
    assert_eq!((r.sim_version, r.engine.as_str()), (6, "0.1.0 (c8fb23e)"));
    // (the walk back's farthest hips x, the ring-out: frame and hips, P1's ground after it)
    let (mut walk, mut out, mut ground) = (0i32, None::<(u64, [i32; 3])>, None::<i32>);
    let sum = simulate(&assets, r, |s| {
        let Some(m) = s.fight.as_ref().map(|g| &g.m) else { return };
        let (ring, walls) = (&m.world.stage.ring, &m.world.stage.walls);
        assert_eq!((ring.kind, ring.hx, ring.hz, ring.drop, walls.hp), (0, 4096, 4096, 256, [0; 4]));
        let f = &m.fighters[0];
        match out {
            None if f.o_00 == 0x81 => out = Some((m.frame, f.hips)),
            None if f.move_cur == 3 => walk = walk.min(f.hips[0]),
            Some(_) if ground.is_none() && f.flags[crate::fight::fighter::F5C] & 0x80 == 0 => ground = Some(f.m_t[1]),
            _ => {}
        }
    })
    .expect("in sync");
    assert_eq!(sum.frames, 3167);
    // the walk back ends at the box (body sphere radius inside ±4096), far past a normal ring's ±1536
    assert_eq!(walk, -4039);
    // hit over the open side: the ring-out KO with the hips past −4096, then the ground 256 below
    assert_eq!(out, Some((999, [-4133, -164, 43])));
    assert_eq!(ground, Some(256));
}

/// The extra stages' engine-made bounds (simulation v7, `fight::stage::EXTRA_BOUNDS`),
/// 1: the inputs of `v6_extra_stage_bounds` (ST9) run at v7 (the CPU then plays another fight, so no
/// checkpoints). The hut's invisible walls (x ±1344, z ±1728, hp −1 as ST4's) stop the walk back at the pillars,
/// nobody leaves the hut and nobody is rung out.
#[test]
fn v7_extra_stage_hut_walls() {
    let Some(assets) = export() else {
        eprintln!("skipped: no okb_export");
        return;
    };
    let mut r = Replay::decode(include_bytes!("../fixtures/fight_st9_v6.okbr")).expect("decodes");
    r.sim_version = 7;
    r.body.checks.clear();
    let (mut walk, mut far, mut outs) = (0i32, [0i32; 2], 0);
    simulate(&assets, r, |s| {
        let Some(m) = s.fight.as_ref().map(|g| &g.m) else { return };
        let (ring, walls) = (&m.world.stage.ring, &m.world.stage.walls);
        assert_eq!((ring.hx, ring.hz, ring.drop, walls.hp, walls.model, m.fx.arena.half), (1344, 1728, 1, [-1; 4], [-1; 4], (1344, 1728)));
        for f in &m.fighters {
            far = [far[0].max(f.hips[0].abs()), far[1].max(f.hips[2].abs())];
            outs += (f.o_00 == 0x81) as u32;
        }
        if m.fighters[0].move_cur == 3 {
            walk = walk.min(m.fighters[0].hips[0]);
        }
    })
    .expect("runs");
    // at v6 the same walk went on to −4039 and ended in a ring-out at frame 999
    assert_eq!(walk, -1301);
    // the farthest hips (a knocked-down body's hips may lie past the pillars' inner face, inside the wall's 1440)
    assert_eq!((far, outs), ([1435, 1036], 0));
}

/// The extra stages' engine-made bounds (simulation v7), 2: a v7 fight on STB (EXTRA3; recorded by d577843+: YUG
/// walks in and attacks, the CPU's BAKURYU fights back) still plays in sync. The dome's ring is the normal ±1536
/// box on the tiled platform with open sides: YUG is rung out over it, lands on the level floor (drop 1, not under
/// it) and slides to a stop inside the outer circle (r 3456, the floor's edge 3584).
#[test]
fn v7_extra_stage_ring_out() {
    let Some(assets) = export() else {
        eprintln!("skipped: no okb_export");
        return;
    };
    let r = Replay::decode(include_bytes!("../fixtures/fight_stb_v7.okbr")).expect("decodes");
    assert_eq!((r.sim_version, r.engine.as_str()), (7, "0.1.0 (d577843+)"));
    let (mut out, mut ground, mut far) = (None::<(u64, [i32; 3])>, None::<i32>, 0i64);
    let sum = simulate(&assets, r, |s| {
        let Some(m) = s.fight.as_ref().map(|g| &g.m) else { return };
        let (ring, walls) = (&m.world.stage.ring, &m.world.stage.walls);
        assert_eq!((ring.hx, ring.hz, ring.drop, walls.hp, m.world.stage.outer.map(|o| (o.kind, o.w4))), (1536, 1536, 1, [0; 4], Some((1, 3456 * 3456))));
        let f = &m.fighters[0];
        match out {
            None if f.o_00 == 0x81 => out = Some((m.frame, f.hips)),
            Some(_) if f.o_00 == 0x81 => {
                far = far.max(f.hips[0] as i64 * f.hips[0] as i64 + f.hips[2] as i64 * f.hips[2] as i64);
                if ground.is_none() && f.flags[crate::fight::fighter::F5C] & 0x80 == 0 {
                    ground = Some(f.m_t[1]);
                }
            }
            _ => {}
        }
    })
    .expect("in sync");
    assert_eq!(sum.frames, 3417);
    // the ring-out KO with the hips past −1536, at v6's ±4096 box still inside the ring
    assert_eq!(out, Some((718, [-1570, -71, 193])));
    assert_eq!(ground, Some(1));
    assert!(far < 3456 * 3456, "{far}");
}

/// The extra stage Q3DM17 at simulation v10 and v11, recorded by the build before Q3DM6 (65b3c39, `OKB_SIM_VERSION`
/// 10 / 11, the export with Q3DM17's files): VS from the title, L1 on the stage select onto Q3DM17 (the row after
/// EXTRA4), a fight on it; and a direct fight `--fight --p1 YUG --p2 ALI --stage Q3DM17` (YUG walks back and
/// attacks). Both play in sync on this build without the extras files (the map is drawing only), on stage 13 with the
/// ring of their version (v10: 1280, v11: 1120 each way, the drop below), as recorded.
#[cfg(unix)]
#[test]
fn q3dm17_v10_v11_fixtures() {
    let Some(assets) = export() else {
        eprintln!("skipped: no okb_export");
        return;
    };
    // the export without `extras` (its folders linked)
    // (removed when the guard drops, also on a panic)
    if crate::session::tests::q3_elsewhere() {
        return;
    }
    let bare = crate::session::tests::BareExport::new(&assets, "br2_fixture_no_extras");
    let without = crate::assets::Assets::new(&bare.0);
    assert_eq!(crate::assets::extras::present(&without), 0);
    let q = crate::fight::stage::extra::Q3DM17;
    let fixtures: [(&[u8], u32, bool, u32); 4] = [
        (include_bytes!("../fixtures/select_q3dm17_v10.okbr"), 10, true, 1314),
        (include_bytes!("../fixtures/select_q3dm17_v11.okbr"), 11, true, 1314),
        (include_bytes!("../fixtures/fight_q3dm17_v10.okbr"), 10, false, 1217),
        (include_bytes!("../fixtures/fight_q3dm17_v11.okbr"), 11, false, 1249),
    ];
    for (bytes, v, select, frames) in fixtures {
        let r = Replay::decode(bytes).expect("decodes");
        assert_eq!((r.sim_version, r.header.start.extra_stage, r.header.start.extra_q3dm6), (v, true, false));
        let direct = matches!(&r.header.start.entry, Entry::DirectFight { stage, .. } if stage == q.code);
        assert_eq!(direct, !select, "v{v}");
        let (half, drop) = (q.ring_half(crate::fight::version::Gates::of(v)), q.ring_drop(crate::fight::version::Gates::of(v)));
        let mut fought = 0;
        let sum = simulate(&without, r, |s| {
            if let Some(m) = s.fight.as_ref().map(|g| &g.m) {
                let st = &m.world.stage;
                assert_eq!((st.id, st.ring.hx, st.ring.hz, st.ring.drop, st.walls.hp), (q.stage as usize, half[0], half[1], drop, [0; 4]), "v{v}");
                fought += 1;
            }
            if select && let Some(spec) = s.spec {
                assert_eq!(spec.stage, q.stage, "v{v}");
            }
        })
        .unwrap_or_else(|e| panic!("v{v} select {select}: {e}"));
        assert_eq!(sum.frames, frames, "v{v} select {select}");
        assert!(fought > 500, "v{v} select {select}: {fought} fight steps");
    }
}

/// The NETWORK row (simulation v8, `SessionStart::network`): a replay header without the field (every older file)
/// reads false; a session with it recorded as the app drives it (START to the mode menu, DOWN to NETWORK, ×: the
/// NETWORK request, the menu left, DOWN to OPTIONS, ×: OPTIONS) keeps the flag through the file and plays back in
/// sync; the same inputs at v7 never show the row (the 7th DOWN is OPTIONS there).
#[test]
fn v8_network_row() {
    use crate::fight::input::pad::{CROSS, DOWN, START};
    use crate::session::{AssetStore, Request, new_state};
    let old = Replay::decode(include_bytes!("../fixtures/fight_stb_v7.okbr")).expect("decodes");
    assert!(!old.header.start.network);
    let Some(assets) = export() else {
        eprintln!("skipped: no okb_export");
        return;
    };
    let front = std::sync::Arc::new(pollster::block_on(crate::frontend::load(&assets)).expect("front data"));
    let store = AssetStore { front: Some(front), ..Default::default() };
    // simulation v21: the rows of v8..v21 (v22's TEAM BATTLE row moves REPLAY and NETWORK one down)
    let start = SessionStart { seed: 3, entry: Entry::Title, network: true, sim_version: crate::fight::version::v::TEAM - 1, ..Default::default() };
    let prefs = Prefs::default();
    let mut s = new_state(&start, &store.assets(prefs), &mut Out::default());
    let mut rec = Recorder::new(&start, prefs, [0, 1]);
    let (mut asked, mut options) = (None, None);
    let describe = |s: &GameState| s.front.as_ref().map(|f| f.describe()).unwrap_or_default();
    for k in 0..1500u32 {
        let d = describe(&s);
        let pad = if k % 10 != 0 {
            0
        } else if !d.starts_with("mode menu") {
            if options.is_none() { START } else { 0 }
        } else if asked.is_none() {
            if d.starts_with("mode menu (NETWORK)") { CROSS } else { DOWN }
        } else if d.starts_with(&format!("mode menu ({})", crate::assets::text::item("modes", 6))) {
            CROSS
        } else {
            DOWN
        };
        let i = FrameInput { pad: [pad, 0], ..Default::default() };
        let mut out = Out::default();
        step(&mut s, &store.assets(prefs), i, &[], &mut out);
        rec.step(i, &[], prefs, &s).expect("recorded");
        if asked.is_none() && out.requests.iter().any(|r| matches!(r, Request::Network)) {
            asked = Some(s.frame);
        }
        if asked.is_some() && options.is_none() && describe(&s).starts_with("menu Options") {
            options = Some(s.frame);
        }
    }
    assert!(asked.is_some() && options > asked, "{asked:?} {options:?}");
    let r = Replay::decode(&rec.replay(&s, 1_700_000_000).encode()).expect("decodes");
    assert!(r.header.start.network && r.sim_version == crate::fight::version::v::TEAM - 1);
    let mut seen = Vec::new();
    let sum = simulate(&assets, r.clone(), |g| seen.push(describe(g).split(" [").next().unwrap_or("").to_string())).expect("in sync");
    assert_eq!((sum.frames, sum.hash), (1500, hash(&s)));
    assert!(seen.iter().any(|d| d == "mode menu (NETWORK)"));
    let mut v7 = r;
    v7.sim_version = 7;
    v7.body.checks.clear();
    let mut seen = Vec::new();
    simulate(&assets, v7, |g| seen.push(describe(g))).expect("runs");
    assert!(!seen.iter().any(|d| d.contains("NETWORK")) && seen.iter().any(|d| d.starts_with("menu Options")));
}

/// A playback with the app's seeks (`app::replays`): snapshots every 60 steps of the watched part from its
/// first frame (frame 0's start state for a whole session), played to the end; then back to the first (a
/// match file's VS screen: before its match loaded and its fight started) and on to the end, and from there a
/// few steps, forward to a later snapshot and on: every checkpoint in sync, the same last state as straight
/// through.
fn seeks_in_sync(assets: &crate::assets::Assets, r: Replay) {
    let want = simulate(assets, r.clone(), |_| {}).expect("in sync");
    let (from, fight_from) = (r.header.from, r.header.fight_from);
    let mut h = pollster::block_on(Headless::new(assets, r)).expect("starts");
    let mut keys = Vec::new();
    if from == 0 {
        keys.push((0, h.key()));
    }
    while pollster::block_on(h.step()).expect("in sync") {
        let f = h.s.frame;
        if f >= from && (f - from).is_multiple_of(60) {
            keys.push((f, h.key()));
        }
    }
    let end = |h: &mut Headless, what: &str| {
        while pollster::block_on(h.step()).unwrap_or_else(|e| panic!("{what}: {e}")) {}
        let s = h.summary();
        assert_eq!((s.frames, s.checks, s.hash), (want.frames, want.checks, want.hash), "{what}");
    };
    end(&mut h, "straight");
    let sizes: Vec<usize> = keys.iter().map(|k| k.1.snap.len()).collect();
    eprintln!("seek snapshots: {} from frame {}, {}..{} bytes each", keys.len(), keys[0].0, sizes.iter().min().unwrap(), sizes.iter().max().unwrap());
    let first = &keys[0];
    assert!(first.0 == from && fight_from.is_none_or(|f| first.0 < f), "{} {fight_from:?}", first.0);
    assert!(from == 0 || first.1.held.fight.is_none(), "the VS screen's snapshot is before the match loaded");
    h.seek(&first.1).expect("seek back");
    assert_eq!(h.s.frame, from);
    end(&mut h, "after the seek back");
    h.seek(&first.1).expect("seek back");
    for _ in 0..10 {
        pollster::block_on(h.step()).expect("in sync");
    }
    let later = &keys[keys.len() * 2 / 3];
    h.seek(&later.1).expect("seek on");
    assert_eq!(h.s.frame, later.0);
    end(&mut h, "after the seek on");
}

/// A live front-end session recorded as the app drives it (title → ARCADE → select → VS → the match loaded a
/// few steps later → the fight, each round won on time by P1 through the F2 / F4 cheats → the next ladder VS
/// screen) and a direct fight: `simulate` plays both back in sync, to the same final state; the match's file
/// ends at the match's end and starts at its VS screen; a changed input is caught at the next checkpoint.
#[test]
fn recorded_sessions_replay() {
    use crate::session::{AssetStore, Request, new_state};
    use std::sync::Arc;
    let Some(assets) = export() else {
        eprintln!("skipped: no okb_export");
        return;
    };
    let front = Arc::new(pollster::block_on(crate::frontend::load(&assets)).expect("front data"));
    let fe = |k: u32| match k {
        40 => crate::fight::input::pad::START,
        70 | 75 => crate::fight::input::pad::CROSS,
        k if (150..400).contains(&k) && k % 40 == 0 => crate::fight::input::pad::SQUARE,
        _ => 0,
    };
    let title = SessionStart { seed: 3, entry: Entry::Title, ..Default::default() };
    let fight = SessionStart { seed: 1, entry: Entry::DirectFight { p1: "LON".into(), p2: "BAK".into(), stage: "ST5".into() }, ..Default::default() };
    let data = Arc::new(
        pollster::block_on(crate::fight::fight_data::load_match(&assets, ["LON", "BAK"], [0, 1], "ST5", false, crate::session::SIM_VERSION))
            .expect("the fight"),
    );
    for (start, mut store, n) in
        [(title, AssetStore { front: Some(front), ..Default::default() }, 3000u32), (fight, AssetStore { fight: Some(data), ..Default::default() }, 700)]
    {
        let prefs = Prefs::default();
        let mut s = new_state(&start, &store.assets(prefs), &mut Out::default());
        let mut rec = Recorder::new(&start, prefs, [0, 1]);
        let (mut flags, mut wait, mut file) = (0u8, None::<(MatchSpec, u32)>, None);
        for k in 0..n {
            // the load completes 3 steps after it was asked for, as the app's VS screen wait
            if let Some((spec, w)) = wait.take() {
                if w == 0 {
                    let chr = spec.chr.map(crate::assets::roster::code);
                    let l = pollster::block_on(crate::fight::fight_data::load_match(
                        &assets,
                        chr,
                        spec.costume,
                        crate::fight::fight_data::stage_code(spec.stage),
                        false,
                        crate::session::SIM_VERSION,
                    ));
                    store.fight = Some(Arc::new(l.expect("the match")));
                    flags |= FrameInput::LOADED;
                } else {
                    wait = Some((spec, w - 1));
                }
            }
            let pad = if s.fight.is_some() { if k % 30 < 2 { crate::fight::input::pad::SQUARE } else { 0 } } else { fe(k) };
            let i = FrameInput { pad: [pad, 0], forced: [0; 2], flags: std::mem::take(&mut flags), ..Default::default() };
            let cheats = match &s.fight {
                Some(g) if g.m.round.state == 4 && g.m.round.timer > 60 && s.front.is_some() => vec![Cheat::OneHp(1), Cheat::Timer],
                _ => Vec::new(),
            };
            let mut out = Out::default();
            step(&mut s, &store.assets(prefs), i, &cheats, &mut out);
            rec.step(i, &cheats, prefs, &s).expect("recorded");
            for r in out.requests {
                match r {
                    Request::Load(spec) => {
                        store.fight = None;
                        wait = Some((spec, 3));
                        rec.match_load(&s);
                    }
                    Request::Fight => rec.match_start(&s),
                    Request::FightOver { backdrop, outcome, rounds, demo } => {
                        let data = store.fight.take();
                        if backdrop {
                            store.backdrop = data;
                        }
                        if file.is_none() {
                            file = rec.match_end(&s, outcome, rounds, demo, 0);
                        }
                    }
                    _ => {}
                }
            }
            if s.backdrop.is_none() {
                store.backdrop = None;
            }
            store.prune(&s);
        }
        let whole = rec.replay(&s, 0);
        let bytes = whole.encode();
        let sum = simulate(&assets, Replay::decode(&bytes).expect("decodes"), |_| {}).expect("in sync");
        assert_eq!((sum.frames, sum.hash), (n, hash(&s)));
        assert!(sum.strict && sum.checks == n / 60 + u32::from(n % 60 != 0), "{sum:?}");
        eprintln!("{:?}: {n} steps, {} bytes, {} checkpoints", start.entry, bytes.len(), sum.checks);
        if let Some(f) = file {
            let h = &f.header;
            eprintln!("match file: {} {:?} {:?} {:?} from {} fight {:?} to {}", h.mode, h.spec, h.outcome, h.rounds, h.from, h.fight_from, h.frames);
            // ARCADE's banner (the text list `screen_title`, entry 10)
            assert!(
                h.mode == crate::assets::text::item("screen_title", 10) && h.outcome.is_some() && h.from > 0 && h.fight_from > Some(h.from) && h.frames < n
            );
            let sum = simulate(&assets, Replay::decode(&f.encode()).expect("decodes"), |_| {}).expect("the match file in sync");
            assert_eq!(sum.frames, h.frames);
            seeks_in_sync(&assets, Replay::decode(&f.encode()).expect("decodes"));
        } else {
            assert!(matches!(start.entry, Entry::DirectFight { .. }), "no match file");
            seeks_in_sync(&assets, Replay::decode(&bytes).expect("decodes"));
        }
        // a pad changed in the fight: caught at the next checkpoint
        let mut x = Replay::decode(&bytes).expect("decodes");
        let mut p = Player::new(x.clone());
        let mut ins = Vec::new();
        while let Some((i, ..)) = p.next_step() {
            ins.push(i);
        }
        // the title's START, the fight's punches
        let k = if store.front.is_some() { 40 } else { 400 };
        ins[k].pad[0] ^= if k == 40 { crate::fight::input::pad::START } else { crate::fight::input::pad::SQUARE };
        let mut w = InputWriter::default();
        ins.into_iter().for_each(|i| w.push(i));
        x.body.inputs = w.bytes(false);
        let e = simulate(&assets, x.clone(), |_| {}).unwrap_err();
        assert!(e.starts_with("out of sync at frame"), "{e}");
        eprintln!("changed pad at step {k}: {e}");
        // and by the sync hash alone, as a file of another build plays (the title's START: a menu-only change)
        x.engine = "0.0.0 (other)".into();
        let e = simulate(&assets, x, |_| {}).unwrap_err();
        assert!(e.starts_with("out of sync at frame") && !e.contains("full state"), "{e}");
        eprintln!("changed pad at step {k}, sync hash only: {e}");
    }
}

/// The head turn (simulation v13, `fight::move_script::head_turn`): the ST5 fight of
/// `v5_fights_still_play` run at the current version turns both heads while they stand or walk (in round state 4
/// each step at most 0x2D yaw / 0x22 pitch, else a reset to 0 for an attack, a reaction or a KO; the KO replay
/// puts its snapshot's values back), and at v12 never.
#[test]
fn v13_head_turn() {
    let Some(assets) = export() else {
        eprintln!("skipped: no okb_export");
        return;
    };
    let run = |v: u32| {
        let mut r = Replay::decode(include_bytes!("../fixtures/fight_st5_v5.okbr")).expect("decodes");
        r.sim_version = v;
        r.body.checks.clear();
        let (mut prev, mut prev_state, mut turned, mut resets) = ([(0i16, 0i16); 2], 0, 0, 0);
        simulate(&assets, r, |s| {
            let Some(m) = s.fight.as_ref().map(|g| &g.m) else { return };
            let fighting = (prev_state, m.round.state) == (4, 4);
            prev_state = m.round.state;
            for (k, f) in m.fighters.iter().enumerate() {
                let now = (f.head_yaw, f.head_pitch);
                if now == (0, 0) {
                    resets += (prev[k] != (0, 0)) as u32;
                } else if fighting {
                    turned += 1;
                    let (dy, dp) = (now.0 as i32 - prev[k].0 as i32, now.1 as i32 - prev[k].1 as i32);
                    assert!(dy.abs() <= 0x2D && dp.abs() <= 0x22, "frame {} P{}: {prev:?} -> {now:?}", m.frame, k + 1);
                }
                prev[k] = now;
            }
        })
        .expect("plays");
        (turned, resets)
    };
    let (turned, resets) = run(SIM_VERSION);
    assert_eq!((turned, resets), (7372, 18));
    assert_eq!(run(12), (0, 0));
}

/// The imported game's install next to the engine (`BR1_EXPORT`, else `../okb_export/br1`), None when missing.
#[cfg(unix)]
fn br1_export() -> Option<std::path::PathBuf> {
    let root = std::env::var("BR1_EXPORT").unwrap_or_else(|_| concat!(env!("CARGO_MANIFEST_DIR"), "/../okb_export/br1").into());
    let p = std::path::PathBuf::from(root);
    p.join("manifest.json").exists().then_some(p)
}

/// A temporary export of `assets`' folders linked plus `br1` linked as the imported game's install (`br1/`), removed
/// when dropped (the folder `BareExport` makes, with the install added).
#[cfg(unix)]
fn with_br1(assets: &crate::assets::Assets, br1: &std::path::Path, name: &str) -> crate::session::tests::BareExport {
    let e = crate::session::tests::BareExport::new(assets, name);
    let _ = std::fs::remove_file(e.0.join(crate::assets::manifest::BR1_DIR));
    std::os::unix::fs::symlink(std::fs::canonicalize(br1).expect("the BR1 install"), e.0.join(crate::assets::manifest::BR1_DIR)).unwrap();
    e
}

/// `assets`' folders linked without the imported game's install (`br1/`): the data an imported replay is refused on;
/// removed when dropped.
#[cfg(unix)]
fn without_br1(assets: &crate::assets::Assets, name: &str) -> crate::session::tests::BareExport {
    let e = crate::session::tests::BareExport::new(assets, name);
    let _ = std::fs::remove_file(e.0.join(crate::assets::manifest::BR1_DIR));
    e
}

/// The v15 fixtures with imported fighters (`record_br1_fixtures`): the direct fights, their scripts, steps.
#[cfg(unix)]
const BR1_FIXTURES: [(&str, &str, &str, &str, &str); 2] = [
    (
        "fight_br1_mixed_v15",
        "MIT",
        "ALI",
        "300:,40:R,6:J,10:,6:J,10:,6:K,40:,30:L,6:J,10:,6:K,10:,6:DK,40:,1200:",
        "300:,30:L,6:K,20:,6:J,20:,6:J,60:,40:L,6:K,30:,1200:",
    ),
    ("fight_br1_v15", "URI", "GRE", "300:,40:R,6:J,10:,6:K,10:,6:C,120:,6:J,10:,6:J,30:,1200:", "300:,30:L,6:K,15:,6:K,15:,6:K,15:,6:J,15:,6:J,15:,6:J,1200:"),
];

/// Writes the fixtures of `BR1_FIXTURES` (run by hand: `cargo test --release br1_fixtures_record -- --ignored`, with
/// the export and the imported game's install beside the engine): the session as the app starts a direct fight with
/// the install's content id, both pads scripted.
#[cfg(unix)]
#[test]
#[ignore]
fn br1_fixtures_record() {
    use crate::session::{AssetStore, new_state};
    let (Some(assets), Some(br1)) = (export(), br1_export()) else { panic!("no okb_export / okb_export/br1") };
    let with = with_br1(&assets, &br1, "br2_fixture_br1_record");
    let a = crate::assets::Assets::new(&with.0);
    let id = crate::assets::manifest::br1_install_id(&a).expect("a usable BR1 install");
    for (name, p1, p2, s1, s2) in BR1_FIXTURES {
        let entry = Entry::DirectFight { p1: p1.into(), p2: p2.into(), stage: "ST1".into() };
        // both players human (OKB_P2=human), as the scripts drive them
        let fight = crate::session::FightDebug { p2_human: true, ..Default::default() };
        let start = SessionStart { seed: 5, entry, br1: Some(id), fight, ..Default::default() };
        let data = pollster::block_on(crate::fight::fight_data::load_match(&a, [p1, p2], [0, 1], "ST1", false, SIM_VERSION)).expect("load");
        let store = AssetStore { fight: Some(std::sync::Arc::new(data)), ..Default::default() };
        let prefs = Prefs::default();
        let mut s = new_state(&start, &store.assets(prefs), &mut Out::default());
        let mut rec = Recorder::new(&start, prefs, [0, 1]);
        let (a1, a2) = (crate::fight::input_script::parse_script(s1), crate::fight::input_script::parse_script(s2));
        let at = |sc: &[u16], k: usize| sc.get(k).copied().unwrap_or(0);
        for k in 0..a1.len().max(a2.len()) {
            let i = FrameInput { pad: [at(&a1, k), at(&a2, k)], ..Default::default() };
            step(&mut s, &store.assets(prefs), i, &[], &mut Out::default());
            rec.step(i, &[], prefs, &s).expect("recorded");
        }
        let path = format!("{}/src/session/fixtures/{name}.okbr", env!("CARGO_MANIFEST_DIR"));
        std::fs::write(&path, rec.replay(&s, 1_700_000_000).encode()).expect("written");
        eprintln!("{path}: {} steps", s.frame);
    }
}

/// The fight's trace of a fixture of `BR1_FIXTURES`: FNV-1a 64 of the match's bytes (postcard) after every step,
/// folded, at every 100th step and the last (`fixture frame hash` lines, `fixtures/br1_v15_hashes.txt`).
#[cfg(unix)]
fn br1_trace(a: &crate::assets::Assets, name: &str, r: Replay) -> Vec<String> {
    use crate::base::hash::{FNV_BASIS, fnv1a};
    let (mut h, mut out) = (FNV_BASIS, Vec::new());
    let sum = simulate(a, r, |s| {
        if let Some(g) = &s.fight {
            h = fnv1a(h, &postcard::to_allocvec(&g.m).expect("serialises"));
        }
        if s.frame % 100 == 0 {
            out.push(format!("{name} {} {h:016x}", s.frame));
        }
    })
    .unwrap_or_else(|e| panic!("{name}: {e}"));
    out.push(format!("{name} {} {h:016x}", sum.frames));
    out
}

/// Writes `fixtures/br1_v15_hashes.txt`, the traces of `BR1_FIXTURES` (run by hand after `br1_fixtures_record`, or
/// after an intended change of matches with imported fighters or of the match's state: `cargo test --release
/// br1_fixture_hashes_record -- --ignored`, with the export and the imported game's install beside the engine).
#[cfg(unix)]
#[test]
#[ignore]
fn br1_fixture_hashes_record() {
    let (Some(assets), Some(br1)) = (export(), br1_export()) else { panic!("no okb_export / okb_export/br1") };
    let with = with_br1(&assets, &br1, "br2_fixture_br1_hashes");
    let a = crate::assets::Assets::new(&with.0);
    let mut lines = Vec::new();
    for (name, ..) in BR1_FIXTURES {
        let path = format!("{}/src/session/fixtures/{name}.okbr", env!("CARGO_MANIFEST_DIR"));
        let r = Replay::decode(&std::fs::read(&path).expect("the fixture")).expect("decodes");
        lines.extend(br1_trace(&a, name, r));
    }
    let path = format!("{}/src/session/fixtures/br1_v15_hashes.txt", env!("CARGO_MANIFEST_DIR"));
    std::fs::write(&path, lines.join("\n") + "\n").expect("written");
    eprintln!("{path}: {} lines", lines.len());
}

/// The v15 fixtures with imported fighters (`BR1_FIXTURES`, recorded by `br1_fixtures_record`: MIT vs ALI,
/// a mixed pair, and URI vs GRE): over the export with the same data of the imported game they play in sync to their last
/// step, both fighters hit, the match's state step by step as recorded (`br1_trace`, re-recorded by
/// `br1_fixture_hashes_record` with an intended change); without the data, or with a header's other content id, they
/// are refused.
#[cfg(unix)]
#[test]
fn br1_v15_fixtures() {
    let fixtures: [(&[u8], u32); 2] =
        [(include_bytes!("../fixtures/fight_br1_mixed_v15.okbr"), 1726), (include_bytes!("../fixtures/fight_br1_v15.okbr"), 1750)];
    let (Some(assets), Some(br1)) = (export(), br1_export()) else {
        eprintln!("skipped: no okb_export / okb_export/br1");
        return;
    };
    let with = with_br1(&assets, &br1, "br2_fixture_br1");
    let a = crate::assets::Assets::new(&with.0);
    for (k, (bytes, frames)) in fixtures.into_iter().enumerate() {
        let r = Replay::decode(bytes).expect("decodes");
        let (_, p1, p2, ..) = BR1_FIXTURES[k];
        assert!(matches!(&r.header.start.entry, Entry::DirectFight { p1: a, p2: b, .. } if a == p1 && b == p2));
        assert_eq!((r.sim_version, r.header.start.br1), (15, crate::assets::manifest::br1_install_id(&a)), "{p1} {p2}");
        let mut hit = [false; 2];
        let sum = simulate(&a, r.clone(), |s| {
            if let Some(m) = s.fight.as_ref().map(|g| &g.m) {
                assert!(m.br1.is_some());
                for i in 0..2 {
                    hit[1 - i] |= m.fighters[i].o_ac > 0;
                }
            }
        })
        .unwrap_or_else(|e| panic!("{p1} {p2}: {e}"));
        assert_eq!(sum.frames, frames, "{p1} {p2}");
        assert_eq!(hit, [true; 2], "{p1} {p2}: both hit");
        let name = BR1_FIXTURES[k].0;
        let want: Vec<&str> = include_str!("../fixtures/br1_v15_hashes.txt").lines().filter(|l| l.starts_with(&format!("{name} "))).collect();
        assert_eq!(br1_trace(&a, name, r.clone()), want, "{name}: the match's trace");
        let e = simulate(&crate::assets::Assets::new(&without_br1(&assets, "br2_fixture_no_br1").0), r.clone(), |_| {}).err().unwrap_or_default();
        assert!(e.contains("Bloody Roar 1"), "{p1} {p2} without the data: {e}");
        let mut other = r;
        other.header.start.br1 = other.header.start.br1.map(|i| i ^ 1);
        assert!(simulate(&a, other, |_| {}).is_err_and(|e| e.contains("Bloody Roar 1")), "{p1} {p2}: another id");
    }
}

/// The v20 direct fights with imported fighters (`br1_v20_fixtures_record`): name, P1, P2 (the CPU), stage,
/// P1's script. `fight_br1_cpu_v20`: MITSUKO against the imported game's own CPU AI (FOX) on FOX's stage;
/// `fight_br1_uri00_v20`: a mixed pair on Uriko's stage, URIKO (BR1) pressing ○ until her beast form, against the
/// main game's CPU (ALICE): her walls shatter into the imported game's debris and drop back (simulation v18's events).
#[cfg(unix)]
fn br1_v20_fight(k: usize) -> (&'static str, &'static str, &'static str, &'static str, String) {
    match k {
        0 => (
            "fight_br1_cpu_v20",
            "MIT",
            "FOX",
            "FOX00",
            "300:,40:R,6:J,10:,6:J,10:,6:K,40:,30:L,6:J,10:,6:K,10:,6:DK,40:,40:R,6:J,10:,6:J,10:,6:C,60:,40:R,6:J,10:,6:K,10:,6:J,1600:".into(),
        ),
        _ => ("fight_br1_uri00_v20", "URI", "ALI", "URI00", format!("330:{}", ",3:C,57:".repeat(75))),
    }
}

/// The v20 ARCADE fixture (`br1_v20_fixtures_record`): ARCADE from the title with the QUICK select, MITSUKO (the
/// imported row: DOWN × 3), every fight won (P2 at 1 HP once a round fights, `Cheat::OneHp`; P1 □ and × in
/// turn), to the end of the boss fight against "URIKO (BR1)" on her stage.
#[cfg(unix)]
const BR1_ARCADE: &str = "arcade_br1_boss_v20";

/// The session start of the v20 fixtures: a direct fight with P2 the CPU, or ARCADE from the title (`stage` None).
#[cfg(unix)]
fn br1_v20_start(id: u32, fight: Option<(&str, &str, &str)>) -> SessionStart {
    match fight {
        Some((p1, p2, stage)) => {
            let entry = Entry::DirectFight { p1: p1.into(), p2: p2.into(), stage: stage.into() };
            SessionStart { seed: 5, entry, br1: Some(id), sim_version: 20, ..Default::default() }
        }
        None => {
            let mut card = crate::frontend::options::Card::new(None, None);
            assert!(crate::frontend::options::Settings { char_select: 1, ..Default::default() }.save(&mut card));
            SessionStart { seed: 5, entry: Entry::Title, br1: Some(id), card: card.file().map(|b| b.to_vec()), sim_version: 20, ..Default::default() }
        }
    }
}

/// Writes the v20 fixtures (run by hand: `cargo test --release br1_v20_fixtures_record -- --ignored`, with the export
/// and the imported game's install beside the engine), then `fixtures/br1_v20_hashes.txt`, their traces (`br1_trace`).
/// The direct fights as `br1_fixtures_record` makes them, P2 the CPU; the ARCADE run's loads and fetches served at
/// once, as the headless playback serves them.
#[cfg(unix)]
#[test]
#[ignore]
fn br1_v20_fixtures_record() {
    use crate::fight::input::pad::{CROSS, DOWN, SQUARE, START};
    use crate::session::{AssetStore, Request, new_state};
    use std::sync::Arc;
    let (Some(assets), Some(br1)) = (export(), br1_export()) else { panic!("no okb_export / okb_export/br1") };
    let with = with_br1(&assets, &br1, "br2_fixture_br1_v20_record");
    let a = crate::assets::Assets::new(&with.0);
    let id = crate::assets::manifest::br1_install_id(&a).expect("a usable BR1 install");
    let prefs = Prefs::default();
    let write = |name: &str, r: Replay| {
        let path = format!("{}/src/session/fixtures/{name}.okbr", env!("CARGO_MANIFEST_DIR"));
        std::fs::write(&path, r.encode()).expect("written");
        eprintln!("{path}: {} steps", r.header.frames);
    };
    for k in 0..2 {
        let (name, p1, p2, stage, script) = br1_v20_fight(k);
        let start = br1_v20_start(id, Some((p1, p2, stage)));
        let data = pollster::block_on(crate::fight::fight_data::load_match(&a, [p1, p2], [0, 1], stage, false, 20)).expect("load");
        let store = AssetStore { fight: Some(Arc::new(data)), ..Default::default() };
        let mut s = new_state(&start, &store.assets(prefs), &mut Out::default());
        let mut rec = Recorder::new(&start, prefs, [0, 1]);
        for pad in crate::fight::input_script::parse_script(&script) {
            let i = FrameInput { pad: [pad, 0], ..Default::default() };
            step(&mut s, &store.assets(prefs), i, &[], &mut Out::default());
            rec.step(i, &[], prefs, &s).expect("recorded");
        }
        write(name, rec.replay(&s, 1_700_000_000));
    }
    // ARCADE
    let start = br1_v20_start(id, None);
    let front = Arc::new(pollster::block_on(crate::frontend::load(&a)).expect("front data"));
    let mut store = AssetStore { front: Some(front), ..Default::default() };
    let mut s = new_state(&start, &store.assets(prefs), &mut Out::default());
    let mut rec = Recorder::new(&start, prefs, [0, 0]);
    let boss = (crate::assets::roster::BR1_FIRST + 3, crate::fight::stage::br1::STAGES[8].stage);
    let (mut flags, mut on_select, mut cheated, mut in_boss, mut end, mut fights) = (0u8, None, None, false, None, 0);
    for k in 0..80_000usize {
        let d = s.front.as_ref().map(|f| f.describe()).unwrap_or_default();
        if d.starts_with("select") && on_select.is_none() {
            on_select = Some(k);
        }
        let mut cheats = Vec::new();
        let pad = if let Some(m) = s.fight.as_ref().map(|g| &g.m) {
            // a round fighting: P2 at 1 HP once, P1 □ / × in turn
            if m.round.state == 4 && cheated != Some((fights, m.round.index)) {
                cheated = Some((fights, m.round.index));
                cheats.push(Cheat::OneHp(1));
            }
            match k % 12 {
                0..=1 => SQUARE,
                6..=7 => CROSS,
                _ => 0,
            }
        } else {
            match (k, on_select) {
                (40, _) => START,
                (80, _) => CROSS,
                (_, Some(t)) if k > t + 40 && k <= t + 100 && (k - t - 40) % 20 == 0 => DOWN,
                (_, Some(t)) if k > t + 100 && (k - t) % 40 == 0 => CROSS,
                _ => 0,
            }
        };
        let i = FrameInput { pad: [pad, 0], flags, ..Default::default() };
        flags = 0;
        let mut out = Out::default();
        step(&mut s, &store.assets(prefs), i, &cheats, &mut out);
        rec.step(i, &cheats, prefs, &s).expect("recorded");
        for r in out.requests {
            match r {
                Request::Load(spec) => {
                    let chr = spec.chr.map(crate::assets::roster::code);
                    let bgm = s.front.as_ref().is_some_and(|f| f.arcade_bgm());
                    let stage = crate::fight::fight_data::stage_code(spec.stage);
                    let data = pollster::block_on(crate::fight::fight_data::load_match(&a, chr, spec.costume, stage, bgm, s.sim_version)).expect("load");
                    store.fight = Some(Arc::new(data));
                    flags |= FrameInput::LOADED;
                    in_boss = (spec.chr[1], spec.stage) == boss;
                    fights += 1;
                    eprintln!("frame {}: load {:?} stage {}", s.frame, spec.chr, spec.stage);
                }
                Request::Fetch(names) => {
                    for n in names {
                        store.files.remove(&n);
                        if let Ok(b) = pollster::block_on(a.fetch(&n)) {
                            store.files.insert(n, b.into());
                        }
                    }
                    flags |= FrameInput::FETCHED;
                }
                Request::FightOver { backdrop, outcome, .. } => {
                    let data = store.fight.take();
                    if backdrop {
                        store.backdrop = data;
                    }
                    eprintln!("frame {}: fight over {outcome:?}", s.frame);
                    if in_boss {
                        end = Some(k);
                    }
                }
                _ => {}
            }
        }
        if s.backdrop.is_none() {
            store.backdrop = None;
        }
        store.prune(&s);
        if end.is_some_and(|e| k >= e + 60) {
            break;
        }
    }
    assert!(end.is_some(), "the boss fight never ended");
    write(BR1_ARCADE, rec.replay(&s, 1_700_000_000));
    // the traces
    let mut lines = Vec::new();
    for name in [br1_v20_fight(0).0, br1_v20_fight(1).0, BR1_ARCADE] {
        let path = format!("{}/src/session/fixtures/{name}.okbr", env!("CARGO_MANIFEST_DIR"));
        let r = Replay::decode(&std::fs::read(&path).expect("the fixture")).expect("decodes");
        lines.extend(br1_trace(&a, name, r));
    }
    let path = format!("{}/src/session/fixtures/br1_v20_hashes.txt", env!("CARGO_MANIFEST_DIR"));
    std::fs::write(&path, lines.join("\n") + "\n").expect("written");
    eprintln!("{path}: {} lines", lines.len());
}

/// The v20 fixtures with imported fighters (`br1_v20_fixtures_record`): over the export with the same
/// imported-game data they play in sync to their last step, the match's state step by step as recorded (`br1_trace`,
/// `fixtures/br1_v20_hashes.txt`). `fight_br1_cpu_v20`: the imported game's AI drives P2 (its block leaves state 0) and
/// both fighters hit; `fight_br1_uri00_v20`: Uriko's walls shatter into the imported game's debris and all four drop back;
/// `arcade_br1_boss_v20`: MITSUKO's ladder (fights 3 and 6 imported fighters) ends with the boss "URIKO (BR1)" on
/// her stage, who is hit. Without the data, or with another content id, they are refused.
#[cfg(unix)]
#[test]
fn br1_v20_fixtures() {
    let fixtures: [(&[u8], &str, u32); 3] = [
        (include_bytes!("../fixtures/fight_br1_cpu_v20.okbr"), br1_v20_fight(0).0, 2342),
        (include_bytes!("../fixtures/fight_br1_uri00_v20.okbr"), br1_v20_fight(1).0, 4830),
        (include_bytes!("../fixtures/arcade_br1_boss_v20.okbr"), BR1_ARCADE, 19857),
    ];
    let (Some(assets), Some(br1)) = (export(), br1_export()) else {
        eprintln!("skipped: no okb_export / okb_export/br1");
        return;
    };
    let with = with_br1(&assets, &br1, "br2_fixture_br1_v20");
    let a = crate::assets::Assets::new(&with.0);
    let first = crate::assets::roster::BR1_FIRST;
    let boss = ([first, first + 3], crate::fight::stage::br1::STAGES[8].stage);
    for (k, (bytes, name, frames)) in fixtures.into_iter().enumerate() {
        let r = Replay::decode(bytes).expect("decodes");
        assert_eq!((r.sim_version, r.header.start.br1), (20, crate::assets::manifest::br1_install_id(&a)), "{name}");
        let (mut hit, mut ai, mut debris, mut risen, mut specs) = ([false; 2], false, 0, false, Vec::new());
        let sum = simulate(&a, r.clone(), |s| {
            if let Some(spec) = s.spec
                && specs.last() != Some(&(spec.chr, spec.stage))
            {
                specs.push((spec.chr, spec.stage));
            }
            if let Some(m) = s.fight.as_ref().map(|g| &g.m)
                && let Some(b) = m.br1.as_deref()
            {
                for i in 0..2 {
                    hit[1 - i] |= m.fighters[i].o_ac > 0;
                }
                ai |= m.cpu[1] && b.side[1] && b.ai[1].state != 0;
                debris = debris.max(m.world.stage.debris.iter().filter(|p| p.alive != 0).count());
                risen |= m.world.stage.br1.g50 == [16; 4];
            }
        })
        .unwrap_or_else(|e| panic!("{name}: {e}"));
        assert_eq!(sum.frames, frames, "{name}");
        match k {
            0 => assert!(ai && hit == [true; 2], "{name}: the AI ran {ai}, hits {hit:?}"),
            1 => assert!((1..=8).contains(&debris) && risen, "{name}: debris {debris}, risen {risen}"),
            _ => {
                assert_eq!(specs.len(), 10, "{name}: {specs:?}");
                assert!(specs.iter().all(|(c, _)| c[0] == first), "{name}: {specs:?}");
                assert!([specs[2].0[1], specs[5].0[1]].iter().all(|&c| c > first && c < first + 3), "{name}: {specs:?}");
                assert_eq!(specs[9], boss, "{name}");
                assert!(hit[1], "{name}: the boss hit");
            }
        }
        let want: Vec<&str> = include_str!("../fixtures/br1_v20_hashes.txt").lines().filter(|l| l.starts_with(&format!("{name} "))).collect();
        assert_eq!(br1_trace(&a, name, r.clone()), want, "{name}: the match's trace");
        let e = simulate(&crate::assets::Assets::new(&without_br1(&assets, "br2_fixture_no_br1").0), r.clone(), |_| {}).err().unwrap_or_default();
        assert!(e.contains("Bloody Roar 1"), "{name} without the data: {e}");
        let mut other = r;
        other.header.start.br1 = other.header.start.br1.map(|i| i ^ 1);
        assert!(simulate(&a, other, |_| {}).is_err_and(|e| e.contains("Bloody Roar 1")), "{name}: another id");
    }
}

/// The team battle's fixture (engine addition, T4): a FREE FOR ALL reached through the menus.
const TEAM_FFA: &str = "team_ffa_v22";

/// The pads of `TEAM_FFA`'s session at step `k` (pads 1 / 2, pads 3 / 4), from the title: TEAM BATTLE (DOWN ×6),
/// FREE FOR ALL, pad 3 joins (START) and chooses, P1 chooses, picks P2's fighter and gives P4 a random one (SELECT),
/// TATAMI; in the battle (`fight`) P1 punches, P3 kicks and cycles its target with L2 now and then.
fn team_ffa_pads(k: u32, fight: bool) -> ([u16; 2], [u16; 2]) {
    use crate::fight::input::pad::{CROSS, DOWN, L2, LEFT, RIGHT, SELECT, SQUARE, START};
    if fight {
        let p1 = if k % 12 < 2 { SQUARE } else { 0 };
        let p3 = if k % 600 == 300 {
            L2
        } else if k % 16 >= 4 && k % 16 < 6 {
            CROSS
        } else {
            0
        };
        return ([p1, 0], [p3, 0]);
    }
    let p1 = match k {
        40 => START,
        60 | 70 | 80 | 90 | 100 | 110 | 150 => DOWN,
        130 | 170 | 380 => CROSS,
        260 | 300 => SQUARE,
        280 | 360 => RIGHT,
        290 => DOWN,
        320 => LEFT,
        330 => SELECT,
        _ => 0,
    };
    let p3 = match k {
        200 => START,
        230 => RIGHT,
        240 => CROSS,
        _ => 0,
    };
    ([p1, 0], [p3, 0])
}

/// The team battle's trace: FNV-1a 64 of `TeamFight`'s postcard bytes folded after every step of the battle, every
/// 100th frame and the last (as `br1_trace`).
fn team_trace(a: &crate::assets::Assets, name: &str, r: Replay) -> Vec<String> {
    use crate::base::hash::{FNV_BASIS, fnv1a};
    let (mut h, mut out) = (FNV_BASIS, Vec::new());
    let sum = simulate(a, r, |s| {
        if let Some(t) = &s.team {
            h = fnv1a(h, &postcard::to_allocvec(t.as_ref()).expect("serialises"));
        }
        if s.frame % 100 == 0 {
            out.push(format!("{name} {} {h:016x}", s.frame));
        }
    })
    .unwrap_or_else(|e| panic!("{name}: {e}"));
    out.push(format!("{name} {} {h:016x}", sum.frames));
    out
}

/// Writes `fixtures/team_ffa_v22.okbr` and its trace `fixtures/team_ffa_v22_hashes.txt` (run by hand after an intended
/// change of the team battle's menus, state or logic: `cargo test --release team_ffa_fixture_record -- --ignored`,
/// with the export beside the engine): the session from the title as the app records it, the loads served at once.
#[test]
#[ignore]
fn team_ffa_fixture_record() {
    use crate::session::{AssetStore, Request, new_state};
    use std::sync::Arc;
    let a = export().expect("okb_export");
    // (the fixture is simulation v22's, whatever the build's current version)
    let start = SessionStart { seed: 22, entry: Entry::Title, sim_version: crate::fight::version::v::TEAM, ..Default::default() };
    let prefs = Prefs::default();
    let front = Arc::new(pollster::block_on(crate::frontend::load(&a)).expect("front data"));
    let mut store = AssetStore { front: Some(front), ..Default::default() };
    let mut s = new_state(&start, &store.assets(prefs), &mut Out::default());
    let mut rec = Recorder::new(&start, prefs, [0, 0]);
    let (mut flags, mut end, mut saved) = (0u8, None, None);
    for k in 0..40_000u32 {
        let (pad, ext) = team_ffa_pads(k, s.team.is_some());
        let i = FrameInput { pad, ext, flags, ..Default::default() };
        flags = 0;
        let mut out = Out::default();
        step(&mut s, &store.assets(prefs), i, &[], &mut out);
        rec.step(i, &[], prefs, &s).expect("recorded");
        for r in out.requests {
            match r {
                Request::Load(_) => panic!("a match"),
                Request::LoadTeam(spec) => {
                    rec.match_load(&s);
                    let chr = spec.chr.map(crate::assets::roster::code);
                    let stage = crate::fight::fight_data::stage_code(spec.stage);
                    let data = pollster::block_on(crate::fight::fight_data::load_team(&a, chr, spec.costume, stage, s.sim_version)).expect("load");
                    store.fight = Some(Arc::new(data));
                    flags |= FrameInput::LOADED;
                    eprintln!("frame {}: load {spec:?}", s.frame);
                }
                Request::Fight => rec.match_start(&s),
                Request::TeamOver { outcome, rounds } => {
                    store.fight = None;
                    eprintln!("frame {}: team battle over {outcome:?} {rounds:?}", s.frame);
                    saved = rec.team_end(&s, outcome, rounds, 1_700_000_000);
                    end = Some(k);
                }
                _ => {}
            }
        }
        store.prune(&s);
        if end.is_some_and(|e| k >= e + 60) {
            break;
        }
    }
    let r = saved.expect("the team battle ended");
    assert_eq!(r.format, FORMAT_TEAM);
    let path = format!("{}/src/session/fixtures/{TEAM_FFA}.okbr", env!("CARGO_MANIFEST_DIR"));
    std::fs::write(&path, r.encode()).expect("written");
    eprintln!("{path}: {} steps", r.header.frames);
    let lines = team_trace(&a, TEAM_FFA, r);
    let path = format!("{}/src/session/fixtures/{TEAM_FFA}_hashes.txt", env!("CARGO_MANIFEST_DIR"));
    std::fs::write(&path, lines.join("\n") + "\n").expect("written");
}

/// `fixtures/team_ffa_v22.okbr` (`team_ffa_fixture_record`): a FORMAT 2 file whose header holds the FREE FOR ALL
/// (P1 and P3 human, P3 on pad 3); it plays in sync through the menus and the battle to its result screen, the battle
/// step by step as recorded (`team_trace`). A save state taken mid-battle (the runtime snapshot and the `.okbs` file)
/// plays on to the same end, and seeking back in the playback to it too.
#[test]
fn team_ffa_fixture() {
    let Some(a) = export() else {
        eprintln!("skipped: no okb_export");
        return;
    };
    let r = Replay::decode(include_bytes!("../fixtures/team_ffa_v22.okbr")).expect("decodes");
    let t = r.header.team.expect("a team battle's header");
    assert_eq!((r.format, r.sim_version, t.mode, t.cpu), (FORMAT_TEAM, 22, crate::fight::team::TeamMode::Ffa, [false, true, false, true]));
    assert!(matches!(r.header.outcome, Some(Outcome::Winner(w)) if w < 4));
    let (mut screens, mut l2) = (Vec::new(), false);
    let sum = simulate(&a, r.clone(), |s| {
        if let Some(f) = &s.front {
            let d = f.describe();
            let d = d.split([' ', '[']).take(2).collect::<Vec<_>>().join(" ");
            if screens.last() != Some(&d) {
                screens.push(d);
            }
        }
        if let Some(t) = &s.team {
            l2 |= t.locked.l2[2];
        }
    })
    .expect("in sync");
    assert_eq!(sum.frames, r.header.frames);
    for want in ["mode menu", "team mode", "team select", "team load", "team fight", "team result"] {
        assert!(screens.iter().any(|d| d.starts_with(want)), "{want}: {screens:?}");
    }
    assert!(l2, "pad 3's L2 locked a target");
    let want: Vec<&str> = include_str!("../fixtures/team_ffa_v22_hashes.txt").lines().collect();
    assert_eq!(team_trace(&a, TEAM_FFA, r.clone()), want, "the battle's trace");
    // a save state mid-battle
    let mut h = pollster::block_on(Headless::new(&a, r.clone())).expect("opens");
    while h.s.team.as_ref().is_none_or(|t| t.frame < 1500) {
        assert!(pollster::block_on(h.step()).expect("in sync"), "the battle reached frame 1500");
    }
    let key = h.key();
    let file = crate::session::save_file(&h.s);
    let held = key.held.clone();
    let from_file = crate::session::load_file(&file, &held.assets()).expect("the .okbs file loads");
    assert_eq!(hash(&from_file), hash(&h.s));
    // (R1) a state holding a match beside the team battle is refused
    let (mut both, _) = crate::session::decode_file(&file).expect("decodes");
    let md = pollster::block_on(crate::fight::fight_data::load_match(&a, ["YUG", "ALI"], [0, 0], "ST0", false, r.sim_version)).expect("a match's data");
    both.fight = Some(crate::session::fight::Fight::new(&md, &Default::default()));
    assert_eq!(crate::session::rebind(&mut both, &held.assets()), Err("a match and a team battle".to_string()));
    while pollster::block_on(h.step()).expect("in sync") {}
    let end = hash(&h.s);
    h.seek(&key).expect("seek back");
    while pollster::block_on(h.step()).expect("in sync") {}
    assert_eq!(hash(&h.s), end);
}

/// The imported team battle's fixture (engine addition, T6): a 2 VS 2 on BIG STAGE reached through the menus, at
/// simulation v24.
#[cfg(unix)]
const TEAM_BR1: &str = "team_br1_menus_v24";

/// The pads of `TEAM_BR1`'s session at step `k` (pads 1 / 2, pads 3 / 4), from the title: TEAM BATTLE (DOWN ×6), 2 VS
/// 2, pad 3 joins (START); P1 takes MITSUKO (DOWN ×2: the imported row), P3 GREG, P1 picks FOX for P2 (DOWN,
/// RIGHT) and keeps URIKO for P4, the stage panel's BIG STAGE (LEFT ×2 past RANDOM). In the battle (`frame`: the
/// battle's frame) P1 opens the pause at 600, CONTROLLER CONFIG. (v24), changes its first button (RIGHT), leaves with
/// START and closes the pause; then P1 punches, P3 kicks and cycles its target with L2 now and then.
#[cfg(unix)]
fn team_br1_pads(k: u32, frame: Option<u64>) -> ([u16; 2], [u16; 2]) {
    use crate::fight::input::pad::{CROSS, DOWN, L2, LEFT, RIGHT, SQUARE, START};
    if let Some(f) = frame {
        let p1 = match f {
            600 | 680 | 700 => START,
            620 => DOWN,
            640 => CROSS,
            660 => RIGHT,
            590..=720 => 0,
            _ if f % 12 < 2 => SQUARE,
            _ => 0,
        };
        let p3 = match f {
            590..=720 => 0,
            _ if f % 600 == 300 => L2,
            _ if f % 16 >= 4 && f % 16 < 6 => CROSS,
            _ => 0,
        };
        return ([p1, 0], [p3, 0]);
    }
    let p1 = match k {
        40 => START,
        60 | 70 | 80 | 90 | 100 | 110 | 210 | 220 | 280 => DOWN,
        130 | 170 | 300 | 380 => CROSS,
        230 | 320 => SQUARE,
        290 => RIGHT,
        350 | 360 => LEFT,
        _ => 0,
    };
    let p3 = match k {
        200 => START,
        240 | 250 => DOWN,
        260 => CROSS,
        _ => 0,
    };
    ([p1, 0], [p3, 0])
}

/// Writes `fixtures/team_br1_menus_v24.okbr` and its trace `fixtures/team_br1_menus_v24_hashes.txt` (run by hand after
/// an intended change of the team battle's menus, state or logic: `cargo test --release team_br1_fixture_record --
/// --ignored`, with the export and the imported game's install beside the engine), as `team_ffa_fixture_record`.
#[cfg(unix)]
#[test]
#[ignore]
fn team_br1_fixture_record() {
    use crate::session::{AssetStore, Request, new_state};
    use std::sync::Arc;
    let (Some(assets), Some(br1)) = (export(), br1_export()) else { panic!("no okb_export / okb_export/br1") };
    let with = with_br1(&assets, &br1, "br2_fixture_team_br1_record");
    let a = crate::assets::Assets::new(&with.0);
    let id = crate::assets::manifest::br1_install_id(&a).expect("a usable BR1 install");
    // (the fixture is simulation v24's, whatever the build's current version)
    let start = SessionStart { seed: 24, entry: Entry::Title, br1: Some(id), sim_version: crate::fight::version::v::TEAM_CONFIG, ..Default::default() };
    let prefs = Prefs::default();
    let front = Arc::new(pollster::block_on(crate::frontend::load(&a)).expect("front data"));
    let mut store = AssetStore { front: Some(front), ..Default::default() };
    let mut s = new_state(&start, &store.assets(prefs), &mut Out::default());
    let mut rec = Recorder::new(&start, prefs, [0, 0]);
    let (mut flags, mut end, mut saved) = (0u8, None, None);
    for k in 0..40_000u32 {
        let (pad, ext) = team_br1_pads(k, s.team.as_ref().map(|t| t.frame));
        let i = FrameInput { pad, ext, flags, ..Default::default() };
        flags = 0;
        let mut out = Out::default();
        step(&mut s, &store.assets(prefs), i, &[], &mut out);
        rec.step(i, &[], prefs, &s).expect("recorded");
        for r in out.requests {
            match r {
                Request::Load(_) => panic!("a match"),
                Request::LoadTeam(spec) => {
                    rec.match_load(&s);
                    let chr = spec.chr.map(crate::assets::roster::code);
                    let stage = crate::fight::fight_data::stage_code(spec.stage);
                    let data = pollster::block_on(crate::fight::fight_data::load_team(&a, chr, spec.costume, stage, s.sim_version)).expect("load");
                    store.fight = Some(Arc::new(data));
                    flags |= FrameInput::LOADED;
                    eprintln!("frame {}: load {spec:?}", s.frame);
                }
                Request::Fight => rec.match_start(&s),
                Request::TeamOver { outcome, rounds } => {
                    store.fight = None;
                    eprintln!("frame {}: team battle over {outcome:?} {rounds:?}", s.frame);
                    saved = rec.team_end(&s, outcome, rounds, 1_700_000_000);
                    end = Some(k);
                }
                _ => {}
            }
        }
        store.prune(&s);
        if end.is_some_and(|e| k >= e + 60) {
            break;
        }
    }
    let r = saved.expect("the team battle ended");
    assert_eq!(r.format, FORMAT_TEAM);
    let path = format!("{}/src/session/fixtures/{TEAM_BR1}.okbr", env!("CARGO_MANIFEST_DIR"));
    std::fs::write(&path, r.encode()).expect("written");
    eprintln!("{path}: {} steps", r.header.frames);
    let lines = team_trace(&a, TEAM_BR1, r);
    let path = format!("{}/src/session/fixtures/{TEAM_BR1}_hashes.txt", env!("CARGO_MANIFEST_DIR"));
    std::fs::write(&path, lines.join("\n") + "\n").expect("written");
}

/// `fixtures/team_br1_menus_v24.okbr` (`team_br1_fixture_record`): a FORMAT 2 file of simulation v24 whose header holds
/// the 2 VS 2 MITSUKO FOX GREG URIKO on BIG STAGE (P1 and P3 human); over the export with the same data of the imported game it
/// plays in sync through the menus (the select's imported row, the stage panel's BIG STAGE) and the battle to its
/// result screen, the battle step by step as recorded (`team_trace`); the pause's CONTROLLER CONFIG. opens and gives P1
/// its changed map (and the save its settings). Without the imported game's data it is refused.
#[cfg(unix)]
#[test]
fn team_br1_fixture() {
    let (Some(assets), Some(br1)) = (export(), br1_export()) else {
        eprintln!("skipped: no okb_export / okb_export/br1");
        return;
    };
    let with = with_br1(&assets, &br1, "br2_fixture_team_br1");
    let a = crate::assets::Assets::new(&with.0);
    let r = Replay::decode(include_bytes!("../fixtures/team_br1_menus_v24.okbr")).expect("decodes");
    let t = r.header.team.expect("a team battle's header");
    let first = crate::assets::roster::BR1_FIRST;
    assert_eq!(
        (r.format, r.sim_version, r.header.start.br1),
        (FORMAT_TEAM, crate::fight::version::v::TEAM_CONFIG, crate::assets::manifest::br1_install_id(&a))
    );
    assert_eq!((t.mode, t.chr, t.stage, t.cpu), (crate::fight::team::TeamMode::Vs2, [first, first + 1, first + 2, 3], 25, [false, true, false, true]));
    let (mut screens, mut config, mut mapped, mut br1_hit) = (Vec::new(), false, false, false);
    let sum = simulate(&a, r.clone(), |s| {
        if let Some(f) = &s.front {
            let d = f.describe();
            let d = d.split([' ', '[']).take(2).collect::<Vec<_>>().join(" ");
            if screens.last() != Some(&d) {
                screens.push(d);
            }
        }
        config |= s.team_config.is_some();
        if let Some(t) = &s.team {
            mapped |= s.team_config.is_none()
                && t.buttons[0] != crate::assets::game_tables::default_buttons()
                && t.buttons[1] == crate::assets::game_tables::default_buttons();
            br1_hit |= t.br1.is_some() && (0..4).any(|k| t.fighters[k].o_ac > 0);
        }
    })
    .expect("in sync");
    assert_eq!(sum.frames, r.header.frames);
    for want in ["mode menu", "team mode", "team select", "team load", "team fight", "team result"] {
        assert!(screens.iter().any(|d| d.starts_with(want)), "{want}: {screens:?}");
    }
    assert!(config && mapped, "the button config opened {config}, P1's map changed {mapped}");
    assert!(br1_hit, "a fighter was hit");
    let want: Vec<&str> = include_str!("../fixtures/team_br1_menus_v24_hashes.txt").lines().collect();
    assert_eq!(team_trace(&a, TEAM_BR1, r.clone()), want, "the battle's trace");
    let e = simulate(&crate::assets::Assets::new(&without_br1(&assets, "br2_fixture_no_br1").0), r, |_| {}).err().unwrap_or_default();
    assert!(e.contains("Bloody Roar 1"), "without the data: {e}");
}

/// TEAM ARCADE's fixture (engine addition, A4): a run reached through the menus at simulation v26.
const TEAM_ARCADE: &str = "team_arcade_v26";

/// The pads of `TEAM_ARCADE`'s session after the step that left `s` (pads 1 / 2, pads 3 / 4), from the title: TEAM
/// BATTLE (DOWN ×6), TEAM ARCADE (DOWN ×2 on the mode choice). Before the first continue pad 3 joins the arcade select
/// (START), P1 takes □ and P3 ×; nobody fights, the battle is lost, and P1's START in the countdown continues. Then
/// pad 3 leaves the select (△), P1 picks its fighter and the CPU partner's (□), and mashes □ through the retry.
fn team_arcade_pads(k: u32, s: &GameState) -> ([u16; 2], [u16; 2]) {
    use crate::fight::input::pad::{CROSS, DOWN, SQUARE, START, TRIANGLE};
    if k < 150 {
        return (team_ffa_pads(k, false).0, [0; 2]);
    }
    let Some(f) = &s.front else { return ([0; 2], [0; 2]) };
    let continues = f.team_arcade_info().map_or(0, |i| i.2);
    let partner = f.team_slot_ports().is_some_and(|p| p[2].is_some());
    if let Some(t) = &s.team {
        let p1 = match continues {
            0 if t.round.state == 0xF && t.frame.is_multiple_of(30) => START,
            1.. if t.frame % 12 < 2 => SQUARE,
            _ => 0,
        };
        return ([p1, 0], [0; 2]);
    }
    let d = f.describe();
    if let Some(row) = d.strip_prefix("team mode ").and_then(|r| r.split(' ').next()).and_then(|r| r.parse::<u8>().ok()) {
        let p1 = match row {
            _ if !k.is_multiple_of(10) => 0,
            0 | 1 => DOWN,
            _ => CROSS,
        };
        return ([p1, 0], [0; 2]);
    }
    if !d.starts_with("team select") {
        return ([0; 2], [0; 2]);
    }
    match (continues, partner) {
        (0, false) => ([0; 2], [if k.is_multiple_of(20) { START } else { 0 }, 0]),
        (0, true) => ([if k.is_multiple_of(40) { SQUARE } else { 0 }, 0], [if k % 40 == 20 { CROSS } else { 0 }, 0]),
        (_, true) => ([0; 2], [if k % 20 == 10 { TRIANGLE } else { 0 }, 0]),
        (_, false) => ([if k.is_multiple_of(40) { SQUARE } else { 0 }, 0], [0; 2]),
    }
}

/// Writes `fixtures/team_arcade_v26.okbr` and its trace `fixtures/team_arcade_v26_br2r_hashes.txt` (run by hand after an
/// intended change of TEAM ARCADE's menus, state or logic: `cargo test --release team_arcade_fixture_record --
/// --ignored`, with the export beside the engine), as `team_ffa_fixture_record`: the file of the retry's battle (the
/// whole session, its header the run's stage 1 with one continue).
#[test]
#[ignore]
fn team_arcade_fixture_record() {
    use crate::session::{AssetStore, Request, new_state};
    use std::sync::Arc;
    let a = export().expect("okb_export");
    // (the fixture is simulation v26's, whatever the build's current version)
    let start = SessionStart { seed: 26, entry: Entry::Title, sim_version: crate::fight::version::v::TEAM_ARCADE, ..Default::default() };
    let prefs = Prefs::default();
    let front = Arc::new(pollster::block_on(crate::frontend::load(&a)).expect("front data"));
    let mut store = AssetStore { front: Some(front), ..Default::default() };
    let mut s = new_state(&start, &store.assets(prefs), &mut Out::default());
    let mut rec = Recorder::new(&start, prefs, [0, 0]);
    let (mut flags, mut end, mut saved) = (0u8, None, None);
    for k in 0..80_000u32 {
        let (pad, ext) = team_arcade_pads(k, &s);
        let i = FrameInput { pad, ext, flags, ..Default::default() };
        flags = 0;
        let mut out = Out::default();
        step(&mut s, &store.assets(prefs), i, &[], &mut out);
        rec.step(i, &[], prefs, &s).expect("recorded");
        for r in out.requests {
            match r {
                Request::Load(_) => panic!("a match"),
                Request::LoadTeam(spec) => {
                    rec.match_load(&s);
                    let chr = spec.chr.map(crate::assets::roster::code);
                    let stage = crate::fight::fight_data::stage_code(spec.stage);
                    let data = pollster::block_on(crate::fight::fight_data::load_team(&a, chr, spec.costume, stage, s.sim_version)).expect("load");
                    store.fight = Some(Arc::new(data));
                    flags |= FrameInput::LOADED;
                    eprintln!("frame {}: load {spec:?}", s.frame);
                }
                Request::Fight => rec.match_start(&s),
                Request::TeamOver { outcome, rounds } => {
                    store.fight = None;
                    eprintln!("frame {}: team battle over {outcome:?} {rounds:?}", s.frame);
                    let r = rec.team_end(&s, outcome, rounds, 1_700_000_000);
                    if outcome != Outcome::Continue {
                        (saved, end) = (r, Some(k));
                    }
                }
                _ => {}
            }
        }
        store.prune(&s);
        if end.is_some_and(|e| k >= e + 60) {
            break;
        }
    }
    let r = saved.expect("the retry ended");
    assert_eq!((r.format, r.header.outcome, r.header.team_arcade.map(|a| (a.stage, a.continues))), (FORMAT_TEAM, Some(Outcome::Winner(0)), Some((1, 1))));
    let path = format!("{}/src/session/fixtures/{TEAM_ARCADE}.okbr", env!("CARGO_MANIFEST_DIR"));
    std::fs::write(&path, r.encode()).expect("written");
    eprintln!("{path}: {} steps", r.header.frames);
    let lines = team_trace(&a, TEAM_ARCADE, r);
    let path = format!("{}/src/session/fixtures/{TEAM_ARCADE}_br2r_hashes.txt", env!("CARGO_MANIFEST_DIR"));
    std::fs::write(&path, lines.join("\n") + "\n").expect("written");
}

/// `fixtures/team_arcade_v26.okbr` (`team_arcade_fixture_record`): a FORMAT 2 file of simulation v26 whose header holds
/// a TEAM ARCADE retry (stage 1 of 5 after one continue, won); it plays in sync through the menus (the mode choice's
/// TEAM ARCADE, pad 3 joining the arcade select), the lost battle's countdown and continue, the select again (pad 3
/// leaving, a CPU partner), the retry won and the next stage loading, the battles step by step as recorded
/// (`team_trace`); the run's play time counts the battles' fighting frames, its rounds won / lost the rounds the
/// battles ended with. A save state taken mid-retry (the runtime snapshot and the `.okbs` file) plays on to the same
/// end, and seeking back in the playback to it too.
#[test]
fn team_arcade_fixture() {
    let Some(a) = export() else {
        eprintln!("skipped: no okb_export");
        return;
    };
    let r = Replay::decode(include_bytes!("../fixtures/team_arcade_v26.okbr")).expect("decodes");
    let t = r.header.team.expect("a team battle's header");
    assert_eq!((r.format, r.sim_version, r.header.mode.as_str()), (FORMAT_TEAM, crate::fight::version::v::TEAM_ARCADE, "TEAM ARCADE"));
    assert_eq!((t.mode, t.cpu, r.header.outcome), (crate::fight::team::TeamMode::Vs2, [false, true, true, true], Some(Outcome::Winner(0))));
    assert_eq!(r.header.team_arcade, Some(TeamArcadeInfo { stage: 1, stages: 5, continues: 1 }));
    let (mut screens, mut countdown, mut play, mut fighting) = (Vec::new(), false, 0, 0);
    // the rounds each battle ended with (its last frame's `round.wins`, which `end_team` passes on) and the ladder's
    // won / lost (`team_ladder_sync` bytes 8..12)
    let (mut wins, mut battles, mut ladder) = (None, Vec::new(), [0u16; 2]);
    let sum = simulate(&a, r.clone(), |s| {
        if let Some(f) = &s.front {
            let d = f.describe();
            let d = d.split([' ', '[']).take(2).collect::<Vec<_>>().join(" ");
            if screens.last() != Some(&d) {
                screens.push(d);
            }
            play = f.sync_key()[3];
            if let Some(l) = f.team_ladder_sync() {
                ladder = [u16::from_le_bytes([l[8], l[9]]), u16::from_le_bytes([l[10], l[11]])];
            }
        }
        if let Some(t) = &s.team {
            countdown |= t.round.state == 0xF;
            fighting += (t.round.state == 4) as u32;
            wins = Some(t.round.wins);
        } else if let Some(w) = wins.take() {
            battles.push(w);
        }
    })
    .expect("in sync");
    assert_eq!(sum.frames, r.header.frames);
    let want = ["mode menu", "team mode", "team select", "team load", "team fight", "team select", "team load", "team fight", "team load"];
    assert_eq!(screens.iter().skip_while(|d| !d.starts_with("mode menu")).map(|d| d.as_str()).collect::<Vec<_>>(), want);
    assert!(countdown, "the lost battle's countdown");
    assert!(play > 0 && play <= fighting, "play time {play} of {fighting} fighting frames");
    // the lost battle 0-2, the retry 2-0: the ladder counts the rounds played
    assert_eq!(battles, [[0, 2, 0, 0], [2, 0, 0, 0]], "the battles' rounds");
    assert_eq!(ladder, [2, 2], "rounds won / lost");
    let want: Vec<&str> = include_str!("../fixtures/team_arcade_v26_br2r_hashes.txt").lines().collect();
    assert_eq!(team_trace(&a, TEAM_ARCADE, r.clone()), want, "the battles' trace");
    // a save state mid-retry
    let mut h = pollster::block_on(Headless::new(&a, r.clone())).expect("opens");
    let retry = |s: &GameState| s.front.as_ref().and_then(|f| f.team_arcade_info()).is_some_and(|i| i.2 == 1);
    while !retry(&h.s) || h.s.team.as_ref().is_none_or(|t| t.frame < 1500) {
        assert!(pollster::block_on(h.step()).expect("in sync"), "the retry reached frame 1500");
    }
    let key = h.key();
    let held = key.held.clone();
    let snap = crate::session::save_state(&h.s);
    assert_eq!(hash(&crate::session::load_state(&snap, &held.assets()).expect("the snapshot loads")), hash(&h.s));
    let file = crate::session::save_file(&h.s);
    let from_file = crate::session::load_file(&file, &held.assets()).expect("the .okbs file loads");
    assert_eq!(hash(&from_file), hash(&h.s));
    while pollster::block_on(h.step()).expect("in sync") {}
    let end = hash(&h.s);
    h.seek(&key).expect("seek back");
    while pollster::block_on(h.step()).expect("in sync") {}
    assert_eq!(hash(&h.s), end);
}

/// (TEAM ARCADE A4) The header's `team_arcade` comes back from the file (CBOR, so older builds skip it) and is not
/// written without a run (a team battle's header keeps its bytes); it is refused before v26, without a 2 VS 2 battle,
/// or outside a ladder's bounds; so is the debug start OKB_TEAM_ARCADE (`Entry::TeamArcade`) before v26 or outside its bounds.
#[test]
fn team_arcade_header() {
    let r = Replay::decode(include_bytes!("../fixtures/team_arcade_v26.okbr")).expect("decodes");
    let b = r.encode();
    assert_eq!(Replay::decode(&b).expect("decodes").header.team_arcade, r.header.team_arcade);
    let with = |f: &dyn Fn(&mut Replay)| {
        let mut x = r.clone();
        f(&mut x);
        Replay::decode(&x.encode())
    };
    let plain = with(&|x| x.header.team_arcade = None).expect("a team battle's header");
    assert!(plain.encode().len() < b.len());
    let ffa = crate::frontend::TeamSpec { mode: crate::fight::team::TeamMode::Ffa, ..r.header.team.expect("a team battle") };
    let info = |stage, stages| Some(TeamArcadeInfo { stage, stages, continues: 0 });
    for (what, f) in [
        ("v25", &(|x: &mut Replay| x.sim_version = crate::fight::version::v::TEAM_ARCADE - 1) as &dyn Fn(&mut Replay)),
        ("no team battle", &|x: &mut Replay| x.header.team = None),
        ("free for all", &|x: &mut Replay| x.header.team = Some(ffa)),
        ("stage 0", &|x: &mut Replay| x.header.team_arcade = info(0, 5)),
        ("stage 6 of 5", &|x: &mut Replay| x.header.team_arcade = info(6, 5)),
        ("7 stages", &|x: &mut Replay| x.header.team_arcade = info(1, 7)),
        ("start stage 0", &|x: &mut Replay| x.header.start.entry = Entry::TeamArcade { stage: 0, chr: [0, 1], partner: false }),
        ("start BR1 fighter", &|x: &mut Replay| x.header.start.entry = Entry::TeamArcade { stage: 1, chr: [11, 1], partner: false }),
        ("start before v26", &|x: &mut Replay| {
            (x.sim_version, x.header.team_arcade) = (crate::fight::version::v::TEAM_ARCADE - 1, None);
            x.header.start.entry = Entry::TeamArcade { stage: 1, chr: [0, 1], partner: false };
        }),
    ] {
        assert!(with(f).is_err(), "{what}");
    }
    assert!(with(&|x| x.header.team_arcade = info(6, 6)).is_ok());
    assert!(with(&|x| (x.sim_version, x.header.team_arcade) = (crate::fight::version::v::TEAM_ARCADE - 1, None)).is_ok(), "a v25 team battle");
    assert!(with(&|x| x.header.start.entry = Entry::TeamArcade { stage: 6, chr: [10, 0], partner: true }).is_ok());
}
