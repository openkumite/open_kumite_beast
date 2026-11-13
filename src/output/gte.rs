//! The GTE projection the effects draw with.
//!
//! State: none
//!
//! The GTE projection the effects draw with (`hit_effects::Fx::quads`, the effect models, the drive grid): RTPS onto the
//! 640×480 interlaced display and the OT slot depth.

use glam::{Mat4, Vec4};

/// Projected point: screen position in display pixels of the 640×480 interlaced mode (origin at the centre,
/// y down), otz = SZ / 4.
pub struct Proj {
    pub sx: f32,
    pub sy: f32,
    pub otz: i32,
}

/// The GTE as the effects see it: the game camera's view (game units) and H = 360 on the logical 320×240 screen,
/// scaled ×2 by the screen-scale matrix for the 640×480 interlaced display (the mode in which the effect sprites
/// are square).
pub struct Gte {
    pub view: Mat4,
    pub proj: Mat4,
    pub h: f32,
    /// the 3D view's field (aspect_x, gain_y), `Viewport::fov3d`
    pub fov: [f32; 2],
    /// camera pitch in 4096 units (kind-6 floor puffs)
    pub pitch: i32,
}

impl Gte {
    pub fn new(view: Mat4, proj: Mat4, fov: [f32; 2], pitch: i32) -> Self {
        Self { view, proj, h: 720.0, fov, pitch }
    }

    pub fn rtps(&self, p: [i32; 3]) -> Option<Proj> {
        let v = self.view * Vec4::new(p[0] as f32, p[1] as f32, p[2] as f32, 1.0);
        if v.z <= 1.0 {
            return None;
        }
        Some(Proj { sx: self.h * v.x / v.z, sy: self.h * v.y / v.z, otz: (v.z / 4.0) as i32 })
    }

    /// The depth an effect quad in OT slot `slot` is tested at against the fighters (engine addition: the OT as a depth
    /// test). Fighter polygons are linked at slot 0x10 + avg SZ / 4 (the OT base M.8 + 0x40), effects at
    /// OT[otz] (otz = SZ / 4, or a fixed slot), and the higher slot is drawn first: a quad covers a fighter pixel whose
    /// SZ is at least 4 · (slot − 0x10). Slots up to 0x10 are in front of every fighter.
    pub fn slot_depth(&self, slot: i32) -> f32 {
        let sz = 4 * (slot - 0x10);
        if sz <= 0 {
            return 0.0;
        }
        let c = self.proj * Vec4::new(0.0, 0.0, sz as f32, 1.0);
        (c.z / c.w).clamp(0.0, 1.0)
    }

    pub(crate) fn clip(&self, xy: [f32; 2], z: f32) -> [f32; 4] {
        [xy[0] / (240.0 * self.fov[0]), -xy[1] / 240.0 * self.fov[1], z, 1.0]
    }
}
