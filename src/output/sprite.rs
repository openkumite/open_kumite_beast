//! The 2D sprite vocabulary: texture pages, palettes, sheets, named pictures, blend modes.
//!
//! Engine addition.
//! State: vocabulary: serialised where state embeds it
//!
//! The 2D sprite vocabulary (engine addition): texture pages, palettes, sheets, named pictures and the GPU's blend
//! modes, as the draw lists name them. The renderer's atlas (`render::atlas_cache`) bakes what they address.

use serde::{Deserialize, Serialize};

/// Texel depth of a page.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Depth {
    P4,
    P8,
    Direct,
}

/// A 256×256-texel texture page of the loaded data: origin in halfwords and texel depth.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Page {
    pub x: u16,
    pub y: u16,
    pub depth: Depth,
}

impl Page {
    pub const fn p4(x: u16, y: u16) -> Self {
        Self { x, y, depth: Depth::P4 }
    }

    /// The GPU tpage word of this page (bits 0-3 x / 64, bit 4 y / 256, bits 7-8 depth; blend bits clear).
    pub fn tpage(self) -> u16 {
        (self.x / 64) | (self.y / 256) << 4 | (self.depth as u16) << 7
    }
}

/// A palette (16 or 256 colours in a row) of the loaded data.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Pal {
    pub x: u16,
    pub y: u16,
}

impl Pal {
    /// The GPU CLUT word of this palette.
    pub fn clut(self) -> u16 {
        (self.y << 6) | (self.x / 16)
    }
}

/// A block of palettes stacked one per row (the HUD's CLUT images: 16 rows from y = 480).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct PalSet {
    pub x: u16,
    pub y: u16,
}

impl PalSet {
    pub const fn new(x: u16, y: u16) -> Self {
        Self { x, y }
    }

    pub fn row(self, k: u16) -> Pal {
        Pal { x: self.x, y: self.y + k }
    }
}

/// A palette cycle by frame counter: row `first + ((cnt >> shift) & (n − 1))` (`n` a power of two).
#[derive(Clone, Copy, Debug)]
pub struct PalCycle {
    pub first: u16,
    pub n: u16,
    pub shift: u32,
}

impl PalCycle {
    pub fn row(&self, cnt: i32) -> u16 {
        self.first + ((cnt >> self.shift) as u16 & (self.n - 1))
    }
}

/// A page decoded with one palette (direct pages ignore it): what a textured quad samples.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Sheet {
    pub page: Page,
    pub pal: Pal,
}

/// A named picture: a texel rect (u, v, w, h) of a page, drawn with a palette chosen at the draw.
#[derive(Clone, Copy, Debug)]
pub struct Sprite {
    pub page: Page,
    pub uv: (i32, i32, i32, i32),
}

impl Sprite {
    pub const fn new(page: Page, u: i32, v: i32, w: i32, h: i32) -> Self {
        Self { page, uv: (u, v, w, h) }
    }

    pub fn sheet(&self, pal: Pal) -> Sheet {
        Sheet { page: self.page, pal }
    }
}

/// How a semi-transparent quad mixes with the screen (B = screen, F = quad): the GPU's four modes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Blend {
    /// B/2 + F/2
    Avg,
    /// B + F
    Add,
    /// B − F
    Sub,
    /// B + F/4
    Quarter,
}

impl Blend {
    /// The mode's tpage bits (5-6).
    pub fn abr(self) -> u16 {
        (self as u16) << 5
    }
}
