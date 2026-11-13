//! The datagram links netplay runs over (`Transport`) and the simulated network.
//!
//! Engine addition.
//!
//! The datagram links netplay runs over (`Transport`): UDP (`super::udp`, desktop), a WebRTC data channel to the
//! other player of a room (`super::webrtc`, feature `netplay-webrtc`), an in-memory pair (tests), and `NetSim`, which
//! delays, drops and reorders another link's datagrams (debug OKB_NETSIM).

use std::cell::RefCell;
use std::collections::VecDeque;
use std::rc::Rc;

/// Where a link is before the handshake (`Transport::link`).
#[derive(Clone, Debug, PartialEq)]
pub enum Link {
    /// not up yet: what it waits for (the connect screen's line; the connect counts the seconds since it changed)
    Pending(String),
    /// datagrams reach the peer; the port this side plays when the link decided it (a room's first comer hosts),
    /// None when the caller gave it (`--host` / `--join`)
    Up(Option<usize>),
}

/// An unreliable, unordered datagram link to one peer. Never blocks; a datagram may be lost, duplicated or
/// reordered.
pub trait Transport {
    /// Sends one datagram to the peer (dropped while no peer is known: a host before the first Hello).
    fn send(&mut self, b: &[u8]);
    /// The next datagram received, None when none waits. Before `lock_peer` a host's link takes any sender's and
    /// answers the last one; after it only the peer's.
    fn recv(&mut self) -> Option<Vec<u8>>;
    /// The peer is the last sender from now on: datagrams from anyone else are dropped.
    fn lock_peer(&mut self) {}
    /// Back to before `lock_peer` with no peer (a host whose joiner went before the session started).
    fn unlock_peer(&mut self) {}
    /// Where the peer is (logs).
    fn peer(&self) -> String {
        String::new()
    }
    /// Before the handshake, polled with the time (seconds, monotonic): Err when the link cannot come up.
    fn link(&mut self, _now: f64) -> Result<Link, String> {
        Ok(Link::Up(None))
    }
    /// The link to the peer broke (a room's peer left it, the data channel closed): why. A link with no way to tell
    /// leaves it to the timeouts.
    fn lost(&mut self) -> Option<String> {
        None
    }
}

impl<T: Transport + ?Sized> Transport for Box<T> {
    fn send(&mut self, b: &[u8]) {
        (**self).send(b)
    }
    fn recv(&mut self) -> Option<Vec<u8>> {
        (**self).recv()
    }
    fn lock_peer(&mut self) {
        (**self).lock_peer()
    }
    fn unlock_peer(&mut self) {
        (**self).unlock_peer()
    }
    fn peer(&self) -> String {
        (**self).peer()
    }
    fn link(&mut self, now: f64) -> Result<Link, String> {
        (**self).link(now)
    }
    fn lost(&mut self) -> Option<String> {
        (**self).lost()
    }
}

/// One end of an in-memory link (`mem_pair`): what one end sends the other receives, in order, nothing lost.
pub struct Mem {
    inbox: Rc<RefCell<VecDeque<Vec<u8>>>>,
    outbox: Rc<RefCell<VecDeque<Vec<u8>>>>,
}

/// Two connected in-memory ends (single-threaded tests).
pub fn mem_pair() -> (Mem, Mem) {
    let (a, b) = (Rc::new(RefCell::new(VecDeque::new())), Rc::new(RefCell::new(VecDeque::new())));
    (Mem { inbox: a.clone(), outbox: b.clone() }, Mem { inbox: b, outbox: a })
}

impl Transport for Mem {
    fn send(&mut self, b: &[u8]) {
        self.outbox.borrow_mut().push_back(b.to_vec());
    }
    fn recv(&mut self) -> Option<Vec<u8>> {
        self.inbox.borrow_mut().pop_front()
    }
    fn peer(&self) -> String {
        "memory".into()
    }
}

/// OKB_NETSIM's settings: added one-way latency, loss and jitter of every datagram sent.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct SimNet {
    /// seconds
    pub latency: f64,
    /// 0..=1
    pub loss: f64,
    /// seconds: each datagram's latency is off by up to this either way (so datagrams reorder)
    pub jitter: f64,
}

impl SimNet {
    /// `lat_ms,loss_pct[,jitter_ms]` (e.g. `80,5` or `150,10,20`).
    pub fn parse(s: &str) -> Result<Self, String> {
        let v: Vec<f64> = s.split(',').map(|x| x.trim().parse::<f64>().map_err(|_| format!("OKB_NETSIM={s}: not a number: {x}"))).collect::<Result<_, _>>()?;
        let ok = |x: f64, max: f64| x.is_finite() && (0.0..=max).contains(&x);
        match v[..] {
            [lat, loss] | [lat, loss, _] if ok(lat, 5000.0) && ok(loss, 100.0) && v.get(2).is_none_or(|&j| ok(j, 5000.0)) => {
                Ok(Self { latency: lat / 1e3, loss: loss / 100.0, jitter: v.get(2).map_or(0.0, |j| j / 1e3) })
            }
            _ => Err(format!("OKB_NETSIM={s}: lat_ms,loss_pct[,jitter_ms] (latency and jitter up to 5000 ms, loss 0..100)")),
        }
    }

    /// OKB_NETSIM, if set (an unparsable value is an error).
    pub fn from_env() -> Result<Option<Self>, String> {
        match crate::base::env::var("NETSIM") {
            Ok(v) if !v.is_empty() => Self::parse(&v).map(Some),
            _ => Ok(None),
        }
    }
}

/// `inner` with OKB_NETSIM's latency, loss and jitter on what it sends (each side runs one: both directions).
pub struct NetSim<T: Transport> {
    inner: T,
    cfg: SimNet,
    /// (due time, datagram), in sending order
    queue: Vec<(f64, Vec<u8>)>,
    rng: u64,
    clock: fn() -> f64,
}

impl<T: Transport> NetSim<T> {
    /// `seed`: the loss and jitter draws (any value). `clock`: seconds, monotonic.
    pub fn new(inner: T, cfg: SimNet, seed: u64, clock: fn() -> f64) -> Self {
        Self { inner, cfg, queue: Vec::new(), rng: seed | 1, clock }
    }

    /// A uniform draw in 0..1 (xorshift64).
    fn draw(&mut self) -> f64 {
        self.rng ^= self.rng << 13;
        self.rng ^= self.rng >> 7;
        self.rng ^= self.rng << 17;
        (self.rng >> 11) as f64 / (1u64 << 53) as f64
    }

    /// The datagrams due by now go out, earliest due first (jitter reorders them).
    fn pump(&mut self) {
        let now = (self.clock)();
        let (mut due, wait): (Vec<_>, Vec<_>) = std::mem::take(&mut self.queue).into_iter().partition(|(t, _)| *t <= now);
        self.queue = wait;
        due.sort_by(|a, b| a.0.total_cmp(&b.0));
        for (_, b) in due {
            self.inner.send(&b);
        }
    }
}

impl<T: Transport> Transport for NetSim<T> {
    fn send(&mut self, b: &[u8]) {
        if self.draw() >= self.cfg.loss {
            let jitter = (self.draw() * 2.0 - 1.0) * self.cfg.jitter;
            let due = (self.clock)() + (self.cfg.latency + jitter).max(0.0);
            self.queue.push((due, b.to_vec()));
        }
        self.pump();
    }
    fn recv(&mut self) -> Option<Vec<u8>> {
        self.pump();
        self.inner.recv()
    }
    fn lock_peer(&mut self) {
        self.inner.lock_peer()
    }
    fn unlock_peer(&mut self) {
        self.inner.unlock_peer()
    }
    fn peer(&self) -> String {
        self.inner.peer()
    }
    fn link(&mut self, now: f64) -> Result<Link, String> {
        self.inner.link(now)
    }
    fn lost(&mut self) -> Option<String> {
        self.inner.lost()
    }
}

/// What is still queued goes out now (a session's last Byes, sent just before the app quits).
impl<T: Transport> Drop for NetSim<T> {
    fn drop(&mut self) {
        let mut q = std::mem::take(&mut self.queue);
        q.sort_by(|a, b| a.0.total_cmp(&b.0));
        for (_, b) in q {
            self.inner.send(&b);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn netsim_parse() {
        assert_eq!(SimNet::parse("0,0"), Ok(SimNet::default()));
        assert_eq!(SimNet::parse("80,5"), Ok(SimNet { latency: 0.08, loss: 0.05, jitter: 0.0 }));
        assert_eq!(SimNet::parse("150, 10, 20"), Ok(SimNet { latency: 0.15, loss: 0.1, jitter: 0.02 }));
        for bad in ["", "80", "80,5,1,2", "a,b", "-1,0", "80,101", "inf,0", "80,5,NaN"] {
            assert!(SimNet::parse(bad).is_err(), "{bad}");
        }
    }

    thread_local!(static NOW: std::cell::Cell<f64> = const { std::cell::Cell::new(0.0) });
    fn now() -> f64 {
        NOW.with(|n| n.get())
    }

    /// Latency holds datagrams back until due, loss drops about its share, jitter reorders.
    #[test]
    fn netsim_delays_drops_reorders() {
        let (a, mut b) = mem_pair();
        let mut s = NetSim::new(a, SimNet { latency: 0.1, loss: 0.0, jitter: 0.0 }, 1, now);
        s.send(b"x");
        assert!(b.recv().is_none());
        NOW.with(|n| n.set(0.1));
        s.pump();
        assert_eq!(b.recv().as_deref(), Some(&b"x"[..]));

        let (a, mut b) = mem_pair();
        let mut s = NetSim::new(a, SimNet { latency: 0.0, loss: 0.25, jitter: 0.0 }, 7, now);
        for _ in 0..4000 {
            s.send(b"y");
        }
        let got = std::iter::from_fn(|| b.recv()).count();
        assert!((2800..3200).contains(&got), "{got} of 4000 with 25% loss");

        let (a, mut b) = mem_pair();
        let mut s = NetSim::new(a, SimNet { latency: 0.05, loss: 0.0, jitter: 0.04 }, 3, now);
        for k in 0..100u8 {
            s.send(&[k]);
        }
        NOW.with(|n| n.set(1.0));
        s.pump();
        let got: Vec<u8> = std::iter::from_fn(|| b.recv()).map(|v| v[0]).collect();
        assert_eq!(got.len(), 100);
        let mut sorted = got.clone();
        sorted.sort();
        assert!(got != sorted && sorted == (0..100).collect::<Vec<_>>());

        // dropped: what is queued goes out
        let (a, mut b) = mem_pair();
        let mut s = NetSim::new(a, SimNet { latency: 5.0, loss: 0.0, jitter: 0.0 }, 1, now);
        s.send(b"bye");
        assert!(b.recv().is_none());
        drop(s);
        assert_eq!(b.recv().as_deref(), Some(&b"bye"[..]));
    }
}
