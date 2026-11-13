//! Main-game containers (`*.DAT`) and their LZ compression (the game's).
//!
//! Container: u32 count, u32 offsets[count] relative to the end of the table (0xFFFFFFFF = empty slot).
//! Entry: u32 type (2 = LZ), u32 packed size, u32 unpacked size, data.
//! LZ: flag bits MSB-first from bytes fetched when needed; `1` literal, `0 0 b b` short match
//! (length bb+2, offset byte − 256), `0 1` long match (13-bit offset − 0x2000, length low 3 bits + 2,
//! or an extra byte: 0 = end, else length + 1).

/// Largest unpacked entry accepted (the biggest real one is a few hundred KB): a corrupt size field must not
/// allocate gigabytes.
const MAX_UNPACKED: usize = 32 << 20;

pub fn decompress(src: &[u8], mut pos: usize, out_size: usize) -> Vec<u8> {
    let out_size = out_size.min(MAX_UNPACKED);
    let mut out: Vec<u8> = Vec::with_capacity(out_size);
    let (mut flags, mut nbits) = (0u8, 0u32);
    let mut bit = |pos: &mut usize| -> u8 {
        if nbits == 0 {
            flags = src.get(*pos).copied().unwrap_or(0);
            *pos += 1;
            nbits = 8;
        }
        nbits -= 1;
        (flags >> nbits) & 1
    };
    while out.len() < out_size && pos < src.len() {
        if bit(&mut pos) == 1 {
            let Some(&b) = src.get(pos) else { break };
            out.push(b);
            pos += 1;
            continue;
        }
        let (off, len): (isize, usize);
        if bit(&mut pos) == 0 {
            let hi = bit(&mut pos);
            let lo = bit(&mut pos);
            len = ((hi << 1 | lo) + 2) as usize;
            off = src.get(pos).copied().unwrap_or(0) as isize - 256;
            pos += 1;
        } else {
            let at = |p: usize| src.get(p).copied().unwrap_or(0) as isize;
            let (b0, b1) = (at(pos), at(pos + 1));
            pos += 2;
            off = ((b0 << 5) | (b1 >> 3)) - 0x2000;
            let n = (b1 & 7) as usize;
            if n != 0 {
                len = n + 2;
            } else {
                let c = src.get(pos).copied().unwrap_or(0) as usize;
                pos += 1;
                if c == 0 {
                    break;
                }
                len = c + 1;
            }
        }
        let start = out.len() as isize + off;
        if start < 0 {
            break;
        }
        for i in 0..len {
            let b = out[start as usize + i];
            out.push(b);
        }
    }
    out
}

/// Every entry of a container, decompressed (`None` = empty slot).
pub fn read(data: &[u8]) -> Vec<Option<Vec<u8>>> {
    let u32at = |o: usize| data.get(o..o + 4).map(|s| u32::from_le_bytes([s[0], s[1], s[2], s[3]])).unwrap_or(0);
    // at most one offset per 4 bytes of the file
    let n = (u32at(0) as usize).min(data.len() / 4);
    let base = 4 + 4 * n;
    (0..n)
        .map(|i| {
            let o = u32at(4 + 4 * i);
            if o == 0xFFFF_FFFF {
                return None;
            }
            let e = base + o as usize;
            let (t, usz) = (u32at(e), u32at(e + 8) as usize);
            if t == 2 { Some(decompress(data, e + 12, usz)) } else { data.get(e + 12..(e + 12).saturating_add(usz)).map(|s| s.to_vec()) }
        })
        .collect()
}
