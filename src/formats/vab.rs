//! Sony VAB sound banks (`*.SND`, "pBAV") and their SPU-ADPCM samples, decoded for `audio`'s mixer.
//!
//! VAB layout: 0x20-byte header (u16 program count at 0x12, tone count 0x14, VAG count 0x16), 128 programs ×
//! 16 bytes (u8 tone count, master volume, priority, mode, master pan …), then 16 tones × 32 bytes for every
//! program that has tones (in program order), a u16 table of 256 VAG sizes (÷8), then the ADPCM samples.
//! Tone: 0 priority, 1 mode, 2 volume, 3 pan, 4 centre note, 5 fine tune, 6/7 key range, 16/18 ADSR,
//! 20 program, 22 VAG number.

use std::sync::Arc;

#[derive(Clone, Copy, Debug, Default)]
pub struct Tone {
    pub volume: u8,
    pub pan: u8,
    pub center: u8,
    pub fine: u8,
    pub min: u8,
    pub max: u8,
    pub adsr1: u16,
    pub adsr2: u16,
    pub vag: u16,
}

#[derive(Clone, Debug, Default)]
pub struct Program {
    pub volume: u8,
    pub pan: u8,
    pub tones: Vec<Tone>,
}

pub struct Sample {
    pub pcm: Vec<i16>,
    /// sample index the loop returns to (ADPCM loop-start flag) when the block end has the repeat flag
    pub looped: Option<usize>,
}

pub struct Vab {
    pub programs: Vec<Program>,
    /// tone records in file order (32 bytes at 0x820 + 32·t): sound ids address these (slot·64 + t)
    pub tones: Vec<Tone>,
    pub samples: Vec<Arc<Sample>>, // index = VAG number - 1
}

const SPU_K: [(i32, i32); 5] = [(0, 0), (60, 0), (115, -52), (98, -55), (122, -60)];

/// SPU ADPCM: 16-byte blocks (shift/filter, flags, 14 data bytes = 28 samples).
pub fn decode_adpcm(data: &[u8]) -> Sample {
    let mut pcm = Vec::with_capacity(data.len() / 16 * 28);
    let (mut s1, mut s2) = (0i32, 0i32);
    let mut loop_start = None;
    let mut looped = None;
    for (bi, b) in data.as_chunks::<16>().0.iter().enumerate() {
        let shift = (b[0] & 0xF) as i32;
        let (k0, k1) = SPU_K[((b[0] >> 4) & 7).min(4) as usize];
        let flags = b[1];
        if flags & 4 != 0 {
            loop_start = Some(pcm.len());
        }
        for &byte in &b[2..16] {
            for n in [byte & 0xF, byte >> 4] {
                let n = ((n as i32) << 28) >> 28;
                let v = if shift <= 12 { (n << 12) >> shift } else { 0 };
                let v = (v + ((s1 * k0 + s2 * k1 + 32) >> 6)).clamp(-32768, 32767);
                pcm.push(v as i16);
                s2 = s1;
                s1 = v;
            }
        }
        if flags & 1 != 0 && bi > 0 {
            if flags & 2 != 0 {
                looped = Some(loop_start.unwrap_or(0));
            }
            break;
        }
    }
    Sample { pcm, looped }
}

fn u16at(b: &[u8], o: usize) -> u16 {
    b.get(o..o + 2).map(|s| u16::from_le_bytes([s[0], s[1]])).unwrap_or(0)
}

impl Vab {
    pub fn parse(b: &[u8]) -> Option<Self> {
        if b.get(0..4)? != b"pBAV" {
            return None;
        }
        let ps = u16at(b, 0x12) as usize;
        let vs = u16at(b, 0x16) as usize;
        let prog_off = 0x20;
        let tone_off = prog_off + 128 * 16;
        let ts = u16at(b, 0x14) as usize;
        // the program and tone tables must lie inside the bank
        if b.len() < tone_off + ps * 16 * 32 {
            return None;
        }
        let tones = (0..ps * 16)
            .map(|t| {
                let to = tone_off + t * 32;
                Tone {
                    volume: b[to + 2],
                    pan: b[to + 3],
                    center: b[to + 4],
                    fine: b[to + 5],
                    min: b[to + 6],
                    max: b[to + 7],
                    adsr1: u16at(b, to + 16),
                    adsr2: u16at(b, to + 18),
                    vag: u16at(b, to + 22),
                }
            })
            .take(ts.max(1).max(ps * 16).min(ps * 16))
            .collect();
        let mut programs = Vec::with_capacity(128);
        let mut block = 0usize;
        for p in 0..128 {
            let o = prog_off + 16 * p;
            let n = b[o] as usize;
            let mut prog = Program { volume: b[o + 1], pan: b[o + 4], tones: Vec::new() };
            if n > 0 && block < ps {
                for t in 0..n.min(16) {
                    let to = tone_off + (block * 16 + t) * 32;
                    prog.tones.push(Tone {
                        volume: b[to + 2],
                        pan: b[to + 3],
                        center: b[to + 4],
                        fine: b[to + 5],
                        min: b[to + 6],
                        max: b[to + 7],
                        adsr1: u16at(b, to + 16),
                        adsr2: u16at(b, to + 18),
                        vag: u16at(b, to + 22),
                    });
                }
                block += 1;
            }
            programs.push(prog);
        }
        let vag_tab = tone_off + ps * 16 * 32;
        let mut pos = vag_tab + 512;
        let mut samples = Vec::with_capacity(vs);
        for i in 1..=vs {
            let sz = u16at(b, vag_tab + 2 * i) as usize * 8;
            let chunk = b.get(pos..(pos + sz).min(b.len())).unwrap_or(&[]);
            pos += sz;
            samples.push(Arc::new(decode_adpcm(chunk)));
        }
        Some(Self { programs, tones, samples })
    }
}
