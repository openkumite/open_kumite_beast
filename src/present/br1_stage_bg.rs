//! An imported stage's background: the panorama strip and the colour bands behind the scene (engine addition, the
//! the import S8).
//!
//! State: presentation (reads the state, never writes it)
//!
//! Imported draws no sky model: behind the 3-D scene lie a strip of 16×16 sprites (4 rows of the stage's BGD map,
//! 64 cells = 1024 pixels for a full turn) scrolled by the camera, and flat or Gouraud bands above and below it
//! (in its 320×240 screen): the strip's left edge at `(yaw & 0xFFF) >> 2`, its bottom at `a2 − c` with
//! `a2 = (−pitch · 16) / 45 + C.1E` (the division toward zero, `·0xB60B60B7 >> 5`) and `c` the camera's height
//! `>> 6` rounded toward −∞ (`+0x3F` when negative), its top `C.26` above; over the strip (from `C.20` above its
//! bottom) a band of `C.24` rows from colour 1 down to colour 2, above it colour 0 up to the screen's top; under the
//! strip (from `C.22` below its bottom) a band of `C.25` rows from colour 3 to colour 4. A strip whose bottom is not
//! below the screen's top (and no band reaching down) is not drawn at all. `C` = the stage's 44-byte record (colours
//! 0..4 as 0x00BBGGRR words at +0, +4, …, +0x10).
//!
//! The engine draws it as a model in the imported game's screen units behind everything (depth at the far end), its
//! picture the stage's panorama as ok_br1_converter decoded it (`<code>_sky` in the stage's glb, the BGD map's cells
//! with the stage's CLUT row 0), from the main game's camera: its yaw and height in place of the imported game's, and
//! for the pitch term the row of its own horizon (`bottom`; the imported game's term is its projection's horizon); 320
//! units across the 4:3 view as on the PS1 screen (a wider view sees more of the strip), 240 down. The strip's texels
//! are drawn opaque (ok_br1_converter's picture keeps no transparent texel 0, the cells' semi-transparency bit is not
//! used).

use crate::formats::gltf_model::{Image, ModelData, Node, Prim, Vertex};
use glam::{Mat4, Vec4};

/// The strip's copies side by side in the model (enough for the widest view: 1024 units each).
const COPIES: i32 = 3;
/// The sky colour band's top (far above any screen).
const SKY_TOP: f32 = -4096.0;

/// An imported stage's background record (by G.94) and its panorama.
pub struct Background {
    pub cfg: [u8; 44],
    pub image: Image,
}

impl Background {
    fn colour(&self, k: usize) -> [f32; 4] {
        let c = &self.cfg[4 * k..4 * k + 3];
        // a flat / Gouraud colour (the model shader's factor of its white texture: colour / 255)
        [c[0] as f32 / 255.0, c[1] as f32 / 255.0, c[2] as f32 / 255.0, 1.0]
    }
    fn i16(&self, o: usize) -> i32 {
        i16::from_le_bytes([self.cfg[o], self.cfg[o + 1]]) as i32
    }
    fn u8(&self, o: usize) -> i32 {
        self.cfg[o] as i32
    }

    /// The screen's clear colour (the record's +0x14, +0x15, +0x16).
    pub fn clear(&self) -> [u8; 3] {
        [self.cfg[0x14], self.cfg[0x15], self.cfg[0x16]]
    }

    /// The model: the strip (`COPIES` periods from x 0, its bottom at y 0) with the bands at their places relative
    /// to the strip's bottom, in imported screen units (y down).
    pub fn model(&self) -> ModelData {
        let mut m = ModelData::empty();
        m.nodes.push(Node { name: "br1_bg".into(), parent: None, t: glam::Vec3::ZERO, r: glam::Quat::IDENTITY, s: glam::Vec3::ONE });
        let strip = self.u8(0x26) as f32;
        let period = self.image.width as f32;
        let w = period * COPIES as f32;
        let quad = |m: &mut ModelData, x: [f32; 2], y: [f32; 2], c: [[f32; 4]; 2], uv: Option<[f32; 2]>| {
            let base = m.vertices.len() as u32;
            for (k, (xx, yy)) in [(x[0], y[0]), (x[1], y[0]), (x[0], y[1]), (x[1], y[1])].into_iter().enumerate() {
                let u = uv.map_or([0.0; 2], |[u0, u1]| [if k & 1 == 0 { u0 } else { u1 }, if k < 2 { 0.0 } else { 1.0 }]);
                m.vertices.push(Vertex {
                    pos: [xx, yy, 0.0],
                    normal: [0.0, 1.0, 0.0],
                    uv: u,
                    color: if k < 2 { c[0] } else { c[1] },
                    joints: [0; 4],
                    weights: [1.0, 0.0, 0.0, 0.0],
                });
            }
            let first = m.indices.len() as u32;
            m.indices.extend([base, base + 2, base + 1, base + 1, base + 2, base + 3]);
            let p = Prim {
                first_index: first,
                count: 6,
                texture: uv.map(|_| 0),
                hidden: false,
                lit: false,
                wall: None,
                shadow: false,
                over: false,
                ot: None,
                semi: false,
                node: 0,
            };
            m.prims.push(p);
        };
        // the strip: one quad per period, the picture repeated
        if strip > 0.0 {
            m.images.push(Image { name: self.image.name.clone(), width: self.image.width, height: self.image.height, rgba: self.image.rgba.clone() });
            for k in 0..COPIES {
                let x0 = k as f32 * period;
                quad(&mut m, [x0, x0 + period], [-strip, 0.0], [[1.0; 4]; 2], Some([0.0, 1.0]));
            }
        }
        // over the strip: band C.24 rows from colour 1 to 2 ending C.20 above the strip's bottom, colour 0 above it
        let top = self.i16(0x20) as f32;
        let b24 = self.u8(0x24) as f32;
        if b24 > 0.0 {
            quad(&mut m, [0.0, w], [top - b24, top], [self.colour(1), self.colour(2)], None);
        }
        quad(&mut m, [0.0, w], [SKY_TOP, top - b24], [self.colour(0); 2], None);
        // under it: band C.25 rows from colour 3 to 4, C.22 below the strip's bottom
        let b25 = self.u8(0x25) as f32;
        if b25 > 0.0 {
            let y0 = self.i16(0x22) as f32;
            quad(&mut m, [0.0, w], [y0, y0 + b25], [self.colour(3), self.colour(4)], None);
        }
        m
    }

    /// The strip's bottom in imported screen units for a camera of height `cam_y` whose horizon (the
    /// vanishing line of the floor's plane) lies at row `horizon` of the imported game's 240: imported moves the strip
    /// with its pitch p by `(−p · 16) / 45`, which is its projection's horizon (`H · tan p` for its distance H ≈ 232)
    /// from the screen's centre row 120, so that the strip stays at `C.1E − 120` from the scene's horizon; the engine
    /// keeps it there under the main game's camera, whose projection differs.
    pub fn bottom(&self, horizon: f32, cam_y: i32) -> i32 {
        let a2 = horizon.round() as i32 - 120 + self.i16(0x1E);
        let c = if cam_y < 0 { (cam_y + 0x3F) >> 6 } else { cam_y >> 6 };
        a2 - c
    }

    /// Whether the strip's draw draws anything for a strip bottom `t3`.
    pub fn shown(&self, t3: i32) -> bool {
        let reach = self.i16(0x22) + self.u8(0x25);
        if reach > 0 { -reach < t3 } else { t3 > 0 }
    }

    /// The view-projection of the model: the imported game's 320×240 screen over the 4:3 view (`aspect` = the 3-D view's,
    /// `layout::Viewport::aspect3d`: a wider one sees more), the depth at the far end; the model scrolled by `yaw` with
    /// its strip's bottom at `t3`.
    pub fn transform(&self, aspect: f32, yaw: i16, t3: i32) -> Mat4 {
        let kx = (4.0 / 3.0) / aspect / 160.0;
        let proj = Mat4::from_cols(
            Vec4::new(kx, 0.0, 0.0, 0.0),
            Vec4::new(0.0, -1.0 / 120.0, 0.0, 0.0),
            Vec4::new(0.0, 0.0, 0.0, 0.0),
            Vec4::new(-160.0 * kx, 1.0, 0.9999, 1.0),
        );
        // the strip's left edge at −scroll (0..1023), one period further left for a wide view
        let scroll = ((yaw as i32 & 0xFFF) >> 2) as f32 + self.image.width as f32;
        proj * Mat4::from_translation(glam::Vec3::new(-scroll, t3 as f32, 0.0))
    }
}
