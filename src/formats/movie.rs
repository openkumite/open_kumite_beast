//! FMV decoding: the BS (version 2) movie frames, as DecDCTvlc + MDEC do it.
//!
//! FMV playback: the BS (version 2) bitstream frames of the /MOV/*.STR movies decoded the way the game's
//! DecDCTvlc + MDEC pair does it — per 16×16 macroblock (column-major) the blocks Cr, Cb, Y1..Y4, each a 10-bit
//! DC then MPEG-1 table B.14 run/level codes ('10' = end, '000001' + 6-bit run + 10-bit level = escape),
//! dequantised with the MDEC intra matrix × the frame's qscale, inverse DCT'd and converted to 15-bit RGB.
//! Frames come from `video/<NAME>.bsf` (ok_br2_converter::convert), with their disc sector for timing.

use std::sync::OnceLock;

/// The MDEC luma/chroma quantisation matrix (row-major).
const IQ: [u16; 64] = [
    2, 16, 19, 22, 26, 27, 29, 34, 16, 16, 22, 24, 27, 29, 34, 37, 19, 22, 26, 27, 29, 34, 34, 38, 22, 22, 26, 27, 29, 34, 37, 40, 22, 26, 27, 29, 32, 35, 40,
    48, 26, 27, 29, 32, 35, 40, 48, 58, 26, 27, 29, 34, 38, 46, 56, 69, 27, 29, 35, 38, 46, 56, 69, 83,
];

const ZIGZAG: [u8; 64] = [
    0, 1, 8, 16, 9, 2, 3, 10, 17, 24, 32, 25, 18, 11, 4, 5, 12, 19, 26, 33, 40, 48, 41, 34, 27, 20, 13, 6, 7, 14, 21, 28, 35, 42, 49, 56, 57, 50, 43, 36, 29,
    22, 15, 23, 30, 37, 44, 51, 58, 59, 52, 45, 38, 31, 39, 46, 53, 60, 61, 54, 47, 55, 62, 63,
];

/// MPEG-1 table B.14 (coefficients after the first), sign bit excluded: (code, run, level).
const AC: &[(&str, u8, u8)] = &[
    ("11", 0, 1),
    ("011", 1, 1),
    ("0100", 0, 2),
    ("0101", 2, 1),
    ("00101", 0, 3),
    ("00111", 3, 1),
    ("00110", 4, 1),
    ("000110", 1, 2),
    ("000111", 5, 1),
    ("000101", 6, 1),
    ("000100", 7, 1),
    ("0000110", 0, 4),
    ("0000100", 2, 2),
    ("0000111", 8, 1),
    ("0000101", 9, 1),
    ("00100110", 0, 5),
    ("00100001", 0, 6),
    ("00100101", 1, 3),
    ("00100100", 3, 2),
    ("00100111", 10, 1),
    ("00100011", 11, 1),
    ("00100010", 12, 1),
    ("00100000", 13, 1),
    ("0000001010", 0, 7),
    ("0000001100", 1, 4),
    ("0000001011", 2, 3),
    ("0000001111", 4, 2),
    ("0000001001", 5, 2),
    ("0000001110", 14, 1),
    ("0000001101", 15, 1),
    ("0000001000", 16, 1),
    ("000000011101", 0, 8),
    ("000000011000", 0, 9),
    ("000000010011", 0, 10),
    ("000000010000", 0, 11),
    ("000000011011", 1, 5),
    ("000000010100", 2, 4),
    ("000000011100", 3, 3),
    ("000000010010", 4, 3),
    ("000000011110", 6, 2),
    ("000000010101", 7, 2),
    ("000000010001", 8, 2),
    ("000000011111", 17, 1),
    ("000000011010", 18, 1),
    ("000000011001", 19, 1),
    ("000000010111", 20, 1),
    ("000000010110", 21, 1),
    ("0000000011010", 0, 12),
    ("0000000011001", 0, 13),
    ("0000000011000", 0, 14),
    ("0000000010111", 0, 15),
    ("0000000010110", 1, 6),
    ("0000000010101", 1, 7),
    ("0000000010100", 2, 5),
    ("0000000010011", 3, 4),
    ("0000000010010", 5, 3),
    ("0000000010001", 9, 2),
    ("0000000010000", 10, 2),
    ("0000000011111", 22, 1),
    ("0000000011110", 23, 1),
    ("0000000011101", 24, 1),
    ("0000000011100", 25, 1),
    ("0000000011011", 26, 1),
    ("00000000011111", 0, 16),
    ("00000000011110", 0, 17),
    ("00000000011101", 0, 18),
    ("00000000011100", 0, 19),
    ("00000000011011", 0, 20),
    ("00000000011010", 0, 21),
    ("00000000011001", 0, 22),
    ("00000000011000", 0, 23),
    ("00000000010111", 0, 24),
    ("00000000010110", 0, 25),
    ("00000000010101", 0, 26),
    ("00000000010100", 0, 27),
    ("00000000010011", 0, 28),
    ("00000000010010", 0, 29),
    ("00000000010001", 0, 30),
    ("00000000010000", 0, 31),
    ("000000000011000", 0, 32),
    ("000000000010111", 0, 33),
    ("000000000010110", 0, 34),
    ("000000000010101", 0, 35),
    ("000000000010100", 0, 36),
    ("000000000010011", 0, 37),
    ("000000000010010", 0, 38),
    ("000000000010001", 0, 39),
    ("000000000010000", 0, 40),
    ("000000000011111", 1, 8),
    ("000000000011110", 1, 9),
    ("000000000011101", 1, 10),
    ("000000000011100", 1, 11),
    ("000000000011011", 1, 12),
    ("000000000011010", 1, 13),
    ("000000000011001", 1, 14),
    ("0000000000010011", 1, 15),
    ("0000000000010010", 1, 16),
    ("0000000000010001", 1, 17),
    ("0000000000010000", 1, 18),
    ("0000000000010100", 6, 3),
    ("0000000000011010", 11, 2),
    ("0000000000011001", 12, 2),
    ("0000000000011000", 13, 2),
    ("0000000000010111", 14, 2),
    ("0000000000010110", 15, 2),
    ("0000000000010101", 16, 2),
    ("0000000000011111", 27, 1),
    ("0000000000011110", 28, 1),
    ("0000000000011101", 29, 1),
    ("0000000000011100", 30, 1),
    ("0000000000011011", 31, 1),
];

const EOB: u16 = 0xFFFF;
const ESC: u16 = 0xFFFE;

/// Lookup of the next 16 bits: (code length, run, level) packed; EOB / escape marked in the level.
fn table() -> &'static [(u8, u8, u16)] {
    static T: OnceLock<Vec<(u8, u8, u16)>> = OnceLock::new();
    T.get_or_init(|| {
        let mut t = vec![(0u8, 0u8, 0u16); 1 << 16];
        let mut put = |code: &str, run: u8, level: u16| {
            let n = code.len();
            let v = u32::from_str_radix(code, 2).unwrap() << (16 - n);
            for k in 0..1u32 << (16 - n) {
                t[(v | k) as usize] = (n as u8, run, level);
            }
        };
        put("10", 0, EOB);
        put("000001", 0, ESC);
        for &(c, r, l) in AC {
            put(c, r, l as u16);
        }
        t
    })
}

struct Bits<'a> {
    d: &'a [u8],
    pos: usize,
}

impl Bits<'_> {
    /// The stream is read as little-endian halfwords, most significant bit first.
    fn bit_at(&self, p: usize) -> u32 {
        let w = p / 16;
        let h = self.d.get(2 * w).copied().unwrap_or(0) as u32 | (self.d.get(2 * w + 1).copied().unwrap_or(0) as u32) << 8;
        (h >> (15 - p % 16)) & 1
    }
    fn peek(&self, n: usize) -> u32 {
        (0..n).fold(0, |a, k| a << 1 | self.bit_at(self.pos + k))
    }
    fn get(&mut self, n: usize) -> u32 {
        let v = self.peek(n);
        self.pos += n;
        v
    }
    fn sget(&mut self, n: usize) -> i32 {
        let v = self.get(n) as i32;
        (v << (32 - n)) >> (32 - n)
    }
}

fn idct_basis() -> &'static [[f32; 8]; 8] {
    static B: OnceLock<[[f32; 8]; 8]> = OnceLock::new();
    B.get_or_init(|| {
        let mut b = [[0.0; 8]; 8];
        for (x, row) in b.iter_mut().enumerate() {
            for (u, v) in row.iter_mut().enumerate() {
                let c = if u == 0 { std::f32::consts::FRAC_1_SQRT_2 } else { 1.0 };
                *v = 0.5 * c * (((2 * x + 1) as f32 * u as f32 * std::f32::consts::PI) / 16.0).cos();
            }
        }
        b
    })
}

fn idct(blk: &[i32; 64], out: &mut [f32; 64]) {
    let b = idct_basis();
    let mut tmp = [0.0f32; 64];
    for y in 0..8 {
        for x in 0..8 {
            tmp[y * 8 + x] = (0..8).map(|u| b[x][u] * blk[y * 8 + u] as f32).sum();
        }
    }
    for x in 0..8 {
        for y in 0..8 {
            out[y * 8 + x] = (0..8).map(|v| b[y][v] * tmp[v * 8 + x]).sum();
        }
    }
}

/// Decode one frame into 15-bit pixels (bit 15 set so that black is drawn), `w`×`h`.
pub fn decode(frame: &[u8], w: usize, h: usize) -> Vec<u16> {
    let mut px = vec![0x8000u16; w * h];
    if frame.len() < 8 {
        return px;
    }
    let q = u16::from_le_bytes([frame[4], frame[5]]) as i32;
    let tab = table();
    let mut bits = Bits { d: &frame[8..], pos: 0 };
    let (mbw, mbh) = (w.div_ceil(16), h.div_ceil(16));
    let mut planes = [[0.0f32; 64]; 6];
    for mx in 0..mbw {
        for my in 0..mbh {
            for plane in planes.iter_mut() {
                let mut blk = [0i32; 64];
                blk[0] = bits.sget(10) * IQ[0] as i32;
                let mut k = 0usize;
                loop {
                    let (n, run, level) = tab[bits.peek(16) as usize];
                    if n == 0 {
                        break; // corrupt stream
                    }
                    bits.pos += n as usize;
                    let (run, lv) = match level {
                        EOB => break,
                        ESC => (bits.get(6) as usize, bits.sget(10)),
                        _ => (run as usize, if bits.get(1) == 1 { -(level as i32) } else { level as i32 }),
                    };
                    k += run + 1;
                    if k > 63 {
                        break;
                    }
                    let p = ZIGZAG[k] as usize;
                    let v = lv.wrapping_abs().wrapping_mul(q).wrapping_mul(IQ[p] as i32) >> 3;
                    blk[p] = if lv < 0 { -v } else { v };
                }
                idct(&blk, plane);
            }
            let [cr, cb, y1, y2, y3, y4] = &planes;
            for yy in 0..16 {
                for xx in 0..16 {
                    let (sx, sy) = (mx * 16 + xx, my * 16 + yy);
                    if sx >= w || sy >= h {
                        continue;
                    }
                    let yb = match (xx >= 8, yy >= 8) {
                        (false, false) => y1,
                        (true, false) => y2,
                        (false, true) => y3,
                        (true, true) => y4,
                    };
                    let l = yb[(yy & 7) * 8 + (xx & 7)] + 128.0;
                    let c = (yy / 2) * 8 + xx / 2;
                    let (r, b) = (cr[c], cb[c]);
                    let rgb = [l + 1.402 * r, l - 0.3437 * b - 0.7143 * r, l + 1.772 * b];
                    let [r5, g5, b5] = rgb.map(|v| (v.round().clamp(0.0, 255.0) as u16) >> 3);
                    px[sy * w + sx] = 0x8000 | r5 | g5 << 5 | b5 << 10;
                }
            }
        }
    }
    px
}

/// The disc's stream rate (double speed): a movie's frames are timed by their sector.
pub const SECTORS_PER_SEC: f64 = 150.0;

/// A movie file: frames with their stream sector.
#[derive(Default)]
pub struct Movie {
    pub w: usize,
    pub h: usize,
    pub frames: Vec<(u32, std::ops::Range<usize>)>,
    /// the fetched file (shared with the app's store while it holds it)
    pub bytes: std::sync::Arc<[u8]>,
}

impl Movie {
    pub fn parse(bytes: impl Into<std::sync::Arc<[u8]>>) -> Option<Self> {
        let bytes = bytes.into();
        if bytes.get(0..4)? != b"BRBS" {
            return None;
        }
        let rd = |o: usize| bytes.get(o..o + 4).map(|s| u32::from_le_bytes([s[0], s[1], s[2], s[3]]));
        let n = rd(8)? as usize;
        let (w, h) = (u16::from_le_bytes([bytes[4], bytes[5]]) as usize, u16::from_le_bytes([bytes[6], bytes[7]]) as usize);
        // a corrupt header must not allocate huge pictures (the movies are 320×240)
        if w == 0 || h == 0 || w > 1024 || h > 1024 {
            return None;
        }
        let mut o = 12;
        let mut frames = Vec::with_capacity(n.min(bytes.len() / 8));
        for _ in 0..n {
            let (s, len) = (rd(o)?, rd(o + 4)? as usize);
            let end = (o + 8).checked_add(len).filter(|&e| e <= bytes.len())?;
            frames.push((s, o + 8..end));
            o = end;
        }
        Some(Self { w, h, frames, bytes })
    }

    /// The frame shown `t` seconds in (the disc streams `SECTORS_PER_SEC`).
    pub fn frame_at(&self, t: f64) -> usize {
        let s = (t * SECTORS_PER_SEC) as u32;
        self.frames.partition_point(|f| f.0 <= s).saturating_sub(1)
    }

    /// Frame `k` as 15-bit pixels (empty past the last frame).
    pub fn decode(&self, k: usize) -> Vec<u16> {
        self.frames.get(k).map_or_else(Vec::new, |f| decode(&self.bytes[f.1.clone()], self.w, self.h))
    }
}
