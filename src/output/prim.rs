//! The shared 2D primitive: PS1 quads in the 512×480 HUD space.
//!
//! State: vocabulary: `Prim` is serialised where state embeds it (the HUD, the effects)
//!
//! The shared 2D primitive: PS1 quads (FT4 / F4 / G4) in the game's fixed 512×480 HUD space (interlaced layout),
//! as the HUD, the fonts, the menus and the front end build them (`output::layout::to_quads` maps them to the window).

use crate::output::layout::Anchor;
use crate::output::sprite::{Blend, Pal, Sheet, Sprite};
use serde::{Deserialize, Serialize};

/// One primitive in HUD space (x right, y down, 512×480).
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub struct Prim {
    pub xy: [[f32; 2]; 4],
    pub uv: [[f32; 2]; 4],
    pub rgb: [[u8; 3]; 4],
    pub tpage: u16,
    pub clut: u16,
    pub textured: bool,
    /// semi-transparent (texels with the STP bit blend with the tpage mode; untextured: the whole prim)
    pub semi: bool,
    /// textured from the sprite atlas: `uv` are texels of this sheet and `tpage` holds only the blend bits
    /// (menus, story and effects still address VRAM by `tpage` / `clut`)
    pub sheet: Option<Sheet>,
    /// where the prim sits in a window wider or taller than 4:3 (engine addition, `output::layout`)
    pub at: Anchor,
}

impl Prim {
    /// Semi-transparent with this blend mode.
    pub fn blend(self, b: Blend) -> Self {
        Prim { semi: true, tpage: (self.tpage & !0x60) | b.abr(), ..self }
    }

    /// Placed by this anchor.
    pub fn at(self, at: Anchor) -> Self {
        Prim { at, ..self }
    }
}

pub(crate) fn ft4(x: i32, y: i32, w: i32, h: i32, uv: (i32, i32, i32, i32), tpage: u16, clut: u16) -> Prim {
    let (x0, y0, x1, y1) = (x as f32, y as f32, (x + w) as f32, (y + h) as f32);
    let (u, v, uw, vh) = (uv.0 as f32, uv.1 as f32, uv.2 as f32, uv.3 as f32);
    Prim {
        xy: [[x0, y0], [x1, y0], [x0, y1], [x1, y1]],
        uv: [[u, v], [u + uw, v], [u, v + vh], [u + uw, v + vh]],
        rgb: [[128; 3]; 4],
        tpage,
        clut,
        textured: true,
        semi: false,
        sheet: None,
        at: Anchor::CORE,
    }
}

/// A sprite of the atlas drawn at (x, y) with size (w, h) and palette `pal`.
pub(crate) fn spr(x: i32, y: i32, w: i32, h: i32, s: Sprite, pal: Pal) -> Prim {
    Prim { sheet: Some(s.sheet(pal)), ..ft4(x, y, w, h, s.uv, 0, 0) }
}

pub(crate) fn f4(x: i32, y: i32, w: i32, h: i32, rgb: [u8; 3]) -> Prim {
    let (x0, y0, x1, y1) = (x as f32, y as f32, (x + w) as f32, (y + h) as f32);
    Prim {
        xy: [[x0, y0], [x1, y0], [x0, y1], [x1, y1]],
        uv: [[0.0; 2]; 4],
        rgb: [rgb; 4],
        tpage: 0,
        clut: 0,
        textured: false,
        semi: false,
        sheet: None,
        at: Anchor::CORE,
    }
}

/// The full-screen subtract fade (a semi-transparent F4 of (v, v, v) with tpage 0x40, mode 2 = B − F, over 512×480).
pub(crate) fn fade_sub(v: u8) -> Prim {
    Prim { semi: true, tpage: 0x40, ..f4(0, 0, 512, 480, [v; 3]) }
}

pub(crate) fn g4(x: i32, y: i32, w: i32, h: i32, top: [u8; 3], bottom: [u8; 3]) -> Prim {
    Prim { rgb: [top, top, bottom, bottom], ..f4(x, y, w, h, top) }
}

pub(crate) fn clut(x: u16, y: u16) -> u16 {
    (y << 6) | (x / 16)
}
