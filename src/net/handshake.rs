//! The netplay handshake: versions checked, the host's session start sent to the joiner.
//!
//! Engine addition.
//!
//! The netplay handshake (game-state stage 8), non-blocking (`Handshake::poll`, polled from the frame loop by
//! `super::Connect` once the link is up):
//!
//! 1. the joiner sends Hello (its protocol, engine and simulation versions, its own CONTROLLER CONFIG. P1 map, a
//!    timestamp, whether it has the extra stage's files, its game data's content id) every 100 ms; the host answers each with Welcome (the timestamp back: the joiner's round trips),
//!    or Reject when a version differs (both builds must be the same: same simulation, same state layout) or the
//!    content id does (both players' data must be the same: `DATA_DIFFERS`);
//! 2. after 4 round trips the joiner sends Sync (its slowest round trip, the first chunk it lacks) every 50 ms; the
//!    host fixes the input delay (`--delay`, else from that round trip), builds the start blob (CBOR: the session's
//!    `SessionStart`, preferences, a direct fight's costumes, the delay, the host's clock) with its memory card's
//!    PLAYER 2 map replaced by the joiner's and the extra stage only if both have its files, and answers each Sync with the next `CHUNKS_PER_SYNC` chunks of it;
//! 3. the joiner checks the blob (its checksum, the versions in it, the same checks as a replay header's) and sends
//!    Ready every 100 ms; the host starts on the first Ready and answers with Go (its session answers a late Ready
//!    too), the joiner starts on Go or the host's first Inputs.
//!
//! A joiner that gives up (its Bye: the blob did not check out) or stays silent for `HOST_FORGET` s before Ready is
//! forgotten: the host takes the next joiner. A Reject goes at most `REFUSALS` times to one address (`MAX_REFUSED`
//! addresses, each forgotten `REFUSED_TTL` s after its first), so that a host does not reflect traffic at forged
//! senders.
//!
//! The start blob carries the host's whole `SessionStart`, its `browser` flag (the browser build's menus) too: a
//! session of a desktop and a browser player runs the host's menus on both.

use super::proto::{self, CHUNK, MAX_BLOB, Msg, Packet};
use super::{Agreed, MAX_DELAY, Transport};
use crate::base::hash::{FNV_BASIS, fnv1a};
use crate::frontend::options::Prefs;
use crate::session::{SIM_VERSION, STATE_VERSION, SessionStart, replay};
use serde::{Deserialize, Serialize};

/// Round trips the joiner measures before Sync.
const PINGS: usize = 4;
/// Chunks the host sends per Sync.
const CHUNKS_PER_SYNC: usize = 16;
/// The joiner gives up when the host stays silent this long (seconds).
const JOIN_TIMEOUT: f64 = 15.0;
/// Memory the decoded blob may take.
const DECODE_BUDGET: usize = 8 << 20;
/// The host forgets an accepted joiner silent this long before Ready (seconds).
const HOST_FORGET: f64 = 5.0;
/// Rejects sent to one address at most, and the addresses told.
const REFUSALS: u8 = 3;
const MAX_REFUSED: usize = 16;
/// Seconds an address stays in the refused table (a joiner that updated its build is told again).
const REFUSED_TTL: f64 = 60.0;
/// Why a peer whose game data has another content id (`SessionStart::content`) is refused.
pub const DATA_DIFFERS: &str = "the other player's game data differs: convert it again";

/// What the host offers: its session's start and setup.
#[derive(Clone, Debug)]
pub struct Offer {
    pub start: SessionStart,
    /// the logic's preferences (the GRAPHICS screen shows them; the host's)
    pub prefs: Prefs,
    /// a direct fight's costumes
    pub costume: [u8; 2],
    /// the host's clock at the start (seconds since 1970: the replays' times)
    pub time: u64,
    /// --delay N, else from the round trip
    pub delay: Option<u32>,
}

/// The start blob.
#[derive(Clone, Debug, Serialize, Deserialize)]
struct Blob {
    engine: String,
    sim_version: u32,
    state_version: u32,
    start: SessionStart,
    prefs: Prefs,
    costume: [u8; 2],
    delay: u32,
    time: u64,
}

/// The input delay for a round trip of `rtt` s (the slowest of the handshake's, so its jitter is in it): the frames
/// a one-way trip takes (rounded), one more for the 60 Hz send and poll of both sides, one more for a lost packet
/// (its inputs come with the next).
pub fn auto_delay(rtt: f64) -> u32 {
    ((rtt / 2.0 / super::FRAME).round().max(0.0) as u32 + 2).clamp(2, MAX_DELAY)
}

/// The host's memory card file with its PLAYER 2 CONTROLLER CONFIG. map replaced by `p2` (the joiner's own P1
/// map): a card that holds no file, or one that does not parse, stays as it is (player 2 then has the card's map).
pub fn with_p2_buttons(card: Option<Vec<u8>>, p2: [u16; 8]) -> Option<Vec<u8>> {
    let file = card?;
    let c = crate::frontend::options::Card::new(Some(file.clone()), None);
    match crate::frontend::options::Settings::try_load(&c) {
        Ok(mut s) if s.buttons[1] != p2 => {
            s.buttons[1] = p2;
            let mut c = crate::frontend::options::Card::new(None, None);
            if s.save(&mut c) { c.file().map(<[u8]>::to_vec) } else { Some(file) }
        }
        _ => Some(file),
    }
}

/// A session's id (the joiner's): random, never 0 (a Reject to anyone).
pub fn nonce() -> u32 {
    use std::hash::BuildHasher;
    std::collections::hash_map::RandomState::new().hash_one(crate::clock_seed()) as u32 | 1
}

/// The P1 map of the player's own stored save (the joiner's, read only); the default map without one.
pub fn own_buttons() -> [u16; 8] {
    let c = crate::frontend::options::Card::new(crate::platform::storage::read_card(), None);
    let b = crate::frontend::options::Settings::try_load(&c).map_or(crate::assets::game_tables::default_buttons(), |s| s.buttons[0]);
    if valid_buttons(&b) { b } else { crate::assets::game_tables::default_buttons() }
}

/// Each entry one of CONTROLLER CONFIG.'s actions (a peer's map is untrusted).
fn valid_buttons(b: &[u16; 8]) -> bool {
    b.iter().all(|v| crate::frontend::menus::cc_values().contains(v))
}

/// The handshake of one side.
pub enum Handshake {
    Host(Host),
    Join(Join),
}

impl Handshake {
    pub fn host(offer: Offer) -> Self {
        Handshake::Host(Host {
            offer,
            nonce: None,
            buttons: crate::assets::game_tables::default_buttons(),
            extras: 0,
            br1: 0,
            blob: None,
            heard: 0.0,
            refused: Vec::new(),
        })
    }

    /// `nonce`: the session's id (any value, e.g. from the clock); `buttons`: the joiner's own P1 map; `extras`: the
    /// extra stages whose files it has (`proto::EXTRA_Q3DM17`, `proto::EXTRA_Q3DM6`); `br1`: the imported game's
    /// content id (`SessionStart::br1`, 0 = none); `content`: its game data's (`SessionStart::content`, 0 = none).
    pub fn join(nonce: u32, buttons: [u16; 8], extras: u8, br1: u32, content: u32, now: f64) -> Self {
        Handshake::Join(Join {
            nonce,
            buttons,
            extras,
            br1,
            content,
            rtts: Vec::new(),
            started: now,
            heard: now,
            sent: f64::NEG_INFINITY,
            total: None,
            buf: Vec::new(),
            have: Vec::new(),
            agreed: None,
        })
    }

    /// Sends what is due and takes what came: the agreed session once through, Err when it cannot be.
    pub fn poll(&mut self, link: &mut dyn Transport, now: f64) -> Result<Option<Agreed>, String> {
        match self {
            Handshake::Host(h) => h.poll(link, now),
            Handshake::Join(j) => j.poll(link, now),
        }
    }
}

pub struct Host {
    offer: Offer,
    /// the joiner accepted (its nonce)
    nonce: Option<u32>,
    buttons: [u16; 8],
    /// the accepted joiner's Hello `extras`
    extras: u8,
    /// the accepted joiner's Hello `br1` (the imported game's content id, 0 = none)
    br1: u32,
    /// (blob, its checksum, the delay and the start in it)
    blob: Option<(Vec<u8>, u64, u32, SessionStart)>,
    /// the accepted joiner's last packet
    heard: f64,
    /// (address, Rejects sent to it, the time of the first)
    refused: Vec<(String, u8, f64)>,
}

fn send(link: &mut dyn Transport, nonce: u32, msg: Msg) {
    link.send(&proto::encode(&Packet { nonce, msg }));
}

impl Host {
    /// A Reject to the last sender, unless it was told `REFUSALS` times in the last `REFUSED_TTL` s (or too many
    /// were told).
    fn refuse(&mut self, link: &mut dyn Transport, nonce: u32, why: String, now: f64) {
        self.refused.retain(|r| now - r.2 < REFUSED_TTL);
        let at = link.peer();
        let k = match self.refused.iter().position(|r| r.0 == at) {
            Some(k) => k,
            None if self.refused.len() < MAX_REFUSED => {
                self.refused.push((at, 0, now));
                self.refused.len() - 1
            }
            None => return,
        };
        if self.refused[k].1 < REFUSALS {
            self.refused[k].1 += 1;
            log::warn!("netplay: refused a joiner from {}: {why}", self.refused[k].0);
            send(link, nonce, Msg::Reject { why });
        }
    }

    /// The accepted joiner forgotten: the next one's Hello is taken.
    fn forget(&mut self, link: &mut dyn Transport, why: &str) {
        log::warn!("netplay: player 2 at {} {why}; waiting for player 2 again", link.peer());
        eprintln!("netplay: player 2 {why}; waiting for player 2 again…");
        self.nonce = None;
        self.buttons = crate::assets::game_tables::default_buttons();
        self.extras = 0;
        self.br1 = 0;
        self.blob = None;
        link.unlock_peer();
    }

    fn poll(&mut self, link: &mut dyn Transport, now: f64) -> Result<Option<Agreed>, String> {
        // a direct fight on an extra stage this side has no files of (`--fight --stage Q3DM6` without them): no
        // session to offer
        if let Some(e) = direct_extra(&self.offer.start)
            && self.offer.start.extras() & 1 << e.index() == 0
        {
            return Err(format!("this side's fight is on the extra stage {}, whose files this side does not have", e.code));
        }
        while let Some(b) = link.recv() {
            let p = match proto::decode(&b) {
                Ok(p) => p,
                Err(e) => {
                    // a joiner of another protocol: told, if nobody was accepted yet
                    if let (None, Some(v)) = (self.nonce, proto::proto_of(&b)) {
                        let why = format!("the host speaks netplay protocol {}, this joiner {v}: both players need the same build", proto::PROTO);
                        self.refuse(link, 0, why, now);
                    } else {
                        log::debug!("netplay: bad packet from {}: {e}", link.peer());
                    }
                    continue;
                }
            };
            match p.msg {
                Msg::Hello { sim_version, state_version, engine, buttons, t, extras, br1, content } if self.nonce.is_none_or(|n| n == p.nonce) => {
                    if (sim_version, state_version, engine.as_str()) != (SIM_VERSION, STATE_VERSION, crate::platform::support::VERSION) {
                        let why = format!(
                            "version mismatch: the host runs engine {} (simulation v{SIM_VERSION}, state v{STATE_VERSION}), the joiner {engine} \
                             (simulation v{sim_version}, state v{state_version}): both players need the same build",
                            crate::platform::support::VERSION
                        );
                        self.refuse(link, p.nonce, why, now);
                        continue;
                    }
                    // both sides run the same game data (a data of another conversion would desync)
                    if Some(content).filter(|&c| c != 0) != self.offer.start.content {
                        log::info!(
                            "netplay: a joiner's game data differs (content id {content:08x}, this side's {:08x})",
                            self.offer.start.content.unwrap_or(0)
                        );
                        self.refuse(link, p.nonce, DATA_DIFFERS.into(), now);
                        continue;
                    }
                    // a direct fight on an extra stage needs its files on both sides (the menus' select offers
                    // one only then: `set_extras` below)
                    if let Some(e) = direct_extra(&self.offer.start)
                        && extras & 1 << e.index() == 0
                    {
                        let why = format!("the host's fight is on the extra stage {}, whose files the joiner does not have", e.code);
                        self.refuse(link, p.nonce, why, now);
                        continue;
                    }
                    // a direct fight with an imported fighter or stage needs the same data of the imported game on both sides
                    if direct_br1(&self.offer.start) && self.offer.start.br1 != Some(br1).filter(|&b| b != 0) {
                        let why =
                            "the host's fight has a Bloody Roar 1 fighter or stage, and the joiner has no Bloody Roar 1 disc data or other data".to_string();
                        self.refuse(link, p.nonce, why, now);
                        continue;
                    }
                    if self.nonce.is_none() {
                        log::info!("netplay: player 2 at {}", link.peer());
                        link.lock_peer();
                        self.nonce = Some(p.nonce);
                        self.buttons = if valid_buttons(&buttons) { buttons } else { crate::assets::game_tables::default_buttons() };
                        self.extras = extras;
                        self.br1 = br1;
                    }
                    self.heard = now;
                    send(link, p.nonce, Msg::Welcome { t });
                }
                Msg::Sync { rtt, have } if Some(p.nonce) == self.nonce => {
                    self.heard = now;
                    let (blob, sum, ..) = self.blob.get_or_insert_with(|| {
                        let delay = self.offer.delay.unwrap_or_else(|| auto_delay(rtt as f64 / 1e3)).clamp(1, MAX_DELAY);
                        log::info!("netplay: round trip {rtt} ms, input delay {delay} frames");
                        let o = &self.offer;
                        // an extra stage only when both sides have its files
                        let mut start = SessionStart { card: with_p2_buttons(o.start.card.clone(), self.buttons), ..o.start.clone() };
                        start.set_extras(o.start.extras() & self.extras);
                        // The imported game's data only when both sides have the same
                        start.br1 = o.start.br1.filter(|&b| b == self.br1);
                        let b = Blob {
                            engine: crate::platform::support::VERSION.into(),
                            sim_version: o.start.sim_version,
                            state_version: STATE_VERSION,
                            start: start.clone(),
                            prefs: o.prefs,
                            costume: o.costume,
                            delay,
                            time: o.time,
                        };
                        let mut bytes = Vec::new();
                        ciborium::into_writer(&b, &mut bytes).expect("the blob serialises");
                        let sum = fnv1a(FNV_BASIS, &bytes);
                        (bytes, sum, delay, start)
                    });
                    let n = blob.len().div_ceil(CHUNK);
                    for k in 0..CHUNKS_PER_SYNC.min(n) {
                        let at = (have as usize).min(n - 1).saturating_add(k) % n * CHUNK;
                        let data = blob[at..(at + CHUNK).min(blob.len())].to_vec();
                        send(link, p.nonce, Msg::Chunk { total: blob.len() as u32, sum: *sum, offset: at as u32, data });
                    }
                }
                Msg::Ready { sum } if Some(p.nonce) == self.nonce => {
                    self.heard = now;
                    let Some((_, mine, delay, start)) = &self.blob else { continue };
                    if sum != *mine {
                        continue;
                    }
                    send(link, p.nonce, Msg::Go);
                    let o = &self.offer;
                    return Ok(Some(Agreed { port: 0, nonce: p.nonce, delay: *delay, start: start.clone(), prefs: o.prefs, costume: o.costume, time: o.time }));
                }
                Msg::Bye { why } if Some(p.nonce) == self.nonce => self.forget(link, &format!("gave up ({why})")),
                _ => {}
            }
        }
        if self.nonce.is_some() && now - self.heard > HOST_FORGET {
            self.forget(link, &format!("went silent for {HOST_FORGET:.0} s"));
        }
        Ok(None)
    }
}

pub struct Join {
    nonce: u32,
    buttons: [u16; 8],
    /// its Hello `extras` (`proto::EXTRA_Q3DM17`)
    extras: u8,
    /// its Hello `br1` (the imported game's content id, 0 = none)
    br1: u32,
    /// its Hello `content` (its game data's content id, 0 = none)
    content: u32,
    rtts: Vec<f64>,
    started: f64,
    /// the host's last packet
    heard: f64,
    sent: f64,
    /// the blob's size and checksum (from its first chunk)
    total: Option<(usize, u64)>,
    buf: Vec<u8>,
    have: Vec<bool>,
    agreed: Option<Agreed>,
}

impl Join {
    fn poll(&mut self, link: &mut dyn Transport, now: f64) -> Result<Option<Agreed>, String> {
        while let Some(b) = link.recv() {
            let p = match proto::decode(&b) {
                Ok(p) => p,
                Err(e) => {
                    if proto::proto_of(&b).is_some_and(|v| v != proto::PROTO) {
                        return Err(format!("the host speaks another netplay protocol ({e}): both players need the same build"));
                    }
                    continue;
                }
            };
            if let Msg::Reject { why } = &p.msg
                && (p.nonce == self.nonce || p.nonce == 0)
            {
                return Err(format!("the host refused: {why}"));
            }
            if p.nonce != self.nonce {
                continue;
            }
            self.heard = now;
            match p.msg {
                Msg::Welcome { t } if self.rtts.len() < PINGS => {
                    let rtt = now - t as f64 / 1e3;
                    if (0.0..10.0).contains(&rtt) {
                        self.rtts.push(rtt);
                    }
                }
                Msg::Chunk { total, sum, offset, data } if self.agreed.is_none() => {
                    let (n, s) = *self.total.get_or_insert_with(|| {
                        self.buf = vec![0; total as usize];
                        self.have = vec![false; (total as usize).div_ceil(CHUNK)];
                        (total as usize, sum)
                    });
                    // every chunk of one blob: the same size and sum, at a chunk boundary
                    if total as usize != n || sum != s || !(offset as usize).is_multiple_of(CHUNK) || data.len() != CHUNK.min(n - offset as usize) {
                        continue;
                    }
                    self.buf[offset as usize..offset as usize + data.len()].copy_from_slice(&data);
                    self.have[offset as usize / CHUNK] = true;
                    if self.have.iter().all(|&h| h) {
                        let a = if fnv1a(FNV_BASIS, &self.buf) != s { Err("the host's session start is damaged (checksum)".into()) } else { self.adopt() };
                        // the host is told (it waits for the next joiner)
                        if let Err(e) = &a {
                            for _ in 0..3 {
                                send(link, self.nonce, Msg::Bye { why: e.clone() });
                            }
                        }
                        self.agreed = Some(a?);
                        self.sent = f64::NEG_INFINITY;
                    }
                }
                Msg::Go | Msg::Inputs { .. } if self.agreed.is_some() => return Ok(self.agreed.take()),
                _ => {}
            }
        }
        if now - self.heard > JOIN_TIMEOUT {
            return Err(format!("no answer from the host at {} for {JOIN_TIMEOUT:.0} s", link.peer()));
        }
        let msg = if let Some((_, sum)) = self.total.filter(|_| self.agreed.is_some()) {
            (now - self.sent >= 0.1).then_some(Msg::Ready { sum })
        } else if self.rtts.len() >= PINGS || (!self.rtts.is_empty() && now - self.started > 2.0) {
            let rtt = (self.rtts.iter().copied().fold(0.0, f64::max) * 1e3).round().min(u16::MAX as f64) as u16;
            let have = self.have.iter().position(|&h| !h).unwrap_or(0) as u32;
            (now - self.sent >= 0.05).then_some(Msg::Sync { rtt, have })
        } else {
            let t = (now * 1e3) as u32;
            (now - self.sent >= 0.1).then_some(Msg::Hello {
                sim_version: SIM_VERSION,
                state_version: STATE_VERSION,
                engine: crate::platform::support::VERSION.into(),
                buttons: self.buttons,
                t,
                extras: self.extras,
                br1: self.br1,
                content: self.content,
            })
        };
        if let Some(m) = msg {
            send(link, self.nonce, m);
            self.sent = now;
        }
        Ok(None)
    }

    /// The host's blob as this side's session: checked as a replay header is, the versions this build's.
    fn adopt(&self) -> Result<Agreed, String> {
        if self.buf.len() > MAX_BLOB {
            return Err("the host's session start is too large".into());
        }
        let crate::base::decode_limit::Budgeted::<Blob, DECODE_BUDGET>(b) =
            ciborium::from_reader(self.buf.as_slice()).map_err(|e| format!("the host's session start: {e}"))?;
        if (b.sim_version, b.state_version, b.engine.as_str()) != (SIM_VERSION, STATE_VERSION, crate::platform::support::VERSION) {
            return Err(format!(
                "version mismatch: the host runs engine {} (simulation v{}, state v{}), this is {} (v{SIM_VERSION}, v{STATE_VERSION})",
                b.engine,
                b.sim_version,
                b.state_version,
                crate::platform::support::VERSION
            ));
        }
        if !(1..=MAX_DELAY).contains(&b.delay) {
            return Err(format!("the host's input delay {}", b.delay));
        }
        let start = SessionStart { sim_version: b.sim_version, ..b.start };
        let h = replay::Header { start, costume: b.costume, prefs: b.prefs, ..Default::default() };
        replay::check_header(&h, b.sim_version).map_err(|e| format!("the host's session start: {e}"))?;
        if h.start.content != Some(self.content).filter(|&c| c != 0) {
            return Err(DATA_DIFFERS.into());
        }
        if let Some(e) = direct_extra(&h.start)
            && h.start.extras() & 1 << e.index() == 0
        {
            return Err(format!("the host's fight is on the extra stage {}, whose files this side does not have", e.code));
        }
        if h.start.br1.is_some_and(|b| b != self.br1) {
            return Err("the host's session has Bloody Roar 1 disc data this side does not have (or other data)".into());
        }
        log::info!("netplay: joined, input delay {} frames", b.delay);
        Ok(Agreed { port: 1, nonce: self.nonce, delay: b.delay, start: h.start, prefs: b.prefs, costume: b.costume, time: b.time })
    }
}

/// `start`'s direct fight has an imported fighter (`fight_data::is_br1`) or stage (`br1_stage`).
fn direct_br1(start: &SessionStart) -> bool {
    match &start.entry {
        crate::session::Entry::DirectFight { p1, p2, stage } => {
            crate::fight::fight_data::is_br1(p1)
                || crate::fight::fight_data::is_br1(p2)
                || crate::fight::stage::br1::by_code(stage, crate::fight::version::Gates::of(start.sim_version)).is_some()
        }
        _ => false,
    }
}

/// The extra stage of `start`'s direct fight (`--fight --stage Q3DM17` / `Q3DM6`), None for another start.
fn direct_extra(start: &SessionStart) -> Option<&'static crate::fight::stage::extra::Extra> {
    match &start.entry {
        crate::session::Entry::DirectFight { stage, .. } => crate::fight::stage::extra::by_code(stage, crate::fight::version::Gates::of(start.sim_version)),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::net::link::mem_pair;

    fn offer() -> Offer {
        let s = crate::frontend::options::Settings { time_limit: 0, ..Default::default() };
        let mut card = crate::frontend::options::Card::new(None, None);
        assert!(s.save(&mut card));
        let start = SessionStart { seed: 9, card: card.file().map(<[u8]>::to_vec), entry: crate::session::Entry::Title, ..Default::default() };
        Offer { start, prefs: Prefs { hud_size: 2, ..Default::default() }, costume: [1, 2], time: 1_700_000_000, delay: None }
    }

    /// Both ends of a handshake over an in-memory link, polled in turn every 10 ms.
    fn handshake(o: Offer, mine: [u16; 8]) -> (Agreed, Agreed) {
        handshake_extras(o, mine, 0)
    }

    /// `handshake` with the joiner's Hello `extras`.
    fn handshake_extras(o: Offer, mine: [u16; 8], extras: u8) -> (Agreed, Agreed) {
        handshake_with(o, mine, extras, 0)
    }

    /// `handshake` with the joiner's Hello `extras` and `br1`.
    fn handshake_with(o: Offer, mine: [u16; 8], extras: u8, br1: u32) -> (Agreed, Agreed) {
        let (mut a, mut b) = mem_pair();
        let mut h = Handshake::host(o);
        let mut j = Handshake::join(77, mine, extras, br1, 0, 0.0);
        let (mut ha, mut ja) = (None, None);
        let mut t = 0.0;
        for _ in 0..2000 {
            t += 0.01;
            if ja.is_none() {
                ja = j.poll(&mut b, t).unwrap();
            }
            if ha.is_none() {
                ha = h.poll(&mut a, t).unwrap();
            }
            if ha.is_some() && ja.is_some() {
                break;
            }
        }
        (ha.expect("host agreed"), ja.expect("joiner agreed"))
    }

    /// An extra stage is in the agreed start only when the host has its files and the joiner's Hello says it has
    /// them too, each stage on its own (Q3DM17, Q3DM6).
    #[test]
    fn extra_stage_needs_both() {
        let (q17, q6) = (proto::EXTRA_Q3DM17, proto::EXTRA_Q3DM6);
        for host in 0..4u8 {
            for joiner in 0..4u8 {
                let mut o = offer();
                o.start.set_extras(host);
                let (ha, ja) = handshake_extras(o, crate::assets::game_tables::default_buttons(), joiner);
                let agreed = host & joiner;
                assert_eq!((ha.start.extras(), ja.start.extras()), (agreed, agreed), "host {host} joiner {joiner}");
                assert_eq!((ha.start.extra_stage, ha.start.extra_q3dm6), (agreed & q17 != 0, agreed & q6 != 0));
            }
        }
        // a direct fight on one: a joiner without its files is refused (with the other's only too); with them it joins
        for (e, bit, other) in [(crate::fight::stage::extra::Q3DM17, q17, q6), (crate::fight::stage::extra::Q3DM6, q6, q17)] {
            let mut o = offer();
            o.start.set_extras(3);
            o.start.entry = crate::session::Entry::DirectFight { p1: "YUG".into(), p2: "ALI".into(), stage: e.code.into() };
            for joiner in [0, other] {
                let (mut a, mut b) = mem_pair();
                let mut h = Handshake::host(o.clone());
                let mut j = Handshake::join(5, crate::assets::game_tables::default_buttons(), joiner, 0, 0, 0.0);
                assert!(j.poll(&mut b, 0.0).unwrap().is_none());
                assert!(h.poll(&mut a, 0.0).unwrap().is_none());
                let err = j.poll(&mut b, 0.01).unwrap_err();
                assert!(err.contains("extra stage") && err.contains(e.code), "{err}");
            }
            let (ha, ja) = handshake_extras(o.clone(), crate::assets::game_tables::default_buttons(), bit);
            assert!(ha.start.extras() == bit && ja.start.extras() == bit, "{}", e.code);
            // the host without the stage's own files: refused on the host, before any joiner
            o.start.set_extras(other);
            let (mut a, _b) = mem_pair();
            let err = Handshake::host(o).poll(&mut a, 0.0).unwrap_err();
            assert!(err.contains("this side does not have") && err.contains(e.code), "{err}");
        }
    }

    /// The imported game's content id is in the agreed start only when both sides have the same; a direct fight with a
    /// imported fighter refuses a joiner without it.
    #[test]
    fn br1_needs_both() {
        for (host, joiner, agreed) in [(None, 0, None), (Some(5), 0, None), (Some(5), 6, None), (Some(5), 5, Some(5)), (None, 5, None)] {
            let mut o = offer();
            o.start.br1 = host;
            let (ha, ja) = handshake_with(o, crate::assets::game_tables::default_buttons(), 0, joiner);
            assert_eq!((ha.start.br1, ja.start.br1), (agreed, agreed), "host {host:?} joiner {joiner}");
        }
        let mut o = offer();
        o.start.br1 = Some(5);
        o.start.entry = crate::session::Entry::DirectFight { p1: "MIT".into(), p2: "ALI".into(), stage: "ST0".into() };
        for joiner in [0, 6] {
            let (mut a, mut b) = mem_pair();
            let mut h = Handshake::host(o.clone());
            let mut j = Handshake::join(5, crate::assets::game_tables::default_buttons(), 0, joiner, 0, 0.0);
            assert!(j.poll(&mut b, 0.0).unwrap().is_none());
            assert!(h.poll(&mut a, 0.0).unwrap().is_none());
            let err = j.poll(&mut b, 0.01).unwrap_err();
            assert!(err.contains("Bloody Roar 1"), "{err}");
        }
        let (ha, ja) = handshake_with(o, crate::assets::game_tables::default_buttons(), 0, 5);
        assert_eq!((ha.start.br1, ja.start.br1), (Some(5), Some(5)));
    }

    /// Both sides need the same game data: a joiner whose content id differs from the host's (or that has none when
    /// the host has one, or one when the host has none) is refused with `DATA_DIFFERS`; the same id agrees, and the
    /// agreed start carries it. A host's start of other data is refused by the joiner too.
    #[test]
    fn content_must_match() {
        let refused = |host: Option<u32>, joiner: u32| {
            let mut o = offer();
            o.start.content = host;
            let (mut a, mut b) = mem_pair();
            let mut h = Handshake::host(o);
            let mut j = Handshake::join(5, crate::assets::game_tables::default_buttons(), 0, 0, joiner, 0.0);
            assert!(j.poll(&mut b, 0.0).unwrap().is_none());
            assert!(h.poll(&mut a, 0.0).unwrap().is_none());
            j.poll(&mut b, 0.01).unwrap_err()
        };
        for (host, joiner) in [(Some(7), 8), (Some(7), 0), (None, 7)] {
            let err = refused(host, joiner);
            assert!(err.contains(DATA_DIFFERS), "host {host:?} joiner {joiner}: {err}");
        }
        let mut o = offer();
        o.start.content = Some(7);
        let (mut a, mut b) = mem_pair();
        let mut h = Handshake::host(o.clone());
        let mut j = Handshake::join(77, crate::assets::game_tables::default_buttons(), 0, 0, 7, 0.0);
        let (mut ha, mut ja) = (None, None);
        let mut t = 0.0;
        while ha.is_none() || ja.is_none() {
            t += 0.01;
            assert!(t < 20.0, "no agreement");
            if ja.is_none() {
                ja = j.poll(&mut b, t).unwrap();
            }
            if ha.is_none() {
                ha = h.poll(&mut a, t).unwrap();
            }
        }
        assert_eq!((ha.unwrap().start.content, ja.unwrap().start.content), (Some(7), Some(7)));
        // the joiner's own check of the host's blob
        let Handshake::Join(mut j) = Handshake::join(77, [0; 8], 0, 0, 8, 0.0) else { unreachable!() };
        let blob = Blob {
            engine: crate::platform::support::VERSION.into(),
            sim_version: SIM_VERSION,
            state_version: STATE_VERSION,
            start: o.start,
            prefs: o.prefs,
            costume: o.costume,
            delay: 2,
            time: o.time,
        };
        ciborium::into_writer(&blob, &mut j.buf).unwrap();
        assert_eq!(j.adopt().unwrap_err(), DATA_DIFFERS);
        j.content = 7;
        assert_eq!(j.adopt().unwrap().start.content, Some(7));
    }

    /// Both sides agree on the host's start, the joiner's map lands in the card's PLAYER 2 map, the delay follows
    /// the round trip; a big card (many chunks) comes through.
    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn agree() {
        // the host checks the joiner's map against the tables' actions (`valid_buttons`): installed here, the test's
        // own map; without the test data (no tables) only the map of "nothing" is one
        let mine = match crate::formats::tables::export_tables() {
            Some(_) => [0x40, 0x80, 0x20, 0xC0, 0x408, 0x02, 0xE0, 0x01],
            None => [0; 8],
        };
        let (ha, ja) = handshake(offer(), mine);
        assert_eq!((ha.port, ja.port, ha.nonce, ja.nonce), (0, 1, 77, 77));
        assert_eq!((ha.delay, ja.delay), (2, 2));
        assert_eq!(ha.start.card, ja.start.card);
        assert_eq!((ja.start.seed, ja.prefs.hud_size, ja.costume, ja.time), (9, 2, [1, 2], 1_700_000_000));
        let s = crate::frontend::options::Settings::try_load(&crate::frontend::options::Card::new(ja.start.card.clone(), None)).unwrap();
        // the host's unset map: the tables' default, or without them `options::UNSET` (`Settings::resolve_buttons`)
        let p1 = match crate::assets::game_tables::default_buttons() {
            [0, 0, 0, 0, 0, 0, 0, 0] => crate::frontend::options::UNSET,
            d => d,
        };
        assert_eq!((s.buttons[1], s.buttons[0], s.time_limit), (mine, p1, 0));
        // padding inside the JSON (whitespace): 40 chunks; with the tables the joiner's map is the card's, so the card is
        // not rewritten; without them the card's unset map is not a valid one and the default replaces it
        let mut o = offer();
        o.delay = Some(5);
        if let Some(c) = &mut o.start.card {
            c.splice(1..1, std::iter::repeat_n(b' ', 40_000));
        }
        let (ha, ja) = handshake(o.clone(), crate::assets::game_tables::default_buttons());
        assert_eq!((ha.delay, ja.delay), (5, 5));
        let card = with_p2_buttons(o.start.card.clone(), crate::assets::game_tables::default_buttons());
        assert!(ja.start.card == card && ha.start.card == card);
        assert_eq!(card == o.start.card, p1 != crate::frontend::options::UNSET);
    }

    /// A desktop and a browser or Android player: the joiner runs the host's menus (`SessionStart::browser` /
    /// `android` in the blob), either way round.
    #[test]
    fn platform_flags_are_the_hosts() {
        for (host_browser, host_android) in [(true, false), (false, true), (false, false)] {
            let mut o = offer();
            o.start.browser = host_browser;
            o.start.android = host_android;
            let (ha, ja) = handshake(o, crate::assets::game_tables::default_buttons());
            assert_eq!((ha.start.browser, ja.start.browser), (host_browser, host_browser));
            assert_eq!((ha.start.android, ja.start.android), (host_android, host_android));
            let s = crate::session::new_state(&ja.start, &crate::session::AssetStore::default().assets(ja.prefs), &mut Default::default());
            assert_eq!((s.browser, s.android), (host_browser, host_android));
        }
    }

    /// A joiner of another build is refused with a message naming both versions; garbage is ignored.
    #[test]
    fn version_refused() {
        let (mut a, mut b) = mem_pair();
        let mut h = Handshake::host(offer());
        b.send(b"garbage");
        let hello = Msg::Hello {
            sim_version: SIM_VERSION + 1,
            state_version: STATE_VERSION,
            engine: crate::platform::support::VERSION.into(),
            buttons: [0; 8],
            t: 0,
            extras: 0,
            br1: 0,
            content: 0,
        };
        b.send(&proto::encode(&Packet { nonce: 5, msg: hello }));
        assert!(h.poll(&mut a, 0.0).unwrap().is_none());
        let mut j = Handshake::join(5, crate::assets::game_tables::default_buttons(), 0, 0, 0, 0.0);
        let e = j.poll(&mut b, 0.01).unwrap_err();
        assert!(e.contains("version mismatch") && e.contains(&format!("simulation v{}", SIM_VERSION + 1)), "{e}");
        // another protocol
        let (mut a, mut b) = mem_pair();
        let mut bad = proto::encode(&Packet { nonce: 5, msg: Msg::Go });
        bad[4..6].copy_from_slice(&(proto::PROTO + 1).to_le_bytes());
        b.send(&bad);
        assert!(h.poll(&mut a, 0.0).unwrap().is_none());
        let e = Handshake::join(5, [0; 8], 0, 0, 0, 0.0).poll(&mut b, 0.01).unwrap_err();
        assert!(e.contains("protocol"), "{e}");
        // no host
        let (_a, mut b) = mem_pair();
        let mut j = Handshake::join(5, [0; 8], 0, 0, 0, 0.0);
        assert!(j.poll(&mut b, 1.0).unwrap().is_none());
        assert!(j.poll(&mut b, JOIN_TIMEOUT + 1.0).is_err_and(|e| e.contains("no answer")));
    }

    /// A joiner that leaves before Ready (its Bye, or silence for `HOST_FORGET` s) is forgotten: the next one
    /// (another nonce) gets through. Rejects to one address stop after `REFUSALS`.
    #[test]
    fn joiner_forgotten() {
        let (mut a, mut b) = mem_pair();
        let mut h = Handshake::host(offer());
        let welcomed = |b: &mut crate::net::link::Mem, nonce: u32| {
            std::iter::from_fn(|| b.recv()).any(|x| proto::decode(&x).is_ok_and(|p| p.nonce == nonce && matches!(p.msg, Msg::Welcome { .. })))
        };
        let hello = |nonce: u32| {
            let msg = Msg::Hello {
                sim_version: SIM_VERSION,
                state_version: STATE_VERSION,
                engine: crate::platform::support::VERSION.into(),
                buttons: [0; 8],
                t: 0,
                extras: 0,
                br1: 0,
                content: 0,
            };
            proto::encode(&Packet { nonce, msg })
        };
        b.send(&hello(5));
        assert!(h.poll(&mut a, 0.0).unwrap().is_none() && welcomed(&mut b, 5));
        b.send(&hello(6));
        assert!(h.poll(&mut a, 1.0).unwrap().is_none() && !welcomed(&mut b, 6));
        b.send(&proto::encode(&Packet { nonce: 5, msg: Msg::Bye { why: "bad blob".into() } }));
        b.send(&hello(6));
        assert!(h.poll(&mut a, 1.1).unwrap().is_none() && welcomed(&mut b, 6));
        // 6 goes silent
        assert!(h.poll(&mut a, 1.1 + HOST_FORGET + 0.1).unwrap().is_none());
        let mut j = Handshake::join(7, crate::assets::game_tables::default_buttons(), 0, 0, 0, 10.0);
        let (mut ha, mut ja) = (None, None);
        let mut t = 10.0;
        for _ in 0..2000 {
            t += 0.01;
            if ja.is_none() {
                ja = j.poll(&mut b, t).unwrap();
            }
            if ha.is_none() {
                ha = h.poll(&mut a, t).unwrap();
            }
        }
        assert_eq!((ha.unwrap().nonce, ja.unwrap().nonce), (7, 7));
        // refusals
        let (mut a, mut b) = mem_pair();
        let mut h = Handshake::host(offer());
        let mut bad = proto::encode(&Packet { nonce: 5, msg: Msg::Go });
        bad[4..6].copy_from_slice(&(proto::PROTO + 1).to_le_bytes());
        for _ in 0..10 {
            b.send(&bad);
        }
        assert!(h.poll(&mut a, 0.0).unwrap().is_none());
        assert_eq!(std::iter::from_fn(|| b.recv()).count(), REFUSALS as usize);
        // told again once its entry expired
        b.send(&bad);
        assert!(h.poll(&mut a, REFUSED_TTL - 1.0).unwrap().is_none());
        assert_eq!(std::iter::from_fn(|| b.recv()).count(), 0);
        b.send(&bad);
        assert!(h.poll(&mut a, REFUSED_TTL + 1.0).unwrap().is_none());
        assert_eq!(std::iter::from_fn(|| b.recv()).count(), 1);
        assert_ne!(nonce(), nonce());
    }

    #[test]
    fn delays() {
        assert_eq!(auto_delay(0.0), 2);
        assert_eq!(auto_delay(0.01), 2);
        assert_eq!(auto_delay(0.02), 3);
        assert_eq!(auto_delay(0.16), 7);
        assert_eq!(auto_delay(0.34), 12);
        assert_eq!(auto_delay(100.0), MAX_DELAY);
        // an unparsable card is left alone
        assert_eq!(with_p2_buttons(Some(b"x".to_vec()), [0; 8]), Some(b"x".to_vec()));
        assert_eq!(with_p2_buttons(None, [0; 8]), None);
        assert!(!valid_buttons(&[0x1234; 8]) && valid_buttons(&crate::assets::game_tables::default_buttons()));
    }
}
