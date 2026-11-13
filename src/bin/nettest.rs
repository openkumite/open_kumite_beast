//! Headless netplay test: a host and a joiner run one whole session in lockstep.
//!
//! Headless netplay test (game-state stages 8 and 9): a host and a joiner, each in a thread of its own with its own
//! data, state and replay recorder, connect (`net::Connect`, polled every 2 ms) and run one session in lockstep over
//! loopback UDP (127.0.0.1) or, with NETTEST_SIGNAL, a room over WebRTC (feature `netplay-webrtc`; the host's thread
//! enters the room first, so it must come out as player 1) at 60 Hz real time, from the title through the mode menu
//! (VS), the character select and the VS screen into a fight, to the match's end.
//! Both players are bots that read the shared state (their pads are a function of it: the host's START, VS, both
//! picking with □, then walking in and attacking; P2 pauses once and resumes). The host's card: a 20 s TIME LIMIT,
//! one round (MATCH POINT 1); the joiner's own map swaps □ and × (it must arrive as PLAYER 2's).
//! `nettest ../okb_export [OUT_DIR]` — OUT_DIR (default $TMPDIR/okb_nettest) gets both match replays
//! (`p1.okbr`, `p2.okbr`) and, on a desync, the dumps (`desync_<frame>_p<n>.*`).
//! env: OKB_NETSIM=lat_ms,loss_pct[,jitter_ms] (both directions), NETTEST_PORT=n (47123), NETTEST_SIGNAL=ws://… (a
//! matchbox signaling server: a room instead of UDP), NETTEST_ROOM=CODE (default a fresh one), NETTEST_DELAY=n (else from
//! the round trip), NETTEST_LAG=h,j (ticks each side's loads take to report done; 2,30), NETTEST_DESYNC=frame (the
//! host alone gets a cheat at that frame: the run passes when both sides stop with a desync), NETTEST_LOCKED=h,j
//! (`SessionStart::locked` each thread offers, 0 or 1; default 0,0: the host's must be the session's on both sides),
//! NETTEST_STAGE=NAME (the host turns the stage select with L1 to that row, e.g. EXTRA3, before the characters are
//! picked; both sides must fight there), NETTEST_BR1=1 (both offer the imported game's install, OKB_BR1=dir or the data
//! folder's `br1`, and both pick a fighter of the select's imported row; with NETTEST_STAGE an imported stage),
//! NETTEST_WATCH=1 (the host picks WATCH: both sides CPU, with NETTEST_BR1 imported fighters' AI, the import S10).
//! Exit 0 on a pass: the match finished on both sides, every checkpoint matched, the replays are byte-equal and both
//! re-simulate in sync (or, NETTEST_DESYNC, the desync was detected on both); 1 otherwise.

#[cfg(not(target_arch = "wasm32"))]
fn main() {
    desktop::main();
}

#[cfg(target_arch = "wasm32")]
fn main() {}

/// UDP sockets and threads: the desktop's.
#[cfg(not(target_arch = "wasm32"))]
mod desktop {
    use open_kumite_beast::fight::input::pad::{CIRCLE, CROSS, DOWN, L1, LEFT, RIGHT, SQUARE, START};
    use open_kumite_beast::frontend::options::Prefs;
    use open_kumite_beast::net::{self, Connect, Netplay, Offer, Setup, Stop, udp};
    use open_kumite_beast::session::{self, Cheat, Entry, FrameInput, GameState, Out, Request, SessionStart, replay};
    use std::sync::Arc;
    use std::time::{Duration, Instant};

    /// The joiner's own P1 map: □ and × swapped.
    const JOIN_BUTTONS: [u16; 8] = [0x40, 0x80, 0x20, 0xC0, 0x408, 0x02, 0xE0, 0x01];
    /// Frames run after the match's end before the host says Bye.
    const AFTER: u32 = 120;
    /// No match end by this frame: a failure.
    const MAX_FRAMES: u32 = 12_000;
    /// No session by then (seconds): a failure.
    const CONNECT_TIMEOUT: f64 = 60.0;

    #[derive(Clone)]
    struct Cfg {
        root: String,
        out: std::path::PathBuf,
        port: u16,
        /// a room (NETTEST_SIGNAL, NETTEST_ROOM)
        room: Option<(String, String)>,
        sim: Option<net::link::SimNet>,
        delay: Option<u32>,
        lag: [u32; 2],
        desync: Option<u32>,
        /// the `SessionStart::locked` each thread offers (NETTEST_LOCKED)
        locked: [bool; 2],
        /// the stage select row the host picks (NETTEST_STAGE)
        stage: Option<String>,
        /// both threads offer the imported game's install of the data (NETTEST_BR1=1, with OKB_BR1=dir or the data folder's
        /// `br1`) and both players pick an imported fighter from the select's imported row
        br1: bool,
        /// the host picks WATCH instead of VS (NETTEST_WATCH=1): both sides CPU (with NETTEST_BR1 imported fighters' AI)
        watch: bool,
    }

    /// One side's result.
    #[derive(Default)]
    struct Report {
        port: usize,
        delay: u32,
        frames: u32,
        stop: Option<Stop>,
        stats: net::Stats,
        /// the round trip at the end (seconds)
        rtt: Option<f64>,
        replay: Option<Vec<u8>>,
        /// PLAYER 2's map in the fight (the joiner's)
        p2_buttons: Option<[u16; 8]>,
        /// (frame, screen) at every screen change
        screens: Vec<(u32, String)>,
        /// seconds between consecutive steps
        gaps: Vec<f64>,
        dumps: Vec<String>,
        /// the agreed session's `SessionStart::locked`
        locked: bool,
        /// the selectable characters and fourth costumes at the character select (`Front::selectable`)
        selectable: Option<(u32, u32)>,
        /// the stage of every match, in order (`Request::Load`)
        stages: Vec<u8>,
    }

    pub fn main() {
        let args: Vec<String> = std::env::args().skip(1).collect();
        let var = |k: &str| std::env::var(k).ok().filter(|v| !v.is_empty());
        let cfg = Cfg {
            root: args.first().cloned().unwrap_or("../okb_export".into()),
            out: args.get(1).map(Into::into).unwrap_or_else(|| std::env::temp_dir().join("okb_nettest")),
            port: var("NETTEST_PORT").and_then(|v| v.parse().ok()).unwrap_or(47123),
            room: var("NETTEST_SIGNAL").map(|s| (s, var("NETTEST_ROOM").unwrap_or_else(|| format!("NT{}", open_kumite_beast::clock_seed() % 1_000_000)))),
            sim: match net::link::SimNet::from_env() {
                Ok(s) => s,
                Err(e) => {
                    eprintln!("{e}");
                    std::process::exit(2);
                }
            },
            delay: var("NETTEST_DELAY").and_then(|v| v.parse().ok()),
            lag: var("NETTEST_LAG").and_then(|v| v.split_once(',').and_then(|(a, b)| Some([a.parse().ok()?, b.parse().ok()?]))).unwrap_or([2, 30]),
            desync: var("NETTEST_DESYNC").and_then(|v| v.parse().ok()),
            locked: var("NETTEST_LOCKED").and_then(|v| v.split_once(',').map(|(a, b)| [a == "1", b == "1"])).unwrap_or([false; 2]),
            stage: var("NETTEST_STAGE"),
            br1: var("NETTEST_BR1").is_some_and(|v| v == "1"),
            watch: var("NETTEST_WATCH").is_some_and(|v| v == "1"),
        };
        // keep the test away from the player's save (the joiner reads none: its map is `JOIN_BUTTONS`)
        // SAFETY: still single-threaded here; the two sides' threads are spawned below
        #[allow(unsafe_code)]
        unsafe {
            std::env::set_var("OKB_SAVE_DIR", cfg.out.join("save"))
        };
        let _ = std::fs::create_dir_all(&cfg.out);
        open_kumite_beast::platform::support::init(&open_kumite_beast::base::env::var("LOG").unwrap_or("warn".into()));
        println!(
            "nettest: {}, netsim {:?}, delay {:?}, load lag {:?}, desync {:?}",
            cfg.room.as_ref().map_or(format!("UDP 127.0.0.1:{}", cfg.port), |(s, r)| format!("room {r} on {s}")),
            cfg.sim,
            cfg.delay,
            cfg.lag,
            cfg.desync
        );
        let clock = Instant::now();
        let spawn = |port: usize| {
            let cfg = cfg.clone();
            std::thread::Builder::new().name(format!("p{}", port + 1)).spawn(move || side(port, &cfg, clock)).expect("a thread")
        };
        let host = spawn(0);
        // (a room: the host's thread is the first in it)
        std::thread::sleep(Duration::from_millis(if cfg.room.is_some() { 1000 } else { 200 }));
        let join = spawn(1);
        let reports: Vec<Result<Report, String>> = [host, join].into_iter().map(|t| t.join().unwrap_or_else(|_| Err("panicked".into()))).collect();
        let mut ok = true;
        let mut replays = Vec::new();
        for r in &reports {
            match r {
                Ok(r) => {
                    print_report(r);
                    replays.push(r.replay.clone());
                }
                Err(e) => {
                    println!("a side failed: {e}");
                    ok = false;
                }
            }
        }
        let reports: Vec<Report> = reports.into_iter().filter_map(Result::ok).collect();
        if reports.len() != 2 {
            std::process::exit(1);
        }
        if reports[0].port != 0 || reports[1].port != 1 {
            println!("the sides are not the threads': the host's thread is player {}, the joiner's {}", reports[0].port + 1, reports[1].port + 1);
            ok = false;
        }
        // the host's progression choice is the session's on both sides (its card's defaults when locked: 9
        // characters, no fourth costume)
        let want = if cfg.locked[0] { (0x1FF, 0) } else { (0x7FF, 0x7FF) };
        for r in &reports {
            let sel = r.selectable.map_or("none".into(), |(c, k)| format!("characters {c:#x}, fourth costumes {k:#x}"));
            println!("p{}: offered locked {}, session locked {}, select: {sel}", r.port + 1, cfg.locked[r.port], r.locked);
            if r.locked != cfg.locked[0] || r.selectable != Some(want) {
                println!("p{}: the session's progression is not the host's", r.port + 1);
                ok = false;
            }
        }
        // every match's stage: the same on both sides, the first the host's pick (NETTEST_STAGE)
        for r in &reports {
            let st: Vec<_> =
                r.stages.iter().map(|&k| (open_kumite_beast::fight::fight_data::stage_code(k), open_kumite_beast::frontend::stage_name(k))).collect();
            println!("p{}: fight stages {st:?}", r.port + 1);
            if r.stages != reports[0].stages || cfg.stage.as_deref().is_some_and(|want| st.first().map(|s| s.1) != Some(want)) {
                println!("p{}: the matches' stages are not the host's", r.port + 1);
                ok = false;
            }
        }
        if let Some(f) = cfg.desync {
            let both = reports.iter().all(|r| matches!(r.stop, Some(Stop::Desync { .. })));
            println!("desync injected on the host at frame {f}: {}", if both { "detected on both sides" } else { "NOT detected on both sides" });
            std::process::exit(if both && ok { 0 } else { 1 });
        }
        for r in &reports {
            let finished = r.replay.is_some();
            let stop_ok = matches!(&r.stop, Some(Stop::Quit(_)) | Some(Stop::Left(_)));
            if !finished || !stop_ok {
                println!("p{}: the match did not finish cleanly (replay {finished}, stop {:?})", r.port + 1, r.stop);
                ok = false;
            }
        }
        if reports[0].p2_buttons != Some(JOIN_BUTTONS) || reports[1].p2_buttons != Some(JOIN_BUTTONS) {
            println!("PLAYER 2's map in the fight is not the joiner's: {:?} / {:?}", reports[0].p2_buttons, reports[1].p2_buttons);
            ok = false;
        }
        if let [Some(a), Some(b)] = &replays[..] {
            let same = a == b;
            println!("replays: {} and {} bytes, {}", a.len(), b.len(), if same { "byte-equal" } else { "DIFFERENT" });
            ok &= same;
            let assets = open_kumite_beast::assets::Assets::new(cfg.root.clone());
            for (k, b) in [a, b].into_iter().enumerate() {
                let path = cfg.out.join(format!("p{}.okbr", k + 1));
                if let Err(e) = std::fs::write(&path, b) {
                    println!("{}: {e}", path.display());
                }
                match replay::Replay::decode(b).and_then(|r| replay::simulate(&assets, r, |_| {})) {
                    Ok(s) => println!("{}: re-simulated {} frames, {} checkpoints in sync (strict {})", path.display(), s.frames, s.checks, s.strict),
                    Err(e) => {
                        println!("{}: replay FAILED: {e}", path.display());
                        ok = false;
                    }
                }
            }
        }
        println!("{}", if ok { "PASS" } else { "FAIL" });
        std::process::exit(if ok { 0 } else { 1 });
    }

    fn print_report(r: &Report) {
        let s = &r.stats;
        println!(
            "p{}: delay {}, rtt {:.1} ms, {} frames, stop {:?}; {} checkpoints matched; stalls {} ({:.2} s, longest {:.0} ms), {} slots let pass; packets sent {} ({} bytes), received {}, ignored {}",
            r.port + 1,
            r.delay,
            r.rtt.unwrap_or(f64::NAN) * 1e3,
            r.frames,
            r.stop.as_ref().map(|s| s.to_string()),
            s.checks,
            s.stalls,
            s.stall_secs,
            s.max_stall * 1e3,
            s.skips,
            s.sent,
            s.sent_bytes,
            s.received,
            s.bad
        );
        let mut g = r.gaps.clone();
        g.sort_by(f64::total_cmp);
        if !g.is_empty() {
            let q = |p: f64| g[((g.len() - 1) as f64 * p) as usize] * 1e3;
            let late = g.iter().filter(|&&x| x > 1.5 / 60.0).count();
            println!(
                "p{}: step gaps p50 {:.1} ms, p99 {:.1} ms, max {:.1} ms; {late} of {} steps more than 25 ms after the last ({:.2} %)",
                r.port + 1,
                q(0.5),
                q(0.99),
                q(1.0),
                g.len(),
                late as f64 * 100.0 / g.len() as f64
            );
        }
        for (f, d) in &r.screens {
            println!("p{}: {f:5}: {d}", r.port + 1);
        }
        for d in &r.dumps {
            println!("p{}: dumped {d}", r.port + 1);
        }
    }

    /// The host's memory card: the defaults with a 20 s TIME LIMIT and one round to win.
    fn host_card() -> Vec<u8> {
        let s = open_kumite_beast::frontend::options::Settings { time_limit: 0, match_point: 0, ..Default::default() };
        let mut c = open_kumite_beast::frontend::options::Card::new(None, None);
        assert!(s.save(&mut c));
        c.file().expect("a card").to_vec()
    }

    /// A bot's pad for frame `f`, read from the state before it: the host opens the mode menu with START and picks VS;
    /// both pick a character and a stage with □; in a fight both walk in and punch / kick, and P2 pauses at fight
    /// frame 300 (the pause's RESUME is the next □).
    fn bot(port: usize, s: &GameState, f: u32, stage: Option<&str>, br1: bool, watch: bool) -> u16 {
        if let Some(g) = &s.fight {
            let k = g.m.frame;
            if port == 1 && k == 300 {
                return START;
            }
            let walk = if port == 0 { RIGHT } else { LEFT };
            return match (k + port as u64 * 7) % 30 {
                0 => SQUARE,
                10 => CROSS | walk,
                20 => SQUARE | CROSS,
                25 if k % 300 == 25 => CIRCLE,
                _ => walk,
            };
        }
        let Some(front) = &s.front else { return 0 };
        let d = front.describe();
        let pulse = |every: u32, at: u32| f % every == at;
        if d.starts_with("title ") || d.starts_with("title fade") {
            return if port == 0 && f >= 60 && pulse(30, 0) { START } else { 0 };
        }
        // the host goes DOWN to VS (NETTEST_WATCH: WATCH) and takes it
        if let Some(m) = d.strip_prefix("mode menu (").and_then(|t| t.split(')').next()) {
            let take = m == if watch { "WATCH" } else { "VS" };
            return match (port, take) {
                (0, false) if pulse(20, 0) => DOWN,
                (0, true) if pulse(20, 10) => CROSS,
                _ => 0,
            };
        }
        if d.starts_with("select") {
            // NETTEST_STAGE: the host turns the stage select first (L1), nobody picks until it is there
            let row = d.split(" stage ").nth(1).and_then(|t| t.split(" [").next());
            if stage.is_some_and(|w| row != Some(w)) {
                return if port == 0 && pulse(10, 5) { L1 } else { 0 };
            }
            // NETTEST_BR1: each player first goes DOWN to the select's imported row
            let name = |p: &str, end: &str| d.split(p).nth(1).and_then(|t| t.split(end).next()).unwrap_or("").to_string();
            let mine = if port == 0 { name("select P1 ", " P2 ") } else { name(" P2 ", " stage ") };
            if br1 && !open_kumite_beast::assets::roster::BR1_CODES.iter().any(|c| open_kumite_beast::assets::roster::br1_name(c) == Some(mine.as_str())) {
                return if pulse(20, 10 * port as u32) { DOWN } else { 0 };
            }
            return if pulse(40, 20 * port as u32) { SQUARE } else { 0 };
        }
        0
    }

    fn now(clock: Instant) -> f64 {
        clock.elapsed().as_secs_f64()
    }

    /// The link of the thread `k` (0: the host's, 1: the joiner's).
    fn setup(k: usize, cfg: &Cfg) -> Result<Setup, String> {
        if let Some((signal, room)) = &cfg.room {
            #[cfg(feature = "netplay-webrtc")]
            return Ok(Setup::Room(net::webrtc::Room::new(signal, room)?));
            #[cfg(not(feature = "netplay-webrtc"))]
            return Err(format!("NETTEST_SIGNAL={signal} (room {room}): build with --features netplay-webrtc"));
        }
        let addr: std::net::SocketAddr = format!("127.0.0.1:{}", cfg.port).parse().expect("an address");
        Ok(Setup::Udp(if k == 0 { udp::Role::Host(addr) } else { udp::Role::Join(addr) }))
    }

    /// One side (thread `k`): the connect, then the session at 60 Hz until the match ended (or the session stopped).
    fn side(k: usize, cfg: &Cfg, clock: Instant) -> Result<Report, String> {
        let setup = setup(k, cfg)?;
        // both threads offer the same session and send the same map: the side the link gives decides which counts
        let assets = open_kumite_beast::assets::Assets::new(cfg.root.clone());
        // the front end's data before the connect, as the app has it: its tables (the CONTROLLER CONFIG. actions a
        // joiner's map is checked against, the default map) come with it
        let front = pollster::block_on(open_kumite_beast::frontend::load(&assets))?;
        let br1 = cfg.br1.then(|| open_kumite_beast::assets::manifest::br1_install_id(&assets)).flatten();
        // the game data's content id, as the app's start reads it (both threads: the same data)
        let content = Some(pollster::block_on(open_kumite_beast::assets::manifest::install_id(&assets))?);
        let start =
            SessionStart { seed: 1, card: Some(host_card()), entry: Entry::Title, locked: cfg.locked[k], br1, content, netplay: true, ..Default::default() };
        let offer = Offer { start, prefs: Prefs::default(), costume: [0; 2], time: open_kumite_beast::clock_seed() / 1000, delay: cfg.delay };
        let mut c = Connect::new(setup.open(cfg.sim, now(clock))?, offer, JOIN_BUTTONS, setup.describe(), now(clock));
        let agreed = loop {
            if let Some(a) = c.poll(now(clock))? {
                break a;
            }
            if now(clock) > CONNECT_TIMEOUT {
                return Err(format!("no session after {CONNECT_TIMEOUT:.0} s ({})", c.line()));
            }
            std::thread::sleep(Duration::from_millis(2));
        };
        let link = c.finish();
        let port = agreed.port;
        println!("p{}: connected to {} (thread {k}), input delay {}", port + 1, link.peer(), agreed.delay);
        let mut store = session::AssetStore { front: Some(Arc::new(front)), ..Default::default() };
        let mut np = Netplay::new(link, &agreed, now(clock));
        let mut s = session::new_state(&agreed.start, &store.assets(np.prefs()), &mut Out::default());
        let mut rec = replay::Recorder::new(&agreed.start, np.prefs(), agreed.costume);
        np.begin(now(clock));
        let mut r = Report { port, delay: agreed.delay, locked: agreed.start.locked, ..Default::default() };
        let (mut done, mut due): (u8, Option<(u32, u8)>) = (0, None);
        let (mut ticks, mut behind, mut over_at, mut last_step) = (0u32, 0u32, None::<u32>, None::<f64>);
        let mut next_tick = now(clock);
        let mut screen = String::new();
        while np.stopped().is_none() {
            next_tick += net::FRAME;
            let wait = next_tick - now(clock);
            if wait > 0.0 {
                std::thread::sleep(Duration::from_secs_f64(wait));
            } else if wait < -0.25 {
                next_tick = now(clock); // a blocking load: no burst after it
            }
            ticks += 1;
            np.poll(now(clock));
            if let Some((t, bit)) = due
                && ticks >= t
            {
                done |= bit;
                due = None;
            }
            // one step per tick, and the ones a stall held back (at most 6)
            behind = (behind + 1).min(6);
            // this side's clock leads: this slot passes
            if np.ahead() {
                behind -= 1;
            }
            while behind > 0 {
                let pad = if np.wants_local() { bot(port, &s, np.local_frame(), cfg.stage.as_deref(), cfg.br1, cfg.watch) } else { 0 };
                let cheats = match cfg.desync {
                    Some(f) if port == 0 && s.frame == f => vec![if s.fight.is_some() { Cheat::Gauges } else { Cheat::Unlock }],
                    _ => Vec::new(),
                };
                let Some(i) = np.input(pad, &mut done, now(clock)) else { break };
                behind -= 1;
                let t = now(clock);
                if let Some(l) = last_step {
                    r.gaps.push(t - l);
                }
                last_step = Some(t);
                let mut out = Out::default();
                session::step(&mut s, &store.assets(np.prefs()), i, &cheats, &mut out);
                np.stepped(&s, &out);
                rec.step(i, &cheats, np.prefs(), &s)?;
                for q in out.requests {
                    match q {
                        Request::Load(spec) => {
                            rec.match_load(&s);
                            r.stages.push(spec.stage);
                            let chr = spec.chr.map(open_kumite_beast::assets::roster::code);
                            let bgm = s.front.as_ref().is_some_and(|f| f.arcade_bgm());
                            let data = pollster::block_on(open_kumite_beast::fight::fight_data::load_match(
                                &assets,
                                chr,
                                spec.costume,
                                open_kumite_beast::fight::fight_data::stage_code(spec.stage),
                                bgm,
                                s.sim_version,
                            ))?;
                            store.fight = Some(Arc::new(data));
                            due = Some((ticks + cfg.lag[port], FrameInput::LOADED));
                        }
                        Request::Fetch(names) => {
                            for n in names {
                                store.files.remove(&n);
                                if let Some(Ok(b)) = assets.route(&n).map(|(a, rel)| pollster::block_on(a.load(rel))) {
                                    store.files.insert(n, b.into());
                                }
                            }
                            due = Some((ticks + cfg.lag[port], FrameInput::FETCHED));
                        }
                        Request::Fight => {
                            rec.match_start(&s);
                            r.p2_buttons = s.fight.as_ref().map(|g| g.m.buttons[1]);
                        }
                        Request::FightOver { backdrop, outcome, rounds, demo } => {
                            if r.replay.is_none() {
                                r.replay = rec.match_end(&s, outcome, rounds, demo, np.time_at(s.frame)).map(|x| x.encode());
                            }
                            over_at.get_or_insert(s.frame);
                            let data = store.fight.take();
                            if backdrop {
                                store.backdrop = data;
                            }
                        }
                        Request::Prefs(p) => np.set_prefs(p),
                        // no team battle in netplay (`SessionStart::netplay`)
                        Request::Music(_)
                        | Request::Exit
                        | Request::Replays
                        | Request::Network
                        | Request::ChangeData
                        | Request::LoadTeam(_)
                        | Request::TeamOver { .. } => {}
                    }
                }
                if s.backdrop.is_none() {
                    store.backdrop = None;
                }
                store.prune(&s);
                let d = s.front.as_ref().map(|f| f.describe()).unwrap_or_default();
                let d = d.split(" [").next().unwrap_or_default().to_string();
                if r.selectable.is_none() && d.starts_with("select") {
                    r.selectable = s.front.as_ref().map(|f| f.selectable());
                }
                if d.split(' ').next() != screen.split(' ').next() {
                    r.screens.push((s.frame, d.clone()));
                    screen = d;
                }
            }
            np.flush(now(clock));
            if over_at.is_some_and(|f| s.frame >= f + AFTER) {
                np.close("the test is over");
            } else if s.frame >= MAX_FRAMES {
                np.close("the test ran out of frames");
            }
        }
        r.frames = s.frame;
        // a desync: both sides' section hashes go back and forth for a second, then the dumps
        if matches!(np.stopped(), Some(Stop::Desync { .. })) {
            let until = now(clock) + 1.0;
            while now(clock) < until {
                np.poll(now(clock));
                np.flush(now(clock));
                std::thread::sleep(Duration::from_millis(16));
            }
            let rp = rec.replay(&s, np.time_at(s.frame)).encode();
            r.dumps = np.write_desync(&cfg.out, Some(&rp));
        }
        r.stop = np.stopped().cloned();
        r.stats = np.stats;
        r.rtt = np.rtt();
        Ok(r)
    }
}
