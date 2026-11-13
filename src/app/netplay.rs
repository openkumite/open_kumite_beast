//! The app's netplay: the connect screen, the lockstep's input to the steps, the panel, the desync dump.
//!
//! Engine addition.
//!
//! The app's netplay (game-state stages 8 and 9; `crate::net`): `--host [ADDR:]PORT` / `--join ADDR:PORT` (desktop,
//! UDP) or `--signal URL --room CODE` / `?signal=URL&room=CODE` (a room over WebRTC, feature `netplay-webrtc`; the
//! first player in the room hosts) [`--delay N` / `?delay=N`: the host's]. The window opens at once and the connect
//! runs in the frame loop (`net_connect_tick`), its progress on the screen (the small font over black: the front
//! end's data is loaded meanwhile) and on stderr; a failure shows its message (the desktop app quits `FAIL_SHOWN` s
//! later with exit code 1). Once through, the session is the host's (a direct fight's match loads then), and every
//! step's input comes from the lockstep: this side's pads (keyboard and gamepads, both key sets) drive its port only,
//! a step waits (no step, "WAITING FOR PLAYER n" after `WAIT_SHOWN` s) until the peer's input for it is there, and a
//! load or fetch counts as done once both sides reported it (`UserEvent::Loaded` / `Fetched` report this side's).
//! Netplay never writes the memory card or the preferences (the logic runs with the host's; this window keeps its
//! own); the debug tools and cheats are off; the replay list is not opened (REPLAY leaves the mode menu as it is);
//! EXIT GAME quits both. The session is recorded as usual (its matches into the replay store, the same files on
//! both sides). A stop (the other player left, timed out, a desync) freezes the session with its message; a desync
//! also dumps this side's state there and the session so far (`Netplay::write_desync`, in the save folder). The
//! netplay panel shows the round trip throughout.
//! While the window is occluded or the tab hidden the link is still polled and fed every `KEEPALIVE` s
//! (`net_keepalive`: desktop from `about_to_wait`, with a redraw asked for each time, so the steps go on wherever the
//! system still delivers redraws; the browser from a timer, which a hidden tab runs about once a second, with no
//! redraws: the steps pause): the other player waits instead of timing out. Closing the page says Bye (best effort:
//! the data channel may close first; the room's server then tells the other side the player left).
//! Debug OKB_NETSCRIPT: this side's scripted pad by session frame (OKB_SCRIPT's syntax; with OKB_SHOTS the app quits
//! 5 frames after the last shot, as with OKB_SCRIPT); OKB_NETSIM=lat_ms,loss_pct[,jitter_ms] simulates the network.
//!
//! The NETWORK menu (engine addition, `docs/network_menu.md`; the page is `net_menu`): the mode menu's NETWORK opens
//! it over the waiting session (`open_net_menu`); HOST and JOIN run the same connect from the frame loop, its
//! progress on the page, △ cancels it, a failure is the page's message. Once through, the live session is put aside
//! (as for a replay played from the list) and the agreed one starts at VS's character select (`Entry::Versus`);
//! it ends when either player picks NETWORK on its mode menu (both sides step the same press: `Request::Network`)
//! or when it stops (the other player left, timed out, a desync: its message), and then the live session comes
//! back with the page on it (`net_leave`). The command line's netplay is unchanged (its sessions have no NETWORK
//! row). Debug OKB_NETCODE (the host's room code), OKB_NETBIND (a direct-IP host's address), OKB_NETSHOTS (session
//! frames saved as `okb_net_<n>.png`).

use super::net_menu::{Cmd, Conn, NetMenu, NetPrefs};
use super::{App, UserEvent};
use crate::net::{Connect, Setup};
use crate::session::{FrameInput, SessionStart};

/// Seconds a step waits before the waiting line shows.
const WAIT_SHOWN: f64 = 0.1;
/// Step slots a stall may hold back for the catch-up after it.
pub(super) const CATCH_UP: f64 = 6.0;
/// Drawn frames after a desync before its dump is written (the other side's section hashes come in meanwhile).
const DUMP_AFTER: u32 = 60;
/// Seconds between the link's polls and sends while the window is occluded (desktop: a redraw is asked for at each,
/// at the steps' rate; a hidden tab's timer runs about once a second whatever is asked).
pub(super) const KEEPALIVE: f64 = super::FRAME;
/// Seconds a failed connect stays on the desktop's screen before the app quits.
#[cfg_attr(target_arch = "wasm32", allow(dead_code))]
const FAIL_SHOWN: f64 = 5.0;

/// The app's part of a netplay session.
#[derive(Default)]
pub(super) struct NetUi {
    /// the app runs a netplay session (set in `run`, for the app's lifetime: whatever the connect's state, the
    /// local path never runs and nothing is saved)
    pub(super) netplay: bool,
    /// the connect under way (`net_connect_tick`)
    pub(super) connect: Option<Connect>,
    /// why the connect failed (shown), and when the desktop app quits for it
    pub(super) failed: Option<String>,
    quit_at: Option<f64>,
    /// the session's state exists (`App::begin_session`)
    pub(super) started: bool,
    /// the agreed direct fight's match is loading (`net_start`)
    loading: bool,
    /// the connect screen's font is in the renderer's VRAM
    font_up: bool,
    /// the pads since this side's last input was taken (a press between two takes still counts)
    latch: u16,
    /// drawn frames since the session stopped
    stopped_for: u32,
    /// the window is occluded (`net_keepalive` runs the link) and the time of its next poll
    pub(super) occluded: bool,
    keepalive: f64,
    /// the NETWORK menu (the mode menu's NETWORK; the session waits on the mode menu while it is up)
    pub(super) menu: Option<NetMenu>,
    /// a netplay session of the NETWORK menu runs: the app's own start and costumes and the debug fast-forwards
    /// (OKB_FAST, OKB_FEFAST), back with the session put aside when it ends (`net_leave`)
    local: Option<(SessionStart, [u8; 2], [Option<i32>; 2])>,
    /// that session ends after this drawn frame's steps (`Request::Network` on both sides, or a stop), with this
    /// message (None: left by choice)
    leave: Option<Option<String>>,
    /// the last OKB_NETSHOTS frame saved
    #[cfg_attr(target_arch = "wasm32", allow(dead_code))]
    pub(super) shot_done: Option<u32>,
    /// debug OKB_NETEXIT: the UI frame of the menu's `okb_net_end.png` after a session (then the app quits)
    exit_at: Option<u64>,
}

impl NetUi {
    /// The agreed direct fight's match is loading.
    pub(super) fn loading(&self) -> bool {
        self.loading
    }

    /// Netplay waits for its session: connecting, failed, closed, or the direct fight's match loading.
    pub(super) fn before_session(&self) -> bool {
        self.netplay && !self.started
    }

    /// A netplay session of the NETWORK menu is ending: no more steps of it.
    pub(super) fn leaving(&self) -> bool {
        self.leave.is_some()
    }
}

impl App {
    /// The next step's input from the lockstep (None: it waits for the peer, or the session stopped). This side's
    /// pads, or-ed since its last input, go in when the lockstep takes one, with the loads done since.
    pub(super) fn net_input(&mut self, pads: [u16; 2]) -> Option<FrameInput> {
        // no step before the session's state exists (`begin_session`)
        if !self.net_ui.started {
            return None;
        }
        self.net_ui.latch |= pads[0] | pads[1];
        let n = self.net.as_mut()?;
        let pad =
            if n.wants_local() { std::mem::take(&mut self.net_ui.latch) | self.env.netscript.get(n.local_frame() as usize).copied().unwrap_or(0) } else { 0 };
        n.input(pad, &mut self.flags, crate::perf_now())
    }

    /// The connect's next poll (before a drawn frame's steps, and on the keep-alive timer): once through, the
    /// session agreed on starts (`net_start`); a failure is shown.
    pub(super) fn net_connect_tick(&mut self) {
        // not before the game data's content id is known (`content_found`): the handshake sends and checks it
        if self.content.is_none() {
            return;
        }
        let Some(c) = &mut self.net_ui.connect else { return };
        let now = crate::perf_now();
        let seen = c.line().to_string();
        match c.poll(now) {
            Ok(None) => {
                if c.line() != seen {
                    eprintln!("netplay: {}", c.line().to_lowercase());
                }
            }
            Ok(Some(agreed)) if self.net_ui.menu.is_some() => {
                let link = self.net_ui.connect.take().expect("the connect").finish();
                self.net_from_menu(&agreed, link, now);
            }
            Ok(Some(agreed)) => {
                let link = self.net_ui.connect.take().expect("the connect").finish();
                eprintln!(
                    "netplay: connected to {} as player {}, input delay {} frames, the {} build's menus (the host's)",
                    link.peer(),
                    agreed.port + 1,
                    agreed.delay,
                    if agreed.start.browser { "browser" } else { "desktop" }
                );
                self.start = agreed.start.clone();
                self.costume = agreed.costume;
                self.net = Some(crate::net::Netplay::new(link, &agreed, now));
                self.net_start();
            }
            Err(e) => {
                // the link goes first (a simulated network's queued datagrams leave with it)
                self.net_ui.connect = None;
                log::error!("netplay: {e}");
                eprintln!("netplay: {e}");
                // the NETWORK menu's: its message, the menu again
                if let Some(m) = &mut self.net_ui.menu {
                    m.message("NOT CONNECTED", &e);
                    return;
                }
                self.net_ui.failed = Some(e);
                self.net_ui.quit_at = Some(now + FAIL_SHOWN);
            }
        }
    }

    /// The agreed session starts once the startup's data is in too (`UserEvent::Ready` and the agreement may come in
    /// either order): `begin_session`, or for a direct fight the match's load first (its `Ready` begins it).
    pub(super) fn net_start(&mut self) {
        if self.net.is_none() || self.net_ui.started || self.net_ui.loading || self.renderer.is_none() {
            return;
        }
        let crate::session::Entry::DirectFight { p1, p2, stage } = &self.start.entry else {
            self.begin_session();
            return;
        };
        // without the front end, as a direct fight's session is (`resumed`)
        self.store.front = None;
        let Some(assets) = self.assets.clone() else { return };
        self.net_ui.loading = true;
        let (p1, p2, stage) = (p1.clone(), p2.clone(), stage.clone());
        let (proxy, costume, sim_version) = (self.proxy.clone(), self.costume, self.start.sim_version);
        let fut = async move {
            let ev = match crate::fight::fight_data::load_match(&assets, [&p1, &p2], costume, &stage, false, sim_version).await {
                Ok(l) => UserEvent::Ready(Box::new((None, None, Some(l)))),
                Err(e) => UserEvent::Failed(e),
            };
            let _ = proxy.send_event(ev);
        };
        #[cfg(not(target_arch = "wasm32"))]
        pollster::block_on(fut);
        #[cfg(target_arch = "wasm32")]
        wasm_bindgen_futures::spawn_local(fut);
    }

    /// A failed connect has been on the screen long enough: the desktop app quits (exit code 1, `run`).
    #[cfg_attr(target_arch = "wasm32", allow(dead_code))]
    pub(super) fn net_quit_due(&self) -> bool {
        self.net_ui.quit_at.is_some_and(|t| crate::perf_now() >= t)
    }

    /// After a drawn frame's steps: this side's inputs sent; a stop's message shown once, a desync's dump written;
    /// a session of the NETWORK menu that stopped or was left ends here (`net_leave`).
    pub(super) fn net_tick(&mut self) {
        if let Some(why) = self.net_ui.leave.take() {
            self.net_leave(why);
            return;
        }
        let Some(n) = &mut self.net else { return };
        n.flush(crate::perf_now());
        let Some(stop) = n.stopped().cloned() else { return };
        self.net_ui.stopped_for += 1;
        // the NETWORK menu's session: back to the menu with the message (a desync once its dump is written)
        let desync = matches!(stop, crate::net::Stop::Desync { .. });
        if self.net_ui.local.is_some() && self.net_ui.stopped_for == if desync { DUMP_AFTER + 1 } else { 1 } {
            self.net_ui.leave = Some(Some(stop.to_string()));
        }
        // the stop's line stays in the netplay panel (`net_lines`)
        if self.net_ui.stopped_for == 1 {
            self.toast = Some((stop.to_string(), 300));
        }
        #[cfg(not(target_arch = "wasm32"))]
        if matches!(stop, crate::net::Stop::Desync { .. }) && self.net_ui.stopped_for == DUMP_AFTER {
            let replay = self.rec.as_ref().map(|r| r.replay(&self.state, self.rec_time()).encode());
            let dir = crate::platform::storage::debug_path("");
            let Some(n) = &self.net else { return };
            for p in n.write_desync(&dir, replay.as_deref()) {
                log::error!("netplay: desync dump {p}");
            }
            self.toast = Some((format!("{stop} (dumps in {})", dir.display()), 600));
        }
        #[cfg(target_arch = "wasm32")]
        let _ = DUMP_AFTER;
    }

    /// Takes what came from the peer (before a drawn frame's steps).
    pub(super) fn net_poll(&mut self) {
        if let Some(n) = &mut self.net {
            n.poll(crate::perf_now());
        }
    }

    /// The window is occluded (desktop: redraws may not come) or the tab hidden: the connect polled, the link polled
    /// and this side's inputs sent every `KEEPALIVE` s. The time to come back and whether this call ran them; None
    /// when not needed.
    pub(super) fn net_keepalive(&mut self) -> Option<(f64, bool)> {
        if !self.net_ui.occluded || self.net.is_none() && self.net_ui.connect.is_none() {
            return None;
        }
        let now = crate::perf_now();
        let due = now >= self.net_ui.keepalive;
        if due {
            self.net_connect_tick();
            if let Some(n) = &mut self.net {
                n.poll(now);
                n.flush(now);
            }
            self.net_ui.keepalive = now + KEEPALIVE;
        }
        Some((self.net_ui.keepalive, due))
    }

    /// The netplay panel: the connect's lines (or why it failed); in the session the round trip, the waiting line
    /// while a step waits for the peer, the stop's message once stopped.
    pub(super) fn net_lines(&self) -> Vec<String> {
        // the NETWORK menu shows its connect itself
        if self.net_ui.menu.is_some() {
            return Vec::new();
        }
        if let Some(e) = &self.net_ui.failed {
            return super::wrap(&format!("netplay: {e}"), 62);
        }
        if let Some(c) = &self.net_ui.connect {
            return c.lines(crate::perf_now());
        }
        let Some(n) = &self.net else { return Vec::new() };
        if let Some(stop) = n.stopped() {
            return super::wrap(&stop.to_string(), 62);
        }
        if !self.net_ui.started {
            return vec!["NETPLAY: LOADING".into()];
        }
        let mut v = vec![match n.rtt() {
            Some(r) => format!("P{} RTT {:.0} MS", n.port + 1, r * 1e3),
            None => format!("P{} RTT -", n.port + 1),
        }];
        if let Some(t) = n.waiting(crate::perf_now()).filter(|&t| t >= WAIT_SHOWN) {
            v.push(format!("WAITING FOR PLAYER {} {:.1} S", 2 - n.port, t));
        }
        v
    }

    /// The connect screen (no session yet): the netplay panel over black, in the front end's small font.
    pub(super) fn draw_net_screen(&mut self, overlay: &[crate::output::prim::Prim]) {
        let Some(r) = &mut self.renderer else { return };
        if !self.net_ui.font_up
            && let Some(data) = &self.store.front
        {
            r.upload_vram(&data.font_vram().words);
            self.net_ui.font_up = true;
            // the session's front end puts its own up
            self.vram_view.invalidate();
        }
        r.afterimage = 0;
        r.ambient = crate::output::layout::ambient_split(overlay, r.viewport());
        let quads = crate::output::layout::to_quads(overlay, r.viewport());
        r.frame_capture(&[], &quads, &[], [0.0, 0.0, 0.0], false);
    }

    /// The overlay's netplay line (F1).
    pub(super) fn net_status(&self) -> Option<String> {
        let n = self.net.as_ref()?;
        let s = &n.stats;
        Some(format!(
            "NETPLAY P{} delay {} frame {} (peer {}) rtt {:.0} ms stalls {} ({:.1} s) checks {}",
            n.port + 1,
            n.delay,
            n.next(),
            n.peer_at(),
            n.rtt().unwrap_or(0.0) * 1e3,
            s.stalls,
            s.stall_secs,
            s.checks
        ))
    }

    /// The app quits or the page goes: the other player is told (desktop after the event loop, the browser from
    /// `exiting` / `suspended`); a connect under way is dropped (a room's server tells the other side).
    pub(super) fn net_close(&mut self) {
        // a page restored from the back-forward cache shows why nothing goes on
        if self.net_ui.connect.take().is_some() && self.net_ui.failed.is_none() {
            self.net_ui.failed = Some("the page was left: the connect stopped (reload it to connect again)".into());
        }
        // once (the page's `suspended` and `exiting` may both come)
        if let Some(n) = &mut self.net
            && !matches!(n.stopped(), Some(crate::net::Stop::Quit(_)))
        {
            n.close(&format!("player {} quit", n.port + 1));
        }
    }

    /// A replay's time: the session's clock in netplay (the host's, the same on both sides), else now.
    pub(super) fn rec_time(&self) -> u64 {
        self.net.as_ref().map_or(crate::now() as u64, |n| n.time_at(self.state.frame))
    }

    /// The app may write the card and the preferences: not in a playback, not in netplay.
    pub(super) fn saves(&self) -> bool {
        self.player.is_none() && !self.net_ui.netplay
    }
}

/// The connect of `setup` (`--host` / `--join` / `--signal` + `--room`, or the URL's): the link opened, the host's
/// offer its own session (the env's debug fight setup and card fault left out: no cheats in netplay), this side's
/// own P1 map for when it joins (read again once the data is in: `content_found`). Err when the link cannot be opened.
pub(super) fn connect(setup: &Setup, delay: Option<u32>, mut start: crate::session::SessionStart, costume: [u8; 2]) -> Result<Connect, String> {
    let sim = crate::net::link::SimNet::from_env()?;
    let now = crate::perf_now();
    let link = setup.open(sim, now)?;
    start.fight = Default::default();
    // no TEAM BATTLE over the network (phase 2 of the team battle)
    start.netplay = true;
    start.card_fault = None;
    let offer = crate::net::Offer {
        start,
        prefs: crate::frontend::options::Prefs::from_stored(crate::platform::storage::read_prefs()),
        costume,
        time: crate::now() as u64,
        delay,
    };
    let what = setup.describe();
    eprintln!("{}…", what.to_lowercase());
    Ok(Connect::new(link, offer, crate::net::handshake::own_buttons(), what, now))
}

impl App {
    /// The mode menu's NETWORK: the page opens over the waiting session with the stored settings.
    pub(super) fn open_net_menu(&mut self, held: [u16; 2]) {
        let prefs = NetPrefs::from_stored(crate::platform::storage::read_net_prefs());
        self.net_ui.menu = Some(NetMenu::new(prefs, held));
    }

    /// One 60 Hz frame of the NETWORK menu with these pads: its sounds through the front end's banks, its command
    /// run; the connect's lines onto its screen.
    pub(super) fn net_menu_tick(&mut self, pads: [u16; 2]) {
        let pads = self.ui_pads(pads);
        if let Some(at) = self.net_ui.exit_at {
            if self.ui_frame == at {
                self.shot_req = true;
                self.ui_shot_name = Some("okb_net_end.png".into());
            } else if self.ui_frame > at + 5 {
                self.exit_req = true;
            }
        }
        let now = crate::perf_now();
        let lines = self.net_ui.connect.as_ref().map(|c| c.lines(now));
        let Some(m) = &mut self.net_ui.menu else { return };
        if let Some(l) = lines {
            m.conn.status = l.into_iter().skip(1).collect();
        }
        let mut sounds = Vec::new();
        let cmd = m.tick(pads, &mut sounds);
        if let Some(f) = &self.state.front {
            let mut q = crate::output::sound::Queue::new(false);
            for id in sounds {
                f.play_se(&mut q, id);
            }
            self.audio.apply(&mut q);
        }
        match cmd {
            Cmd::None => {}
            Cmd::Close => {
                self.net_ui.menu = None;
                // the △ / × that closed it does not reach the mode menu
                self.swallow = pads[0] | pads[1];
            }
            Cmd::Prefs(p) => {
                if let Some(text) = p.to_stored() {
                    crate::platform::storage::write_net_prefs(&text);
                }
            }
            Cmd::Cancel => {
                if self.net_ui.connect.take().is_some() {
                    log::info!("netplay: the connect was cancelled");
                }
            }
            Cmd::Host => {
                let r = self.net_host();
                self.connect_from_menu(r);
            }
            Cmd::Join(text) => {
                if let Some(text) = self.net_ui.menu.as_ref().and_then(|m| m.prefs.to_stored()) {
                    crate::platform::storage::write_net_prefs(&text);
                }
                let r = self.net_join(&text);
                self.connect_from_menu(r);
            }
        }
    }

    /// HOST with the menu's settings: the setup and the connect's screen (the room code to tell, or this machine's
    /// addresses).
    fn net_host(&mut self) -> Result<(Setup, Conn), String> {
        let p = self.net_ui.menu.as_ref().map(|m| m.prefs.clone()).unwrap_or_default();
        if p.direct {
            #[cfg(not(target_arch = "wasm32"))]
            {
                let ip = self.env.netbind.unwrap_or(std::net::IpAddr::V4(std::net::Ipv4Addr::UNSPECIFIED));
                let role = crate::net::udp::Role::Host(std::net::SocketAddr::new(ip, p.port));
                let addrs: Vec<std::net::IpAddr> = match self.env.netbind {
                    Some(ip) => vec![ip],
                    None => crate::net::udp::lan_address().into_iter().collect(),
                };
                let mut info = Vec::new();
                let big = match addrs.first() {
                    Some(a) => Some(("YOUR ADDRESS", std::net::SocketAddr::new(*a, p.port).to_string())),
                    None => {
                        info.push(format!("NO NETWORK ADDRESS FOUND (PORT {})", p.port));
                        None
                    }
                };
                info.push("THE OTHER PLAYER CHOOSES JOIN AND TYPES IT.".into());
                return Ok((Setup::Udp(role), Conn { title: "HOST", big, info, status: Vec::new() }));
            }
        }
        #[cfg(feature = "netplay-webrtc")]
        {
            let code = self.env.netcode.clone().unwrap_or_else(|| super::net_menu::room_code(crate::net::handshake::nonce() as u64 ^ crate::clock_seed()));
            let room = room_of(&p, &code)?;
            let info = vec!["TELL THE OTHER PLAYER THIS CODE:".into(), "THEY CHOOSE JOIN AND TYPE IT.".into()];
            Ok((Setup::Room(room), Conn { title: "HOST", big: Some(("ROOM CODE", room_code_shown(&code))), info, status: Vec::new() }))
        }
        #[cfg(not(feature = "netplay-webrtc"))]
        Err("this build has no room codes (feature netplay-webrtc): choose DIRECT IP in SETTINGS".into())
    }

    /// JOIN `text` (a room code, or by direct IP the host's address) with the menu's settings.
    fn net_join(&mut self, text: &str) -> Result<(Setup, Conn), String> {
        let p = self.net_ui.menu.as_ref().map(|m| m.prefs.clone()).unwrap_or_default();
        let text = text.trim();
        if p.direct {
            #[cfg(not(target_arch = "wasm32"))]
            {
                // parsed, not resolved (`--join`'s `Role::join` looks names up)
                let role = crate::net::udp::Role::Join(super::net_menu::host_address(text)?);
                let info = vec![format!("JOINING {text}")];
                return Ok((Setup::Udp(role), Conn { title: "JOIN", big: None, info, status: Vec::new() }));
            }
        }
        #[cfg(feature = "netplay-webrtc")]
        {
            let room = room_of(&p, text)?;
            let info = vec![format!("JOINING ROOM {}", room.code)];
            Ok((Setup::Room(room), Conn { title: "JOIN", big: None, info, status: Vec::new() }))
        }
        #[cfg(not(feature = "netplay-webrtc"))]
        {
            // (a browser build without the feature has no way at all)
            let _ = text;
            Err("this build has no room codes (feature netplay-webrtc): choose DIRECT IP in SETTINGS".into())
        }
    }

    /// The connect of HOST / JOIN started (its screen up), or the reason it cannot be the page's message. The
    /// session offered: the app's own start as a new session would have it (the stored card, a new seed) at VS's
    /// character select with the NETWORK row (`Entry::Versus`); the debug tools refuse it (their cheats and state
    /// loads would change one side only).
    fn connect_from_menu(&mut self, r: Result<(Setup, Conn), String>) {
        let r = r.and_then(|(setup, conn)| {
            if self.tools.is_some() {
                return Err("the debug tools are on (--debug / OKB_DEBUG): no netplay with them".into());
            }
            let extra = self.assets.as_ref().map_or(0, crate::assets::extras::present);
            let br1 = self.assets.as_ref().and_then(crate::assets::manifest::br1_install_id);
            let mut start = self.env.start(None, self.start.locked, extra, br1);
            start.entry = crate::session::Entry::Versus;
            start.content = self.content;
            let delay = self.net_ui.menu.as_ref().and_then(|m| m.prefs.delay());
            connect(&setup, delay, start, crate::fight::fight_data::direct_costume()).map(|c| (c, conn))
        });
        let Some(m) = &mut self.net_ui.menu else { return };
        match r {
            Ok((c, conn)) => {
                m.connecting(conn);
                self.net_ui.connect = Some(c);
            }
            Err(e) => {
                log::warn!("netplay: {e}");
                m.message("NOT CONNECTED", &e);
            }
        }
    }

    /// The NETWORK menu's connect is through: the live session put aside (with its start), the agreed one begun.
    fn net_from_menu(&mut self, agreed: &crate::net::Agreed, link: Box<dyn crate::net::Transport>, now: f64) {
        eprintln!("netplay: connected to {} as player {}, input delay {} frames (the NETWORK menu)", link.peer(), agreed.port + 1, agreed.delay);
        let start = std::mem::replace(&mut self.start, agreed.start.clone());
        let costume = std::mem::replace(&mut self.costume, agreed.costume);
        // the debug fast-forwards are off in netplay (as on the command line), back after it
        self.net_ui.local = Some((start, costume, [self.env.fast.take(), self.env.fefast.take()]));
        self.net_ui.menu = None;
        self.put_aside();
        self.net = Some(crate::net::Netplay::new(link, agreed, now));
        self.net_ui.netplay = true;
        self.begin_session();
    }

    /// `Request::Network` in a netplay session: one of the NETWORK menu's ends after this frame's steps (both sides
    /// step the same press); the command line's has no NETWORK row (`SessionStart::network`).
    pub(super) fn net_leave_request(&mut self) {
        if self.net_ui.local.is_some() {
            self.net_ui.leave.get_or_insert(None);
        } else {
            self.toast = Some(("NETWORK: not in a command-line netplay session".into(), 180));
        }
    }

    /// The NETWORK menu's session ends (left, or stopped with `why`): the other player told, its recording dropped
    /// (its finished matches are in the replay store), the session put aside back with the menu over it.
    fn net_leave(&mut self, why: Option<String>) {
        let Some((start, costume, [fast, fefast])) = self.net_ui.local.take() else { return };
        if let Some(mut n) = self.net.take() {
            // the inputs of the drawn frame that left go out before the Bye, so the other side reaches the same frame
            n.flush(crate::perf_now());
            n.close(&format!("player {} went back to the NETWORK menu", n.port + 1));
        }
        log::info!("netplay: the session ended{}", why.as_deref().map(|w| format!(": {w}")).unwrap_or_default());
        self.rec = None;
        self.start = start;
        self.costume = costume;
        (self.env.fast, self.env.fefast) = (fast, fefast);
        self.net_ui = NetUi { occluded: self.net_ui.occluded, ..Default::default() };
        if self.toast.as_ref().is_some_and(|t| t.0.starts_with("netplay:")) {
            self.toast = None;
        }
        self.bring_back();
        let held = self.pads();
        self.open_net_menu(held);
        self.net_ui.exit_at = self.env.netexit.map(|n| self.ui_frame + n);
        if let (Some(m), Some(w)) = (&mut self.net_ui.menu, why) {
            m.message("SESSION ENDED", w.trim_start_matches("netplay: "));
        }
    }
}

/// The room of `code` on the settings' signaling server, with their STUN / TURN server (none unless set).
#[cfg(feature = "netplay-webrtc")]
fn room_of(p: &NetPrefs, code: &str) -> Result<crate::net::webrtc::Room, String> {
    let opt = |s: &str| (!s.trim().is_empty()).then(|| s.trim().to_string());
    let turn = match (opt(&p.turn), opt(&p.turn_user), opt(&p.turn_pass)) {
        (Some(t), Some(u), Some(w)) => Some((t, u, w)),
        (None, _, _) => None,
        _ => return Err("the TURN SERVER needs its TURN USER and TURN PASSWORD (SETTINGS)".into()),
    };
    crate::net::webrtc::Room::new(&p.signal, code)?.with_ice(opt(&p.stun).as_deref(), turn.as_ref().map(|(t, u, w)| (t.as_str(), u.as_str(), w.as_str())))
}

/// A room code as the host's screen shows it.
#[cfg(feature = "netplay-webrtc")]
fn room_code_shown(code: &str) -> String {
    code.trim().to_ascii_uppercase()
}

/// The browser's keep-alive timer (a hidden tab runs it about once a second): `UserEvent::KeepAlive` to the app.
#[cfg(target_arch = "wasm32")]
pub(super) fn keepalive_timer(proxy: winit::event_loop::EventLoopProxy<UserEvent>) {
    use wasm_bindgen::JsCast;
    let tick = wasm_bindgen::closure::Closure::<dyn FnMut()>::new(move || {
        let _ = proxy.send_event(UserEvent::KeepAlive);
    });
    if let Some(w) = web_sys::window() {
        let _ = w.set_interval_with_callback_and_timeout_and_arguments_0(tick.as_ref().unchecked_ref(), (KEEPALIVE * 1e3) as i32);
    }
    // the timer lives as long as the page
    tick.forget();
}

/// `?signal=URL&room=CODE[&delay=N][&stun=URL][&turn=URL&turnuser=U&turnpass=P]` (the values percent-decoded): the
/// browser's netplay setup, None without `room` and the others; Err for a value that does not check out, or one
/// without `room` (as the desktop's `--signal` without `--room`).
#[cfg(target_arch = "wasm32")]
pub(super) fn web_setup(arg: impl Fn(&str) -> Option<String>) -> Result<Option<(Setup, Option<u32>)>, String> {
    let (Some(room), signal) = (arg("room"), arg("signal")) else {
        return match ["signal", "delay", "stun", "turn", "turnuser", "turnpass"].into_iter().find(|k| arg(k).is_some()) {
            Some(k) => Err(format!("{k}= needs room=CODE (and signal=ws://HOST:PORT)")),
            None => Ok(None),
        };
    };
    let delay = match arg("delay") {
        Some(d) => match d.parse::<u32>() {
            Ok(n) if (1..=crate::net::MAX_DELAY).contains(&n) => Some(n),
            _ => return Err(format!("delay={d}: 1..={}", crate::net::MAX_DELAY)),
        },
        None => None,
    };
    #[cfg(feature = "netplay-webrtc")]
    {
        let Some(signal) = signal else { return Err("room= needs signal=ws://HOST:PORT (a matchbox signaling server)".into()) };
        // all or none, as the desktop's --turn / --turn-user / --turn-pass
        let turn = match (arg("turn"), arg("turnuser"), arg("turnpass")) {
            (Some(t), Some(u), Some(p)) => Some((t, u, p)),
            (None, None, None) => None,
            _ => return Err("turn=URL needs turnuser=U and turnpass=P".into()),
        };
        let r = crate::net::webrtc::Room::new(&signal, &room)?
            .with_ice(arg("stun").as_deref(), turn.as_ref().map(|(t, u, p)| (t.as_str(), u.as_str(), p.as_str())))?;
        Ok(Some((Setup::Room(r), delay)))
    }
    #[cfg(not(feature = "netplay-webrtc"))]
    {
        let _ = (signal, room, delay);
        Err("this build has no netplay (feature netplay-webrtc)".into())
    }
}
