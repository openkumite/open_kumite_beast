//! Headless front-end flow test: the title, select, VS screen and the routing after matches.
//!
//! Headless front-end flow test: drives the title, select, VS screen and the routing after matches.
//! `cargo run --release --bin fetest -- ../okb_export`
//! ROLLBACK=N: the rollback test (`session::rollback`; its report to stderr or ROLLBACK_LOG, exit 3 on a mismatch).
//! STEPLOG=file: the run's step log (`session::determinism_trace`), for `replay --trace` on another target.
//! SIM_VERSION=n: run simulation version n (an older one's logic, as its replays; default the newest).
//! OKB_LOCKED=1: the game's own progression (the save's characters and costumes; default all selectable from v5);
//! FE_UNLOCK=1: everything open from the start (characters, costumes, movies, pictures: the F11 debug unlock).
//! FESCRIPT2=script: player 2's pad (default: □ on the select every 40 frames); FESCRIPT3 / FESCRIPT4: pads 3 / 4 (the
//! team battle's screens; WINS=Wn its winner team or player, D a DRAW GAME, C a TEAM ARCADE continue, G its GAME OVER,
//! anything else QUIT). FESHOTS=n,…: screenshots of the front end
//! (`FeShots`). FELOAD=1: the matches asked for loaded and reported (`fe_load`). OKB_BR1=dir: the imported game's install (as
//! the app: the select's imported row and stages from simulation v19).

use open_kumite_beast::fight::sim::Outcome;
use open_kumite_beast::fight::team::TeamOutcome;
use open_kumite_beast::frontend::options::Prefs;
use open_kumite_beast::session::determinism_trace::{self, StepLog};
use open_kumite_beast::session::rollback::{Held, Rollback};
use open_kumite_beast::session::{self, FrameInput, Request};

/// What a frame of the test does to the state, in order (the rollback test's log: `resim` does it again).
enum Op {
    Step(FrameInput),
    /// `session::inputs` with these flags right after the step (a load or fetch completing at once), with the
    /// data then when the store changed (the fetched files) and the preferences then (a `Request::Prefs` earlier in
    /// the step's requests changed them)
    Inputs(u8, Option<Box<Held>>, Prefs),
    /// the fight's result from WINS: its play time and `session::end_match`, with the preferences then
    Fight(Outcome, Prefs),
    /// a team battle's result from WINS (engine addition): `session::end_team`, with the preferences then
    Team(TeamOutcome, Prefs),
    /// the card writes made after the step taken (`Card::take_dirty`)
    TakeDirty,
}

fn main() {
    let root = std::env::args().nth(1).unwrap_or("../okb_export".into());
    // keep the test away from the player's save
    let dir = std::env::temp_dir().join("okb_fetest");
    // SAFETY: the first statement after reading the arguments, before this process starts any thread
    #[allow(unsafe_code)]
    unsafe {
        std::env::set_var("OKB_SAVE_DIR", &dir)
    };
    let assets = open_kumite_beast::assets::Assets::new(root);
    let data = pollster::block_on(open_kumite_beast::frontend::load(&assets)).expect("front data");
    let mut audio = open_kumite_beast::audio::Audio::new();
    audio.load_front_banks(&data);
    let mut store = session::AssetStore { front: Some(std::sync::Arc::new(data)), ..Default::default() };
    let mut prefs = open_kumite_beast::frontend::options::Prefs::default();
    let mut hashes = session::HashTrace::from_env("STATEHASH");
    // OKB_LOCKED: the game's own progression (as the app); FE_UNLOCK: everything open from the start (as F11)
    let mut start = open_kumite_beast::session_start(open_kumite_beast::base::env::var("LOCKED").is_ok(), &assets);
    start.unlock = std::env::var("FE_UNLOCK").is_ok();
    match session::sim_version_env() {
        Ok(v) => start.sim_version = v.unwrap_or(start.sim_version),
        Err(e) => {
            eprintln!("{e}");
            std::process::exit(2)
        }
    }
    let mut out = session::Out::default();
    let mut s = session::new_state(&start, &store.assets(prefs), &mut out);
    audio.apply(&mut out.audio);
    let mut rollback = Rollback::<Vec<Op>>::from_env("ROLLBACK");
    // a logged frame again (muted; its output thrown away)
    let resim = |s: &mut session::GameState, a: &session::Assets, ops: &Vec<Op>| {
        for op in ops {
            match op {
                Op::Step(i) => session::step(s, a, *i, &[], &mut session::Out::default()),
                Op::Inputs(flags, held, prefs) => {
                    let a = session::Assets { prefs: *prefs, ..held.as_deref().map_or(*a, |h| h.assets()) };
                    session::inputs(s, &a, *flags, &mut session::Out::default());
                }
                Op::Fight(w, prefs) => {
                    if let Some(f) = &mut s.front {
                        f.add_play_time(2400);
                    }
                    session::end_match(s, &session::Assets { prefs: *prefs, ..*a }, *w, [0, 0], false, &mut session::Out::default());
                }
                Op::Team(w, prefs) => {
                    team_time(s);
                    session::end_team(s, &session::Assets { prefs: *prefs, ..*a }, *w, [0; 4], &mut session::Out::default())
                }
                Op::TakeDirty => {
                    if let Some(f) = &mut s.front {
                        f.save.memory_card.take_dirty();
                    }
                }
            }
        }
    };
    if let Some(r) = &mut rollback {
        r.start(&s);
    }
    let steplog_path = std::env::var("STEPLOG").ok().filter(|v| !v.is_empty());
    let mut steplog = steplog_path.as_ref().map(|_| StepLog::new(determinism_trace::Data::Front, prefs, &s));
    let mut log = |op: determinism_trace::Op| {
        if let Some(l) = &mut steplog {
            l.push(op);
        }
    };
    let script = open_kumite_beast::fight::input_script::parse_script(&std::env::var("FESCRIPT").unwrap_or("40:,1:S,29:,1:K,29:,1:K,30:".into()));
    // FESCRIPT2: player 2's pad by script (then no automatic □ on the select)
    let script2 = std::env::var("FESCRIPT2").ok().map(|v| open_kumite_beast::fight::input_script::parse_script(&v));
    // FESCRIPT3 / FESCRIPT4: pads 3 / 4 (engine addition: the team battle's screens)
    let script34 = ["FESCRIPT3", "FESCRIPT4"].map(|k| open_kumite_beast::fight::input_script::parse_script(&std::env::var(k).unwrap_or_default()));
    #[cfg(not(target_arch = "wasm32"))]
    let mut shots = FeShots::from_env();
    let frames: usize = std::env::var("FRAMES").ok().and_then(|v| v.parse().ok()).unwrap_or(4000);
    let mut fights = 0;
    let mut frame = 0usize;
    let mut log_last = String::new();
    // the card store's result for the next step; a card write not stored yet
    let mut flags = 0u8;
    let mut dirty = false;
    // winners to feed: P1 wins twice, loses once, continues, then wins
    // W0 / W1 = winner P1 / P2, C = continue accepted, G = game over, D = a team battle's DRAW GAME (T7)
    let wins = std::env::var("WINS").unwrap_or("W0,W0,C,W0".into());
    let mut results = wins
        .split(',')
        .map(|x| match x {
            "C" => Outcome::Continue,
            "G" => Outcome::GameOver,
            "D" => Outcome::Draw,
            x if x.starts_with('X') => Outcome::Challenger(x[1..].parse().unwrap_or(1)),
            w => Outcome::Winner(w[1..].parse().unwrap_or(0)),
        })
        .collect::<Vec<_>>()
        .into_iter();
    let front = |s: &session::GameState| s.front.as_ref().expect("front end").describe();
    // the step's card write to the disk (as the app); the result goes into the next step's input
    let store_card = |s: &session::GameState, flags: &mut u8| {
        if let Some(file) = s.front.as_ref().and_then(|f| f.save.memory_card.file()) {
            let ok = open_kumite_beast::platform::storage::store_card(file);
            *flags |= FrameInput::CARD_STORED | if ok { 0 } else { FrameInput::CARD_FAILED };
        }
    };
    'run: while frame < frames {
        let mut pad = script.get(frame).copied().unwrap_or(0);
        let mut p2 = 0u16;
        let d = front(&s);
        if (d.starts_with("name entry") || d.starts_with("CONGRAT")) && frame.is_multiple_of(20) {
            pad = 0x40; // × = enter a letter / skip
        }
        if d.starts_with("select") && frame.is_multiple_of(40) {
            if std::env::var("P1_MANUAL").is_err() {
                pad = 0x80; // □ = confirm
            }
            p2 = 0x80;
        }
        if let Some(s2) = &script2 {
            p2 = s2.get(frame).copied().unwrap_or(0);
        }
        audio.log(|| format!("step fe {frame}")); // debug: OKB_SOUNDLOG frame mark
        let ext = script34.each_ref().map(|s| s.get(frame).copied().unwrap_or(0));
        let i = FrameInput {
            pad: [pad, if script2.is_some() || frame.is_multiple_of(2) { p2 } else { 0 }],
            forced: [0; 2],
            flags: std::mem::take(&mut flags),
            ext,
            ..Default::default()
        };
        let held = rollback.is_some().then(|| Held::of(&store, prefs));
        let mut ops = vec![Op::Step(i)];
        let mut out = session::Out::default();
        session::step(&mut s, &store.assets(prefs), i, &[], &mut out);
        log(determinism_trace::Op::Step(i, Vec::new()));
        audio.apply(&mut out.audio);
        frame += 1;
        dirty = out.card_dirty;
        // the loads complete within the frame (blocking, before the next step): `session::inputs` at once
        for r in out.requests {
            match r {
                Request::Load(spec) => {
                    println!("{frame:5}: load {spec:?}");
                    fe_load(&assets, &spec, s.sim_version);
                    session::inputs(&mut s, &store.assets(prefs), FrameInput::LOADED, &mut session::Out::default());
                    ops.push(Op::Inputs(FrameInput::LOADED, None, prefs));
                    log(determinism_trace::Op::Inputs(FrameInput::LOADED));
                }
                Request::LoadTeam(spec) => {
                    println!("{frame:5}: load team {spec:?}");
                    fe_load_team(&assets, &spec, s.sim_version);
                    session::inputs(&mut s, &store.assets(prefs), FrameInput::LOADED, &mut session::Out::default());
                    ops.push(Op::Inputs(FrameInput::LOADED, None, prefs));
                    log(determinism_trace::Op::Inputs(FrameInput::LOADED));
                }
                // a team battle (engine addition): no battle runs, its result comes from WINS (Wn: team / player n won, D a
                // DRAW GAME, C / G a TEAM ARCADE continue / GAME OVER, anything else QUIT)
                Request::Fight if s.team_spec.is_some() => {
                    fights += 1;
                    let Some(w) = results.next() else { break 'run };
                    let w = match w {
                        Outcome::Winner(t) => TeamOutcome::Winner(t),
                        Outcome::Draw => TeamOutcome::Draw,
                        Outcome::Continue => TeamOutcome::Continue,
                        Outcome::GameOver => TeamOutcome::GameOver,
                        _ => TeamOutcome::Quit,
                    };
                    println!("{frame:5}: team battle {fights} → {w:?}");
                    team_time(&mut s);
                    let mut over = session::Out::default();
                    session::end_team(&mut s, &store.assets(prefs), w, [0; 4], &mut over);
                    ops.push(Op::Team(w, prefs));
                    log(determinism_trace::Op::EndTeam(w));
                    audio.apply(&mut over.audio);
                    for r in over.requests {
                        if let Request::LoadTeam(spec) = r {
                            println!("{frame:5}: load team {spec:?}");
                            session::inputs(&mut s, &store.assets(prefs), FrameInput::LOADED, &mut session::Out::default());
                            ops.push(Op::Inputs(FrameInput::LOADED, None, prefs));
                            log(determinism_trace::Op::Inputs(FrameInput::LOADED));
                        }
                    }
                }
                Request::Fight => {
                    // no fight runs: its result comes from WINS
                    fights += 1;
                    let Some(w) = results.next() else { break 'run };
                    let f = s.front.as_mut().expect("front end");
                    println!("{frame:5}: fight {fights} → {w:?} (continue allowed {})", f.continue_allowed());
                    f.add_play_time(2400);
                    let mut over = session::Out::default();
                    session::end_match(&mut s, &store.assets(prefs), w, [0, 0], false, &mut over);
                    ops.push(Op::Fight(w, prefs));
                    log(determinism_trace::Op::EndMatch(w));
                    audio.apply(&mut over.audio);
                    for r in over.requests {
                        if let Request::Load(spec) = r {
                            println!("{frame:5}: load {spec:?}");
                            fe_load(&assets, &spec, s.sim_version);
                            session::inputs(&mut s, &store.assets(prefs), FrameInput::LOADED, &mut session::Out::default());
                            ops.push(Op::Inputs(FrameInput::LOADED, None, prefs));
                            log(determinism_trace::Op::Inputs(FrameInput::LOADED));
                        }
                    }
                }
                Request::Music(t) => println!("{frame:5}: music {t:?}"),
                Request::Fetch(names) => {
                    println!("{frame:5}: fetch {names:?}");
                    log(determinism_trace::Op::Fetch(names.clone()));
                    for n in names {
                        store.files.remove(&n);
                        if let Some(Ok(b)) = assets.route(&n).map(|(a, rel)| pollster::block_on(a.load(rel))) {
                            store.files.insert(n, b.into());
                        }
                    }
                    let mut fetched = session::Out::default();
                    session::inputs(&mut s, &store.assets(prefs), FrameInput::FETCHED, &mut fetched);
                    ops.push(Op::Inputs(FrameInput::FETCHED, rollback.is_some().then(|| Box::new(Held::of(&store, prefs))), prefs));
                    log(determinism_trace::Op::Inputs(FrameInput::FETCHED));
                    audio.apply(&mut fetched.audio);
                }
                Request::Prefs(p) => {
                    println!("{frame:5}: prefs {p:?}");
                    prefs = p; // applied and saved, as the app
                    log(determinism_trace::Op::Prefs(p));
                }
                Request::Exit => {
                    println!("{frame:5}: exit");
                    break 'run;
                }
                // the replay list is the app's: the session stays on the mode menu
                Request::Replays => println!("{frame:5}: replays"),
                Request::Network => println!("{frame:5}: network"),
                Request::ChangeData => println!("{frame:5}: change data"),
                Request::FightOver { .. } | Request::TeamOver { .. } => {}
            }
        }
        store.prune(&s);
        hashes.log(&s);
        // the writes made after the step (the match result) count with the step's
        dirty |= s.front.as_mut().is_some_and(|f| f.save.memory_card.take_dirty());
        ops.push(Op::TakeDirty);
        log(determinism_trace::Op::Prune);
        log(determinism_trace::Op::Hash);
        log(determinism_trace::Op::TakeDirty);
        if let (Some(r), Some(held)) = (&mut rollback, held) {
            r.check(&s, held, ops, resim);
        }
        if std::mem::take(&mut dirty) {
            store_card(&s, &mut flags);
        }
        let d = front(&s);
        #[cfg(not(target_arch = "wasm32"))]
        if let Some(sh) = &mut shots {
            sh.take(frame, &s, &store);
        }
        if std::env::var("ALL").is_ok() && d != log_last || d.split(' ').next() != log_last.split(' ').next() {
            println!("{frame:5}: {d}");
            log_last = d;
        }
    }
    // a save of the last frame (the loop's breaks)
    if s.front.as_mut().is_some_and(|f| f.save.memory_card.take_dirty()) || dirty {
        store_card(&s, &mut flags);
    }
    if let (Some(path), Some(l)) = (&steplog_path, steplog)
        && let Err(e) = std::fs::write(path, l.encode())
    {
        eprintln!("STEPLOG: {path}: {e}");
    }
    if rollback.is_some_and(|mut r| r.finish() > 0) {
        std::process::exit(3);
    }
}

/// A team battle not run (WINS): in a TEAM ARCADE run its play time as a match's (2400 frames).
fn team_time(s: &mut session::GameState) {
    if let Some(f) = &mut s.front
        && f.arcade_team().is_some()
    {
        f.add_play_time(2400);
    }
}

/// FESHOTS=n,m,… (SHOTDIR=dir, default the working directory; SHOTSIZE=WxH, default 640x480): the front end drawn after
/// those frames' steps through the windowless renderer (as `replay --video` draws it, 4:3), saved as `fe_<frame>.png` —
/// headless screenshots of the menus.
#[cfg(not(target_arch = "wasm32"))]
struct FeShots {
    r: open_kumite_beast::render::Renderer,
    vram: open_kumite_beast::present::front_vram::VramView,
    frames: Vec<usize>,
    dir: std::path::PathBuf,
}

#[cfg(not(target_arch = "wasm32"))]
impl FeShots {
    fn from_env() -> Option<Self> {
        let frames: Vec<usize> = std::env::var("FESHOTS").ok()?.split(',').filter_map(|v| v.trim().parse().ok()).collect();
        let (w, h) = std::env::var("SHOTSIZE")
            .ok()
            .and_then(|v| v.split_once('x').and_then(|(a, b)| Some((a.parse().ok()?, b.parse().ok()?))))
            .unwrap_or((640u32, 480u32));
        let mut r = pollster::block_on(open_kumite_beast::render::Renderer::offscreen(w, h, 2)).unwrap_or_else(|e| {
            eprintln!("FESHOTS: {e}");
            std::process::exit(2)
        });
        r.set_fit(open_kumite_beast::output::layout::Fit::Classic);
        let dir = std::env::var("SHOTDIR").map(Into::into).unwrap_or_else(|_| ".".into());
        Some(Self { r, vram: Default::default(), frames, dir })
    }

    fn take(&mut self, frame: usize, s: &session::GameState, store: &session::AssetStore) {
        let (true, Some(f), Some(data)) = (self.frames.contains(&frame), &s.front, &store.front) else { return };
        // a playing movie's stream (its frame due now goes into the image), as the app's `VramView::step` follows it
        self.vram.follow(f, &store.files);
        if self.vram.sync(f, data, &store.files) {
            self.r.upload_vram(&self.vram.vram.words);
        }
        let prims = f.draw();
        let vp = self.r.viewport();
        self.r.ambient = open_kumite_beast::output::layout::ambient_split(&prims, vp);
        let quads = open_kumite_beast::output::layout::to_quads(&prims, self.r.viewport());
        self.r.frame_capture(&[], &quads, &[], [0.0, 0.0, 0.0], false);
        let [w, h] = self.r.viewport().win;
        let path = self.dir.join(format!("fe_{frame}.png"));
        let mut sink = |px: &[u8]| {
            let rgba: Vec<u8> = px.as_chunks::<4>().0.iter().flat_map(|c| [c[2], c[1], c[0], 255]).collect();
            image::save_buffer(&path, &rgba, w, h, image::ColorType::Rgba8).map_err(|e| e.to_string())
        };
        if let Err(e) = self.r.take_frames(0, &mut sink) {
            eprintln!("FESHOTS: {e}");
        }
    }
}

/// FELOAD=1: a match the front end asks for loaded as the app loads it (`fight_data::load_match` with its fighter and
/// stage codes, the imported game's install `Assets::br1` beside the data or OKB_BR1), its result printed: the menus'
/// choices reach a match that loads (the test's own result still comes from WINS).
fn fe_load(assets: &open_kumite_beast::assets::Assets, spec: &open_kumite_beast::frontend::MatchSpec, sim_version: u32) {
    if std::env::var("FELOAD").is_err() {
        return;
    }
    let chr = spec.chr.map(open_kumite_beast::assets::roster::code);
    let stage = open_kumite_beast::fight::fight_data::stage_code(spec.stage);
    match pollster::block_on(open_kumite_beast::fight::fight_data::load_match(assets, chr, spec.costume, stage, false, sim_version)) {
        Ok(l) => println!(
            "       loaded {} {} costumes {:?} stage {stage} ({}): imported sides {:?}, imported stage {:?}",
            chr[0],
            chr[1],
            spec.costume,
            l.stage_name,
            l.br1.iter().map(|b| b.as_ref().map(|b| b.res.code.clone())).collect::<Vec<_>>(),
            l.br1_stage.map(|b| b.g94)
        ),
        Err(e) => println!("       load failed: {e}"),
    }
}

/// FELOAD=1: a team battle the front end asks for loaded as the app loads it (`fight_data::load_team`), its result
/// printed (engine addition).
fn fe_load_team(assets: &open_kumite_beast::assets::Assets, spec: &open_kumite_beast::frontend::TeamSpec, sim_version: u32) {
    if std::env::var("FELOAD").is_err() {
        return;
    }
    let chr = spec.chr.map(open_kumite_beast::assets::roster::code);
    let stage = open_kumite_beast::fight::fight_data::stage_code(spec.stage);
    match pollster::block_on(open_kumite_beast::fight::fight_data::load_team(assets, chr, spec.costume, stage, sim_version)) {
        Ok(l) => println!("       loaded team {chr:?} costumes {:?} stage {stage} ({})", spec.costume, l.stage_name),
        Err(e) => println!("       load failed: {e}"),
    }
}
