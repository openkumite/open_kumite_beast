//! Netplay: two players run the same session in delay-based lockstep.
//!
//! Engine addition.
//!
//! Netplay (game-state plan stages 8 and 9, N1 / N2): two players run the same session in delay-based lockstep, from the
//! boot (or the title) through the menus, the character select, the VS screens and the fights. No prediction, no
//! rollback: frame f is stepped only once both players' inputs for it are there.
//!
//! - Port 1 is the host, port 2 the joiner; each side's own pads drive only its port. The joiner adopts the host's
//!   `SessionStart` (seed, memory card, unlocks, entry, browser menus), its logic preferences and, for a direct
//!   fight, its costumes; the host's card carries the joiner's own CONTROLLER CONFIG. P1 map as PLAYER 2's
//!   (`handshake`). Netplay never writes the card or the preferences on either side; both record the session as a
//!   replay, the same bytes on both (the replays' times come from the host's clock: `Netplay::time_at`).
//! - The input for frame f + delay is taken when frame f is stepped (frames 0..delay have no pads), and every
//!   packet carries all of it the peer has not acknowledged (`proto::MAX_INPUTS`), so a lost packet is covered by the
//!   next without timers.
//! - Loads are inputs (as in replays): each side reports "my load / fetch is done" in its input stream
//!   (`proto::Input::done`); `FrameInput::LOADED` / `FETCHED` is applied at the first frame where both have
//!   reported (a report from before the step that asked for the load does not count). `FrameInput::CARD_STORED`
//!   follows every step that wrote the card (nothing is stored; the dialog's SAVE succeeds).
//! - Every `replay::CHECK_EVERY` frames each side sends its state hash (the last `proto::MAX_CHECKS` in every
//!   packet); a hash that differs stops both with "desync at frame F" (the section hashes exchanged and logged, the
//!   state there kept for a dump: `Netplay::write_desync`).
//! - No packet from the peer for `TIMEOUT` s, its Bye, or a link that reports the peer gone (`Transport::lost`: a
//!   room's peer left) stops the session with a message.
//! - Round trip: every Inputs packet carries the sender's clock (ms), the last one it received from the peer and how
//!   long it held that one; the difference is the round trip as both frame loops see it (`Netplay::rtt`, smoothed).
//! - Frame advantage: each side measures how far its steps run ahead of the peer's (as last told, so the one-way
//!   latency is in it) and sends that; half the difference of both is the true lead (the latency cancels), and the
//!   side more than a frame ahead lets one step's slot pass (`Netplay::ahead`), so that the waits that the delay
//!   does not cover fall on both sides instead of always on the one whose clock leads.
//!
//! The debug tools, the debug key cheats and the env cheats are off in netplay; the in-fight pause is the game's
//! own (a START press, an input like any other). Transports: `link` (the trait, an in-memory pair, OKB_NETSIM's
//! simulated network), `udp` (desktop), `webrtc` (a room's data channel through a matchbox signaling server; feature
//! `netplay-webrtc`). `connect` brings a link up and runs the handshake, polled from a frame loop. `proto` has the
//! packet layout.

pub mod connect;
pub mod handshake;
pub mod link;
pub mod proto;
#[cfg(not(target_arch = "wasm32"))]
pub mod udp;
#[cfg(feature = "netplay-webrtc")]
pub mod webrtc;

pub use connect::{Connect, Setup};
pub use handshake::{Handshake, Offer};
pub use link::Transport;

use crate::frontend::options::Prefs;
use crate::session::replay::CHECK_EVERY;
use crate::session::{FrameInput, GameState, Out, Request, SessionStart, hash, section_hashes};
use proto::{DONE, Input, MAX_CHECKS, MAX_INPUTS, Msg, NO_PONG, Packet};
use std::collections::VecDeque;

/// One 60 Hz step.
pub const FRAME: f64 = 1.0 / 60.0;
/// The largest input delay (frames; `--delay N`).
pub const MAX_DELAY: u32 = 30;
/// No packet from the peer for this long (seconds) ends the session.
pub const TIMEOUT: f64 = 10.0;
/// A peer's inputs are taken at most this many frames past the next step (a peer is at most 2 × delay + 1 ahead).
const AHEAD: u32 = 2 * MAX_DELAY + 8;
/// Own checkpoints kept until the peer's hash for them came (each with its state, for a desync dump).
const KEEP_CHECKS: usize = 8;
/// The peer's checkpoints kept until this side reached them.
const KEEP_THEIRS: usize = 16;
/// Desync packets sent after a mismatch (one per flush).
const DESYNC_SENDS: u32 = 30;
/// At most one step slot in this many is let pass for the frame advantage.
const SKIP_EVERY: u32 = 20;
/// Round trips longer than this (seconds) are no sample (a stale echo).
const MAX_RTT: f64 = 10.0;

/// What both sides agreed on (`Handshake`).
#[derive(Clone, Debug)]
pub struct Agreed {
    /// 0: the host (player 1), 1: the joiner (player 2)
    pub port: usize,
    pub nonce: u32,
    pub delay: u32,
    /// the host's, with the joiner's map in its card (`handshake::with_p2_buttons`)
    pub start: SessionStart,
    pub prefs: Prefs,
    pub costume: [u8; 2],
    /// the host's clock at the start (seconds since 1970)
    pub time: u64,
}

/// Why a session stopped.
#[derive(Clone, Debug, PartialEq)]
pub enum Stop {
    /// the peer's Bye (its reason)
    Left(String),
    /// no packet for this many seconds
    Timeout(f64),
    /// a checkpoint's hashes differ (this side's, the peer's; 0 = not known)
    Desync { frame: u32, mine: u64, theirs: u64 },
    /// this side ended it (`close`)
    Quit(String),
    /// the peer's data or this side's session does not follow the protocol
    Error(String),
}

impl std::fmt::Display for Stop {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        match self {
            Stop::Left(why) => write!(f, "netplay: the other player left ({why})"),
            Stop::Timeout(s) => write!(f, "netplay: nothing from the other player for {s:.0} s: stopped"),
            Stop::Desync { frame, .. } => write!(f, "netplay: desync at frame {frame}: stopped"),
            Stop::Quit(why) => write!(f, "netplay: {why}"),
            Stop::Error(e) => write!(f, "netplay: {e}"),
        }
    }
}

/// Counters of a session (logged at its end; `nettest` reports them).
#[derive(Clone, Copy, Debug, Default)]
pub struct Stats {
    pub steps: u32,
    /// times a step had to wait for the peer's input, the seconds waited, the longest wait
    pub stalls: u32,
    pub stall_secs: f64,
    pub max_stall: f64,
    pub sent: u64,
    pub sent_bytes: u64,
    pub received: u64,
    /// datagrams that were no packet of this session
    pub bad: u64,
    /// checkpoints both sides hashed the same
    pub checks: u32,
    /// step slots let pass for the frame advantage (`Netplay::ahead`)
    pub skips: u32,
}

/// A desync's evidence: this side's state at the checkpoint and the section hashes of both.
pub struct Desync {
    pub frame: u32,
    pub state: Box<GameState>,
    pub mine: Vec<(&'static str, u64)>,
    pub theirs: Option<Vec<(String, u64)>>,
}

/// One side of a lockstep session.
pub struct Netplay {
    link: Box<dyn Transport>,
    pub port: usize,
    pub delay: u32,
    nonce: u32,
    prefs: Prefs,
    time: u64,
    /// this side's inputs from frame `local_base` on
    local: VecDeque<Input>,
    local_base: u32,
    /// the peer's inputs from frame `next` on (None: not received yet)
    remote: VecDeque<Option<Input>>,
    /// the frame stepped next (the state's `frame` before that step)
    next: u32,
    /// the peer has this side's inputs of every frame below
    peer_ack: u32,
    /// the peer's next step, as last told
    peer_at: u32,
    /// this side's frame advantage (`next` − the peer's `at`, averaged) and the peer's (as told), in frames
    adv: f64,
    peer_adv: f64,
    /// steps since the last slot let pass for the frame advantage
    since_skip: u32,
    /// the completions reported per port since the step that asked for them
    seen: [u8; 2],
    /// the last step wrote the card: the next input has `CARD_STORED`
    card_stored: bool,
    /// own checkpoints not matched yet (frame, hash, the state then)
    mine: VecDeque<(u32, u64, Box<GameState>)>,
    /// the latest own checkpoints (sent in every packet)
    recent: VecDeque<(u32, u64)>,
    /// the peer's checkpoints not matched yet
    theirs: VecDeque<(u32, u64)>,
    /// checkpoints up to this frame are compared (or can no longer be)
    compared: u32,
    /// the last packet of the peer (seconds)
    heard: f64,
    /// a step waits for the peer since then
    stall: Option<f64>,
    /// the peer's newest clock (ms) and when it came, echoed in every packet
    peer_ping: Option<(u16, f64)>,
    /// the round trip (seconds, smoothed)
    rtt: Option<f64>,
    stop: Option<Stop>,
    desync: Option<Desync>,
    desync_sent: u32,
    pub stats: Stats,
}

impl Netplay {
    /// The session `a` over `link` at `now` (seconds, monotonic; the same clock for every later call).
    pub fn new(link: Box<dyn Transport>, a: &Agreed, now: f64) -> Self {
        // the first `input` reads this side's input of frame 0 (one of the delay's)
        assert!((1..=MAX_DELAY).contains(&a.delay), "netplay: input delay {}", a.delay);
        let d = a.delay as usize;
        Self {
            link,
            port: a.port,
            delay: a.delay,
            nonce: a.nonce,
            prefs: a.prefs,
            time: a.time,
            local: std::iter::repeat_n(Input::default(), d).collect(),
            local_base: 0,
            remote: std::iter::repeat_n(Some(Input::default()), d).collect(),
            next: 0,
            peer_ack: 0,
            peer_at: 0,
            adv: 0.0,
            peer_adv: 0.0,
            since_skip: 0,
            seen: [0; 2],
            card_stored: false,
            mine: VecDeque::new(),
            recent: VecDeque::new(),
            theirs: VecDeque::new(),
            compared: 0,
            heard: now,
            stall: None,
            peer_ping: None,
            rtt: None,
            stop: None,
            desync: None,
            desync_sent: 0,
            stats: Stats::default(),
        }
    }

    /// The session's state exists (after the data loaded): the timeout counts from here.
    pub fn begin(&mut self, now: f64) {
        self.heard = now;
    }

    /// The preferences the logic runs with (the host's, then the GRAPHICS screen's changes: `set_prefs`).
    pub fn prefs(&self) -> Prefs {
        self.prefs
    }

    /// A `Request::Prefs` of the session (both sides get it at the same step).
    pub fn set_prefs(&mut self, p: Prefs) {
        self.prefs = p;
    }

    /// The session's clock at `frame` (seconds since 1970; a replay's time, the same on both sides).
    pub fn time_at(&self, frame: u32) -> u64 {
        self.time + (frame / 60) as u64
    }

    /// The frame stepped next.
    pub fn next(&self) -> u32 {
        self.next
    }

    /// The peer's next step, as last told.
    pub fn peer_at(&self) -> u32 {
        self.peer_at
    }

    pub fn stopped(&self) -> Option<&Stop> {
        self.stop.as_ref()
    }

    pub fn desync(&self) -> Option<&Desync> {
        self.desync.as_ref()
    }

    /// This side runs more than a frame ahead of the peer (the half difference of both sides' frame advantages):
    /// the caller lets this step slot pass (no step; at most one in `SKIP_EVERY` steps).
    pub fn ahead(&mut self) -> bool {
        if self.stop.is_some() || self.since_skip < SKIP_EVERY || (self.adv - self.peer_adv) / 2.0 <= 1.0 {
            return false;
        }
        self.since_skip = 0;
        self.stats.skips += 1;
        true
    }

    /// The round trip to the peer (seconds, smoothed; None before the first echo).
    pub fn rtt(&self) -> Option<f64> {
        self.rtt
    }

    /// The step waits for the peer's input since this many seconds.
    pub fn waiting(&self, now: f64) -> Option<f64> {
        self.stall.map(|t| now - t)
    }

    fn local_end(&self) -> u32 {
        self.local_base + self.local.len() as u32
    }

    /// The next `input` takes this side's pad (for frame `local_frame`); else it has it already.
    pub fn wants_local(&self) -> bool {
        self.stop.is_none() && self.local_end() <= self.next + self.delay
    }

    /// The frame the pad `input` takes next is for.
    pub fn local_frame(&self) -> u32 {
        self.local_end()
    }

    /// The next step's input, None while the peer's is missing (or the session stopped). When `wants_local`,
    /// `pad` is this side's for `local_frame` and the completions in `done` (`proto::DONE`) are reported with it
    /// (and cleared there).
    pub fn input(&mut self, pad: u16, done: &mut u8, now: f64) -> Option<FrameInput> {
        if self.wants_local() {
            self.local.push_back(Input { pad, done: *done & DONE });
            *done &= !DONE;
        }
        if self.stop.is_some() {
            return None;
        }
        let Some(theirs) = self.remote.front().copied().flatten() else {
            if self.stall.is_none() {
                self.stall = Some(now);
                self.stats.stalls += 1;
            }
            return None;
        };
        if let Some(t) = self.stall.take() {
            self.stats.stall_secs += now - t;
            self.stats.max_stall = self.stats.max_stall.max(now - t);
        }
        let mine = self.local[(self.next - self.local_base) as usize];
        let ins = if self.port == 0 { [mine, theirs] } else { [theirs, mine] };
        let mut flags = 0;
        for k in 0..2 {
            self.seen[k] |= ins[k].done;
        }
        for bit in [FrameInput::LOADED, FrameInput::FETCHED] {
            if self.seen[0] & self.seen[1] & bit != 0 {
                flags |= bit;
                self.seen = self.seen.map(|s| s & !bit);
            }
        }
        if std::mem::take(&mut self.card_stored) {
            flags |= FrameInput::CARD_STORED;
        }
        self.remote.pop_front();
        self.next += 1;
        self.trim();
        self.stats.steps += 1;
        self.since_skip = self.since_skip.saturating_add(1);
        Some(FrameInput { pad: [ins[0].pad, ins[1].pad], forced: [0; 2], flags, ..Default::default() })
    }

    /// After the step of `input`'s input, which left `s` and gave `out`: the card write, the loads asked for, the
    /// checkpoint.
    pub fn stepped(&mut self, s: &GameState, out: &Out) {
        if s.frame != self.next {
            self.fail(Stop::Error(format!("the session is at frame {}, netplay at {}", s.frame, self.next)));
            return;
        }
        self.card_stored |= out.card_dirty;
        for r in &out.requests {
            match r {
                Request::Load(_) => self.seen = self.seen.map(|x| x & !FrameInput::LOADED),
                Request::Fetch(_) => self.seen = self.seen.map(|x| x & !FrameInput::FETCHED),
                _ => {}
            }
        }
        if s.frame.is_multiple_of(CHECK_EVERY) {
            let h = hash(s);
            self.mine.push_back((s.frame, h, Box::new(s.clone())));
            if self.mine.len() > KEEP_CHECKS
                && let Some(c) = self.mine.pop_front()
            {
                log::warn!("netplay: no hash from the other player for frame {}", c.0);
                self.compared = self.compared.max(c.0);
            }
            self.recent.push_back((s.frame, h));
            if self.recent.len() > MAX_CHECKS {
                self.recent.pop_front();
            }
            self.compare();
        }
    }

    /// Own checkpoints against the peer's: matched ones go, a mismatch stops the session.
    fn compare(&mut self) {
        while let Some(&(f, theirs)) = self.theirs.front() {
            let Some(k) = self.mine.iter().position(|c| c.0 == f) else {
                // one already compared, or before every kept checkpoint, goes; one not reached yet waits
                if f <= self.compared || self.mine.front().is_some_and(|c| c.0 > f) {
                    self.theirs.pop_front();
                    continue;
                }
                break;
            };
            self.theirs.pop_front();
            self.compared = f;
            let (_, mine, state) = self.mine.remove(k).expect("a checkpoint");
            if mine == theirs {
                self.stats.checks += 1;
                continue;
            }
            log::error!("netplay: desync at frame {f}: this side's state hash {mine:016x}, the other player's {theirs:016x}");
            self.evidence(f, state, None);
            self.fail(Stop::Desync { frame: f, mine, theirs });
            return;
        }
    }

    /// A desync's evidence kept: this side's state at frame `f`, its section hashes (logged) and the peer's.
    fn evidence(&mut self, f: u32, state: Box<GameState>, theirs: Option<Vec<(String, u64)>>) {
        let mine = section_hashes(&state);
        for (name, h) in &mine {
            log::error!("netplay:   {name:12} {h:016x}");
        }
        self.desync = Some(Desync { frame: f, state, mine, theirs: None });
        if let Some(t) = theirs {
            self.peer_sections(t);
        }
    }

    /// The peer's section hashes of the desync: the sections that differ logged.
    fn peer_sections(&mut self, theirs: Vec<(String, u64)>) {
        let Some(d) = &mut self.desync else { return };
        let differ: Vec<&str> = d.mine.iter().filter(|(n, h)| theirs.iter().any(|(m, g)| m == n && g != h)).map(|x| x.0).collect();
        log::error!("netplay: desync at frame {}: the sections that differ: {differ:?}", d.frame);
        d.theirs = Some(theirs);
    }

    fn fail(&mut self, why: Stop) {
        if self.stop.is_none() {
            match why {
                Stop::Left(_) | Stop::Quit(_) => log::info!("{why}"),
                _ => log::error!("{why}"),
            }
            self.stop = Some(why);
        }
    }

    /// Own inputs the peer has and that were stepped go.
    fn trim(&mut self) {
        while self.local_base < self.peer_ack.min(self.next) && !self.local.is_empty() {
            self.local.pop_front();
            self.local_base += 1;
        }
    }

    fn send(&mut self, msg: Msg) {
        let b = proto::encode(&Packet { nonce: self.nonce, msg });
        self.stats.sent += 1;
        self.stats.sent_bytes += b.len() as u64;
        self.link.send(&b);
    }

    /// Takes what came from the peer; a silent peer times out.
    pub fn poll(&mut self, now: f64) {
        while let Some(b) = self.link.recv() {
            let p = match proto::decode(&b) {
                Ok(p) if p.nonce == self.nonce => p,
                _ => {
                    self.stats.bad += 1;
                    continue;
                }
            };
            self.stats.received += 1;
            self.heard = now;
            match p.msg {
                Msg::Inputs { at, adv, ack, first, inputs, checks, ping, pong, held } => {
                    // the newest (a reordered older packet would read as a lead)
                    if at >= self.peer_at {
                        self.adv += (self.next as f64 - at as f64 - self.adv) * 0.1;
                        self.peer_adv = adv as f64 / 16.0;
                    }
                    // a newer clock than the one kept (a stalled peer's `at` stays: a reordered older packet would
                    // send back a stale echo and read its own as a longer trip; the u16 wraps after 65 s, far beyond
                    // `TIMEOUT`)
                    if self.peer_ping.is_none_or(|(p, _)| (1..0x8000).contains(&ping.wrapping_sub(p))) {
                        self.peer_ping = Some((ping, now));
                        if held != NO_PONG {
                            let t = (clock_ms(now).wrapping_sub(pong) as f64 - held as f64).max(0.0) / 1e3;
                            if t < MAX_RTT {
                                self.rtt = Some(self.rtt.map_or(t, |r| r + (t - r) * 0.125));
                            }
                        }
                    }
                    self.peer_at = self.peer_at.max(at);
                    self.peer_ack = self.peer_ack.max(ack.min(self.local_end()));
                    for (k, i) in inputs.into_iter().enumerate() {
                        let f = first + k as u32;
                        if f < self.next {
                            continue;
                        }
                        if f >= self.next + AHEAD {
                            break;
                        }
                        let at = (f - self.next) as usize;
                        if self.remote.len() <= at {
                            self.remote.resize(at + 1, None);
                        }
                        // the first value received stays (a peer never changes an input it sent)
                        self.remote[at].get_or_insert(i);
                    }
                    self.trim();
                    for c in checks {
                        if c.0.is_multiple_of(CHECK_EVERY) && c.0 > self.compared && c.0 < self.next + AHEAD && !self.theirs.iter().any(|t| t.0 == c.0) {
                            self.theirs.push_back(c);
                        }
                    }
                    self.theirs.make_contiguous().sort_unstable();
                    while self.theirs.len() > KEEP_THEIRS {
                        self.theirs.pop_front();
                    }
                    self.compare();
                }
                Msg::Desync { frame, sections } => match &self.desync {
                    Some(d) if d.frame == frame && d.theirs.is_none() => self.peer_sections(sections),
                    Some(_) => {}
                    // the peer saw it first (this side's hash reached it, its own did not come here)
                    None => {
                        let theirs = sections.iter().find(|s| s.0 == "state").map_or(0, |s| s.1);
                        let mine = match self.mine.iter().position(|c| c.0 == frame).and_then(|k| self.mine.remove(k)) {
                            Some((_, h, state)) => {
                                log::error!(
                                    "netplay: desync at frame {frame} (seen by the other player): this side's state hash {h:016x}, theirs {theirs:016x}"
                                );
                                self.evidence(frame, state, Some(sections));
                                h
                            }
                            None => 0,
                        };
                        self.fail(Stop::Desync { frame, mine, theirs });
                    }
                },
                Msg::Bye { why } => self.fail(Stop::Left(why)),
                // the joiner did not get the handshake's Go
                Msg::Ready { .. } if self.port == 0 => self.send(Msg::Go),
                _ => {}
            }
        }
        if self.stop.is_none()
            && let Some(why) = self.link.lost()
        {
            self.fail(Stop::Left(why));
        }
        if self.stop.is_none() && now - self.heard > TIMEOUT {
            self.fail(Stop::Timeout(now - self.heard));
        }
    }

    /// Sends this side's inputs the peer lacks, the latest checkpoints and the round trip's echo (once per drawn
    /// frame, at `now`); after a desync, the section hashes.
    pub fn flush(&mut self, now: f64) {
        if self.stop.is_none() {
            let ack = self.next + self.remote.iter().take_while(|i| i.is_some()).count() as u32;
            let first = self.peer_ack.max(self.local_base);
            let from = (first - self.local_base) as usize;
            let inputs: Vec<Input> = self.local.iter().skip(from).take(MAX_INPUTS).copied().collect();
            let checks = self.recent.iter().copied().collect();
            let adv = (self.adv * 16.0).round().clamp(i16::MIN as f64, i16::MAX as f64) as i16;
            let (pong, held) = match self.peer_ping {
                Some((p, got)) => (p, ((now - got) * 1e3).round().clamp(0.0, (NO_PONG - 1) as f64) as u16),
                None => (0, NO_PONG),
            };
            self.send(Msg::Inputs { at: self.next, adv, ack, first, inputs, checks, ping: clock_ms(now), pong, held });
        } else if let Some(d) = &self.desync
            && self.desync_sent < DESYNC_SENDS
        {
            let msg = Msg::Desync { frame: d.frame, sections: d.mine.iter().map(|(n, h)| (n.to_string(), *h)).collect() };
            self.desync_sent += 1;
            self.send(msg);
        }
    }

    /// Ends the session from this side: the peer is told (a few times: datagrams get lost).
    pub fn close(&mut self, why: &str) {
        if matches!(self.stop, None | Some(Stop::Desync { .. }) | Some(Stop::Error(_))) {
            for _ in 0..3 {
                self.send(Msg::Bye { why: why.into() });
            }
        }
        if self.stop.is_none() {
            self.stop = Some(Stop::Quit(why.into()));
        }
        let s = &self.stats;
        log::info!(
            "netplay: {} steps, {} stalls ({:.2} s, longest {:.0} ms), {} slots let pass, {} checkpoints matched, {} packets sent ({} bytes), {} received, {} ignored",
            s.steps,
            s.stalls,
            s.stall_secs,
            s.max_stall * 1e3,
            s.skips,
            s.checks,
            s.sent,
            s.sent_bytes,
            s.received,
            s.bad
        );
    }

    /// A desync's evidence written to `dir`: this side's state at the checkpoint as `desync_<frame>_p<port>.okbs`
    /// (a save-state file) and `.json` (one field per line, to diff with the other side's), its section hashes and
    /// the other side's as `.txt`, and `replay` (the session so far) as `.okbr`. The paths written.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn write_desync(&self, dir: &std::path::Path, replay: Option<&[u8]>) -> Vec<String> {
        let Some(d) = &self.desync else { return Vec::new() };
        let base = dir.join(format!("desync_{}_p{}", d.frame, self.port + 1));
        let mut text = format!("desync at frame {} (player {} of 2)\n{:12} {:16} {:16}\n", d.frame, self.port + 1, "section", "this side", "other side");
        for (n, h) in &d.mine {
            let other = d.theirs.as_ref().and_then(|t| t.iter().find(|x| x.0 == *n)).map_or("?".into(), |x| format!("{:016x}", x.1));
            text += &format!("{n:12} {h:016x} {other}{}\n", if other != format!("{h:016x}") { "  *" } else { "" });
        }
        let json = serde_json::to_value(&*d.state).and_then(|v| serde_json::to_string_pretty(&v)).unwrap_or_default();
        let mut files: Vec<(&str, Vec<u8>)> = vec![("okbs", crate::session::save_file(&d.state)), ("json", json.into_bytes()), ("txt", text.into_bytes())];
        if let Some(r) = replay {
            files.push(("okbr", r.to_vec()));
        }
        let _ = std::fs::create_dir_all(dir);
        let mut out = Vec::new();
        for (ext, b) in files {
            let p = base.with_extension(ext);
            match std::fs::write(&p, b) {
                Ok(()) => out.push(p.display().to_string()),
                Err(e) => log::warn!("netplay: {}: {e}", p.display()),
            }
        }
        out
    }
}

/// A monotonic clock (seconds) as the round trip's 16-bit milliseconds (it wraps every 65.5 s).
fn clock_ms(now: f64) -> u16 {
    ((now * 1e3) as u64 & 0xFFFF) as u16
}

#[cfg(test)]
mod tests {
    use super::link::{NetSim, SimNet, mem_pair};
    use super::*;
    use crate::session::{Assets, FileCache, step};

    thread_local!(static NOW: std::cell::Cell<f64> = const { std::cell::Cell::new(0.0) });
    fn now() -> f64 {
        NOW.with(|n| n.get())
    }

    fn agreed(port: usize, delay: u32) -> Agreed {
        Agreed { port, nonce: 3, delay, start: SessionStart::default(), prefs: Prefs::default(), costume: [0; 2], time: 1_700_000_000 }
    }

    /// One side of a test session: the empty state (no front end, no fight: a step only counts the frame), the
    /// inputs it stepped, the fetch it reports done some ticks after asking.
    struct Side {
        np: Netplay,
        s: GameState,
        seen: Vec<FrameInput>,
        done: u8,
        fetch_at: Option<u32>,
        ticks: u32,
        /// the frames whose input reported a completion
        reported: Vec<u32>,
    }

    /// Both sides over `sim`ulated links, `n` ticks of 1/60 s each: every tick each side polls, steps what it
    /// can (its pad: its port and the frame, `pad`), flushes. `hook(port, frame, state, out)` runs after each
    /// step (a fetch asked for, a state changed on one side only).
    fn run(sim: SimNet, delay: u32, n: u32, mut hook: impl FnMut(usize, &mut Side, &mut Out)) -> [Side; 2] {
        NOW.with(|t| t.set(0.0));
        let (a, b) = mem_pair();
        let mk = |port, l: Box<dyn Transport>| Side {
            np: Netplay::new(l, &agreed(port, delay), 0.0),
            s: GameState::default(),
            seen: Vec::new(),
            done: 0,
            fetch_at: None,
            ticks: 0,
            reported: Vec::new(),
        };
        let mut sides = [mk(0, Box::new(NetSim::new(a, sim, 11, now))), mk(1, Box::new(NetSim::new(b, sim, 22, now)))];
        let files = FileCache::default();
        let assets = Assets { front: None, fight: None, backdrop: None, files: &files, prefs: Prefs::default() };
        for _ in 0..n {
            NOW.with(|t| t.set(t.get() + FRAME));
            for x in sides.iter_mut() {
                x.ticks += 1;
                x.np.poll(now());
                if x.fetch_at.is_some_and(|t| x.ticks >= t) {
                    x.fetch_at = None;
                    x.done |= FrameInput::FETCHED;
                }
                if x.np.ahead() {
                    x.np.flush(now());
                    continue;
                }
                let pad = (x.np.port as u16 + 1) << 12 | (x.np.local_frame() & 0xFF) as u16;
                if x.done != 0 && x.np.wants_local() {
                    x.reported.push(x.np.local_frame());
                }
                if let Some(i) = x.np.input(pad, &mut x.done, now()) {
                    let mut out = Out::default();
                    step(&mut x.s, &assets, i, &[], &mut out);
                    hook(x.np.port, x, &mut out);
                    x.np.stepped(&x.s, &out);
                    x.seen.push(i);
                }
                x.np.flush(now());
            }
        }
        sides
    }

    /// Lossless, lossy (25 %), late and jittery links: both sides step the same inputs in the same order, each
    /// port's pad is its side's (its frame's, `delay` frames late), every checkpoint matches, nothing stops.
    #[test]
    fn same_inputs_both_sides() {
        for (sim, delay) in [
            (SimNet::default(), 2),
            (SimNet { latency: 0.0, loss: 0.25, jitter: 0.0 }, 3),
            (SimNet { latency: 0.08, loss: 0.05, jitter: 0.0 }, 6),
            (SimNet { latency: 0.15, loss: 0.1, jitter: 0.02 }, 10),
        ] {
            let [a, b] = run(sim, delay, 1500, |_, _, _| {});
            assert!(a.np.stopped().is_none() && b.np.stopped().is_none(), "{sim:?}: {:?} {:?}", a.np.stopped(), b.np.stopped());
            let n = a.seen.len().min(b.seen.len());
            assert!(n > 1000, "{sim:?}: {n} steps");
            assert_eq!(a.seen[..n], b.seen[..n], "{sim:?}");
            for (f, i) in a.seen[..n].iter().enumerate() {
                let want = |port: u16| if f < delay as usize { 0 } else { (port + 1) << 12 | (f & 0xFF) as u16 };
                assert_eq!(i.pad, [want(0), want(1)], "{sim:?} frame {f}");
            }
            assert!(a.np.stats.checks >= 15 && b.np.stats.checks >= 15, "{sim:?}: {} {}", a.np.stats.checks, b.np.stats.checks);
            // the round trip: twice the latency, within the jitter and the 60 Hz ticks of both sides
            for x in [&a, &b] {
                let r = x.np.rtt().expect("a round trip");
                assert!(r >= 2.0 * (sim.latency - sim.jitter) - 1e-3 && r <= 2.0 * (sim.latency + sim.jitter) + 3.0 * FRAME, "{sim:?}: rtt {r}");
            }
            eprintln!("{sim:?} delay {delay}: {n} steps, stalls {} / {}, rtt {:.0} ms", a.np.stats.stalls, b.np.stats.stalls, a.np.rtt().unwrap_or(0.0) * 1e3);
        }
    }

    /// A fetch asked for at frame 100 on both sides, done 3 ticks later on the host and 40 on the joiner: FETCHED
    /// comes on both at the same frame, the joiner's report's; a report before the ask does not count.
    #[test]
    fn loads_complete_together() {
        let sim = SimNet { latency: 0.05, loss: 0.1, jitter: 0.01 };
        let [a, b] = run(sim, 4, 600, |port, x, out| {
            if x.s.frame == 50 && port == 1 {
                x.done |= FrameInput::FETCHED; // stale: before the ask
            }
            if x.s.frame == 100 {
                out.requests.push(Request::Fetch(Vec::new()));
                x.fetch_at = Some(x.ticks + if port == 0 { 3 } else { 40 });
            }
        });
        let at = |x: &Side| x.seen.iter().position(|i| i.flags & FrameInput::FETCHED != 0);
        let (fa, fb) = (at(&a), at(&b));
        assert_eq!(fa, fb);
        let f = fa.expect("FETCHED applied") as u32;
        assert_eq!((a.reported.len(), b.reported.len()), (1, 2));
        assert!(b.reported[0] < 100 && a.reported[0] < b.reported[1] && b.reported[1] > 100 + 30, "{:?} {:?}", a.reported, b.reported);
        assert_eq!(f, b.reported[1], "applied at {f}");
        assert_eq!(a.seen.iter().filter(|i| i.flags != 0).count(), 1);
    }

    /// A state changed on one side only: both stop with a desync at the next checkpoint, each with its section
    /// hashes and the other's; the card stored flag follows a card write on both.
    #[test]
    fn desync_detected() {
        let [a, b] = run(SimNet { latency: 0.03, loss: 0.05, jitter: 0.0 }, 3, 400, |port, x, out| {
            if port == 0 && x.s.frame == 130 {
                x.s.music_volume = 0x30;
            }
            if x.s.frame == 20 {
                out.card_dirty = true;
            }
        });
        for x in [&a, &b] {
            assert!(matches!(x.np.stopped(), Some(Stop::Desync { frame: 180, .. })), "{:?}", x.np.stopped());
            let d = x.np.desync().expect("evidence");
            assert!(d.theirs.as_ref().is_some_and(|t| t.iter().any(|s| s.0 == "session")));
            assert_eq!(x.seen[20].flags, FrameInput::CARD_STORED);
            assert!(x.seen.len() < 180 + 3 * 3, "{} steps", x.seen.len());
        }
        assert!(a.np.desync().unwrap().theirs.as_ref().unwrap().iter().zip(&b.np.desync().unwrap().mine).all(|(x, y)| x.0 == y.0 && x.1 == y.1));
    }

    /// A peer that goes quiet times out; one that says Bye has left; a bad packet is counted and ignored.
    #[test]
    fn timeout_and_bye() {
        NOW.with(|t| t.set(0.0));
        let (a, mut b) = mem_pair();
        let mut np = Netplay::new(Box::new(a), &agreed(0, 2), 0.0);
        b.send(b"junk");
        b.send(&proto::encode(&Packet { nonce: 99, msg: Msg::Go }));
        np.poll(1.0);
        assert_eq!((np.stats.bad, np.stopped()), (2, None));
        np.poll(TIMEOUT + 0.5);
        assert!(matches!(np.stopped(), Some(Stop::Timeout(_))));
        let (a, mut b) = mem_pair();
        let mut np = Netplay::new(Box::new(a), &agreed(1, 2), 0.0);
        b.send(&proto::encode(&Packet { nonce: 3, msg: Msg::Bye { why: "quit".into() } }));
        np.poll(0.1);
        assert_eq!(np.stopped(), Some(&Stop::Left("quit".into())));
        assert!(np.input(0, &mut 0, 0.1).is_none());
    }

    /// A stalled peer's packets carry the same `at`: a reordered older one neither replaces the echoed clock nor
    /// feeds the round trip.
    #[test]
    fn stale_echo() {
        let (a, mut b) = mem_pair();
        let mut np = Netplay::new(Box::new(a), &agreed(0, 2), 0.0);
        let inputs = |ping, pong, held| {
            proto::encode(&Packet { nonce: 3, msg: Msg::Inputs { at: 5, adv: 0, ack: 0, first: 0, inputs: Vec::new(), checks: Vec::new(), ping, pong, held } })
        };
        // this side's clock 1000 ms echoed after a 50 ms trip, then an older packet (an echo of 900 ms) comes late
        b.send(&inputs(65530, 1000, 0));
        np.poll(1.05);
        b.send(&inputs(65500, 900, 0));
        np.poll(1.06);
        assert_eq!(np.peer_ping.map(|p| p.0), Some(65530));
        assert!((np.rtt().unwrap() - 0.05).abs() < 1e-9, "{:?}", np.rtt());
        // a newer one across the u16 wrap is taken
        b.send(&inputs(4, 1010, 0));
        np.poll(1.07);
        assert_eq!(np.peer_ping.map(|p| p.0), Some(4));
    }

    /// Two sides over real UDP on 127.0.0.1 (one thread, polled in turn): the handshake, then 600 steps in
    /// lockstep with the same inputs on both, the checkpoints matched, a Bye.
    #[test]
    #[cfg(not(target_arch = "wasm32"))]
    fn loopback_udp() {
        use super::udp::Udp;
        let clock = std::time::Instant::now();
        let t = || clock.elapsed().as_secs_f64();
        let host = Udp::host("127.0.0.1:0".parse().unwrap()).unwrap();
        let addr = host.local_addr().unwrap();
        let (mut hl, mut jl): (Box<dyn Transport>, Box<dyn Transport>) = (Box::new(host), Box::new(Udp::join(addr).unwrap()));
        let offer = Offer { start: SessionStart { seed: 5, ..Default::default() }, prefs: Prefs::default(), costume: [0; 2], time: 1, delay: Some(3) };
        let (mut h, mut j) = (Handshake::host(offer), Handshake::join(42, crate::assets::game_tables::default_buttons(), 0, 0, 0, t()));
        let (mut ha, mut ja) = (None, None);
        while ha.is_none() || ja.is_none() {
            assert!(t() < 10.0, "handshake timed out");
            if ja.is_none() {
                ja = j.poll(&mut *jl, t()).unwrap();
            }
            if ha.is_none() {
                ha = h.poll(&mut *hl, t()).unwrap();
            }
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        let (ha, ja) = (ha.unwrap(), ja.unwrap());
        assert_eq!((ja.start.seed, ja.delay, ha.delay), (5, 3, 3));
        let files = FileCache::default();
        let assets = Assets { front: None, fight: None, backdrop: None, files: &files, prefs: Prefs::default() };
        let mut sides: Vec<(Netplay, GameState, Vec<FrameInput>)> =
            [(hl, ha), (jl, ja)].into_iter().map(|(l, a)| (Netplay::new(l, &a, t()), GameState::default(), Vec::new())).collect();
        while sides.iter().any(|x| x.2.len() < 600) {
            assert!(t() < 30.0, "lockstep timed out at {:?}", sides.iter().map(|x| x.2.len()).collect::<Vec<_>>());
            for (np, s, seen) in sides.iter_mut() {
                np.poll(t());
                let pad = 0x100 * (np.port as u16 + 1) + (np.local_frame() % 7) as u16;
                while seen.len() < 600
                    && let Some(i) = np.input(pad, &mut 0, t())
                {
                    let out = Out::default();
                    step(s, &assets, i, &[], &mut Out::default());
                    np.stepped(s, &out);
                    seen.push(i);
                }
                np.flush(t());
            }
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        assert_eq!(sides[0].2, sides[1].2);
        assert!(sides.iter().all(|x| x.0.stopped().is_none() && x.0.stats.checks >= 8), "{:?}", sides.iter().map(|x| x.0.stats).collect::<Vec<_>>());
        sides[0].0.close("done");
        for _ in 0..200 {
            sides[1].0.poll(t());
            if sides[1].0.stopped().is_some() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        assert_eq!(sides[1].0.stopped(), Some(&Stop::Left("done".into())));
    }
}
