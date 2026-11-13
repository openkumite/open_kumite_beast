//! The netplay packets: layout, `PROTO`, encode and decode.
//!
//! Engine addition.
//!
//! The netplay packets (game-state stages 8 and 9): one datagram each, little-endian, hand-coded. Every packet starts with
//! the same 11 bytes:
//!
//! | bytes | |
//! |---|---|
//! | 4 | `MAGIC` "BR2N" |
//! | 2 | `PROTO` (u16): a peer of another protocol is refused (`proto_of` reads it from any packet) |
//! | 1 | the kind (`Msg`'s order below, from 1) |
//! | 4 | the session's nonce (the joiner's pick; packets of another session are ignored) |
//!
//! then the kind's fields:
//!
//! | kind | fields |
//! |---|---|
//! | 1 Hello (joiner) | sim_version u32, state_version u32, engine (u8 length + UTF-8), buttons 8 × u16, t u32 (ms), extras u8 (`EXTRA_Q3DM17`: the joiner has the extra stage's files), br1 u32 (the joiner's imported-game content id, 0 = none), content u32 (the joiner's game data's content id, 0 = none) |
//! | 2 Welcome (host) | t u32 (the Hello's, echoed: the round trip) |
//! | 3 Sync (joiner) | rtt u16 (ms, the slowest Hello→Welcome), have u32 (the first chunk still missing) |
//! | 4 Chunk (host) | total u32, sum u64 (FNV-1a of the start blob), offset u32, data (u16 length + bytes) |
//! | 5 Ready (joiner) | sum u64 |
//! | 6 Go (host) | — |
//! | 7 Reject | why (u16 length + UTF-8) |
//! | 8 Inputs | at u32 (the sender's next step), adv i16 (its frame advantage, 1/16 frames), ack u32 (the receiver's inputs the sender has: all frames below), first u32, n u8 × (pad u16, done u8), m u8 × (frame u32, hash u64) |
//! | 9 Desync | frame u32, n u8 × (name: u8 length + ASCII, hash u64) |
//! | 10 Bye | why (u16 length + UTF-8) |
//!
//! A peer's bytes are untrusted, like a replay file's: `decode` gives Err (never a panic) for anything but one
//! whole packet of this protocol within the caps (`MAX_PACKET`, `MAX_INPUTS`, `MAX_CHECKS`, `MAX_SECTIONS`, the text
//! lengths), with no byte after it; the receivers check the values against their state (frames, the nonce).

use crate::session::FrameInput;

/// A packet's first bytes (a historical protocol id; `PROTO` versions the layout).
pub const MAGIC: &[u8; 4] = b"BR2N";
/// The packet layout's version (the module doc's tables). 2: Inputs carry the round-trip echo (stage 9). 3: Hello
/// carries the joiner's extra stages (`EXTRA_Q3DM17`). 4: Hello carries the joiner's imported-game content id
/// (`SessionStart::br1`, the import S7). 5: Hello carries the joiner's game data's content id
/// (`SessionStart::content`). A peer of another protocol is another build, refused as before (`proto_of`).
pub const PROTO: u16 = 5;
/// Hello's `extras` bits (`SessionStart::extras`, bit k = `fight::stage::extra::STAGES[k]`): the joiner has the extra
/// stage Q3DM17's files / Q3DM6's (`assets::extras`); the host offers a stage only when both have its files
/// (`SessionStart::extra_stage`, `SessionStart::extra_q3dm6`). Other bits are ignored.
pub const EXTRA_Q3DM17: u8 = 1;
pub const EXTRA_Q3DM6: u8 = 2;
/// Largest datagram sent or accepted (under a typical path MTU: no IP fragments).
pub const MAX_PACKET: usize = 1200;
/// Inputs per Inputs packet: the unacknowledged ones, oldest first (2 × `super::MAX_DELAY` + 1 fit).
pub const MAX_INPUTS: usize = 64;
/// Checkpoint hashes per Inputs packet (the sender's latest).
pub const MAX_CHECKS: usize = 4;
/// Section hashes in a Desync packet (`session::section_hashes` gives about 20) and the longest name.
pub const MAX_SECTIONS: usize = 32;
const MAX_NAME: usize = 24;
/// The longest text of a Hello (the engine version), a Reject or a Bye.
const MAX_ENGINE: usize = 128;
const MAX_WHY: usize = 400;
/// Bytes of the start blob per Chunk.
pub const CHUNK: usize = 1024;
/// Largest start blob (the memory card file is capped at 1 MiB: `replay::check_header`).
pub const MAX_BLOB: usize = (1 << 20) + (64 << 10);
const HEADER: usize = 11;

/// One frame of a peer's input: its pad and what it finished loading by then (`FrameInput::LOADED` /
/// `FrameInput::FETCHED`; the match or the files asked for are in its store).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Input {
    pub pad: u16,
    pub done: u8,
}

/// An Inputs `held` when the sender has no ping of the receiver's to echo yet.
pub const NO_PONG: u16 = u16::MAX;

/// The completions an `Input` may report.
pub const DONE: u8 = FrameInput::LOADED | FrameInput::FETCHED;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Msg {
    Hello { sim_version: u32, state_version: u32, engine: String, buttons: [u16; 8], t: u32, extras: u8, br1: u32, content: u32 },
    Welcome { t: u32 },
    Sync { rtt: u16, have: u32 },
    Chunk { total: u32, sum: u64, offset: u32, data: Vec<u8> },
    Ready { sum: u64 },
    Go,
    Reject { why: String },
    Inputs { at: u32, adv: i16, ack: u32, first: u32, inputs: Vec<Input>, checks: Vec<(u32, u64)>, ping: u16, pong: u16, held: u16 },
    Desync { frame: u32, sections: Vec<(String, u64)> },
    Bye { why: String },
}

impl Msg {
    fn kind(&self) -> u8 {
        match self {
            Msg::Hello { .. } => 1,
            Msg::Welcome { .. } => 2,
            Msg::Sync { .. } => 3,
            Msg::Chunk { .. } => 4,
            Msg::Ready { .. } => 5,
            Msg::Go => 6,
            Msg::Reject { .. } => 7,
            Msg::Inputs { .. } => 8,
            Msg::Desync { .. } => 9,
            Msg::Bye { .. } => 10,
        }
    }
}

/// A packet: the session's nonce and the message.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Packet {
    pub nonce: u32,
    pub msg: Msg,
}

/// The packet's bytes. The caller keeps within the caps (a longer list or text is cut to them; a chunk to `CHUNK`).
pub fn encode(p: &Packet) -> Vec<u8> {
    let mut b = Vec::with_capacity(64);
    b.extend_from_slice(MAGIC);
    b.extend_from_slice(&PROTO.to_le_bytes());
    b.push(p.msg.kind());
    b.extend_from_slice(&p.nonce.to_le_bytes());
    let text = |b: &mut Vec<u8>, s: &str, max: usize, wide: bool| {
        let s = &s.as_bytes()[..s.floor_char_boundary(max)];
        if wide {
            b.extend_from_slice(&(s.len() as u16).to_le_bytes());
        } else {
            b.push(s.len() as u8);
        }
        b.extend_from_slice(s);
    };
    match &p.msg {
        Msg::Hello { sim_version, state_version, engine, buttons, t, extras, br1, content } => {
            b.extend_from_slice(&sim_version.to_le_bytes());
            b.extend_from_slice(&state_version.to_le_bytes());
            text(&mut b, engine, MAX_ENGINE, false);
            for x in buttons {
                b.extend_from_slice(&x.to_le_bytes());
            }
            b.extend_from_slice(&t.to_le_bytes());
            b.push(*extras);
            b.extend_from_slice(&br1.to_le_bytes());
            b.extend_from_slice(&content.to_le_bytes());
        }
        Msg::Welcome { t } => b.extend_from_slice(&t.to_le_bytes()),
        Msg::Sync { rtt, have } => {
            b.extend_from_slice(&rtt.to_le_bytes());
            b.extend_from_slice(&have.to_le_bytes());
        }
        Msg::Chunk { total, sum, offset, data } => {
            b.extend_from_slice(&total.to_le_bytes());
            b.extend_from_slice(&sum.to_le_bytes());
            b.extend_from_slice(&offset.to_le_bytes());
            let data = &data[..data.len().min(CHUNK)];
            b.extend_from_slice(&(data.len() as u16).to_le_bytes());
            b.extend_from_slice(data);
        }
        Msg::Ready { sum } => b.extend_from_slice(&sum.to_le_bytes()),
        Msg::Go => {}
        Msg::Reject { why } | Msg::Bye { why } => text(&mut b, why, MAX_WHY, true),
        Msg::Inputs { at, adv, ack, first, inputs, checks, ping, pong, held } => {
            b.extend_from_slice(&at.to_le_bytes());
            b.extend_from_slice(&adv.to_le_bytes());
            b.extend_from_slice(&ack.to_le_bytes());
            b.extend_from_slice(&first.to_le_bytes());
            let inputs = &inputs[..inputs.len().min(MAX_INPUTS)];
            b.push(inputs.len() as u8);
            for i in inputs {
                b.extend_from_slice(&i.pad.to_le_bytes());
                b.push(i.done);
            }
            let checks = &checks[checks.len().saturating_sub(MAX_CHECKS)..];
            b.push(checks.len() as u8);
            for (f, h) in checks {
                b.extend_from_slice(&f.to_le_bytes());
                b.extend_from_slice(&h.to_le_bytes());
            }
            for x in [ping, pong, held] {
                b.extend_from_slice(&x.to_le_bytes());
            }
        }
        Msg::Desync { frame, sections } => {
            b.extend_from_slice(&frame.to_le_bytes());
            let sections = &sections[..sections.len().min(MAX_SECTIONS)];
            b.push(sections.len() as u8);
            for (name, h) in sections {
                text(&mut b, name, MAX_NAME, false);
                b.extend_from_slice(&h.to_le_bytes());
            }
        }
    }
    debug_assert!(b.len() <= MAX_PACKET, "a {} byte packet", b.len());
    b
}

/// The protocol version of anything that starts like a packet (a peer of another build: its version for the
/// refusal message).
pub fn proto_of(b: &[u8]) -> Option<u16> {
    (b.len() >= HEADER && &b[..4] == MAGIC).then(|| u16::from_le_bytes([b[4], b[5]]))
}

/// A datagram's packet: Err for anything but one whole packet of this protocol within the caps.
pub fn decode(b: &[u8]) -> Result<Packet, String> {
    if b.len() > MAX_PACKET {
        return Err(format!("a packet of {} bytes", b.len()));
    }
    match proto_of(b) {
        None => return Err("not a netplay packet".into()),
        Some(PROTO) => {}
        Some(v) => return Err(format!("protocol version {v}, this build speaks {PROTO}")),
    }
    let mut r = Reader { b, at: 6 };
    let kind = r.u8()?;
    let nonce = r.u32()?;
    let msg = match kind {
        1 => {
            let (sim_version, state_version) = (r.u32()?, r.u32()?);
            let engine = r.text(MAX_ENGINE, false)?;
            let mut buttons = [0u16; 8];
            for x in &mut buttons {
                *x = r.u16()?;
            }
            Msg::Hello { sim_version, state_version, engine, buttons, t: r.u32()?, extras: r.u8()?, br1: r.u32()?, content: r.u32()? }
        }
        2 => Msg::Welcome { t: r.u32()? },
        3 => Msg::Sync { rtt: r.u16()?, have: r.u32()? },
        4 => {
            let (total, sum, offset) = (r.u32()?, r.u64()?, r.u32()?);
            let n = r.u16()? as usize;
            if n == 0 || n > CHUNK || total as usize > MAX_BLOB || offset as u64 + n as u64 > total as u64 {
                return Err(format!("chunk {offset}+{n} of {total}"));
            }
            Msg::Chunk { total, sum, offset, data: r.bytes(n)?.to_vec() }
        }
        5 => Msg::Ready { sum: r.u64()? },
        6 => Msg::Go,
        7 => Msg::Reject { why: r.text(MAX_WHY, true)? },
        8 => {
            let (at, adv, ack, first) = (r.u32()?, r.u16()? as i16, r.u32()?, r.u32()?);
            let n = r.u8()? as usize;
            if n > MAX_INPUTS || first.checked_add(n as u32).is_none() {
                return Err(format!("{n} inputs from frame {first}"));
            }
            let mut inputs = Vec::with_capacity(n);
            for _ in 0..n {
                let (pad, done) = (r.u16()?, r.u8()?);
                if done & !DONE != 0 {
                    return Err(format!("input completions {done:#x}"));
                }
                inputs.push(Input { pad, done });
            }
            let m = r.u8()? as usize;
            if m > MAX_CHECKS {
                return Err(format!("{m} checkpoints"));
            }
            let mut checks = Vec::with_capacity(m);
            for _ in 0..m {
                checks.push((r.u32()?, r.u64()?));
            }
            Msg::Inputs { at, adv, ack, first, inputs, checks, ping: r.u16()?, pong: r.u16()?, held: r.u16()? }
        }
        9 => {
            let frame = r.u32()?;
            let n = r.u8()? as usize;
            if n > MAX_SECTIONS {
                return Err(format!("{n} sections"));
            }
            let mut sections = Vec::with_capacity(n);
            for _ in 0..n {
                let name = r.text(MAX_NAME, false)?;
                if !name.bytes().all(|c| c.is_ascii_graphic()) {
                    return Err("a section name that is not plain ASCII".into());
                }
                sections.push((name, r.u64()?));
            }
            Msg::Desync { frame, sections }
        }
        10 => Msg::Bye { why: r.text(MAX_WHY, true)? },
        k => return Err(format!("packet kind {k}")),
    };
    if r.at != b.len() {
        return Err(format!("{} bytes after the packet", b.len() - r.at));
    }
    Ok(Packet { nonce, msg })
}

/// Bounds-checked little-endian reads.
struct Reader<'a> {
    b: &'a [u8],
    at: usize,
}

impl<'a> Reader<'a> {
    fn bytes(&mut self, n: usize) -> Result<&'a [u8], String> {
        let x = self.b.get(self.at..self.at.saturating_add(n)).ok_or("the packet is truncated")?;
        self.at += n;
        Ok(x)
    }

    fn u8(&mut self) -> Result<u8, String> {
        Ok(self.bytes(1)?[0])
    }

    fn u16(&mut self) -> Result<u16, String> {
        let b = self.bytes(2)?;
        Ok(u16::from_le_bytes([b[0], b[1]]))
    }

    fn u32(&mut self) -> Result<u32, String> {
        let b = self.bytes(4)?;
        Ok(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    }

    fn u64(&mut self) -> Result<u64, String> {
        let b = self.bytes(8)?;
        Ok(u64::from_le_bytes(b.try_into().expect("8 bytes")))
    }

    /// A length-prefixed UTF-8 text of at most `max` bytes (`wide`: a u16 length, else u8).
    fn text(&mut self, max: usize, wide: bool) -> Result<String, String> {
        let n = if wide { self.u16()? as usize } else { self.u8()? as usize };
        if n > max {
            return Err(format!("a text of {n} bytes"));
        }
        std::str::from_utf8(self.bytes(n)?).map(str::to_string).map_err(|_| "a text that is not UTF-8".into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn samples() -> Vec<Packet> {
        let msgs = vec![
            Msg::Hello {
                sim_version: 3,
                state_version: 3,
                engine: "0.1.0 (abc+)".into(),
                buttons: crate::assets::game_tables::default_buttons(),
                t: 12345,
                extras: 1,
                br1: 0xBEEF,
                content: 0x5970_AF0C,
            },
            Msg::Welcome { t: 99 },
            Msg::Sync { rtt: 160, have: 3 },
            Msg::Chunk { total: 3000, sum: 0x1234_5678_9ABC_DEF0, offset: 2048, data: vec![7; 952] },
            Msg::Ready { sum: 42 },
            Msg::Go,
            Msg::Reject { why: "version mismatch: ünïcode".into() },
            Msg::Inputs {
                at: 600,
                adv: -37,
                ack: 590,
                first: 595,
                inputs: (0..MAX_INPUTS as u16).map(|k| Input { pad: k * 0x111, done: if k == 3 { DONE } else { 0 } }).collect(),
                checks: vec![(480, 1), (540, u64::MAX), (600, 3), (660, 4)],
                ping: 0xFFFE,
                pong: 3,
                held: NO_PONG,
            },
            Msg::Desync { frame: 600, sections: (0..MAX_SECTIONS).map(|k| (format!("fighters[{k}]"), k as u64)).collect() },
            Msg::Bye { why: "quit".into() },
        ];
        msgs.into_iter().enumerate().map(|(k, msg)| Packet { nonce: 0xA5A5_0000 + k as u32, msg }).collect()
    }

    /// Every kind encodes within `MAX_PACKET` and decodes back to itself.
    #[test]
    fn round_trip() {
        for p in samples() {
            let b = encode(&p);
            assert!(b.len() <= MAX_PACKET, "{:?}: {} bytes", p.msg, b.len());
            assert_eq!(decode(&b).as_ref(), Ok(&p));
            assert_eq!(proto_of(&b), Some(PROTO));
        }
    }

    /// Garbage gives Err, never a panic: every truncation and every single-byte change of each kind (a change
    /// that still decodes must re-encode to the same bytes), a trailing byte, another protocol, random datagrams
    /// of every length up to past `MAX_PACKET`, and crafted counts and lengths past the caps.
    #[test]
    fn garbage_rejected() {
        for p in samples() {
            let b = encode(&p);
            for k in 0..b.len() {
                assert!(decode(&b[..k]).is_err(), "{:?} truncated at {k}", p.msg);
                for x in [0x01, 0x80, 0xFF] {
                    let mut c = b.clone();
                    c[k] ^= x;
                    if let Ok(q) = decode(&c) {
                        assert_eq!(encode(&q), c, "byte {k} ^ {x:#x} of {:?}", p.msg);
                    }
                }
            }
            let mut tail = b.clone();
            tail.push(0);
            assert!(decode(&tail).is_err());
            let mut other = b.clone();
            other[4..6].copy_from_slice(&(PROTO + 1).to_le_bytes());
            assert!(decode(&other).is_err_and(|e| e.contains("protocol version")));
            assert_eq!(proto_of(&other), Some(PROTO + 1));
        }
        // random bytes, with and without a valid header
        let mut x = 0x9E37_79B9_7F4A_7C15u64;
        let mut rnd = || {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            x
        };
        for n in 0..MAX_PACKET + 64 {
            for header in [false, true] {
                let mut b: Vec<u8> = (0..n).map(|_| rnd() as u8).collect();
                if header && n >= HEADER {
                    b[..4].copy_from_slice(MAGIC);
                    b[4..6].copy_from_slice(&PROTO.to_le_bytes());
                    b[6] = (rnd() % 12) as u8;
                }
                if let Ok(p) = decode(&b) {
                    assert_eq!(encode(&p), b);
                }
            }
        }
        // counts and lengths past the caps
        let head = |kind: u8| {
            let mut b = MAGIC.to_vec();
            b.extend_from_slice(&PROTO.to_le_bytes());
            b.push(kind);
            b.extend_from_slice(&7u32.to_le_bytes());
            b
        };
        let mut b = head(8);
        b.extend_from_slice(&[0; 14]);
        b.push(MAX_INPUTS as u8 + 1);
        b.extend(std::iter::repeat_n(0, 3 * (MAX_INPUTS + 1) + 1));
        assert!(decode(&b).is_err());
        let mut b = head(8);
        b.extend_from_slice(&[0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0xFF, 0xFF, 0xFF, 0xFF]);
        b.push(2);
        b.extend_from_slice(&[0; 7]);
        assert!(decode(&b).is_err(), "frames past u32");
        let mut b = head(8);
        b.extend_from_slice(&[0; 14]);
        b.extend_from_slice(&[1, 0, 0, 0x40, 0]);
        assert!(decode(&b).is_err_and(|e| e.contains("completions")));
        let mut b = head(4);
        b.extend_from_slice(&100u32.to_le_bytes());
        b.extend_from_slice(&[0; 8]);
        b.extend_from_slice(&90u32.to_le_bytes());
        b.extend_from_slice(&20u16.to_le_bytes());
        b.extend_from_slice(&[0; 20]);
        assert!(decode(&b).is_err_and(|e| e.contains("chunk")), "a chunk past the blob's end");
        let mut b = head(7);
        b.extend_from_slice(&(MAX_WHY as u16 + 1).to_le_bytes());
        b.extend(std::iter::repeat_n(b'a', MAX_WHY + 1));
        assert!(decode(&b).is_err());
        let mut b = head(7);
        b.extend_from_slice(&2u16.to_le_bytes());
        b.extend_from_slice(&[0xC3, 0x28]);
        assert!(decode(&b).is_err_and(|e| e.contains("UTF-8")));
        assert!(decode(&head(11)).is_err_and(|e| e.contains("kind")));
        assert!(decode(&vec![0; MAX_PACKET + 1]).is_err());
    }
}
