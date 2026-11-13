//! CD-XA ADPCM: the disc's music (`/XA/BR2_MCnn.XA`, one file per channel) and movie audio (the STR's interleaved
//! sectors) as the converter stores them, decoded to `Music`.
//!
//! File (`.xa`, version 1): magic `BR2XA\0`, u8 version, u8 coding (the stream's subheader coding-info byte: bits
//! 0-1 mono/stereo, 2-3 37.8/18.9 kHz, 4-5 4/8-bit), u32 LE sector count N, u32 0, then N × 2304 bytes: each audio
//! sector's 18 sound groups of 128 bytes, in disc order up to the sector with the EOF submode bit.
//!
//! A sound group: 16 header bytes (the sound parameters, shift in bits 0-3 and filter in bits 4-5, of unit u at
//! 4 + u; the other bytes are copies), then 28 rows of 4 bytes. 4-bit: 8 units of 28 samples, unit u's sample j the
//! low (u even) / high (u odd) nibble of byte 16 + 4j + u/2; 8-bit: 4 units, byte 16 + 4j + u. Stereo: even units
//! left, odd right. Each sample: (nibble << 12, or byte << 8, as i16) >> shift, plus the filter of the last two
//! outputs of its channel ((s1·K0 + s2·K1 + 32) >> 6), clamped to i16; the history runs on across groups and sectors.

use super::music::Music;

/// The file's first bytes (a historical format id, kept so that files of earlier builds stay readable: the converted data writes it).
pub const MAGIC: &[u8; 6] = b"BR2XA\0";
pub const VERSION: u8 = 1;
pub const HEADER: usize = 16;
/// one sector's sound groups (Mode 2 Form 2 user data without its trailing 20 bytes)
pub const SECTOR: usize = 18 * GROUP;
const GROUP: usize = 128;
/// the filters' (K0, K1), /64
const K: [(i32, i32); 4] = [(0, 0), (60, 0), (115, -52), (98, -55)];

/// A stream's coding (the subheader's coding-info byte).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Coding {
    pub stereo: bool,
    /// 37 800 or 18 900 Hz
    pub rate: u32,
    pub eight_bit: bool,
}

impl Coding {
    /// The coding-info byte; reserved values (channels, rate or bits 2-3) are an error. Bit 6 (emphasis) is ignored.
    pub fn from_byte(b: u8) -> Result<Self, String> {
        let (ch, rate, bits) = (b & 3, (b >> 2) & 3, (b >> 4) & 3);
        if ch > 1 || rate > 1 || bits > 1 {
            return Err(format!("XA coding {b:#04x}: reserved value"));
        }
        Ok(Self { stereo: ch == 1, rate: if rate == 1 { 18_900 } else { 37_800 }, eight_bit: bits == 1 })
    }

    /// Sound units per group.
    fn units(self) -> usize {
        if self.eight_bit { 4 } else { 8 }
    }

    /// Output frames (stereo pairs) per sector.
    pub fn frames_per_sector(self) -> usize {
        18 * self.units() * 28 / if self.stereo { 2 } else { 1 }
    }
}

/// The file's header: the coding and the sectors after it (checked against the file's length).
pub fn parse(bytes: &[u8]) -> Result<(Coding, &[u8]), String> {
    let h = bytes.get(..HEADER).ok_or("XA file: truncated header")?;
    if &h[..6] != MAGIC {
        return Err("XA file: bad magic".into());
    }
    if h[6] != VERSION {
        return Err(format!("XA file: version {} (this build reads {VERSION})", h[6]));
    }
    let coding = Coding::from_byte(h[7])?;
    let n = u32::from_le_bytes([h[8], h[9], h[10], h[11]]) as usize;
    let data = &bytes[HEADER..];
    if n.checked_mul(SECTOR) != Some(data.len()) {
        return Err(format!("XA file: {} data bytes for {n} sectors", data.len()));
    }
    Ok((coding, data))
}

/// A `.xa` file decoded to interleaved stereo (mono duplicated) at its rate.
pub fn decode(bytes: &[u8]) -> Result<Music, String> {
    let (coding, data) = parse(bytes)?;
    let frames = data.len() / SECTOR * coding.frames_per_sector();
    let mut pcm = Vec::with_capacity(frames * 2);
    let mut hist = [History::default(); 2];
    for g in data.as_chunks::<GROUP>().0 {
        group(g, coding, &mut hist, &mut pcm);
    }
    debug_assert_eq!(pcm.len(), frames * 2);
    Ok(Music { pcm, rate: coding.rate })
}

/// A channel's filter history: the last output and the one before.
#[derive(Clone, Copy, Default)]
struct History {
    s1: i32,
    s2: i32,
}

/// One sound group appended to `pcm` (stereo: unit pairs L/R interleaved; mono: each unit's samples twice).
fn group(g: &[u8; GROUP], c: Coding, hist: &mut [History; 2], pcm: &mut Vec<i16>) {
    let mut l = [0i16; 28];
    let mut r = [0i16; 28];
    if c.stereo {
        for u in (0..c.units()).step_by(2) {
            unit(g, u, c.eight_bit, &mut hist[0], &mut l);
            unit(g, u + 1, c.eight_bit, &mut hist[1], &mut r);
            for j in 0..28 {
                pcm.extend([l[j], r[j]]);
            }
        }
    } else {
        for u in 0..c.units() {
            unit(g, u, c.eight_bit, &mut hist[0], &mut l);
            for v in l {
                pcm.extend([v, v]);
            }
        }
    }
}

/// Unit `u`'s 28 samples.
fn unit(g: &[u8; GROUP], u: usize, eight_bit: bool, h: &mut History, out: &mut [i16; 28]) {
    let p = g[4 + u];
    // shift 13..15 acts as 9 (psx-spx "CDROM XA Audio ADPCM Compression"; DuckStation, Mednafen)
    let shift = match p & 15 {
        s @ 0..=12 => s,
        _ => 9,
    };
    let (k0, k1) = K[((p >> 4) & 3) as usize];
    for (j, o) in out.iter_mut().enumerate() {
        let raw = if eight_bit {
            (g[16 + 4 * j + u] as u16) << 8
        } else {
            let b = g[16 + 4 * j + u / 2];
            ((if u & 1 == 0 { b & 15 } else { b >> 4 }) as u16) << 12
        };
        let s = ((raw as i16) >> shift) as i32 + ((h.s1 * k0 + h.s2 * k1 + 32) >> 6);
        let v = s.clamp(-32768, 32767);
        (h.s2, h.s1) = (h.s1, v);
        *o = v as i16;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A `.xa` file of `groups` (padded with silent groups to whole sectors).
    fn file(coding: u8, groups: &[[u8; GROUP]]) -> Vec<u8> {
        let n = groups.len().div_ceil(18);
        let mut b = MAGIC.to_vec();
        b.extend([VERSION, coding]);
        b.extend((n as u32).to_le_bytes());
        b.extend([0; 4]);
        for g in groups {
            b.extend(g);
        }
        b.resize(HEADER + n * SECTOR, 0);
        b
    }

    /// A 4-bit group: unit u's parameter `p[u]`, its sample j the nibble `s(u, j)`.
    fn group4(p: [u8; 8], s: impl Fn(usize, usize) -> u8) -> [u8; GROUP] {
        let mut g = [0; GROUP];
        for u in 0..8 {
            g[4 + u] = p[u];
            for j in 0..28 {
                g[16 + 4 * j + u / 2] |= (s(u, j) & 15) << (4 * (u & 1));
            }
        }
        g
    }

    /// Channel `ch` (0 left, 1 right) of a decoded stereo stream.
    fn chan(m: &Music, ch: usize) -> Vec<i16> {
        m.pcm.iter().skip(ch).step_by(2).copied().collect()
    }

    #[test]
    fn shifts_and_sign() {
        // unit 0 (left): shift 0, filter 0: nibble 1 → 4096, nibble 8 → -32768; unit 2: shift 12: 7 → 7, 15 → -1;
        // unit 4: shift 13 acts as 9: 1 → 8
        let g = group4([0, 0, 12, 0, 13, 0, 0, 0], |u, j| match (u, j) {
            (0, 0) | (4, 0) => 1,
            (0, 1) => 8,
            (2, 0) => 7,
            (2, 1) => 15,
            _ => 0,
        });
        let m = decode(&file(0x01, &[g])).unwrap();
        assert_eq!(m.rate, 37_800);
        assert_eq!(m.pcm.len(), 18 * 112 * 2);
        let l = chan(&m, 0);
        assert_eq!(&l[0..2], &[4096, -32768]);
        assert_eq!(&l[28..30], &[7, -1]);
        assert_eq!(l[56], 8);
        assert!(chan(&m, 1).iter().all(|&v| v == 0));
    }

    #[test]
    fn filters_round_down_and_clamp() {
        // one impulse per unit, then the filter alone: (s1·K0 + s2·K1 + 32) >> 6, floored
        let one = |f: u8, n: u8| decode(&file(0x01, &[group4([f << 4, 0, 0, 0, 0, 0, 0, 0], |u, j| if u == 0 && j == 0 { n } else { 0 })])).unwrap();
        assert_eq!(&chan(&one(0, 1), 0)[0..3], &[4096, 0, 0]);
        assert_eq!(&chan(&one(1, 1), 0)[0..3], &[4096, 3840, 3600]);
        assert_eq!(&chan(&one(2, 1), 0)[0..3], &[4096, 7360, 9897]);
        assert_eq!(&chan(&one(3, 1), 0)[0..3], &[4096, 6272, 6084]);
        // -401376 / 64 = -6271.5 → -6272
        assert_eq!(&chan(&one(3, 15), 0)[0..2], &[-4096, -6272]);
        // 28672 + 51520 → 32767; the clamped value is the history
        // (unit 2, filter 1, no input: (32767·60 + 32) >> 6 = 30719)
        let m = decode(&file(0x01, &[group4([2 << 4, 0, 1 << 4, 0, 0, 0, 0, 0], |u, _| if u == 0 { 7 } else { 0 })])).unwrap();
        let l = chan(&m, 0);
        assert_eq!(&l[0..3], &[28672, 32767, 32767]);
        assert_eq!(l[28], 30719);
        let m = decode(&file(0x01, &[group4([2 << 4, 0, 0, 0, 0, 0, 0, 0], |u, _| if u == 0 { 8 } else { 0 })])).unwrap();
        assert_eq!(&chan(&m, 0)[0..2], &[-32768, -32768]);
    }

    #[test]
    fn stereo_interleave_and_history_across_groups() {
        // left units 0, 2 ... right 1, 3 ...: frame 28k + j is unit pair k's sample j; each side its own history,
        // which runs on into the next group (filter 1, no new input)
        let a = group4([0, 0, 0, 0, 0, 0, 0, 0x10], |u, j| match (u, j) {
            (0, 0) => 1,
            (1, 0) => 2,
            (7, 27) => 1,
            _ => 0,
        });
        let b = group4([0, 0x10, 0, 0, 0, 0, 0, 0], |_, _| 0);
        let m = decode(&file(0x01, &[a, b])).unwrap();
        let (l, r) = (chan(&m, 0), chan(&m, 1));
        assert_eq!((l[0], r[0], l[1], r[1]), (4096, 8192, 0, 0));
        assert_eq!(r[111], 4096);
        // group b: left unit 0 filter 0 (silent), right unit 1 filter 1 from 4096
        assert_eq!((l[112], r[112], r[113]), (0, 3840, 3600));
    }

    #[test]
    fn mono_eight_bit_and_rates() {
        // 8-bit mono 18.9 kHz: 4 units, unit u's sample j byte 16 + 4j + u, each sample on both sides
        let mut g = [0; GROUP];
        g[4..8].copy_from_slice(&[0, 4, 0, 8]);
        g[16] = 0x01; // unit 0 j 0: 0x0100
        g[16 + 1] = 0x80; // unit 1 j 0: -32768 >> 4
        g[16 + 4 + 3] = 0xFF; // unit 3 j 1: -256 >> 8
        let m = decode(&file(0x14, &[g])).unwrap();
        assert_eq!(m.rate, 18_900);
        assert_eq!(m.pcm.len(), 18 * 112 * 2);
        assert_eq!(&m.pcm[0..2], &[256, 256]);
        assert_eq!(m.pcm[2 * 28], -2048);
        assert_eq!(m.pcm[2 * (3 * 28 + 1)], -1);
        // the sizes per coding: 4-bit mono 224 frames a group, 8-bit stereo 56
        assert_eq!(Coding::from_byte(0x00).unwrap().frames_per_sector(), 18 * 224);
        assert_eq!(Coding::from_byte(0x11).unwrap().frames_per_sector(), 18 * 56);
        assert_eq!(decode(&file(0x05, &[])).unwrap().rate, 18_900);
    }

    #[test]
    fn bad_files_are_errors() {
        let good = file(0x01, &[[0; GROUP]]);
        assert!(decode(&good).is_ok());
        for n in 0..good.len() {
            assert!(decode(&good[..n]).is_err(), "truncated to {n}");
        }
        let with = |o: usize, v: u8| {
            let mut b = good.clone();
            b[o] = v;
            decode(&b)
        };
        assert!(with(0, b'X').is_err());
        assert!(with(5, 1).is_err());
        assert!(with(6, 2).is_err());
        assert!(with(6, 0).is_err());
        for c in [0x02, 0x03, 0x08, 0x0C, 0x20, 0x30] {
            assert!(with(7, c).is_err(), "coding {c:#x}");
        }
        assert!(with(7, 0x41).is_ok(), "emphasis is ignored");
        assert!(with(8, 2).is_err(), "more sectors than bytes");
        let mut long = good.clone();
        long.push(0);
        assert!(decode(&long).is_err());
        // a huge count must not allocate
        assert!(with(11, 0xFF).is_err());
        assert!(decode(&file(0x01, &[])).unwrap().pcm.is_empty());
    }

    // ---- the disc's own streams (skipped without the user's image) ----------------------------------------------

    /// The disc image: `OKB_DISC`, else the user's beside the export; None unless it has raw 2352-byte sectors
    /// (sector 16 with the sync pattern and the volume descriptor's CD001: an ISO has no XA streams).
    fn disc() -> Option<std::fs::File> {
        let p = crate::base::env::var("DISC").unwrap_or_else(|_| concat!(env!("CARGO_MANIFEST_DIR"), "/../Bloody Roar II (USA).bin").into());
        let mut f = std::fs::File::open(&p).ok()?;
        let raw = f.metadata().ok()?.len() >= 17 * 2352 && {
            let s = sector(&mut f, 16);
            s[..12] == [0, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0] && &s[25..30] == b"CD001"
        };
        if !raw {
            eprintln!("{p}: not a raw 2352-byte image");
        }
        raw.then_some(f)
    }

    fn sector(f: &mut std::fs::File, lba: u32) -> [u8; 2352] {
        use std::io::{Read, Seek, SeekFrom};
        let mut s = [0; 2352];
        f.seek(SeekFrom::Start(lba as u64 * 2352)).unwrap();
        f.read_exact(&mut s).unwrap();
        s
    }

    /// An ISO 9660 file's (first sector, sectors) by its path (`XA/BR2_MC01.XA`).
    fn find(f: &mut std::fs::File, path: &str) -> (u32, u32) {
        let rd = |b: &[u8], o: usize| u32::from_le_bytes([b[o], b[o + 1], b[o + 2], b[o + 3]]);
        let pvd = sector(f, 16);
        let (mut lba, mut size) = (rd(&pvd, 24 + 156 + 2), rd(&pvd, 24 + 156 + 10));
        for name in path.split('/') {
            let want = name.to_string();
            let mut found = None;
            'dir: for k in 0..size.div_ceil(2048) {
                let s = sector(f, lba + k);
                let d = &s[24..24 + 2048];
                let mut o = 0;
                while o < 2048 && d[o] != 0 {
                    let n = &d[o + 33..o + 33 + d[o + 32] as usize];
                    let n = String::from_utf8_lossy(n);
                    if n.split(';').next() == Some(&want) {
                        found = Some((rd(d, o + 2), rd(d, o + 10)));
                        break 'dir;
                    }
                    o += d[o] as usize;
                }
            }
            (lba, size) = found.unwrap_or_else(|| panic!("{path}: {name} not on the disc"));
        }
        (lba, size.div_ceil(2048))
    }

    /// The `.xa` file of channel `ch` of the disc file at `path` (its first audio channel when None): the audio
    /// sectors (submode bit 2) of that channel in order, up to the one with the EOF bit (7). Also the first one's
    /// sector in the file.
    fn extract(f: &mut std::fs::File, path: &str, ch: Option<u8>) -> (Vec<u8>, u32) {
        let (lba, n) = find(f, path);
        let (mut coding, mut ch, mut first, mut data) = (None, ch, 0, Vec::<u8>::new());
        for k in 0..n {
            let s = sector(f, lba + k);
            let sh = &s[16..20];
            if sh[2] & 0x04 == 0 || ch.is_some_and(|c| c != sh[1]) {
                continue;
            }
            if ch.is_none() {
                ch = Some(sh[1]);
            }
            match coding {
                None => (coding, first) = (Some(sh[3]), k),
                Some(c) => assert_eq!(c, sh[3], "{path}: the coding changes"),
            }
            data.extend(&s[24..24 + SECTOR]);
            if sh[2] & 0x80 != 0 {
                break;
            }
        }
        let mut b = MAGIC.to_vec();
        b.extend([VERSION, coding.expect("audio sectors")]);
        b.extend(((data.len() / SECTOR) as u32).to_le_bytes());
        b.extend([0; 4]);
        b.extend(data);
        (b, first)
    }

    /// Mono, as f64.
    fn mono(m: &Music) -> Vec<f64> {
        m.pcm.chunks(2).map(|f| (f[0] as f64 + f[1] as f64) / 2.0).collect()
    }

    /// Normalised correlation of `a[i]` with `b[i + lag]` over their overlap (from `from`, `len` samples or all).
    fn corr(a: &[f64], b: &[f64], lag: i64, from: usize, len: usize) -> f64 {
        let (mut ab, mut aa, mut bb) = (0.0, 0.0, 0.0);
        for i in from..(from + len).min(a.len()) {
            let Some(&y) = usize::try_from(i as i64 + lag).ok().and_then(|j| b.get(j)) else { continue };
            let x = a[i];
            (ab, aa, bb) = (ab + x * y, aa + x * x, bb + y * y);
        }
        ab / (aa * bb).sqrt().max(1e-9)
    }

    /// The disc's stream decoded against the export's MP3 of it: (XA s, MP3 s, MP3 lag in samples, correlation).
    fn against_mp3(xa: &[u8], mp3: &str) -> Option<(f64, f64, i64, f64)> {
        let m = decode(xa).unwrap();
        // the MP3 of the old Python export (a reference only it has: `OKB_REFERENCE`, else `../br2_export`, read only)
        let root = crate::base::env::var("REFERENCE").unwrap_or_else(|_| concat!(env!("CARGO_MANIFEST_DIR"), "/../br2_export").into());
        let p = format!("{root}/{mp3}");
        let Ok(bytes) = std::fs::read(&p) else {
            eprintln!("{p} missing: skipped");
            return None;
        };
        let r = crate::formats::music::decode_mp3(bytes).expect("the MP3 decodes");
        // the XA resampled (linearly) to the MP3's rate (32 kHz: MP3 has no 37.8)
        let x = mono(&m);
        let step = m.rate as f64 / r.rate as f64;
        let a: Vec<f64> = (0..((x.len() - 1) as f64 / step) as usize)
            .map(|i| {
                let t = i as f64 * step;
                let (k, f) = (t as usize, t.fract());
                x[k] * (1.0 - f) + x[(k + 1).min(x.len() - 1)] * f
            })
            .collect();
        let b = mono(&r);
        // the encoder delay: the best lag over a second from 5 s in, then the whole overlap
        let (from, len) = (5 * r.rate as usize, r.rate as usize);
        let lag = (-2000..4000).max_by(|&p, &q| corr(&a, &b, p, from, len).total_cmp(&corr(&a, &b, q, from, len))).unwrap();
        let c = corr(&a, &b, lag, 0, a.len());
        Some((x.len() as f64 / m.rate as f64, b.len() as f64 / r.rate as f64, lag, c))
    }

    /// Test data for the app: every music channel and movie audio of the disc as `.xa` files under `OKB_XA_OUT`
    /// (`cargo test --release write_disc_xa -- --ignored`), laid out as the export's.
    #[test]
    #[ignore]
    fn write_disc_xa() {
        let (Some(mut f), Ok(out)) = (disc(), crate::base::env::var("XA_OUT")) else { panic!("needs the disc image and OKB_XA_OUT") };
        let out = std::path::Path::new(&out);
        std::fs::create_dir_all(out.join("audio/music")).unwrap();
        std::fs::create_dir_all(out.join("video")).unwrap();
        for file in 1..=6 {
            let path = format!("XA/BR2_MC{file:02}.XA");
            let (lba, n) = find(&mut f, &path);
            let mut chans: Vec<u8> = (0..n).map(|k| sector(&mut f, lba + k)).filter(|s| s[18] & 0x04 != 0).map(|s| s[17]).collect();
            chans.sort();
            chans.dedup();
            for ch in chans {
                let (xa, _) = extract(&mut f, &path, Some(ch));
                std::fs::write(out.join(format!("audio/music/BR2_MC{file:02}_ch{ch}.xa")), xa).unwrap();
            }
        }
        for m in ["USAOP", "BR", "ED1", "ED2"] {
            let (xa, _) = extract(&mut f, &format!("MOV/{m}.STR"), None);
            std::fs::write(out.join(format!("video/{m}.xa")), xa).unwrap();
        }
    }

    #[test]
    fn disc_streams_match_the_exported_mp3() {
        let Some(mut f) = disc() else {
            eprintln!("no disc image: skipped");
            return;
        };
        for (path, ch, mp3) in [("XA/BR2_MC01.XA", Some(0), "audio/music/BR2_MC01_ch0.mp3"), ("MOV/USAOP.STR", None, "video/USAOP.mp3")] {
            let (xa, first) = extract(&mut f, path, ch);
            let (c, n) = (Coding::from_byte(xa[7]).unwrap(), u32::from_le_bytes(xa[8..12].try_into().unwrap()));
            let Some((xs, ms, lag, corr)) = against_mp3(&xa, mp3) else { continue };
            println!(
                "{path} ch {ch:?}: {n} sectors (first at file sector {first}), coding {:#04x} {c:?}; XA {xs:.4} s, MP3 {ms:.4} s \
                 (+{:.4} s), MP3 lag {lag} samples ({:.4} s), correlation {corr:.5}",
                xa[7],
                ms - xs,
                lag as f64 / 32000.0
            );
            assert!(corr > 0.98, "{path}: correlation {corr}");
            // the MP3 adds the encoder delay and padding to whole frames (1152 samples), no more
            assert!((0.0..0.1).contains(&(ms - xs)), "{path}: XA {xs} s, MP3 {ms} s");
        }
    }
}
