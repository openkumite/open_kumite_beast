//! WebRTC netplay transport: a room's data channel through a matchbox signaling server.
//!
//! Engine addition.
//!
//! WebRTC netplay (game-state stage 9, feature `netplay-webrtc`): the two players of a room meet through a matchbox
//! signaling server (`matchbox_server`, self-hosted: no server is built in) and then talk over an unreliable,
//! unordered data channel, peer to peer, as UDP does (the protocol's redundancy and acks cover the losses). Desktop
//! (`--signal URL --room CODE`) and browser (`?signal=URL&room=CODE`) builds meet in the same rooms.
//!
//! - The room is `URL/okb-CODE?next=2`: the server pairs the players of a code two by two (a third one waits for a
//!   fourth instead of joining the pair).
//! - Sides: the first player in the room hosts (player 1, port 0). Each side measures how long it waited from
//!   opening its socket (it joins the room at once, on the socket's own task) to the data channel opening, and sends
//!   that in a probe (`PROBE`, with a random tiebreak) every `PROBE_EVERY` s until it has the other's; the one that
//!   waited longer came first (`hosts`). The opening's time is taken off the frame loop on the desktop (a thread
//!   watches the socket until then: a load blocks the loop there); the browser's loop takes it, never blocked for
//!   long (its loads are async), so a side whose first frame comes late may count up to that delay too much. Both
//!   decide from the same two probes, so they always agree; the probes never reach the handshake.
//! - No ICE server unless one is given (`--stun` / `?stun=`, `--turn` / `?turn=` with its user and password): host
//!   candidates only, which reach the same machine and the LAN; nothing is sent to a third party (matchbox's own
//!   default, two public STUN servers, is always replaced; its browser side passes an empty list for no URL).
//! - A peer's datagrams are untrusted: only the paired peer's are taken, at most `MAX_PACKET` bytes each, at most
//!   `INBOX` waiting; the probes are checked to the byte.

use super::link::{Link, Transport};
use super::proto::MAX_PACKET;
use matchbox_socket::{ChannelConfig, Packet, PeerId, PeerState, RtcIceServerConfig, WebRtcSocket, WebRtcSocketBuilder};
use std::collections::VecDeque;
use std::sync::{Arc, Mutex, MutexGuard};

/// The longest room code.
pub const MAX_ROOM: usize = 16;
/// The longest signaling or ICE server URL, user name or password.
const MAX_URL: usize = 256;
/// No answer from the signaling server for this long (seconds) stops the connect.
const SIGNAL_TIMEOUT: f64 = 20.0;
/// Nobody connected in the room for this long (seconds) stops the connect (the other player may also be one this
/// side cannot reach without a STUN / TURN server).
pub const WAIT_PEER: f64 = 300.0;
/// Seconds between the side probes.
const PROBE_EVERY: f64 = 0.1;
/// A side probe: `PROBE`, version 1, flags (bit 0: the sender has the receiver's probe), waited u32 (ms), tiebreak
/// u64, little-endian (`BR2W`: a historical protocol id).
const PROBE: &[u8; 4] = b"BR2W";
const PROBE_LEN: usize = 18;
/// Datagrams kept for the handshake while the sides are decided, and between two polls.
const INBOX: usize = 256;
/// Seconds between the desktop watcher's looks at the socket (until the peer's data channel is open).
#[cfg(not(target_arch = "wasm32"))]
const WATCH_EVERY: f64 = 0.002;

/// An ICE server (`--stun` / `--turn`): one entry of the peer connection's configuration.
#[derive(Clone, Debug, PartialEq)]
pub struct Ice {
    pub urls: Vec<String>,
    pub username: Option<String>,
    pub credential: Option<String>,
}

/// A room on a signaling server.
#[derive(Clone, Debug, PartialEq)]
pub struct Room {
    /// `ws://` or `wss://` host[:port][/path], no trailing `/`
    pub signal: String,
    /// 1..=`MAX_ROOM` letters and digits, upper case
    pub code: String,
    pub ice: Option<Ice>,
}

impl Room {
    /// `signal` and `code` checked (`signal_url`, `room_code`); no ICE server.
    pub fn new(signal: &str, code: &str) -> Result<Self, String> {
        Ok(Self { signal: signal_url(signal)?, code: room_code(code)?, ice: None })
    }

    /// With a STUN server (`stun:` / `stuns:` URL) and / or a TURN server (`turn:` / `turns:` URL, its user and
    /// password).
    pub fn with_ice(mut self, stun: Option<&str>, turn: Option<(&str, &str, &str)>) -> Result<Self, String> {
        let mut ice = Ice { urls: Vec::new(), username: None, credential: None };
        if let Some(s) = stun {
            ice.urls.push(ice_url(s, &["stun:", "stuns:"])?);
        }
        if let Some((url, user, pass)) = turn {
            ice.urls.push(ice_url(url, &["turn:", "turns:"])?);
            for (what, v) in [("TURN user", user), ("TURN password", pass)] {
                if v.is_empty() || v.len() > MAX_URL || v.chars().any(char::is_control) {
                    return Err(format!("{what}: 1..={MAX_URL} characters, no control characters"));
                }
            }
            (ice.username, ice.credential) = (Some(user.into()), Some(pass.into()));
        }
        self.ice = (!ice.urls.is_empty()).then_some(ice);
        Ok(self)
    }

    /// The room's URL on the server (the server pairs its players two by two).
    pub fn url(&self) -> String {
        format!("{}/okb-{}?next=2", self.signal, self.code)
    }
}

/// A room code: 1..=`MAX_ROOM` ASCII letters and digits (upper-cased: `test` and `TEST` meet).
pub fn room_code(s: &str) -> Result<String, String> {
    let s = s.trim();
    if s.is_empty() || s.len() > MAX_ROOM || !s.bytes().all(|c| c.is_ascii_alphanumeric()) {
        return Err(format!("room code {s:?}: 1 to {MAX_ROOM} letters and digits"));
    }
    Ok(s.to_ascii_uppercase())
}

/// A signaling server's URL: `ws://` or `wss://` and a host, at most `MAX_URL` bytes, no query, fragment, space or
/// control character (the room is appended).
pub fn signal_url(s: &str) -> Result<String, String> {
    let s = s.trim().trim_end_matches('/');
    let rest = s.strip_prefix("ws://").or_else(|| s.strip_prefix("wss://"));
    match rest {
        Some(r) if !r.is_empty() && !r.starts_with('/') && s.len() <= MAX_URL && !s.contains(['?', '#', ' ', '\\']) && !s.chars().any(char::is_control) => {
            Ok(s.to_string())
        }
        _ => Err(format!("signaling server {s:?}: ws://HOST[:PORT][/PATH] or wss://… (at most {MAX_URL} characters, no query)")),
    }
}

/// An ICE server's URL with one of `schemes`.
fn ice_url(s: &str, schemes: &[&str]) -> Result<String, String> {
    let s = s.trim();
    let ok = schemes.iter().any(|p| s.strip_prefix(p).is_some_and(|r| !r.is_empty()));
    if !ok || s.len() > MAX_URL || s.contains([' ', '?', '#']) || s.chars().any(char::is_control) {
        return Err(format!("ICE server {s:?}: {}HOST[:PORT]", schemes[0]));
    }
    Ok(s.to_string())
}

/// The socket and the peer changes taken from it, each with its time (`crate::perf_now`): whoever looks first (the
/// desktop's watcher, `pump`) takes them, `pump` acts on them.
struct Shared {
    socket: WebRtcSocket,
    changes: Vec<(PeerId, PeerState, f64)>,
    /// `try_update_peers` reported the socket closed
    closed: bool,
    /// a peer's data channel opened (the watcher's work is done)
    connected: bool,
}

impl Shared {
    fn update(&mut self) {
        if self.closed {
            return;
        }
        match self.socket.try_update_peers() {
            Ok(c) => {
                let now = crate::perf_now();
                self.connected |= c.iter().any(|(_, s)| *s == PeerState::Connected);
                self.changes.extend(c.into_iter().map(|(id, s)| (id, s, now)));
            }
            Err(_) => self.closed = true,
        }
    }
}

/// One side of a room: the socket and the side probes.
pub struct WebRtc {
    sh: Arc<Mutex<Shared>>,
    code: String,
    signal: String,
    /// why the socket's message loop ended (the signaling server went, or could not be reached)
    ended: Arc<Mutex<Option<String>>>,
    opened: f64,
    /// `opened` on `crate::perf_now`'s clock (the peer changes' times)
    opened_perf: f64,
    /// the time of the last `link` (`recv` and `lost` take what came with it: they have no clock)
    last: f64,
    /// when the server gave this side its id
    id_at: Option<f64>,
    /// the paired peer, once its data channel is open
    peer: Option<PeerId>,
    /// ms from opening to the peer's data channel, and this side's tiebreak
    mine: (u32, u64),
    /// the peer's probe, and that it has this side's
    theirs: Option<(u32, u64)>,
    probed: f64,
    port: Option<usize>,
    inbox: VecDeque<Vec<u8>>,
    lost: Option<String>,
}

impl WebRtc {
    /// Joins `room` (the socket's loop runs on a thread of its own on the desktop, as a task in the browser); `now`:
    /// seconds, monotonic (the same clock for every later call).
    pub fn open(room: &Room, now: f64) -> Result<Self, String> {
        let ice = room.ice.clone().unwrap_or(Ice { urls: Vec::new(), username: None, credential: None });
        let (socket, run) = WebRtcSocketBuilder::new(room.url())
            .ice_server(RtcIceServerConfig { urls: ice.urls, username: ice.username, credential: ice.credential })
            .add_channel(ChannelConfig::unreliable())
            .build();
        let ended = Arc::new(Mutex::new(None));
        let end = ended.clone();
        let fut = async move {
            let why = match run.await {
                Ok(()) => "the connection closed".to_string(),
                Err(e) => e.to_string(),
            };
            log::info!("netplay: the WebRTC socket's loop ended: {why}");
            if let Ok(mut e) = end.lock() {
                *e = Some(why);
            }
        };
        #[cfg(not(target_arch = "wasm32"))]
        std::thread::Builder::new().name("webrtc".into()).spawn(move || pollster::block_on(fut)).map_err(|e| format!("cannot start the WebRTC thread: {e}"))?;
        #[cfg(target_arch = "wasm32")]
        wasm_bindgen_futures::spawn_local(fut);
        let opened_perf = crate::perf_now();
        let sh = Arc::new(Mutex::new(Shared { socket, changes: Vec::new(), closed: false, connected: false }));
        // the desktop's watcher: the peer's data channel taken when it opens, whatever the frame loop does meanwhile
        #[cfg(not(target_arch = "wasm32"))]
        {
            let watched = Arc::downgrade(&sh);
            std::thread::Builder::new()
                .name("webrtc-watch".into())
                .spawn(move || {
                    while let Some(sh) = watched.upgrade() {
                        let mut s = lock(&sh);
                        s.update();
                        if s.closed || s.connected {
                            break;
                        }
                        drop(s);
                        drop(sh);
                        std::thread::sleep(std::time::Duration::from_secs_f64(WATCH_EVERY));
                    }
                })
                .map_err(|e| format!("cannot start the WebRTC watcher: {e}"))?;
        }
        log::info!("netplay: room {} on {} (ICE server: {})", room.code, room.signal, room.ice.as_ref().map_or("none".into(), |i| i.urls.join(", ")));
        use std::hash::BuildHasher;
        let tiebreak = std::collections::hash_map::RandomState::new().hash_one((crate::clock_seed(), now.to_bits()));
        Ok(Self {
            sh,
            code: room.code.clone(),
            signal: room.signal.clone(),
            ended,
            opened: now,
            opened_perf,
            last: now,
            id_at: None,
            peer: None,
            mine: (0, tiebreak),
            theirs: None,
            probed: f64::NEG_INFINITY,
            port: None,
            inbox: VecDeque::new(),
            lost: None,
        })
    }

    fn ended(&self) -> Option<String> {
        self.ended.lock().ok().and_then(|e| e.clone())
    }

    fn send_to_peer(&mut self, b: &[u8]) {
        if let (Some(p), None) = (self.peer, &self.lost) {
            // the socket's loop gone: nothing goes out (`lost` says so)
            let _ = lock(&self.sh).socket.channel_mut(0).try_send(Packet::copy_from_slice(b), p);
        }
    }

    fn probe(&mut self) {
        let mut b = Vec::with_capacity(PROBE_LEN);
        b.extend_from_slice(PROBE);
        b.push(1);
        b.push(self.theirs.is_some() as u8);
        b.extend_from_slice(&self.mine.0.to_le_bytes());
        b.extend_from_slice(&self.mine.1.to_le_bytes());
        self.send_to_peer(&b);
    }

    /// The peer's probe: (waited, tiebreak, it has this side's); None for anything else.
    fn read_probe(b: &[u8]) -> Option<(u32, u64, bool)> {
        if b.len() != PROBE_LEN || &b[..4] != PROBE || b[4] != 1 || b[5] > 1 {
            return None;
        }
        Some((u32::from_le_bytes(b[6..10].try_into().ok()?), u64::from_le_bytes(b[10..18].try_into().ok()?), b[5] == 1))
    }

    /// The socket's news: this side's id, the peer's data channel opening or closing, its datagrams.
    fn pump(&mut self, now: f64) {
        let mut sh = lock(&self.sh);
        if self.id_at.is_none() && sh.socket.id().is_some() {
            self.id_at = Some(now);
        }
        sh.update();
        let (changes, closed) = (std::mem::take(&mut sh.changes), sh.closed);
        drop(sh);
        for (id, state, at) in changes {
            match state {
                PeerState::Connected if self.peer.is_none() => {
                    self.peer = Some(id);
                    self.mine.0 = ((at - self.opened_perf) * 1e3).clamp(0.0, u32::MAX as f64) as u32;
                    log::info!("netplay: room {}: the other player's data channel is open (waited {} ms)", self.code, self.mine.0);
                }
                PeerState::Connected => log::warn!("netplay: room {}: another peer ({id}) ignored", self.code),
                PeerState::Disconnected if Some(id) == self.peer => {
                    self.lost.get_or_insert_with(|| "the other player left the room".into());
                }
                PeerState::Disconnected => {}
            }
        }
        // the socket's loop ended: before the peer came that is the signaling server's doing
        if self.lost.is_none() && (closed || self.ended().is_some()) {
            let why = self.ended().unwrap_or_else(|| "closed".into());
            self.lost = Some(match (self.id_at, self.peer) {
                (None, _) => format!("cannot reach the signaling server {} ({why})", self.signal),
                (Some(_), None) => format!("lost the signaling server {} ({why})", self.signal),
                (Some(_), Some(_)) => format!("the connection to the other player closed ({why})"),
            });
        }
        let Some(peer) = self.peer else {
            // nothing comes before the peer's channel is open; anyone else's datagrams go
            let _ = lock(&self.sh).socket.channel_mut(0).receive();
            return;
        };
        let got = lock(&self.sh).socket.channel_mut(0).receive();
        for (from, b) in got {
            if from != peer || b.len() > MAX_PACKET {
                continue;
            }
            if let Some((waited, tiebreak, has_mine)) = Self::read_probe(&b) {
                let first = self.theirs.is_none();
                self.theirs = Some((waited, tiebreak));
                if first || !has_mine {
                    self.probe();
                }
                continue;
            }
            if self.inbox.len() >= INBOX {
                self.inbox.pop_front();
            }
            self.inbox.push_back(b.to_vec());
        }
        if self.port.is_none()
            && let Some(theirs) = self.theirs
        {
            let me = lock(&self.sh).socket.id().map(|i| i.to_string()).unwrap_or_default();
            let me_first = hosts(self.mine, theirs, &me, &peer.to_string());
            self.port = Some(if me_first { 0 } else { 1 });
            log::info!("netplay: room {}: player {} (waited {} ms, the other player {} ms)", self.code, if me_first { 1 } else { 2 }, self.mine.0, theirs.0);
        }
    }
}

impl Transport for WebRtc {
    fn send(&mut self, b: &[u8]) {
        if self.port.is_some() {
            self.send_to_peer(b);
        }
    }

    fn recv(&mut self) -> Option<Vec<u8>> {
        if self.inbox.is_empty() {
            self.pump(self.last);
        }
        self.inbox.pop_front()
    }

    fn peer(&self) -> String {
        match self.peer {
            Some(p) => format!("room {} ({p})", self.code),
            None => format!("room {}", self.code),
        }
    }

    fn link(&mut self, now: f64) -> Result<Link, String> {
        self.last = now;
        self.pump(now);
        if let Some(why) = &self.lost {
            return Err(why.clone());
        }
        if let Some(port) = self.port {
            return Ok(Link::Up(Some(port)));
        }
        let Some(id_at) = self.id_at else {
            if now - self.opened > SIGNAL_TIMEOUT {
                return Err(format!("no answer from the signaling server {} for {SIGNAL_TIMEOUT:.0} s", self.signal));
            }
            return Ok(Link::Pending(format!("CONNECTING TO {}", self.signal)));
        };
        if self.peer.is_none() {
            if now - id_at > WAIT_PEER {
                return Err(format!(
                    "nobody connected in room {} for {WAIT_PEER:.0} s (an other player behind a NAT needs a STUN or TURN server: --stun / ?stun=)",
                    self.code
                ));
            }
            return Ok(Link::Pending(format!("ROOM {}: WAITING FOR THE OTHER PLAYER", self.code)));
        }
        if now - self.probed >= PROBE_EVERY {
            self.probed = now;
            self.probe();
        }
        Ok(Link::Pending(format!("ROOM {}: CONNECTING TO THE OTHER PLAYER", self.code)))
    }

    fn lost(&mut self) -> Option<String> {
        if self.lost.is_none() {
            self.pump(self.last);
        }
        self.lost.clone()
    }
}

/// The socket closed with this side (its channels: the loop ends, the server tells the peer).
impl Drop for WebRtc {
    fn drop(&mut self) {
        lock(&self.sh).socket.close();
    }
}

/// The socket (still taken after a panic elsewhere while it was held: it is at least to be closed).
fn lock(sh: &Mutex<Shared>) -> MutexGuard<'_, Shared> {
    sh.lock().unwrap_or_else(|e| e.into_inner())
}

/// This side hosts: it waited longer (`mine` / `theirs`: the probes' wait in ms and tiebreak; a tie, once in 2^64:
/// the lower id). The other side, with the arguments swapped, gets the other answer.
fn hosts(mine: (u32, u64), theirs: (u32, u64), my_id: &str, peer_id: &str) -> bool {
    match mine.cmp(&theirs) {
        std::cmp::Ordering::Equal => my_id < peer_id,
        o => o == std::cmp::Ordering::Greater,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rooms_and_urls() {
        let r = Room::new("ws://127.0.0.1:3536/", "test42").unwrap();
        assert_eq!((r.signal.as_str(), r.code.as_str(), r.ice.as_ref()), ("ws://127.0.0.1:3536", "TEST42", None));
        assert_eq!(r.url(), "ws://127.0.0.1:3536/okb-TEST42?next=2");
        assert_eq!(Room::new("wss://example.invalid/mb", "A").unwrap().url(), "wss://example.invalid/mb/okb-A?next=2");
        for bad in ["", "a b", "x-y", "ü", "0123456789abcdefg", "../x", "a?b"] {
            assert!(room_code(bad).is_err(), "{bad:?}");
        }
        for bad in ["", "http://x", "ws://", "ws:///x", "ws://x?y=1", "ws://x#f", "ws://a b", "ws://x\ny", "ws://x\\y", &format!("ws://{}", "x".repeat(300))] {
            assert!(signal_url(bad).is_err(), "{bad:?}");
        }
        let r = r.with_ice(Some("stun:127.0.0.1:3478"), Some(("turn:127.0.0.1:3478", "u", "p"))).unwrap();
        assert_eq!(
            r.ice,
            Some(Ice { urls: vec!["stun:127.0.0.1:3478".into(), "turn:127.0.0.1:3478".into()], username: Some("u".into()), credential: Some("p".into()) })
        );
        let base = Room::new("ws://h", "A").unwrap();
        assert!(base.clone().with_ice(Some("turn:x"), None).is_err() && base.clone().with_ice(Some("stun:"), None).is_err());
        assert!(base.clone().with_ice(None, Some(("turn:x", "", "p"))).is_err());
        assert_eq!(base.with_ice(None, None).unwrap().ice, None);
    }

    /// The two sides decide from the same two probes: always opposite answers, the longer wait hosting, a tie on
    /// both the wait and the tiebreak decided by the ids.
    #[test]
    fn sides_opposite() {
        let (a, b) = ("0f6c2d1e-aaaa", "7b1d9e33-bbbb");
        for (x, y) in [((800, 5), (2000, 9)), ((800, 9), (800, 5)), ((0, 0), (0, 1)), ((u32::MAX, 0), (0, u64::MAX)), ((300, 7), (300, 7))] {
            for (ia, ib) in [(a, b), (b, a)] {
                assert_ne!(hosts(x, y, ia, ib), hosts(y, x, ib, ia), "{x:?} {y:?} {ia} {ib}");
            }
        }
        assert!(hosts((2000, 0), (800, 9), b, a) && !hosts((800, 9), (2000, 0), a, b));
        assert!(hosts((300, 7), (300, 7), a, b) && !hosts((300, 7), (300, 7), b, a));
    }

    #[test]
    fn probes() {
        let mut b = PROBE.to_vec();
        b.extend_from_slice(&[1, 1]);
        b.extend_from_slice(&1234u32.to_le_bytes());
        b.extend_from_slice(&7u64.to_le_bytes());
        assert_eq!(WebRtc::read_probe(&b), Some((1234, 7, true)));
        for k in [4, 5] {
            let mut c = b.clone();
            c[k] = 2;
            assert_eq!(WebRtc::read_probe(&c), None);
        }
        assert_eq!(WebRtc::read_probe(&b[..17]), None);
        let mut c = b.clone();
        c.push(0);
        assert_eq!(WebRtc::read_probe(&c), None);
        // a netplay packet is no probe
        assert_eq!(WebRtc::read_probe(&super::super::proto::encode(&super::super::proto::Packet { nonce: 1, msg: super::super::proto::Msg::Go })), None);
    }
}
