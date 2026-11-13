//! The imported game's select portraits as the front end's pictures (engine addition, the import S9).
//!
//! State: static data (`frontend::FrontData::br1`: not serialised)
//!
//! The imported game's character select shows a 15-bit 128×160 portrait per character (`SELE00.DAT` entries 16..24 facing
//! right, `SELE01.DAT` entries 32..40 the same facing left; `SELE10` / `SELE11` hold another look of them), in the order
//! YUGO GADO ALICE FOX LONG MITSUKO BAKURYU GREG URIKO (`PORTRAIT`: identified on the imported game's select and VS select
//! screens in recorded runs). The engine draws them in the main game's pictures' places, built here once
//! as TIMs whose size never depends on the data (a missing or damaged file gives a blank picture of the same size, so
//! the front end's VRAM loads, which are state, are the same with any data):
//! - `select`: the select's big portrait, the facing-left one as it is (15-bit 128×160, drawn mirrored for P1 as
//!   the main game's beast pictures are);
//! - `faces`: the four cells of the select's imported row, the facing-right faces cut out and scaled to the
//!   main game's 56×63 face cells (one 15-bit 224×63 TIM, on a dark red ground as the main game's face icons);
//! - `vs`: the VS screen's portrait in the place and format of the main game's SBxx pictures (8-bit 192×256 with a
//!   256-entry CLUT: the facing-right portrait at its own aspect, 164 texels wide, centred; median-cut colours).

use crate::assets::Assets;

/// `roster::BR1_CODES`' portraits (MIT FOX GRE URI) by their index in the select files.
pub const PORTRAIT: [usize; 4] = [5, 3, 7, 8];
/// The portraits' size (15-bit texels).
pub const W: usize = 128;
pub const H: usize = 160;
/// A face cell (`frontend::select`'s 56×63) and the part of the facing-right portrait it shows (x, y, w, h: as wide
/// as tall on screen, 35:31.5 in the game's 320×240 pixels).
pub const FACE_W: usize = 56;
pub const FACE_H: usize = 63;
const FACE_CUT: (usize, usize, usize, usize) = (18, 8, 96, 86);
/// The VS picture's size (8-bit texels) and the portrait's place in it (x, width).
pub const VS_W: usize = 192;
pub const VS_H: usize = 256;
const VS_X: usize = 14;
const VS_PW: usize = 164;
/// The face cells' ground (15-bit, opaque: a dark red).
const GROUND: u16 = 0x8000 | 6;

/// The pictures (`select[k]`, `vs[k]` for `roster::BR1_CODES[k]`).
#[derive(Clone)]
pub struct Br1Art {
    pub select: Vec<Vec<u8>>,
    pub faces: Vec<u8>,
    pub vs: Vec<Vec<u8>>,
}

impl Default for Br1Art {
    /// Blank pictures of the right sizes.
    fn default() -> Self {
        Self::build(&[None, None, None, None], &[None, None, None, None])
    }
}

impl Br1Art {
    /// The pictures from the facing-right (`right`) and facing-left (`left`) portraits of the four fighters.
    fn build(right: &[Option<Vec<u16>>; 4], left: &[Option<Vec<u16>>; 4]) -> Self {
        let blank = vec![0u16; W * H];
        let select = left.iter().map(|p| tim16(W, H, p.as_ref().unwrap_or(&blank))).collect();
        let mut faces = vec![GROUND; 4 * FACE_W * FACE_H];
        for (k, p) in right.iter().enumerate() {
            let cell = resample(p.as_ref().unwrap_or(&blank), W, FACE_CUT, FACE_W, FACE_H, Some(GROUND));
            for y in 0..FACE_H {
                faces[y * 4 * FACE_W + k * FACE_W..][..FACE_W].copy_from_slice(&cell[y * FACE_W..][..FACE_W]);
            }
        }
        let vs = right
            .iter()
            .map(|p| {
                let pic = resample(p.as_ref().unwrap_or(&blank), W, (0, 0, W, H), VS_PW, VS_H, None);
                let mut px = vec![0u16; VS_W * VS_H];
                for y in 0..VS_H {
                    px[y * VS_W + VS_X..][..VS_PW].copy_from_slice(&pic[y * VS_PW..][..VS_PW]);
                }
                let (clut, idx) = quantize(&px);
                tim8(VS_W, VS_H, &clut, &idx)
            })
            .collect();
        Self { select, faces: tim16(4 * FACE_W, FACE_H, &faces), vs }
    }
}

/// The pictures from the imported game's install `br1` (blank ones without it, or for a portrait it lacks).
pub async fn load(br1: Option<&Assets>) -> Br1Art {
    let Some(dir) = br1 else { return Br1Art::default() };
    let file = async |name: &str| dir.load(&format!("engine/CMN/{name}")).await.unwrap_or_default();
    from_files(&file("SELE00.DAT").await, &file("SELE01.DAT").await)
}

/// The pictures from the bytes of `CMN/SELE00.DAT` (facing right) and `SELE01.DAT` (facing left): any bytes (a
/// container that does not read, or a portrait it lacks, gives the blank one).
pub fn from_files(sele00: &[u8], sele01: &[u8]) -> Br1Art {
    let (right, left) = (crate::formats::container::read(sele00), crate::formats::container::read(sele01));
    let pick = |c: &[Option<Vec<u8>>], first: usize| PORTRAIT.map(|k| c.get(first + k).and_then(|e| e.as_deref()).and_then(portrait));
    Br1Art::build(&pick(&right, 16), &pick(&left, 32))
}

/// A 15-bit 128×160 TIM's texels (None for anything else).
fn portrait(tim: &[u8]) -> Option<Vec<u16>> {
    let u32at = |o: usize| tim.get(o..o + 4).map(|s| u32::from_le_bytes([s[0], s[1], s[2], s[3]]));
    let u16at = |o: usize| tim.get(o..o + 2).map(|s| u16::from_le_bytes([s[0], s[1]]));
    if u32at(0)? != 0x10 || u32at(4)? != 2 || (u16at(16)?, u16at(18)?) != (W as u16, H as u16) {
        return None;
    }
    let px = tim.get(20..20 + 2 * W * H)?;
    Some(px.as_chunks::<2>().0.iter().map(|c| u16::from_le_bytes(*c)).collect())
}

/// A TIM of 15-bit texels (`w` × `h`, at VRAM (0, 0): the loads give the place).
fn tim16(w: usize, h: usize, px: &[u16]) -> Vec<u8> {
    let mut b = Vec::with_capacity(20 + 2 * w * h);
    b.extend(0x10u32.to_le_bytes());
    b.extend(2u32.to_le_bytes());
    b.extend(((12 + 2 * w * h) as u32).to_le_bytes());
    b.extend([0u16, 0, w as u16, h as u16].iter().flat_map(|v| v.to_le_bytes()));
    b.extend(px.iter().flat_map(|v| v.to_le_bytes()));
    b
}

/// A TIM of 8-bit texels (`w` × `h` texels) with its 256-entry CLUT.
fn tim8(w: usize, h: usize, clut: &[u16; 256], idx: &[u8]) -> Vec<u8> {
    let mut b = Vec::with_capacity(20 + 12 + 512 + w * h);
    b.extend(0x10u32.to_le_bytes());
    b.extend(9u32.to_le_bytes());
    b.extend(((12 + 512) as u32).to_le_bytes());
    b.extend([0u16, 0, 256, 1].iter().flat_map(|v| v.to_le_bytes()));
    b.extend(clut.iter().flat_map(|v| v.to_le_bytes()));
    b.extend(((12 + w * h) as u32).to_le_bytes());
    b.extend([0u16, 0, (w / 2) as u16, h as u16].iter().flat_map(|v| v.to_le_bytes()));
    b.extend_from_slice(idx);
    b
}

fn rgb(c: u16) -> [u32; 3] {
    [(c & 31) as u32, (c >> 5 & 31) as u32, (c >> 10 & 31) as u32]
}

fn pack(c: [u32; 3]) -> u16 {
    // opaque (texel 0 is transparent: black is 0x8000)
    0x8000 | c[0] as u16 | (c[1] as u16) << 5 | (c[2] as u16) << 10
}

/// The part `cut` (x, y, w, h) of the `sw`-wide picture `src` scaled to `tw` × `th` by box averaging (a box smaller
/// than a texel takes the nearest one). Transparent texels (0) count as `ground` when given; without one a box more
/// than half transparent stays transparent.
fn resample(src: &[u16], sw: usize, cut: (usize, usize, usize, usize), tw: usize, th: usize, ground: Option<u16>) -> Vec<u16> {
    let (cx, cy, cw, ch) = cut;
    let mut out = vec![0u16; tw * th];
    for ty in 0..th {
        let (y0, y1) = (cy + ty * ch / th, cy + ((ty + 1) * ch / th).max(ty * ch / th + 1));
        for tx in 0..tw {
            let (x0, x1) = (cx + tx * cw / tw, cx + ((tx + 1) * cw / tw).max(tx * cw / tw + 1));
            let (mut sum, mut n, mut clear) = ([0u32; 3], 0u32, 0u32);
            for y in y0..y1 {
                for x in x0..x1 {
                    let c = match src.get(y * sw + x).copied().unwrap_or(0) {
                        0 => match ground {
                            Some(g) => g,
                            None => {
                                clear += 1;
                                continue;
                            }
                        },
                        c => c,
                    };
                    let v = rgb(c);
                    (0..3).for_each(|i| sum[i] += v[i]);
                    n += 1;
                }
            }
            out[ty * tw + tx] = if n == 0 || clear > n { 0 } else { pack(sum.map(|s| (s + n / 2) / n)) };
        }
    }
    out
}

/// `px`'s colours in at most 255 CLUT entries by median cut (entry 0 the transparent texel), and its texels' indices.
fn quantize(px: &[u16]) -> ([u16; 256], Vec<u8>) {
    let mut hist = std::collections::BTreeMap::<u16, u32>::new();
    for &c in px.iter().filter(|&&c| c != 0) {
        *hist.entry(c).or_default() += 1;
    }
    let mut boxes: Vec<Vec<(u16, u32)>> = vec![hist.into_iter().collect()];
    boxes.retain(|b| !b.is_empty());
    while boxes.len() < 255 {
        // the box whose widest channel is widest, split at its weighted median
        let range = |b: &[(u16, u32)], i: usize| {
            let v = b.iter().map(|&(c, _)| rgb(c)[i]);
            v.clone().max().unwrap_or(0) - v.min().unwrap_or(0)
        };
        let Some((k, ch, _)) = boxes
            .iter()
            .enumerate()
            .filter(|(_, b)| b.len() > 1)
            .flat_map(|(k, b)| (0..3).map(move |i| (k, i, range(b, i))))
            .max_by_key(|&(k, i, r)| (r, usize::MAX - k, 2 - i))
        else {
            break;
        };
        let mut b = boxes.swap_remove(k);
        b.sort_by_key(|&(c, _)| (rgb(c)[ch], c));
        let total: u32 = b.iter().map(|&(_, n)| n).sum();
        let (mut acc, mut at) = (0, 1);
        for (j, &(_, n)) in b.iter().enumerate() {
            acc += n;
            if 2 * acc >= total {
                at = (j + 1).clamp(1, b.len() - 1);
                break;
            }
        }
        let rest = b.split_off(at);
        boxes.push(b);
        boxes.push(rest);
    }
    let mut clut = [0u16; 256];
    let mut of = std::collections::BTreeMap::<u16, u8>::new();
    for (k, b) in boxes.iter().enumerate() {
        let n: u32 = b.iter().map(|&(_, n)| n).sum();
        let mut sum = [0u32; 3];
        for &(c, m) in b {
            let v = rgb(c);
            (0..3).for_each(|i| sum[i] += v[i] * m);
        }
        clut[k + 1] = pack(sum.map(|s| (s + n / 2) / n));
        for &(c, _) in b {
            of.insert(c, (k + 1) as u8);
        }
    }
    let idx = px.iter().map(|c| if *c == 0 { 0 } else { of.get(c).copied().unwrap_or(0) }).collect();
    (clut, idx)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The pictures keep their sizes without data (the front end's VRAM loads, which are state, never depend on it).
    #[test]
    fn blank_pictures_have_the_sizes() {
        let a = Br1Art::default();
        let r = |t: &[u8], clut| crate::formats::vram::Vram::tim_rects(t, clut, Some((0, 0)));
        assert_eq!(a.select.len(), 4);
        assert_eq!(r(&a.select[0], None), [None, Some([0, 0, W, H])]);
        assert_eq!(r(&a.faces, None), [None, Some([0, 0, 4 * FACE_W, FACE_H])]);
        assert_eq!(r(&a.vs[3], Some((0, 501))), [Some([0, 501, 256, 1]), Some([0, 0, VS_W / 2, VS_H])]);
    }

    /// Median cut: few colours map exactly, many fit in 255 entries, transparent stays index 0.
    #[test]
    fn quantize_keeps_few_colours() {
        let px: Vec<u16> = (0..64).map(|k| if k % 8 == 0 { 0 } else { 0x8000 | ((k as u16 % 5) * 3) }).collect();
        let (clut, idx) = quantize(&px);
        for (c, i) in px.iter().zip(&idx) {
            assert_eq!(if *c == 0 { 0 } else { clut[*i as usize] }, *c);
        }
        let many: Vec<u16> = (0..20000u32).map(|k| 0x8000 | (k * 7919 % 0x7FFF) as u16).collect();
        let (_, idx) = quantize(&many);
        assert!(idx.iter().all(|&i| i != 0));
    }
}
