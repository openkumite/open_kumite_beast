//! The part of the PS1 VRAM the effects use, filled as the fight loader does it; TIM files.
//!
//! State: static data (not serialised; re-attached by `rebind`)
//!
//! The part of the PS1 VRAM the effects use (1024×512 halfwords), filled the way the fight loader does it:
//! EFT.DAT entries 0x100..0x103, the fighters' <CHR>EFT.DAT entry 0.
//!
//! TIM: u32 id (0x10 .. 0x12), u32 flags (bits 0-2 colour mode, bit 3 = CLUT block present), then the CLUT
//! block and the pixel block, each `u32 length, u16 x, y, w, h` (w in halfwords) and the data.

/// Engine addition (the import S8): the imported game's effect pages (VRAM x, y of a 4-bit page, CMN/EFFECT.DAT) and where
/// the engine puts them in its effects' VRAM image (the PS1 frame buffer's area, which the image does not use; at
/// their own places they would cover the main game's pages): (from, to). `fight_data` loads them there,
/// `present::br1_fx` draws from there.
pub const BR1_PAGES: [((usize, usize), (usize, usize)); 3] = [((384, 0), (0, 0)), ((448, 0), (64, 0)), ((384, 256), (128, 0))];
/// The imported game's effect CLUTs (x 128..160, rows 480..496) moved to x 0.., rows 256.. of the engine's image.
pub const BR1_CLUT_FROM: (usize, usize) = (128, 480);
pub const BR1_CLUT_TO: (usize, usize) = (0, 256);

pub struct Vram {
    pub words: Vec<u16>,
}

impl Default for Vram {
    fn default() -> Self {
        Self { words: vec![0; 1024 * 512] }
    }
}

fn u16at(b: &[u8], o: usize) -> u16 {
    b.get(o..o + 2).map(|s| u16::from_le_bytes([s[0], s[1]])).unwrap_or(0)
}
fn u32at(b: &[u8], o: usize) -> u32 {
    b.get(o..o + 4).map(|s| u32::from_le_bytes([s[0], s[1], s[2], s[3]])).unwrap_or(0)
}

impl Vram {
    pub fn blit(&mut self, x: usize, y: usize, w: usize, h: usize, data: &[u8]) {
        for r in 0..h {
            for c in 0..w {
                let (vx, vy) = ((x + c) & 1023, (y + r) & 511);
                self.words[vy * 1024 + vx] = u16at(data, 2 * (r * w + c));
            }
        }
    }

    /// Upload a TIM; `clut_at` / `pix_at` override the stored positions (a TIM load with explicit coordinates).
    pub fn load_tim(&mut self, tim: &[u8], clut_at: Option<(usize, usize)>, pix_at: Option<(usize, usize)>) {
        let flags = u32at(tim, 4);
        let mut o = 8;
        let block = |o: usize, at: Option<(usize, usize)>, vram: &mut Vram| -> usize {
            let len = u32at(tim, o) as usize;
            let (x, y, w, h) = (u16at(tim, o + 4) as usize, u16at(tim, o + 6) as usize, u16at(tim, o + 8) as usize, u16at(tim, o + 10) as usize);
            let (x, y) = at.unwrap_or((x, y));
            if let Some(d) = tim.get(o + 12..o + 12 + 2 * w * h) {
                vram.blit(x, y, w, h, d);
            }
            len
        };
        if flags & 8 != 0 {
            o += block(o, clut_at, self);
        }
        block(o, pix_at, self);
    }

    /// The rects (x, y, w, h) `load_tim` with these arguments writes: the CLUT's, the pixels' (None = nothing).
    pub fn tim_rects(tim: &[u8], clut_at: Option<(usize, usize)>, pix_at: Option<(usize, usize)>) -> [Option<[usize; 4]>; 2] {
        let flags = u32at(tim, 4);
        let mut o = 8;
        let block = |o: usize, at: Option<(usize, usize)>| -> (usize, Option<[usize; 4]>) {
            let len = u32at(tim, o) as usize;
            let (x, y, w, h) = (u16at(tim, o + 4) as usize, u16at(tim, o + 6) as usize, u16at(tim, o + 8) as usize, u16at(tim, o + 10) as usize);
            let (x, y) = at.unwrap_or((x, y));
            (len, tim.get(o + 12..o + 12 + 2 * w * h).map(|_| [x, y, w, h]))
        };
        let mut clut = None;
        if flags & 8 != 0 {
            let (len, r) = block(o, clut_at);
            clut = r;
            o += len;
        }
        [clut, block(o, pix_at).1]
    }
}
