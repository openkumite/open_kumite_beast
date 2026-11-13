//! The text system: the game's text renderer and menu fonts.
//!
//! State: none (the fonts are static data: `install`ed from the game data's metrics table)
//!
//! The text system: the game's text renderer (with its entry points)
//! and the menu fonts, drawn as named sprites of the sprite atlas (`output/sprite.rs`) from the
//! textures of CMN/INIT.DAT with palettes picked from small tables. The glyphs' widths and the big font's cells come from
//! the converted metrics table (`METRICS`, `Fonts::from_metrics`), the other cells and the glyph map from the
//! renderer's own arithmetic.

use crate::output::prim::{Prim, spr};
use crate::output::sprite::{Blend, Page, PalSet, Sprite};

// ------------------------------------------------------------------ sheets and palettes

/// The HUD pages of CMN/INIT.DAT (4-bit): entry 1 (frames, lamps, labels, big font; tpage 0x0E), entry 0 (small
/// font and digits; 0x0F), entry 25 (the OK burst; 0x18), entry 24 (menu font and its icon page; 0x5D / 0x5E,
/// the 8×8 menu font 0x3E).
pub(crate) const HUD_PAGE: Page = Page::p4(896, 0);
pub(crate) const FONT_PAGE: Page = Page::p4(960, 0);
pub(crate) const OK_PAGE: Page = Page::p4(512, 256);
const MENU_PAGE: Page = Page::p4(832, 256);
const MENU_PAGE2: Page = Page::p4(896, 256);

/// The palette blocks (16 rows from y = 480): INIT.DAT entries 2 (fonts 0 / 2), 3 (font 3, frames), 4 (labels,
/// gauge) and 7 (menu fonts).
pub(crate) const FONT_PALS: PalSet = PalSet::new(496, 480);
pub(crate) const HUD_PALS: PalSet = PalSet::new(480, 480);
pub(crate) const LABEL_PALS: PalSet = PalSet::new(464, 480);
const MENU_PALS: PalSet = PalSet::new(352, 480);

/// Font palettes by colour (`#k` escapes in strings pick them by number as well).
pub mod pal {
    /// fonts 0 and 2
    pub const GREY: u16 = 0;
    pub const ORANGE: u16 = 1;
    pub const BLUE: u16 = 2;
    pub const WHITE: u16 = 3;
    pub const YELLOW: u16 = 4;
    pub const CYAN: u16 = 5;
    pub const RED: u16 = 6;
    pub const GREEN: u16 = 7;
    /// font 3
    pub const BIG_ORANGE: u16 = 8;
    pub const BIG_BLUE: u16 = 0xA;
    pub const BIG_YELLOW: u16 = 0xE;
}

// ------------------------------------------------------------------ text

struct Glyph {
    u: u8,
    v: u8,
    w: u8,
    adv: u8,
}

pub(crate) struct Font {
    glyphs: Vec<Option<Glyph>>, // index = char − 0x20
    space: i32,
    h: i32,
    spacing: i32,
    page: Page,
    pals: PalSet,
}

pub struct Fonts {
    pub(crate) small: Font,  // font 0: 8×8 (names)
    pub(crate) digits: Font, // font 2: 16×24 (combo digits, the attract ranking)
    pub(crate) big: Font,    // font 3: 24×32 (round messages)
    menu: Font,              // font 4: 15×15 menu font, never doubled
}

/// The fonts' metrics table of `tables.okt`: font 0's widths from ' ' at +0, font 2's by glyph index at
/// +0xC0, font 3's widths, cell x and cell y by glyph index at +0xF4 / +0x124 / +0x154, the menu font's widths from
/// ' ' at +0x1E4.
pub const METRICS: &str = "font.metrics";
const SMALL_W: usize = 0;
const DIGITS_W: usize = 0xC0;
const BIG_W: usize = 0xF4;
const BIG_X: usize = 0x124;
const BIG_Y: usize = 0x154;
const MENU_W: usize = 0x1E4;

/// The installed fonts (`install`; built from the game data's metrics).
static FONTS: std::sync::RwLock<Option<(Vec<u8>, &'static Fonts)>> = std::sync::RwLock::new(None);

/// The fonts (static data, not part of any state): the installed ones, without them fonts without glyphs (nothing is
/// drawn, every width 0).
pub fn fonts() -> &'static Fonts {
    static EMPTY: std::sync::OnceLock<Fonts> = std::sync::OnceLock::new();
    match *FONTS.read().unwrap_or_else(|e| e.into_inner()) {
        Some((_, f)) => f,
        None => EMPTY.get_or_init(|| Fonts::from_metrics(&[])),
    }
}

/// Installs the fonts of the metrics table's bytes `m` (`METRICS`; by `Assets::br2_tables`); the same bytes again is a
/// no-op, other bytes (new game data) replace them (the old ones leaked: once a data change).
pub fn install(m: &[u8]) {
    let mut f = FONTS.write().unwrap_or_else(|e| e.into_inner());
    if f.as_ref().is_some_and(|(b, _)| b == m) {
        return;
    }
    *f = Some((m.to_vec(), Box::leak(Box::new(Fonts::from_metrics(m)))));
}

/// Fonts 2 and 3's glyph of character `c` (by jump table): the
/// punctuation the font has by case, digits from 0x26, letters from 0 (lower case as upper case); anything else falls
/// through the letters' subtraction and wraps (a byte), as the game's. Font 2's own cases: '"' 0x24 and '\'' the
/// cell (0xC0, 0x60) with the widths of glyph 0x30 (None: its fixed cell).
fn glyph_index(c: u8, digits: bool) -> Option<u8> {
    Some(match c {
        b'.' => 0x1A,
        b'!' => 0x1B,
        b'?' => 0x1C,
        b'&' => 0x1D,
        b'-' => 0x1E,
        b'*' => 0x1F,
        b'<' => 0x20,
        b'>' => 0x22,
        b'_' => 0x23,
        b'~' => 0x25,
        0x7F => 0x21,
        b'"' if digits => 0x24,
        b'\'' if digits => return None,
        b'0'..=b'9' => c - b'0' + 0x26,
        0x61.. => c - 0x61,
        _ => c.wrapping_sub(0x41),
    })
}

impl Fonts {
    /// The fonts of the metrics table's bytes `m` (`METRICS`; a byte past its end reads 0). The cells: font 0's 8×8
    /// from ' ' in rows of 16, font 2's 16×24 from u 0x80 in rows of 8, font 3's from its
    /// cell tables, the menu font's 16×16 from ' ' in rows of 16 from v 0xC0 (the menu advance its width
    /// + 1) and from 0x60 the icon cells (16 wide, from v 0xD8).
    pub fn from_metrics(m: &[u8]) -> Self {
        let at = |o: usize| m.get(o).copied().unwrap_or(0);
        let font = |space, h, spacing, page, pals, glyph: &dyn Fn(u8) -> Option<Glyph>| Font {
            glyphs: (0x20..0x80u8).map(|c| if m.is_empty() || c == b' ' { None } else { glyph(c) }).collect(),
            space,
            h,
            spacing,
            page,
            pals,
        };
        let small = |c: u8| {
            let i = c - 0x20;
            Some(Glyph { u: 8 * (i & 15), v: 8 * (i >> 4), w: at(SMALL_W + i as usize), adv: at(SMALL_W + i as usize) })
        };
        let digits = |c: u8| {
            let Some(i) = glyph_index(c, true) else { return Some(Glyph { u: 0xC0, v: 0x60, w: at(DIGITS_W + 0x30), adv: at(DIGITS_W + 0x30) }) };
            let w = at(DIGITS_W + i as usize);
            Some(Glyph { u: 0x80 | (i & 7) << 4, v: ((i as u32 >> 3) * 24) as u8, w, adv: w })
        };
        let big = |c: u8| {
            let i = glyph_index(c, false)? as usize;
            Some(Glyph { u: at(BIG_X + i), v: at(BIG_Y + i), w: at(BIG_W + i), adv: at(BIG_W + i) })
        };
        let menu = |c: u8| {
            if c >= 0x60 {
                let n = c - 0x60;
                return Some(Glyph { u: (n & 15) << 4, v: 0xD8 + (n & 0xF0), w: 16, adv: 16 });
            }
            let i = c - 0x20;
            let w = at(MENU_W + i as usize);
            Some(Glyph { u: (i & 15) << 4, v: 0xC0 + (i & 0xF0), w, adv: w.saturating_add(1) })
        };
        Self {
            small: font(4, 8, 0, FONT_PAGE, FONT_PALS, &small),
            digits: font(7, 24, 0, FONT_PAGE, FONT_PALS, &digits),
            big: font(15, 32, 1, HUD_PAGE, HUD_PALS, &big),
            menu: font(8, 15, 0, MENU_PAGE, MENU_PALS, &menu),
        }
    }

    /// Font 4: 15-px glyphs on tpage 0x5D, CLUT (352, 480+pal); semi + raw in the menus.
    pub fn menu_text(&self, out: &mut Vec<Prim>, s: &str, x: i32, y: i32, pal: u16) {
        self.menu_glyphs(out, s, x, y, pal, self.menu.h, None);
    }

    /// Font 4 with 30-line quads (+0xC = 0x1E: the 15-line glyphs stretched ×2).
    pub fn menu_text_tall(&self, out: &mut Vec<Prim>, s: &str, x: i32, y: i32, pal: u16) {
        self.menu_glyphs(out, s, x, y, pal, 2 * self.menu.h, None);
    }

    /// `tall`: font 4 at a fixed pitch of 16 (cell 15 + 1); a digit narrower than the
    /// cell is centred in it ('1' at +4).
    pub fn menu_mono(&self, out: &mut Vec<Prim>, s: &str, x: i32, y: i32, pal: u16, tall: bool) {
        self.menu_glyphs(out, s, x, y, pal, if tall { 2 * self.menu.h } else { self.menu.h }, Some(16));
    }

    fn menu_glyphs(&self, out: &mut Vec<Prim>, s: &str, x: i32, y: i32, pal: u16, h: i32, pitch: Option<i32>) {
        let f = &self.menu;
        let mut cx = x;
        for c in s.chars() {
            if c == ' ' {
                cx += pitch.unwrap_or(f.space);
                continue;
            }
            // codes 0x60.. are 16×16 cells on the next page (tpage 0x5E: the ▲▼ arrows "p" / "q", the
            // pad icons), cell (c − 0x60) at u = 16·(n & 15), v = 0xD8 + (n & 0xF0), 15 px advance
            if c as u32 >= 0x60 && (c as u32) < 0x80 {
                let n = c as i32 - 0x60;
                let cell = Sprite::new(MENU_PAGE2, 16 * (n & 15), 0xD8 + (n & 0xF0), 16, f.h);
                out.push(spr(cx, y, 16, h, cell, f.pals.row(pal)).blend(Blend::Sub));
                cx += pitch.unwrap_or(15);
                continue;
            }
            if let Some(Some(g)) = f.glyphs.get((c as u32).wrapping_sub(0x20) as usize) {
                let dx = match pitch {
                    Some(p) if c.is_ascii_digit() => (p - g.adv as i32).max(0) / 2,
                    _ => 0,
                };
                out.push(spr(cx + dx, y, g.w as i32, h, f.cell(g, f.page), f.pals.row(pal)).blend(Blend::Sub));
                cx += pitch.unwrap_or(g.adv as i32);
            }
        }
    }

    /// The same glyph table on the next page (tpage 0x5E): the lowercase cells hold the pad button
    /// icons (r △, s ○, t ×, u □, v "R", w "L", x "1", y "2"; row 216: d e "R1", h i "L1" small).
    pub fn menu_icons(&self, out: &mut Vec<Prim>, s: &str, x: i32, y: i32, pal: u16) {
        let f = &self.menu;
        let mut cx = x;
        for c in s.chars() {
            if let Some(Some(g)) = f.glyphs.get((c as u32).wrapping_sub(0x20) as usize) {
                out.push(spr(cx, y, g.w as i32, f.h, f.cell(g, MENU_PAGE2), f.pals.row(pal)).blend(Blend::Sub));
                cx += g.adv as i32;
            }
        }
    }

    pub fn menu_width(&self, s: &str) -> i32 {
        let f = &self.menu;
        s.chars()
            .map(|c| {
                if c == ' ' {
                    f.space
                } else {
                    f.glyphs.get((c as u32).wrapping_sub(0x20) as usize).and_then(|g| g.as_ref()).map(|g| g.adv as i32).unwrap_or(0)
                }
            })
            .sum()
    }

    /// The 8×8 menu font on tpage 0x3E: ASCII 0x20..0x7F in rows of 32 from v = 192
    /// (u = (c & 31)·8), advance 9.
    pub fn menu_small(&self, out: &mut Vec<Prim>, s: &str, x: i32, y: i32, pal: u16) {
        let mut cx = x;
        for c in s.chars() {
            let c = c as i32;
            if (0x21..0x80).contains(&c) {
                let cell = Sprite::new(MENU_PAGE2, (c & 31) * 8, 192 + 8 * ((c - 0x20) >> 5), 8, 8);
                out.push(spr(cx, y, 8, 8, cell, MENU_PALS.row(pal)).blend(Blend::Add));
            }
            cx += 9;
        }
    }

    /// Font 0 (8×8, interlaced: glyphs 15 lines tall), as the HUD names and "PRESS START" use it.
    pub fn small_text(&self, out: &mut Vec<Prim>, s: &str, x: i32, y: i32, pal: u16) {
        self.text(out, &self.small, s, x, y, pal);
    }

    /// Font 0 monospaced, every character (space too) an 8-wide cell.
    pub fn small_mono(&self, out: &mut Vec<Prim>, s: &str, x: i32, y: i32, pal: u16) {
        let f = &self.small;
        for (k, c) in s.chars().enumerate() {
            if let Some(Some(g)) = f.glyphs.get((c as u32).wrapping_sub(0x20) as usize) {
                let cell = Sprite::new(f.page, g.u as i32, g.v as i32, 8, f.h);
                out.push(spr(x + 8 * k as i32, y, 8, 2 * f.h - 1, cell, f.pals.row(pal)));
            }
        }
    }

    /// Font 2 (16×24, tpage 0x0F, CLUT (496, 480+pal), glyphs 47 lines tall), opaque.
    pub fn mid_text(&self, out: &mut Vec<Prim>, s: &str, x: i32, y: i32, pal: u16) {
        self.text(out, &self.digits, s, x, y, pal);
    }

    /// Font 3 (24×32 big message font, interlaced) from `x`.
    pub fn big_text(&self, out: &mut Vec<Prim>, s: &str, x: i32, y: i32, pal: u16) {
        self.text(out, &self.big, s, x, y, pal);
    }

    /// Font 3 (24×32 big message font, interlaced) centred on `cx`.
    pub fn big_text_centered(&self, out: &mut Vec<Prim>, s: &str, cx: i32, y: i32, pal: u16) {
        let w = self.width(&self.big, s);
        self.text(out, &self.big, s, cx - w / 2, y, pal);
    }

    /// The text layout: proportional glyphs, `#d` palette escape, glyphs 2h−1 tall (interlaced).
    pub(crate) fn text(&self, out: &mut Vec<Prim>, font: &Font, s: &str, x: i32, y: i32, pal: u16) {
        let (mut cx, mut cy) = (x, y);
        let mut row = pal;
        let mut it = s.chars().peekable();
        while let Some(c) = it.next() {
            match c {
                ' ' => cx += font.space,
                '\n' => {
                    cx = 0;
                    cy += 2 * font.h;
                }
                '#' => match it.next() {
                    Some('n') => {
                        cx = 0;
                        cy += 2 * font.h;
                    }
                    Some(d) if d.is_ascii_digit() => row = (row & !0xF) + d as u16 - '0' as u16,
                    _ => {}
                },
                _ => {
                    let c = if font.big_upper() { c.to_ascii_uppercase() } else { c };
                    if let Some(Some(g)) = font.glyphs.get((c as u32).wrapping_sub(0x20) as usize) {
                        out.push(spr(cx, cy, g.w as i32, 2 * font.h - 1, font.cell(g, font.page), font.pals.row(row)));
                        cx += g.adv as i32 + font.spacing;
                    }
                }
            }
        }
    }

    pub(crate) fn width(&self, font: &Font, s: &str) -> i32 {
        s.chars()
            .map(|c| {
                if c == ' ' {
                    font.space
                } else {
                    let c = if font.big_upper() { c.to_ascii_uppercase() } else { c };
                    font.glyphs.get((c as u32).wrapping_sub(0x20) as usize).and_then(|g| g.as_ref()).map(|g| g.adv as i32 + font.spacing).unwrap_or(0)
                }
            })
            .sum()
    }
}

impl Font {
    /// A glyph's cell (`page`: the font's own or the icon page next to it).
    fn cell(&self, g: &Glyph, page: Page) -> Sprite {
        Sprite::new(page, g.u as i32, g.v as i32, g.w as i32, self.h)
    }

    fn big_upper(&self) -> bool {
        self.h >= 24
    }
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use super::*;

    /// Every glyph of the four fonts in order: u, v, w, advance (four 0xFF without one).
    fn bytes(f: &Fonts) -> Vec<u8> {
        [&f.small, &f.digits, &f.big, &f.menu]
            .iter()
            .flat_map(|font| font.glyphs.iter().flat_map(|g| g.as_ref().map_or([0xFF; 4], |g| [g.u, g.v, g.w, g.adv])))
            .collect()
    }

    /// The fonts from the test data's metrics (data-gated) are the ones checked against the glyphs the engine measured
    /// from the game's screens before (every one of those the same; the others the code's), by the CRC-32 of
    /// `bytes` (no font data here).
    #[test]
    fn metrics_fonts_as_before() {
        let Some(t) = crate::formats::tables::export_tables() else {
            eprintln!("skipped: no okb_export");
            return;
        };
        let f = Fonts::from_metrics(t.table(METRICS).bytes());
        assert_eq!(crate::base::hash::crc32(&bytes(&f)), 0x860E_2BFB);
    }

    /// Without metrics no font has a glyph: nothing is drawn, every width is 0.
    #[test]
    fn no_metrics_no_glyphs() {
        let f = Fonts::from_metrics(&[]);
        assert!(bytes(&f).iter().all(|&b| b == 0xFF));
        assert_eq!(f.menu_width("AB"), 0);
    }
}
