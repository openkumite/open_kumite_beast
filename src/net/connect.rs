//! The netplay connect phase: the link comes up, then the handshake, polled from a frame loop.
//!
//! Engine addition.
//!
//! The connect phase (game-state stage 9): the link comes up (a room: the signaling server, the other player, the
//! sides decided; UDP: at once, the side from the command line), then the handshake runs (`super::handshake`); both
//! polled from a frame loop (the app's, `nettest`'s), never blocking, with a line for the screen at every point and
//! a timeout with its message wherever the other end may stay silent (the link's, the handshake's). A UDP host and a
//! room's first player wait for the other player as long as it takes (a room: `super::webrtc::WAIT_PEER`).

use super::link::{Link, SimNet, Transport};
use super::{Agreed, Handshake, Offer};

/// How to reach the other player.
#[derive(Clone, Debug)]
pub enum Setup {
    /// `--host [ADDR:]PORT` / `--join ADDR:PORT` (desktop): UDP to an address, the side given
    #[cfg(not(target_arch = "wasm32"))]
    Udp(super::udp::Role),
    /// `--signal URL --room CODE` / `?signal=URL&room=CODE`: a room on a matchbox signaling server, WebRTC to the other
    /// player in it, the first one in hosts
    #[cfg(feature = "netplay-webrtc")]
    Room(super::webrtc::Room),
}

impl Setup {
    /// The link (in OKB_NETSIM's simulated network when `sim` is set) and the port the setup fixes (None: the link
    /// decides it).
    pub fn open(&self, sim: Option<SimNet>, now: f64) -> Result<(Box<dyn Transport>, Option<usize>), String> {
        // (a browser build without the feature has no setup: neither is used)
        let _ = (&sim, now);
        match *self {
            #[cfg(not(target_arch = "wasm32"))]
            Setup::Udp(ref role) => {
                let port = matches!(role, super::udp::Role::Join(_)) as usize;
                Ok((super::udp::open(role, sim)?, Some(port)))
            }
            #[cfg(feature = "netplay-webrtc")]
            Setup::Room(ref room) => {
                let rtc = super::webrtc::WebRtc::open(room, now)?;
                let link: Box<dyn Transport> = match sim {
                    Some(cfg) => {
                        log::info!("netplay: simulated network {cfg:?}");
                        Box::new(super::link::NetSim::new(rtc, cfg, crate::clock_seed() ^ now.to_bits(), crate::perf_now))
                    }
                    None => Box::new(rtc),
                };
                Ok((link, None))
            }
        }
    }

    /// The connect screen's first line.
    pub fn describe(&self) -> String {
        match *self {
            #[cfg(not(target_arch = "wasm32"))]
            Setup::Udp(super::udp::Role::Host(a)) => format!("NETPLAY: HOSTING ON {a} (--join THIS-ADDRESS:{})", a.port()),
            #[cfg(not(target_arch = "wasm32"))]
            Setup::Udp(super::udp::Role::Join(a)) => format!("NETPLAY: JOINING {a}"),
            #[cfg(feature = "netplay-webrtc")]
            Setup::Room(ref r) => format!("NETPLAY: ROOM {} ON {}", r.code, r.signal),
        }
    }
}

/// One side's connect: the link coming up, then the handshake of its side.
pub struct Connect {
    link: Box<dyn Transport>,
    port: Option<usize>,
    /// what this side offers if it hosts, and sends if it joins
    offer: Offer,
    buttons: [u16; 8],
    nonce: u32,
    hs: Option<Handshake>,
    what: String,
    line: String,
    /// the current line's start (its seconds are shown)
    since: f64,
}

impl Connect {
    /// `link` (with the port its setup fixes, `Setup::open`); `offer`: the session hosted if this side hosts;
    /// `buttons`: this side's own P1 map if it joins (with the offer's `extras`: the extra stages it has the files
    /// of); `what`: the first line (`Setup::describe`).
    pub fn new(link: (Box<dyn Transport>, Option<usize>), offer: Offer, buttons: [u16; 8], what: String, now: f64) -> Self {
        Self { link: link.0, port: link.1, offer, buttons, nonce: super::handshake::nonce(), hs: None, what, line: "CONNECTING".into(), since: now }
    }

    /// This side's game data's content id (`SessionStart::content`) once known (the browser reads it after the connect
    /// began): offered as the host's, sent as the joiner's. Before the link is up only: the handshake takes the offer
    /// as it then is.
    pub fn set_content(&mut self, content: Option<u32>) {
        if self.hs.is_none() {
            self.offer.start.content = content;
        } else if self.offer.start.content != content {
            log::warn!("netplay: the game data's content id came after the handshake began");
        }
    }

    /// This side's own P1 map (`handshake::own_buttons`) read again once the game data's tables are in (the command
    /// line's connect began before them: its map was read without the default map or the actions to check it against).
    /// Before the link is up only, as `set_content`.
    pub fn set_buttons(&mut self, buttons: [u16; 8]) {
        if self.hs.is_none() {
            self.buttons = buttons;
        }
    }

    /// Sends what is due and takes what came: the agreed session once through (then `finish`), Err with why not.
    pub fn poll(&mut self, now: f64) -> Result<Option<Agreed>, String> {
        if self.hs.is_none() {
            let port = match self.link.link(now)? {
                Link::Pending(l) => {
                    self.set_line(l, now);
                    return Ok(None);
                }
                Link::Up(p) => p.or(self.port).ok_or("the link decided no side")?,
            };
            self.port = Some(port);
            self.hs = Some(if port == 0 {
                log::info!("netplay: player 1 (host)");
                Handshake::host(self.offer.clone())
            } else {
                if self.offer.delay.is_some() {
                    log::info!("netplay: player 2: the input delay is the host's to choose");
                }
                Handshake::join(
                    self.nonce,
                    self.buttons,
                    self.offer.start.extras(),
                    self.offer.start.br1.unwrap_or(0),
                    self.offer.start.content.unwrap_or(0),
                    now,
                )
            });
            self.set_line(if port == 0 { "WAITING FOR PLAYER 2".into() } else { "JOINING PLAYER 1".into() }, now);
        }
        // a room's other player left before the session started
        if let Some(why) = self.link.lost() {
            return Err(why);
        }
        let hs = self.hs.as_mut().expect("the handshake");
        hs.poll(&mut *self.link, now)
    }

    fn set_line(&mut self, l: String, now: f64) {
        if l != self.line {
            log::info!("netplay: {l}");
            self.line = l;
            self.since = now;
        }
    }

    /// The link of the agreed session.
    pub fn finish(self) -> Box<dyn Transport> {
        self.link
    }

    /// The connect screen: the setup, where it is and for how long.
    pub fn lines(&self, now: f64) -> Vec<String> {
        vec![self.what.clone(), format!("{} {:.0} S", self.line, now - self.since)]
    }

    /// The line that changes when the connect gets somewhere (the app's progress on stderr).
    pub fn line(&self) -> &str {
        &self.line
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::net::link::mem_pair;
    use crate::session::SessionStart;

    /// A link whose sides come up after a while (the first one's after 3 polls as the host, the other's after 5).
    struct Slow {
        inner: crate::net::link::Mem,
        polls: u32,
        up_after: u32,
        port: usize,
    }

    impl Transport for Slow {
        fn send(&mut self, b: &[u8]) {
            self.inner.send(b)
        }
        fn recv(&mut self) -> Option<Vec<u8>> {
            self.inner.recv()
        }
        fn link(&mut self, _now: f64) -> Result<Link, String> {
            self.polls += 1;
            Ok(if self.polls > self.up_after { Link::Up(Some(self.port)) } else { Link::Pending(format!("WAITING {}", self.polls)) })
        }
    }

    /// Two connects over a link that decides the sides: both get through as the side it gave, the lines follow.
    #[test]
    fn link_decides_sides() {
        let (a, b) = mem_pair();
        let offer = Offer { start: SessionStart { seed: 4, ..Default::default() }, prefs: Default::default(), costume: [0; 2], time: 9, delay: Some(4) };
        let mk = |inner, up_after, port| -> (Box<dyn Transport>, Option<usize>) { (Box::new(Slow { inner, polls: 0, up_after, port }), None) };
        let mut x = Connect::new(mk(a, 3, 1), offer.clone(), crate::assets::game_tables::default_buttons(), "X".into(), 0.0);
        let mut y = Connect::new(mk(b, 5, 0), offer, crate::assets::game_tables::default_buttons(), "Y".into(), 0.0);
        assert!(x.poll(0.01).unwrap().is_none());
        assert_eq!(x.lines(1.0), ["X", "WAITING 1 1 S"]);
        let (mut xa, mut ya) = (None, None);
        let mut t = 0.0;
        for _ in 0..1000 {
            t += 0.01;
            if xa.is_none() {
                xa = x.poll(t).unwrap();
            }
            if ya.is_none() {
                ya = y.poll(t).unwrap();
            }
        }
        let (xa, ya) = (xa.expect("x through"), ya.expect("y through"));
        assert_eq!((xa.port, ya.port, xa.delay, ya.delay, xa.start.seed), (1, 0, 4, 4, 4));
    }

    /// A link that cannot come up, and one whose peer went during the handshake: the connect stops with why.
    #[test]
    fn link_errors() {
        struct Broken(bool);
        impl Transport for Broken {
            fn send(&mut self, _: &[u8]) {}
            fn recv(&mut self) -> Option<Vec<u8>> {
                None
            }
            fn link(&mut self, _: f64) -> Result<Link, String> {
                if self.0 { Ok(Link::Up(None)) } else { Err("no server".into()) }
            }
            fn lost(&mut self) -> Option<String> {
                self.0.then(|| "the other player left the room".into())
            }
        }
        let offer = Offer { start: SessionStart::default(), prefs: Default::default(), costume: [0; 2], time: 0, delay: None };
        let mut c = Connect::new((Box::new(Broken(false)), Some(0)), offer.clone(), [0; 8], String::new(), 0.0);
        assert_eq!(c.poll(0.1).unwrap_err(), "no server");
        let mut c = Connect::new((Box::new(Broken(true)), Some(1)), offer, [0; 8], String::new(), 0.0);
        assert_eq!(c.poll(0.1).unwrap_err(), "the other player left the room");
    }
}
