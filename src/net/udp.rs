//! UDP netplay transport (desktop).
//!
//! Engine addition.
//!
//! UDP netplay (desktop, game-state stage 8): a non-blocking `std::net::UdpSocket` as a `Transport` (up at once:
//! `super::Connect` runs the handshake over it from the frame loop). Only the addresses given are used:
//! the host binds the port it was told and answers the first joiner whose Hello it accepts (anyone else is ignored
//! from then on); the joiner talks to the host's address only. No discovery, no NAT traversal.

use super::link::{NetSim, SimNet, Transport};
use super::proto::MAX_PACKET;
use std::net::{SocketAddr, ToSocketAddrs, UdpSocket};

/// A UDP socket as a `Transport`.
pub struct Udp {
    sock: UdpSocket,
    peer: Option<SocketAddr>,
    locked: bool,
    buf: Vec<u8>,
}

impl Udp {
    /// A host's socket on `bind` (its first sender becomes the peer, until `lock_peer`).
    pub fn host(bind: SocketAddr) -> Result<Self, String> {
        let sock = UdpSocket::bind(bind).map_err(|e| format!("cannot listen on {bind}: {e}"))?;
        Self::new(sock, None)
    }

    /// A joiner's socket talking to `peer` only (bound to the loopback interface when the host is on it).
    pub fn join(peer: SocketAddr) -> Result<Self, String> {
        let local: SocketAddr = match (peer.is_ipv4(), peer.ip().is_loopback()) {
            (true, true) => "127.0.0.1:0",
            (true, false) => "0.0.0.0:0",
            (false, true) => "[::1]:0",
            (false, false) => "[::]:0",
        }
        .parse()
        .expect("an address");
        let sock = UdpSocket::bind(local).map_err(|e| format!("cannot open a UDP socket: {e}"))?;
        let mut u = Self::new(sock, Some(peer))?;
        u.locked = true;
        Ok(u)
    }

    fn new(sock: UdpSocket, peer: Option<SocketAddr>) -> Result<Self, String> {
        sock.set_nonblocking(true).map_err(|e| e.to_string())?;
        Ok(Self { sock, peer, locked: false, buf: vec![0; MAX_PACKET + 1] })
    }

    /// The address the socket is bound to.
    pub fn local_addr(&self) -> Option<SocketAddr> {
        self.sock.local_addr().ok()
    }
}

impl Transport for Udp {
    fn send(&mut self, b: &[u8]) {
        if let Some(p) = self.peer {
            // a full send buffer or an unreachable peer: the datagram is lost, as any other
            let _ = self.sock.send_to(b, p);
        }
    }

    fn recv(&mut self) -> Option<Vec<u8>> {
        // errors (an ICMP "port unreachable" reported on a later call, on some systems) skip; a bounded number
        for _ in 0..64 {
            match self.sock.recv_from(&mut self.buf) {
                Ok((n, from)) => {
                    if self.locked && Some(from) != self.peer {
                        continue;
                    }
                    // one larger than any packet: cut off, not one
                    if n > MAX_PACKET {
                        continue;
                    }
                    if !self.locked {
                        self.peer = Some(from);
                    }
                    return Some(self.buf[..n].to_vec());
                }
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => return None,
                Err(_) => continue,
            }
        }
        None
    }

    fn lock_peer(&mut self) {
        self.locked = self.peer.is_some();
    }

    fn unlock_peer(&mut self) {
        self.locked = false;
        self.peer = None;
    }

    fn peer(&self) -> String {
        self.peer.map_or_else(String::new, |p| p.to_string())
    }
}

/// `--host [ADDR:]PORT` (ADDR defaults to every interface) or `--join ADDR:PORT`.
#[derive(Clone, Debug, PartialEq)]
pub enum Role {
    Host(SocketAddr),
    Join(SocketAddr),
}

impl Role {
    pub fn host(arg: &str) -> Result<Self, String> {
        let a = if arg.bytes().all(|c| c.is_ascii_digit()) { format!("0.0.0.0:{arg}") } else { arg.to_string() };
        let a: SocketAddr = a.parse().map_err(|_| format!("--host {arg}: a port, or address:port"))?;
        if a.port() == 0 {
            return Err(format!("--host {arg}: port 0"));
        }
        Ok(Role::Host(a))
    }

    /// An IPv4 address of the name first (a bare-port `--host` listens on IPv4 only; "localhost" resolves to ::1
    /// first on some systems).
    pub fn join(arg: &str) -> Result<Self, String> {
        let all: Vec<SocketAddr> = arg.to_socket_addrs().map_err(|e| format!("--join {arg}: {e}"))?.collect();
        let a = all.iter().find(|a| a.is_ipv4()).or(all.first()).copied().ok_or_else(|| format!("--join {arg}: no address"))?;
        if a.port() == 0 || a.ip().is_unspecified() {
            return Err(format!("--join {arg}: not a host's address"));
        }
        Ok(Role::Join(a))
    }
}

/// This machine's address on its network (the interface of its route to a private address: a UDP socket only
/// "connected" there, which sends nothing), for a host to tell the other player; None without a network.
pub fn lan_address() -> Option<std::net::IpAddr> {
    let s = UdpSocket::bind("0.0.0.0:0").ok()?;
    s.connect("10.255.255.255:9").ok()?;
    let ip = s.local_addr().ok()?.ip();
    (!ip.is_unspecified() && !ip.is_loopback()).then_some(ip)
}

/// The link of `role`, wrapped in OKB_NETSIM's simulated network when `sim` is set.
pub fn open(role: &Role, sim: Option<SimNet>) -> Result<Box<dyn Transport + Send>, String> {
    let udp = match role {
        Role::Host(a) => Udp::host(*a)?,
        Role::Join(a) => Udp::join(*a)?,
    };
    Ok(match sim {
        Some(cfg) => {
            log::info!("netplay: simulated network {cfg:?}");
            Box::new(NetSim::new(udp, cfg, crate::clock_seed() ^ udp_seed(role), crate::perf_now))
        }
        None => Box::new(udp),
    })
}

/// The two ends draw different losses.
fn udp_seed(role: &Role) -> u64 {
    match role {
        Role::Host(_) => 0x1234_5678,
        Role::Join(_) => 0x8765_4321_0000,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roles() {
        assert_eq!(Role::host("7000"), Ok(Role::Host("0.0.0.0:7000".parse().unwrap())));
        assert_eq!(Role::host("127.0.0.1:7000"), Ok(Role::Host("127.0.0.1:7000".parse().unwrap())));
        assert!(Role::host("0").is_err() && Role::host("x").is_err() && Role::host("99999").is_err());
        assert_eq!(Role::join("127.0.0.1:7000"), Ok(Role::Join("127.0.0.1:7000".parse().unwrap())));
        assert_eq!(Role::join("localhost:7000"), Ok(Role::Join("127.0.0.1:7000".parse().unwrap())));
        assert_eq!(Role::join("[::1]:7000"), Ok(Role::Join("[::1]:7000".parse().unwrap())));
        assert!(Role::join("0.0.0.0:7000").is_err() && Role::join("127.0.0.1:0").is_err() && Role::join("7000").is_err());
    }

    /// Two sockets on 127.0.0.1: the joiner reaches the host, the host answers it; a third socket's datagrams
    /// are dropped once the host locked onto the joiner.
    #[test]
    fn loopback_and_lock() {
        let mut h = Udp::host("127.0.0.1:0".parse().unwrap()).unwrap();
        let addr = h.local_addr().unwrap();
        let mut j = Udp::join(addr).unwrap();
        let wait = |t: &mut Udp| {
            for _ in 0..500 {
                if let Some(b) = t.recv() {
                    return Some(b);
                }
                std::thread::sleep(std::time::Duration::from_millis(2));
            }
            None
        };
        j.send(b"hello");
        assert_eq!(wait(&mut h).as_deref(), Some(&b"hello"[..]));
        h.lock_peer();
        h.send(b"welcome");
        assert_eq!(wait(&mut j).as_deref(), Some(&b"welcome"[..]));
        let mut other = Udp::join(addr).unwrap();
        other.send(b"intruder");
        j.send(b"again");
        assert_eq!(wait(&mut h).as_deref(), Some(&b"again"[..]));
        assert!(wait(&mut h).is_none());
        // unlocked (the joiner went before Ready): the next sender is the peer
        h.unlock_peer();
        other.send(b"second");
        assert_eq!(wait(&mut h).as_deref(), Some(&b"second"[..]));
        h.lock_peer();
        // an oversized datagram is dropped
        other.send(&vec![0; MAX_PACKET + 1]);
        other.send(b"small");
        assert_eq!(wait(&mut h).as_deref(), Some(&b"small"[..]));
    }
}
