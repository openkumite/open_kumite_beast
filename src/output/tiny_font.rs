//! A small bitmap font for what is drawn before any game data is there: the setup screen (engine addition).
//!
//! State: none
//!
//! The engine's own 5 × 9 glyphs of printable ASCII (`tiny_font.txt`: one line per character, its rows top to bottom,
//! `#` set; 7 rows to the baseline, 2 below it for descenders), in a cell of 6 × 11. Text is drawn as one rectangle
//! per run of set pixels in a row (`runs`), so it needs no texture.

use std::sync::OnceLock;

/// A glyph's width and height in its pixels; the cell's advance and line height.
pub const W: u32 = 5;
pub const H: u32 = 9;
pub const ADVANCE: u32 = 6;
pub const LINE: u32 = 11;

const SRC: &str = include_str!("tiny_font.txt");

/// The glyphs of ' ' ..= '~': per row, bit 4 = the leftmost pixel.
fn glyphs() -> &'static [[u8; H as usize]; 95] {
    static G: OnceLock<[[u8; H as usize]; 95]> = OnceLock::new();
    G.get_or_init(|| {
        let mut g = [[0u8; H as usize]; 95];
        for line in SRC.lines() {
            let mut cs = line.chars();
            let (Some(c), Some(' ')) = (cs.next(), cs.next()) else { continue };
            let Some(k) = (c as usize).checked_sub(0x20).filter(|&k| k < 95) else { continue };
            for (r, row) in cs.as_str().split('|').take(H as usize).enumerate() {
                g[k][r] = row.bytes().take(W as usize).fold(0, |a, b| a << 1 | (b == b'#') as u8) << (W as usize - row.len().min(W as usize));
            }
        }
        g
    })
}

/// The runs of set pixels of `c` (anything outside printable ASCII as '?'): (column, row, length) in glyph pixels.
pub fn runs(c: char) -> impl Iterator<Item = (u32, u32, u32)> {
    let k = if (' '..='~').contains(&c) { c as usize - 0x20 } else { '?' as usize - 0x20 };
    let g = glyphs()[k];
    (0..H).flat_map(move |r| {
        let bits = g[r as usize];
        let mut out = Vec::new();
        let mut x = 0;
        while x < W {
            if bits >> (W - 1 - x) & 1 == 1 {
                let x0 = x;
                while x < W && bits >> (W - 1 - x) & 1 == 1 {
                    x += 1;
                }
                out.push((x0, r, x - x0));
            } else {
                x += 1;
            }
        }
        out
    })
}

/// The width of `s` in glyph pixels (no trailing gap).
pub fn width(s: &str) -> u32 {
    (s.chars().count() as u32 * ADVANCE).saturating_sub(ADVANCE - W)
}

#[cfg(test)]
mod tests {
    #[test]
    fn every_printable_character_has_a_glyph() {
        let g = super::glyphs();
        for (k, rows) in g.iter().enumerate() {
            let c = (k as u8 + 0x20) as char;
            assert_eq!(c == ' ', rows.iter().all(|&r| r == 0), "{c:?}");
        }
        // 'I' is three runs of the top bar, the stem, the bottom bar
        let runs: Vec<_> = super::runs('I').collect();
        assert_eq!(runs[0], (1, 0, 3));
        assert_eq!(runs[1], (2, 1, 1));
        assert_eq!(*runs.last().unwrap(), (1, 6, 3));
        assert_eq!(super::runs('\u{e9}').count(), super::runs('?').count());
        assert_eq!(super::width("AB"), 11);
    }
}
