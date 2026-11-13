//! The effect layer's draw-list unit (engine addition: the PS1 GPU's quad as the renderer takes it).
//!
//! State: none

use crate::output::layout::Clip;
use crate::output::sprite::Sheet;

/// A PS1 GPU quad (POLY_F4 / POLY_FT4) for the effect layer: corners in clip space (order 0 1 2 3 = TL TR BL BR
/// as the PS1 builds them), texel coordinates inside the texture page, colour (128 = texture as is), tpage / clut
/// words as in the primitive, semi-transparency and the blend mode of the tpage (bits 5-6).
#[derive(Clone, Copy, Debug)]
pub struct PsxQuad {
    pub pos: [[f32; 4]; 4],
    pub uv: [[f32; 2]; 4],
    /// colour per corner (flat = all equal; POLY_G4 gradients)
    pub rgb: [[u8; 3]; 4],
    pub tpage: u16,
    pub clut: u16,
    pub textured: bool,
    pub semi: bool,
    /// textured from the sprite atlas (engine addition): `uv` are texels of this sheet, `tpage` holds only the
    /// blend bits; None = `tpage` / `clut` address the VRAM copy
    pub sheet: Option<Sheet>,
    /// the scissor (engine addition, `Anchor::clip`)
    pub clip: Clip,
}
