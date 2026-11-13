//! The app (desktop, browser and Android): the window, the 60 Hz session loop, loading, sound, drawing, the tools.
//!
//! The desktop / browser / Android app: the winit loop, `session::step` at 60 Hz, loading, sound, the card store,
//! drawing the state, the debug tools.
//!
//! `mod.rs`: `Config`, `UserEvent`, `App`, `begin_session`, `run`, `web_main`, `session_start`; `events.rs`: winit's
//! `ApplicationHandler` (the window's events, the redraw's steps); `stepping.rs`: a 60 Hz step; `requests.rs`: what a
//! step asks for; `keys.rs`: the debug F-keys; `overlay.rs`: the overlay; `draw.rs`: drawing a frame; `android.rs` and
//! `touch.rs`: the Android app's input, lifecycle and touch controls (`android_main`); `setup/`: the setup screen
//! (no or unusable game data: pick, copy, check, install); `net_menu.rs`: the NETWORK menu's page (its app side in
//! `netplay.rs`); `video.rs` and `video_ffmpeg.rs`: `replay --video`, a replay drawn and mixed without a window into a
//! video file.

#[cfg(target_os = "android")]
mod android;
#[cfg(not(target_arch = "wasm32"))]
mod capture;
mod debug_env;
mod devices;
mod draw;
mod events;
mod gamepads;
mod keys;
mod net_menu;
mod netplay;
mod overlay;
mod pad_layout;
mod perf;
mod playback;
mod replay_menu;
pub(crate) mod replay_store;
mod replays;
mod requests;
mod setup;
mod stepping;
mod tools;
mod touch;
#[cfg(not(target_arch = "wasm32"))]
pub(crate) mod video;
#[cfg(not(target_arch = "wasm32"))]
pub(crate) mod video_ffmpeg;
#[cfg(target_arch = "wasm32")]
mod web_files;

use crate::fight::fight_data;
use crate::frontend::options;
use crate::platform::support;
use crate::present::{fight_scene, front_vram};
use crate::session::{self, Cheat, FrameInput};
use crate::{assets, audio, formats, frontend, render};
use debug_env::DebugEnv;
use overlay::wrap;
use std::sync::Arc;
use winit::event_loop::{EventLoop, EventLoopProxy};
use winit::keyboard::ModifiersState;
use winit::window::Window;

pub struct Config {
    pub assets: assets::Assets,
    /// --fight (p1, p2, stage): skip the front end and start this fight at once (tests, OKB_SCRIPT); moved into
    /// the session's entry (`Entry::DirectFight`)
    pub fight: Option<(String, String, String)>,
    /// --locked / OKB_LOCKED / `?locked=1`: the game's own progression (the characters and fourth costumes the save
    /// unlocked) instead of all of them selectable (`SessionStart::locked`)
    pub locked: bool,
    /// debug overlay on at start
    pub debug: bool,
    /// performance overlay on at start (OKB_PERF=1): for this session only, GRAPHICS → PERF OVERLAY keeps the saved
    /// preference
    pub perf: bool,
    /// --replay FILE (desktop): the file's name and bytes, played back instead of a session of the player's
    pub replay: Option<(String, Vec<u8>)>,
    /// --host / --join (desktop), --signal + --room (or the URL's), and --delay: a netplay session (`netplay`); Err:
    /// the URL's setup did not check out (shown)
    pub net: Option<Result<(crate::net::Setup, Option<u32>), String>>,
    /// the game data folder's quick check failed (why): the setup screen comes first (desktop, `setup`)
    pub setup: Option<String>,
    /// `assets` is the app's own game data folder (not `--assets`): a start that cannot read it goes to the setup
    /// screen instead of ending (desktop)
    pub own_data: bool,
}

enum UserEvent {
    /// the startup's renderer and data (netplay: the renderer and the front end's data while it connects, then a
    /// direct fight's match alone): the session starts
    Ready(Box<(Option<render::Renderer>, Option<frontend::FrontData>, Option<fight_data::Loaded>)>),
    /// the match asked for (`Request::Load`); dropped, as `Music` and `Fetched` are, when the debug tools put a
    /// state back since it was asked for (`epoch` ≠ `App::restore_gen`), and, as `Fetched` is, when a later request
    /// replaced it (its number ≠ `App::load_seq` / `fetch_seq`)
    Loaded(Box<fight_data::Loaded>, u32, u32),
    /// a music track loaded (`Request::Music`; the debug tools' re-sync): played from `secs` in, looped when
    /// `looped`
    Music {
        m: Arc<formats::music::Music>,
        looped: bool,
        secs: f64,
        epoch: u32,
    },
    /// files asked for by the front end (`frontend::Event::Fetch`)
    Fetched(Vec<(String, Vec<u8>)>, u32, u32),
    Failed(String),
    /// a direct fight's match for a replay played from the list (dropped when `epoch` ≠ `App::restore_gen`)
    ReplayData(Box<fight_data::Loaded>, u32),
    /// that match did not load (dropped as `ReplayData`)
    ReplayFailed(String, u32),
    /// the browser's IMPORT: the files chosen (name, bytes or why not)
    #[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
    Imported(Vec<(String, Result<Vec<u8>, String>)>),
    /// the setup screen's renderer (`setup`)
    SetupReady(Box<render::Renderer>),
    /// the start could not read the app's own game data folder: the setup screen, with why
    DataFailed(String),
    /// the browser's game data checked at start (`assets::manifest::check_opfs`): why it cannot be used, if so
    #[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
    DataChecked(Option<String>),
    /// the browser's install of the imported game found by the start (`assets::manifest::opfs_br1_install`: its OPFS folder
    /// and content id; engine addition, the import S9)
    #[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
    Br1(Option<(String, u32)>),
    /// the browser's Quake III install found by the start (a change on the setup screen reloads the page)
    /// (`assets::extras::opfs_q3_install`: its OPFS folder and the extra stages it has; engine addition)
    #[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
    Q3(Option<(String, u8)>),
    /// the game data's content id read by the start (`assets::manifest::install_id`, before `Ready`)
    Content(u32),
    /// the browser's netplay keep-alive timer (`netplay::keepalive_timer`)
    #[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
    KeepAlive,
}

const FRAME: f64 = 1.0 / 60.0;

struct App {
    cfg: Option<Config>,
    assets: Option<assets::Assets>,
    /// what the session started from (built once in `run`; the fights take its debug setup)
    start: crate::session::SessionStart,
    /// the game data's content id once the start read it (`UserEvent::Content`, `content_found`): the new sessions'
    /// (`SessionStart::content`), a replay's check; None before
    content: Option<u32>,
    proxy: EventLoopProxy<UserEvent>,
    window: Option<Arc<Window>>,
    renderer: Option<render::Renderer>,
    audio: audio::Audio,
    /// the logic: changed only by `session::step`
    state: session::GameState,
    /// the static data the steps read: the front end's, the fight's (or the loaded match), the backdrop's, the
    /// fetched files
    store: session::AssetStore,
    /// the GPU objects of the fight and of the attract demo's fight kept behind the ranking
    view: Option<fight_scene::FightView>,
    backdrop_view: Option<fight_scene::FightView>,
    /// the next step's `FrameInput` flags: the load / fetch that completed, the card store
    flags: u8,
    /// the debug key cheats for the next step, in the order pressed
    cheats: Vec<Cheat>,
    /// the front end's VRAM image and movie media, derived from the state (`present::front_vram::VramView`)
    vram_view: front_vram::VramView,
    /// fight requested by the front end, loaded after the VS screen has been drawn
    load_req: Option<frontend::LoadSpec>,
    load_wait: u32,
    last: Option<f64>,
    acc: f64,
    paused: bool,
    keys: u16,
    keys2: u16,
    /// keys pressed since the last game tick (a press and release inside one frame still count)
    latch: [u16; 2],
    /// pads 3 / 4 as read (gamepads only; engine addition, the team battle: `FrameInput::ext`)
    ext: [u16; 2],
    /// the gamepads, their players and rumble (`gamepads::Pads`)
    gamepads: gamepads::Pads,
    /// the debug env hooks (OKB_SCRIPT, OKB_FESCRIPT, OKB_SHOTS, OKB_LOCKSTEP…), read once at start
    env: DebugEnv,
    #[cfg_attr(target_arch = "wasm32", allow(dead_code))]
    shot_done: u64,
    // debug tools (platform/support.rs, tools.rs)
    overlay: bool,
    fast: bool,
    fps: perf::Fps,
    /// the performance overlay (engine addition)
    perf: perf::Perf,
    shot_req: bool,
    /// a short message on screen: (text, frames left)
    toast: Option<(String, u32)>,
    /// engine-only preferences (OPTIONS → GRAPHICS, F5, Alt+Enter), saved apart from the memory card data
    prefs: options::Prefs,
    /// the keyboard's modifiers (Alt+Enter)
    mods: ModifiersState,
    /// EXIT GAME was chosen: quit after this frame's steps (engine addition)
    exit_req: bool,
    /// the window's fullscreen state last seen, to follow changes made by the system (engine addition)
    seen_fs: bool,
    /// FRAME CAP: when the next frame may be drawn (`perf_now()` seconds)
    #[cfg_attr(target_arch = "wasm32", allow(dead_code))]
    next_due: f64,
    /// the last redraw had no surface texture (an occluded or minimised window): the next one waits for its 1/60 s
    /// slot instead of spinning
    skipped: bool,
    /// debug OKB_STATEHASH: the state's hash after every step
    hashes: session::HashTrace,
    /// debug OKB_ROLLBACK=N (desktop): the rollback test after every step; a frame's log is its input and cheats
    rollback: Option<session::rollback::Rollback<(FrameInput, Vec<Cheat>)>>,
    /// the debug tools (OKB_DEBUG=1 / --debug / `?debug=1`: pause and step everywhere, save slots, rewind)
    tools: Option<tools::Tools>,
    /// +1 whenever the debug tools put a state back: a load, fetch or music asked for before is dropped when it
    /// completes (the restored state asks again for what it waits on)
    restore_gen: u32,
    /// a direct fight's costumes (debug OKB_COSTUME; a replay's)
    costume: [u8; 2],
    /// the numbers of the last `Request::Load` and `Request::Fetch`: a completion of an earlier one is dropped
    load_seq: u32,
    fetch_seq: u32,
    /// the steps' requests so far (OKB_FEFAST ends its run of steps at one)
    #[cfg(not(target_arch = "wasm32"))]
    requests: u32,
    /// the session's always-on recording (`replays`); None in a playback and once stopped
    rec: Option<session::replay::Recorder>,
    /// the replay played back (--replay, or PLAY on the replay list)
    player: Option<replays::Playback>,
    /// the replay files (desktop folder, browser storage)
    replays: replay_store::Store,
    /// the replay list (the mode menu's REPLAY; the session waits meanwhile)
    browser: Option<replay_menu::Browser>,
    /// the live session put aside while a replay from the list plays
    live: Option<Box<replays::Live>>,
    /// pad bits held when the list closed: kept from the session until released
    swallow: u16,
    /// 60 Hz frames of the list and the playback controls (debug OKB_UISCRIPT / OKB_UISHOTS)
    ui_frame: u64,
    #[cfg_attr(target_arch = "wasm32", allow(dead_code))]
    ui_shot_done: u64,
    #[cfg_attr(target_arch = "wasm32", allow(dead_code))]
    ui_shot_name: Option<String>,
    /// the match file being saved (`replays`; desktop)
    #[cfg(not(target_arch = "wasm32"))]
    saving: Option<std::thread::JoinHandle<()>>,
    /// the netplay session (`--host` / `--join`, `--signal` + `--room`): every step's input comes from it
    net: Option<crate::net::Netplay>,
    net_ui: netplay::NetUi,
    /// Android: the activity's input, the touch controls, the lifecycle (`android`)
    #[cfg(target_os = "android")]
    droid: android::Droid,
    /// the setup screen, while it is up (no session meanwhile)
    setup: Option<Box<setup::Run>>,
}

impl App {
    /// OKB_TEAM's battle (engine addition, debug): not in netplay or a playback.
    fn team_debug(&self) -> Option<debug_env::TeamDebug> {
        self.env.team.clone().filter(|_| self.player.is_none() && !self.net_ui.netplay)
    }

    /// The session's state from `start` and the store (the startup's data in it): the recording, the debug tools'
    /// history and the rollback test start from it; netplay's timeout counts from here.
    fn begin_session(&mut self) {
        let mut out = session::Out::new(self.audio.logging());
        let prefs = self.logic_prefs();
        self.state = match (self.team_debug(), self.store.fight.as_deref()) {
            // OKB_TEAM (engine addition, debug): the team battle (not recorded: a replay holds a match or the front end)
            (Some(t), Some(data)) if data.team.is_some() => {
                let cpu = std::array::from_fn(|k| k >= t.humans);
                session::new_team_state(&self.start, data, t.mode, cpu)
            }
            _ => session::new_state(&self.start, &self.store.assets(prefs), &mut out),
        };
        log::info!("session: started ({:?}, simulation v{})", self.start.entry, self.start.sim_version);
        if let Some(n) = &mut self.net {
            n.begin(crate::perf_now());
        }
        self.net_ui.started = true;
        // the recording (none in a playback)
        if self.player.is_some() {
            self.playback_started();
        } else if self.state.team.is_none() {
            self.rec = Some(session::replay::Recorder::new(&self.start, prefs, self.costume));
        }
        if let Some(rb) = &mut self.rollback {
            rb.start(&self.state);
        }
        if let Some(t) = &mut self.tools {
            t.rewind.start(&self.state);
        }
        self.audio.apply(&mut out.audio);
        if let Some(w) = &self.window {
            w.request_redraw();
        }
    }
}

/// The session's start from the debug env hooks, the clock and the stored save, the extra stage when `assets` has
/// its files (`fetest`; the app builds its own in `run`).
pub fn session_start(locked: bool, assets: &assets::Assets) -> crate::session::SessionStart {
    DebugEnv::read().start(None, locked, assets::extras::present(assets), assets::manifest::br1_install_id(assets))
}

pub fn run(mut cfg: Config) {
    // a full localStorage makes room for the card / the preferences by deleting the oldest unmarked replay
    #[cfg(target_arch = "wasm32")]
    crate::platform::storage::set_make_room(replay_store::make_room);
    #[cfg(not(target_os = "android"))]
    let el = EventLoop::<UserEvent>::with_user_event().build().expect("event loop");
    #[cfg(target_os = "android")]
    let el = {
        use winit::platform::android::EventLoopBuilderExtAndroid;
        let app = crate::platform::android::app().expect("android_main sets the app");
        EventLoop::<UserEvent>::with_user_event().with_android_app(app).build().expect("event loop")
    };
    let mut env = DebugEnv::read();
    // --replay: the session the file holds (a newer or damaged file is refused here)
    let br1 = assets::manifest::br1_install_id(&cfg.assets);
    let replay = cfg.replay.take().map(|(name, bytes)| {
        match session::replay::Replay::decode(&bytes).and_then(|r| session::replay::br1_fits(&r.header.start, br1).map(|()| r)) {
            Ok(r) => {
                log::info!("replay {name}: {} steps from frame {} (engine {}, simulation v{})", r.header.frames, r.header.from, r.engine, r.sim_version);
                r
            }
            Err(e) => {
                log::error!("replay {name}: {e}");
                eprintln!("replay {name}: {e}");
                std::process::exit(1);
            }
        }
    });
    // netplay: the link opened here, the connect runs in the frame loop (`netplay`); the session is the host's
    let mut net_ui = netplay::NetUi::default();
    if let Some(net) = cfg.net.take() {
        if replay.is_some() {
            eprintln!("--replay and netplay: one at a time");
            std::process::exit(2);
        }
        // the command line's sessions have no NETWORK row (they are not the NETWORK menu's: `netplay`)
        let start = crate::session::SessionStart {
            network: false,
            ..env.start(cfg.fight.clone(), cfg.locked, assets::extras::present(&cfg.assets), assets::manifest::br1_install_id(&cfg.assets))
        };
        let c = net.and_then(|(setup, delay)| netplay::connect(&setup, delay, start, fight_data::direct_costume()));
        match c {
            Ok(c) => net_ui.connect = Some(c),
            Err(e) => {
                log::error!("netplay: {e}");
                eprintln!("netplay: {e}");
                #[cfg(not(target_arch = "wasm32"))]
                std::process::exit(1);
                #[cfg(target_arch = "wasm32")]
                {
                    net_ui.failed = Some(e);
                }
            }
        }
    }
    let netplay = net_ui.connect.is_some() || net_ui.failed.is_some();
    net_ui.netplay = netplay;
    let (start, costume) = match &replay {
        Some(r) => (r.start(), r.header.costume),
        // netplay: replaced by the agreed one (`net_connect_tick`)
        None => (
            env.start(cfg.fight.take(), cfg.locked, assets::extras::present(&cfg.assets), assets::manifest::br1_install_id(&cfg.assets)),
            fight_data::direct_costume(),
        ),
    };
    if netplay {
        if cfg.debug {
            log::warn!("netplay: the debug tools are off");
        }
        cfg.debug = false;
        env.fast = None;
        env.fefast = None;
    }
    if replay.is_some() && cfg.debug {
        log::warn!("--replay: the debug tools are off in a playback");
    }
    let tools = (cfg.debug && replay.is_none()).then(|| tools::Tools::new(std::mem::take(&mut env.debugkeys)));
    if tools.is_some() {
        log::info!("debug tools on: {}", support::TOOLS_HELP);
    }
    let app = App {
        cfg: Some(cfg),
        assets: None,
        start,
        content: None,
        proxy: el.create_proxy(),
        window: None,
        renderer: None,
        audio: audio::Audio::new(),
        state: session::GameState::default(),
        store: session::AssetStore::default(),
        view: None,
        backdrop_view: None,
        flags: 0,
        cheats: Vec::new(),
        vram_view: Default::default(),
        load_req: None,
        load_wait: 0,
        last: None,
        acc: 0.0,
        paused: false,
        keys: 0,
        keys2: 0,
        latch: [0; 2],
        ext: [0; 2],
        gamepads: gamepads::Pads::new(),
        env,
        shot_done: u64::MAX,
        overlay: false,
        fast: false,
        fps: perf::Fps::default(),
        perf: perf::Perf::default(),
        shot_req: false,
        toast: None,
        prefs: options::Prefs::from_stored(crate::platform::storage::read_prefs()),
        mods: ModifiersState::empty(),
        exit_req: false,
        seen_fs: false,
        next_due: 0.0,
        skipped: false,
        hashes: session::HashTrace::from_env(&crate::base::env::name("STATEHASH")),
        rollback: session::rollback::Rollback::from_env(&crate::base::env::name("ROLLBACK")),
        tools,
        restore_gen: 0,
        costume,
        load_seq: 0,
        fetch_seq: 0,
        #[cfg(not(target_arch = "wasm32"))]
        requests: 0,
        rec: None,
        player: replay.map(|r| replays::Playback::new(r, None, [0; 2])),
        replays: replay_store::Store::new(),
        browser: None,
        live: None,
        swallow: 0,
        ui_frame: 0,
        ui_shot_done: u64::MAX,
        ui_shot_name: None,
        #[cfg(not(target_arch = "wasm32"))]
        saving: None,
        net: None,
        net_ui,
        #[cfg(target_os = "android")]
        droid: android::Droid::new(),
        setup: None,
    };
    #[cfg(not(target_arch = "wasm32"))]
    {
        let mut app = app;
        el.run_app(&mut app).expect("run");
        // a netplay connect that failed (not one the user closed)
        let failed = app.net_ui.failed.is_some();
        app.net_close();
        app.write_record();
        app.join_save();
        if failed {
            std::process::exit(1);
        }
        // the rollback test's summary; a mismatch exits 3
        if app.rollback.as_mut().is_some_and(|rb| rb.finish() > 0) {
            std::process::exit(3);
        }
    }
    #[cfg(target_arch = "wasm32")]
    {
        use winit::platform::web::EventLoopExtWebSys;
        // the command line's netplay or one of the NETWORK menu later: a hidden tab keeps the link alive
        netplay::keepalive_timer(app.proxy.clone());
        el.spawn_app(app);
    }
}

/// The Android entry (`android_main` in `lib.rs`; engine addition, `docs/android.md`): the session's files and the game
/// data folder in the app's storage, checked as on desktop (missing or unusable
/// data: the setup screen); the process ends with the activity (a winit event loop is made once per process).
#[cfg(target_os = "android")]
pub fn android_main(app: winit::platform::android::activity::AndroidApp) {
    crate::platform::android::set_app(app);
    support::init("info");
    let dirs = crate::platform::game_data::Dirs::app();
    dirs.recover();
    dirs.sub(crate::platform::game_data::BR1_DIR).recover();
    dirs.sub(crate::platform::game_data::Q3_DIR).recover();
    let v = assets::manifest::check_dir(&dirs.root);
    log::info!("game data {}: {v:?}", dirs.root.display());
    log::info!(
        "game data {}: the imported game {:?}",
        dirs.root.display(),
        assets::manifest::check_dir_br1(&dirs.sub(crate::platform::game_data::BR1_DIR).root)
    );
    log::info!("game data {}: Quake III {:?}", dirs.root.display(), assets::extras::check_dir_q3(&dirs.sub(crate::platform::game_data::Q3_DIR).root));
    run(Config {
        assets: assets::Assets::new(dirs.root),
        fight: None,
        locked: false,
        debug: false,
        perf: false,
        replay: None,
        net: None,
        setup: v.reason(),
        own_data: true,
    });
    std::process::exit(0);
}

/// The browser's conversion worker (`setup::worker`).
#[cfg(all(target_arch = "wasm32", feature = "converter"))]
pub fn convert_worker_main() {
    setup::worker_main();
}

#[cfg(target_arch = "wasm32")]
pub fn web_main() {
    console_error_panic_hook::set_once();
    let q = web_sys::window().and_then(|w| w.location().search().ok()).unwrap_or_default();
    let arg = |k: &str, d: &str| q.trim_start_matches('?').split('&').find_map(|kv| kv.strip_prefix(&format!("{k}="))).unwrap_or(d).to_string();
    support::init(&arg("log", "info"));
    // netplay (`?signal=…&room=…`): the values percent-decoded (`ws%3A%2F%2F…` as well as `ws://…`)
    let net = netplay::web_setup(|k| {
        q.trim_start_matches('?')
            .split('&')
            .find_map(|kv| kv.strip_prefix(&format!("{k}=")))
            .map(|v| js_sys::decode_uri_component(v).ok().and_then(|s| s.as_string()).unwrap_or_default())
    });
    let net = net.transpose();
    // the game data: an export served over HTTP only when the URL names one (`?assets=URL`, development and
    // self-hosting), else the copy installed in this site's private storage (checked first; the setup screen)
    let served = q.trim_start_matches('?').split('&').any(|kv| kv.starts_with("assets="));
    let assets =
        if served { assets::Assets::new(arg("assets", "../")) } else { assets::Assets::opfs(crate::platform::game_data::web::pointer().unwrap_or_default()) };
    run(Config {
        assets,
        fight: q.contains("fight=1").then(|| (arg("p1", "YUG"), arg("p2", "ALI"), arg("stage", "ST0"))),
        // `?unlock=1` (all characters and costumes, before they were open by default) is accepted and does nothing
        locked: q.trim_start_matches('?').split('&').any(|kv| kv == "locked=1"),
        debug: q.contains("debug=1"),
        perf: q.contains("perf=1"),
        replay: None,
        net,
        setup: None,
        own_data: !served,
    });
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use super::stepping::next_slot;
    use super::{FRAME, wrap};

    #[test]
    fn wrap_cuts_long_words() {
        assert_eq!(wrap("put files in /a/very/long/path here", 10), ["put files", "in", "/a/very/lo", "ng/path", "here"]);
        assert_eq!(wrap("short", 10), ["short"]);
    }

    /// Redraws every `dt` s for 60 s, each held back to its FRAME CAP slot as `about_to_wait` does: the slot never
    /// runs more than a frame ahead, and the rate stays at most 60 per second.
    fn paced(dt: f64) -> usize {
        let (mut t, mut due, mut n) = (0.0, 0.0, 0);
        while t < 60.0 {
            t = f64::max(t + dt, due);
            due = next_slot(due, t);
            assert!(due - t <= FRAME + 1e-9, "slot {due} ahead of {t}");
            n += 1;
        }
        n
    }

    #[test]
    fn frame_cap_slots() {
        for dt in [1.0 / 120.0, 1.0 / 1000.0, 1.0 / 60.0] {
            let n = paced(dt);
            assert!((3595..=3605).contains(&n), "{n} redraws at dt {dt}");
        }
        // uncapped redraws (the slot updated only with the cap on, but a backwards clock / a burst is the same):
        // a burst of 1000 at one instant leaves the slot one frame ahead
        let mut due = 0.0;
        for _ in 0..1000 {
            due = next_slot(due, 5.0);
        }
        assert_eq!(due, 5.0 + FRAME);
    }
}
