//! The desktop binary `okbeast`: the command line, then the app.
//!
//! Desktop entry: `okbeast [--assets DIR] [--locked] [--debug] [--fight --p1 YUG --p2 ALI --stage ST0] [--replay FILE]
//! [--host [ADDR:]PORT | --join ADDR:PORT | --signal URL --room CODE [--stun URL] [--turn URL --turn-user U --turn-pass P]]
//! [--delay N]`
//! (`--help`)
//! (without `--fight` the game starts at the title screen; without `--assets` it reads the game data folder beside the
//! save, `platform::game_data`, checked first by `assets::manifest::check_dir`: missing or unusable data opens the
//! setup screen, `app::setup`)

#[cfg(not(target_arch = "wasm32"))]
fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.iter().any(|a| a == "--help" || a == "-h") {
        println!(
            "OpenKumiteBeast {}\n\nokbeast [--assets DIR] [--locked] [--debug] [--fight --p1 YUG --p2 ALI --stage ST0] [--replay FILE]\n\
             \x20   [--host [ADDR:]PORT | --join ADDR:PORT | --signal URL --room CODE [--stun URL] [--turn URL --turn-user U --turn-pass P]] [--delay N]\n\n\
             --assets DIR   a converted export to read instead of the game data folder beside the save\n\
             \x20              (<config dir>/openkumitebeast/game_data; OKB_SAVE_DIR moves it with the save)\n\
             --locked       the original's progression: only the characters and fourth costumes the save has\n\
             \x20              unlocked, and the original's nine stages (also OKB_LOCKED=1; the browser: ?locked=1).\n\
             \x20              Without it every character and costume is selectable, the stage select adds the\n\
             \x20              extra stages ST9..STC (EXTRA1..4) and, with their files in q3/ (the setup's QUAKE III\n\
             \x20              ARENA MAPS) or extras/<map>/ of the game data, Q3DM17 and Q3DM6 (Quake III's maps;\n\
             \x20              --stage Q3DM17 / Q3DM6), and nothing is written to the save for it;\n\
             \x20              netplay uses the host's choice. --unlock (OKB_UNLOCK) is accepted and does nothing\n\
             --debug        the debug overlay at start (F1 toggles it) and the debug tools (also OKB_DEBUG=1):\n\
             \x20              {}\n\
             --fight        skip the menus and start this fight\n\
             --replay FILE  play a recorded match back (.okbr; finished matches are saved to the replays folder beside\n\
             \x20              the save), then quit\n\
             --host PORT    netplay: wait for player 2 on this UDP port (ADDR:PORT: on that address only, e.g.\n\
             \x20              127.0.0.1:7000); the session (memory card, unlocks, options) is the host's\n\
             --join A:P     netplay: join the host at address A, port P, as player 2 (both need the same build)\n\
             --signal URL   netplay over WebRTC (builds with the netplay-webrtc feature; the browser build too): the\n\
             \x20              matchbox signaling server, ws://HOST:PORT or wss://… (none is built in: run matchbox_server)\n\
             --room CODE    with --signal: the room, 1..16 letters and digits; the first player in it hosts (player 1)\n\
             --stun URL     with --signal: a STUN server (stun:HOST:PORT; default none: same machine / LAN only)\n\
             --turn URL     with --signal: a TURN server (turn:HOST:PORT) with --turn-user U --turn-pass P\n\
             --delay N      netplay (host): input delay in frames, 1..30 (default from the round trip)\n\
             \x20              Netplay never writes the save or the preferences; the debug tools are off.\n\n\
             keys: P1 WASD + J K L I (square cross circle triangle) U O (R1 L1) Enter (START);\n\
             P2 arrows + numpad 1-6, numpad Enter; gamepads work as well.\n\
             debug: {}\n\n\
             env: OKB_LOG=debug (log level), OKB_MUTE=1, OKB_P2=human, OKB_GAUGE=n, OKB_CONT=1, OKB_ASPECT=4:3|wide (F5 toggles, saved),\n\
             OKB_SCRIPT / OKB_SHOTS (scripted fight + screenshots; with OKB_FESCRIPT the fight is reached through the menus), OKB_FESCRIPT / OKB_FESCRIPT2 / OKB_FESHOTS (menus, pad 2),\n\
             OKB_LOCKSTEP=1 (one 60 Hz step per presented frame) / OKB_SEED=n (front-end random seed): reproducible screenshot runs,\n\
             OKB_WINDOW=WxH (surface size in pixels, kept even when the window is clamped) / WxHpt (window size in points), OKB_WINDOWPOS=x,y (window position in points),\n\
             OKB_SPRITES=old|ab (2D sprites from the VRAM copy / screenshots drawn both ways, differences logged), OKB_ATLASDUMP=prefix,\n\
             OKB_CAMYAW=radians (the debug camera orbit at start, Q / E), OKB_PERF=1 (performance overlay at start: frame times, CPU / GPU time, draw counts, memory; ` toggles it),\n\
             OKB_RECORD=path (the whole session written there as a replay at exit),\n\
             OKB_SETUP_FROM=PATH / OKB_SETUP_SCAN=dirs / OKB_SETUP_SLOW=ms (the setup screen: PATH dropped at UI frame 30 and installed at 60, the scan's folders, a pause per copied file),\n\
             OKB_NETSIM=lat_ms,loss_pct[,jitter_ms] (netplay over a simulated network), OKB_NETSCRIPT (netplay: this side's scripted pad by session frame).\n\
             Crash reports go to okb_crash_<time>.txt, F8 writes okb_report_<time>.txt. Every OKB_ variable is also read as BR2_…",
            open_kumite_beast::platform::support::VERSION,
            open_kumite_beast::platform::support::TOOLS_HELP,
            open_kumite_beast::platform::support::HELP
        );
        return;
    }
    open_kumite_beast::platform::support::init(&open_kumite_beast::base::env::var("LOG").unwrap_or("info".into()));
    let arg = |k: &str, d: &str| args.iter().position(|a| a == k).and_then(|i| args.get(i + 1)).cloned().unwrap_or_else(|| d.to_string());
    let opt = |k: &str| {
        args.iter().position(|a| a == k).map(|i| {
            args.get(i + 1).cloned().unwrap_or_else(|| {
                eprintln!("{k} needs a value");
                std::process::exit(2);
            })
        })
    };
    use open_kumite_beast::net::Setup;
    let (signal, room) = (opt("--signal"), opt("--room"));
    let (stun, turn, turn_user, turn_pass) = (opt("--stun"), opt("--turn"), opt("--turn-user"), opt("--turn-pass"));
    let ice = stun.is_some() || turn.is_some() || turn_user.is_some() || turn_pass.is_some();
    let role = match (opt("--host"), opt("--join"), room.is_some() || signal.is_some()) {
        (Some(_), Some(_), _) | (Some(_), _, true) | (_, Some(_), true) => Err("--host, --join or --signal + --room: one of them".to_string()),
        // as the browser's stun= / turn= without room=
        (_, _, false) if ice => Err("--stun / --turn need --signal URL --room CODE".to_string()),
        (Some(h), None, _) => open_kumite_beast::net::udp::Role::host(&h).map(|r| Some(Setup::Udp(r))),
        (None, Some(j), _) => open_kumite_beast::net::udp::Role::join(&j).map(|r| Some(Setup::Udp(r))),
        (None, None, true) => room_setup(signal, room, stun, turn, turn_user, turn_pass).map(Some),
        (None, None, false) => Ok(None),
    };
    let delay = opt("--delay").map(|d| match d.parse::<u32>() {
        Ok(n) if (1..=open_kumite_beast::net::MAX_DELAY).contains(&n) => n,
        _ => {
            eprintln!("--delay {d}: 1..={}", open_kumite_beast::net::MAX_DELAY);
            std::process::exit(2);
        }
    });
    let net = match role {
        Ok(None) if delay.is_some() => {
            eprintln!("--delay is for netplay (--host, or a room's host)");
            std::process::exit(2);
        }
        Ok(r) => r.map(|r| Ok((r, delay))),
        Err(e) => {
            eprintln!("{e}");
            std::process::exit(2);
        }
    };
    let replay = args.iter().position(|a| a == "--replay").map(|i| {
        let Some(path) = args.get(i + 1) else {
            eprintln!("--replay needs a file");
            std::process::exit(2);
        };
        match open_kumite_beast::platform::storage::read_capped(path.as_ref(), open_kumite_beast::session::replay::MAX_BYTES) {
            Ok(b) => (path.clone(), b),
            Err(e) => {
                eprintln!("replay {path}: {e}");
                std::process::exit(1);
            }
        }
    });
    let own_data = opt("--assets").is_none();
    let (assets, setup) = game_data(opt("--assets"));
    open_kumite_beast::run(open_kumite_beast::Config {
        assets: open_kumite_beast::assets::Assets::new(assets),
        fight: (args.iter().any(|a| a == "--fight")
            || (open_kumite_beast::base::env::var("SCRIPT").is_ok() && open_kumite_beast::base::env::var("FESCRIPT").is_err()))
        .then(|| (arg("--p1", "YUG"), arg("--p2", "ALI"), arg("--stage", "ST0"))),
        // --unlock / OKB_UNLOCK: accepted, the default now
        locked: args.iter().any(|a| a == "--locked") || open_kumite_beast::base::env::var("LOCKED").is_ok(),
        debug: args.iter().any(|a| a == "--debug") || open_kumite_beast::base::env::var("DEBUG").is_ok(),
        perf: open_kumite_beast::base::env::var("PERF").is_ok_and(|v| v == "1"),
        replay,
        net,
        setup,
        own_data,
    });
}

/// The folder the game data is read from: `--assets DIR` (a developer's override: checked, a failure ends the
/// program), else the game data folder beside the save (an interrupted install recovered first), with why it
/// cannot be used (the setup screen's reason) when the check fails.
#[cfg(not(target_arch = "wasm32"))]
fn game_data(dir: Option<String>) -> (std::path::PathBuf, Option<String>) {
    use open_kumite_beast::assets::manifest;
    let (root, flag) = match dir {
        Some(d) => (std::path::PathBuf::from(d), true),
        None => {
            let d = open_kumite_beast::platform::game_data::Dirs::app();
            d.recover();
            d.sub(open_kumite_beast::platform::game_data::BR1_DIR).recover();
            d.sub(open_kumite_beast::platform::game_data::Q3_DIR).recover();
            (d.root, false)
        }
    };
    let v = manifest::check_dir(&root);
    // the data of the optional disc of the imported game: checked and reported (the game does not read it yet)
    log::info!("game data {}: the imported game {:?}", root.display(), manifest::check_dir_br1(&root.join(manifest::BR1_DIR)));
    // Quake III Arena's maps (engine addition, the extra stages): checked and reported (`assets::extras::present` reads them)
    log::info!(
        "game data {}: Quake III {:?}",
        root.display(),
        open_kumite_beast::assets::extras::check_dir_q3(&root.join(open_kumite_beast::assets::extras::Q3_DIR))
    );
    match v.reason() {
        Some(why) if flag => {
            log::error!("game data {}: {why}", root.display());
            eprintln!("game data {}: {why}", root.display());
            std::process::exit(1);
        }
        Some(why) => {
            log::warn!("game data {}: {why} (the setup screen)", root.display());
            (root, Some(why))
        }
        None => {
            log::info!("game data {}: {v:?}", root.display());
            (root, None)
        }
    }
}

/// `--signal URL --room CODE [--stun URL] [--turn URL --turn-user U --turn-pass P]`: a room (feature
/// `netplay-webrtc`).
#[cfg(not(target_arch = "wasm32"))]
fn room_setup(
    signal: Option<String>,
    room: Option<String>,
    stun: Option<String>,
    turn: Option<String>,
    user: Option<String>,
    pass: Option<String>,
) -> Result<open_kumite_beast::net::Setup, String> {
    let (Some(signal), Some(room)) = (signal, room) else { return Err("--signal URL and --room CODE go together".into()) };
    #[cfg(feature = "netplay-webrtc")]
    {
        let turn = match (turn, user, pass) {
            (Some(t), Some(u), Some(p)) => Some((t, u, p)),
            (None, None, None) => None,
            _ => return Err("--turn URL needs --turn-user U and --turn-pass P".into()),
        };
        let r = open_kumite_beast::net::webrtc::Room::new(&signal, &room)?
            .with_ice(stun.as_deref(), turn.as_ref().map(|(t, u, p)| (t.as_str(), u.as_str(), p.as_str())))?;
        Ok(open_kumite_beast::net::Setup::Room(r))
    }
    #[cfg(not(feature = "netplay-webrtc"))]
    {
        let _ = (signal, room, stun, turn, user, pass);
        Err("--signal / --room: this build has no WebRTC netplay (cargo build --features netplay-webrtc)".into())
    }
}

#[cfg(target_arch = "wasm32")]
fn main() {
    open_kumite_beast::web_main();
}
