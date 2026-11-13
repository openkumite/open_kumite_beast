//! The replay's input stream: every step's `FrameInput` as runs of LEB128 varints.
//!
//! Engine addition.
//! State: the replay file's format (`FORMAT`)
//!
//! The input stream: every step's `FrameInput` as runs of LEB128 varints (the module doc's layout); the recorder's
//! `InputWriter`, the reader `read_run`. FORMAT 1 has five fields (pads 1 / 2, forced 1 / 2, flags) and a mask byte;
//! FORMAT 2 (the team battle, engine addition) nine (pads 3 / 4 and forced 3 / 4 after them) and a u16 mask.

use crate::session::FrameInput;

/// The fields of FORMAT 2 in their order (FORMAT 1: the first five).
const WIDE: usize = 9;

fn fields(i: &FrameInput) -> [u16; WIDE] {
    [i.pad[0], i.pad[1], i.forced[0], i.forced[1], i.flags as u16, i.ext[0], i.ext[1], i.forced_ext[0], i.forced_ext[1]]
}

fn put_varint(b: &mut Vec<u8>, mut v: u32) {
    loop {
        let x = (v & 0x7F) as u8;
        v >>= 7;
        if v == 0 {
            b.push(x);
            return;
        }
        b.push(x | 0x80);
    }
}

fn get_varint(b: &[u8], at: &mut usize) -> Result<u32, String> {
    let mut v = 0u64;
    for k in 0..5 {
        let x = *b.get(*at).ok_or("the input stream ends inside a number")?;
        *at += 1;
        v |= ((x & 0x7F) as u64) << (7 * k);
        if x & 0x80 == 0 {
            return u32::try_from(v).map_err(|_| "a number of the input stream is too large".into());
        }
    }
    Err("a number of the input stream is too long".into())
}

/// A run of `n` steps of input `i` after one of `prev`; `wide`: FORMAT 2 (a u16 mask: bits 0..8 the fields, bits
/// 9..11 the length code), else FORMAT 1 (a mask byte: bits 0..4 the fields, 5..7 the length code).
fn put_run(b: &mut Vec<u8>, prev: &FrameInput, i: &FrameInput, n: u32, wide: bool) {
    let (p, f) = (fields(prev), fields(i));
    let count = if wide { WIDE } else { 5 };
    let mask = (0..count).filter(|&k| p[k] != f[k]).fold(0u16, |m, k| m | 1 << k);
    let code = n.clamp(1, 8) as u16 - 1;
    if wide {
        b.extend_from_slice(&(mask | code << WIDE).to_le_bytes());
    } else {
        b.push(mask as u8 | (code as u8) << 5);
    }
    if n >= 8 {
        put_varint(b, n - 8);
    }
    for k in (0..count).filter(|&k| mask >> k & 1 != 0) {
        put_varint(b, f[k] as u32);
    }
}

/// The run at `at` (its input into `cur`, which holds the one before); its length, None at the stream's end. `wide`:
/// a FORMAT 2 stream.
pub(super) fn read_run(b: &[u8], at: &mut usize, cur: &mut FrameInput, wide: bool) -> Result<Option<u32>, String> {
    let Some(&m0) = b.get(*at) else { return Ok(None) };
    *at += 1;
    let (mask, code, count) = if wide {
        let m1 = *b.get(*at).ok_or("the input stream ends inside a run")?;
        *at += 1;
        let m = u16::from_le_bytes([m0, m1]);
        if m >> 12 != 0 {
            return Err(format!("input run {m:#x}"));
        }
        (m & 0x1FF, (m >> WIDE) as u32, WIDE)
    } else {
        ((m0 & 0x1F) as u16, (m0 >> 5) as u32, 5)
    };
    let n = if code < 7 { code + 1 } else { get_varint(b, at)?.checked_add(8).ok_or("a run of the input stream is too long")? };
    let mut f = fields(cur);
    for k in (0..count).filter(|&k| mask >> k & 1 != 0) {
        f[k] = u16::try_from(get_varint(b, at)?).map_err(|_| "an input of the stream is too large")?;
    }
    let all = if wide { (FrameInput::ALL | FrameInput::FORCED_EXT[0] | FrameInput::FORCED_EXT[1]) as u16 } else { FrameInput::ALL as u16 };
    if f[4] & !all != 0 {
        return Err(format!("input flags {:#x}", f[4]));
    }
    *cur = FrameInput { pad: [f[0], f[1]], forced: [f[2], f[3]], flags: f[4] as u8, ext: [f[5], f[6]], forced_ext: [f[7], f[8]] };
    Ok(Some(n))
}

/// The input stream as it is recorded: the runs written so far and the one still growing, in both formats (the
/// file a match is saved to takes FORMAT 2 only when it needs it: `uses_ext`).
#[derive(Default)]
pub(super) struct InputWriter {
    buf: Vec<u8>,
    wide: Vec<u8>,
    prev: FrameInput,
    run: Option<(FrameInput, u32)>,
    /// a step used pads 3 / 4 (`FrameInput::uses_ext`)
    pub(super) uses_ext: bool,
}

impl InputWriter {
    pub(super) fn push(&mut self, i: FrameInput) {
        self.uses_ext |= i.uses_ext();
        match &mut self.run {
            Some((x, n)) if *x == i => *n += 1,
            _ => {
                if let Some((x, n)) = self.run.replace((i, 1)) {
                    put_run(&mut self.buf, &self.prev, &x, n, false);
                    put_run(&mut self.wide, &self.prev, &x, n, true);
                    self.prev = x;
                }
            }
        }
    }

    /// The stream in FORMAT 2 (`wide`) or 1.
    pub(super) fn bytes(&self, wide: bool) -> Vec<u8> {
        let mut b = if wide { self.wide.clone() } else { self.buf.clone() };
        if let Some((x, n)) = self.run {
            put_run(&mut b, &self.prev, &x, n, wide);
        }
        b
    }
}
