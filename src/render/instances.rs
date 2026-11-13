//! Instances: a model placed for one frame and its uniform.
//!
//! State: presentation (reads the state, never writes it)
//!
//! Instances: a model placed for one frame (`instance`) and its uniform (`InstUniform`), written by the `set_instance*`
//! family (plain, sub-colour, floor, semi-transparent, lit with a `Lighting` set, shadow) through `write_instance`.

use super::{Instance, MAX_JOINTS, Renderer};
use glam::Mat4;

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct InstUniform {
    view_proj: [[f32; 4]; 4],
    model: [[f32; 4]; 4],
    params: [f32; 4],
    /// `Lighting::dir` (the shadow pass: row 0 = shear and floor height)
    light: [[f32; 4]; 3],
    /// `Lighting::colour`
    colour: [[f32; 4]; 3],
    /// `Lighting::back`
    back: [f32; 4],
    /// a floor's depth row (`Renderer::set_instance_floor`)
    floor: [f32; 4],
    /// hidden-joint bit masks per vertex group (0 = main mesh, 1 = open hands); z: the floor's CLUT row mode
    hide: [u32; 4],
    joints: [[[f32; 4]; 4]; MAX_JOINTS],
}

/// A lit model's light set (`fighter_lighting::Lights` in glTF space, 1.0 = 0x1000): the GTE's light matrix (rows: the
/// directions of lights 1..3), colour matrix (rows R, G, B; columns lights 1..3) and back colour.
#[derive(Clone, Copy, Debug, Default)]
pub struct Lighting {
    pub dir: [[f32; 3]; 3],
    pub colour: [[f32; 3]; 3],
    pub back: [f32; 3],
}

impl Renderer {
    pub fn instance(&self) -> Instance {
        let ubuf = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: None,
            size: std::mem::size_of::<InstUniform>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let bind = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout: &self.inst_layout,
            entries: &[wgpu::BindGroupEntry { binding: 0, resource: ubuf.as_entire_binding() }],
        });
        Instance {
            ubuf,
            bind,
            shadow: std::cell::Cell::new(false),
            shadow_all: std::cell::Cell::new(false),
            semi: std::cell::Cell::new(false),
            stage: std::cell::Cell::new(false),
            ot: std::cell::Cell::new(None),
            lit_clip: std::cell::Cell::new(false),
        }
    }

    /// An unlit instance.
    pub fn set_instance(&self, inst: &Instance, view_proj: Mat4, model: Mat4, joints: &[Mat4]) {
        self.set_instance_lit(inst, view_proj, model, None, joints, [0, u32::MAX], 0);
    }

    /// An unlit instance whose colours are darkened by subtracting `sub` (0..1) per channel (stage darkening).
    pub fn set_instance_sub(&self, inst: &Instance, view_proj: Mat4, model: Mat4, sub: f32) {
        inst.stage.set(true);
        inst.ot.set(None);
        self.write_instance(inst, view_proj, model, None, &[], [0, u32::MAX], sub, 0, None);
    }

    /// `set_instance_sub` for the floor tiles: each tile's CLUT row offset from the depth
    /// SZ = dot(`depth`.xyz, tile centre) + `depth`.w of its centre (model space; shader.wgsl `floor_row`); `cull`:
    /// the far tiles are not drawn (ST9..STC) instead of taking the last row.
    pub fn set_instance_floor(&self, inst: &Instance, view_proj: Mat4, model: Mat4, sub: f32, depth: [f32; 4], cull: bool) {
        inst.stage.set(true);
        inst.ot.set(None);
        self.write_instance(inst, view_proj, model, None, &[], [0, u32::MAX], sub, 0, Some((depth, 1 + cull as u32)));
    }

    /// The semi-transparent stage polygons (`Loaded::stage_semi`), as `set_instance_sub`: texels without the STP
    /// bit drawn opaque, STP texels and the untextured polygons added to the frame (tpage mode 1, B + F).
    pub fn set_instance_semi(&self, inst: &Instance, view_proj: Mat4, sub: f32) {
        inst.semi.set(true);
        self.set_instance_sub(inst, view_proj, Mat4::IDENTITY, sub);
    }

    #[allow(clippy::too_many_arguments)]
    /// `hide`: hidden-joint masks of the main mesh and of the open hands (`[0, u32::MAX]` = fists only).
    /// `clut_sub`: a fighter's side fade, the level subtracted from its palettes (0 = none).
    /// `light`: lit by this light set (None: unlit).
    pub fn set_instance_lit(&self, inst: &Instance, view_proj: Mat4, model: Mat4, light: Option<&Lighting>, joints: &[Mat4], hide: [u32; 2], clut_sub: u8) {
        inst.shadow.set(false);
        inst.stage.set(false);
        inst.ot.set(None);
        self.write_instance(inst, view_proj, model, light, joints, hide, 0.0, clut_sub, None);
    }

    /// A fighter's shadow: the shadow proxies skinned like the body and flattened onto the floor at
    /// height `floor` (world y), drawn black at 50 % (the proxies are semi-transparent black polygons).
    /// `shear`: the world xz offset per unit of height (the stage light's direction).
    pub fn set_instance_shadow(&self, inst: &Instance, view_proj: Mat4, model: Mat4, joints: &[Mat4], floor: f32, shear: [f32; 2], hide: [u32; 2]) {
        inst.shadow.set(true);
        inst.shadow_all.set(false);
        inst.stage.set(false);
        inst.ot.set(None);
        self.write_instance(inst, view_proj, model, None, joints, hide, 0.0, 0, None);
        let u = [shear[0], shear[1], 0.0, floor];
        // light row 0 carries the shear (xy) and the floor height (w) (its offset: 2 matrices + params)
        self.queue.write_buffer(&inst.ubuf, (32 + 4) * 4, bytemuck::bytes_of(&u));
    }

    /// An imported fighter's shadow, as `set_instance_shadow`: its shadow proxies (ok_br1_converter's
    /// `_shadow_proxy` mesh, the shadow lists +0x50 / +0x54), or with `all` every draw of the model (an export without
    /// them), drawn opaque in `colour` (the stage's shadow colour, frame-buffer values 0..1).
    #[allow(clippy::too_many_arguments)]
    pub fn set_instance_shadow_br1(
        &self,
        inst: &Instance,
        view_proj: Mat4,
        model: Mat4,
        joints: &[Mat4],
        floor: f32,
        shear: [f32; 2],
        hide: [u32; 2],
        all: bool,
        colour: [f32; 3],
    ) {
        self.set_instance_shadow(inst, view_proj, model, joints, floor, shear, hide);
        inst.shadow_all.set(all);
        // light row 1 carries the colour (fs_shadow)
        let u = [colour[0], colour[1], colour[2], 0.0];
        self.queue.write_buffer(&inst.ubuf, (32 + 4 + 4) * 4, bytemuck::bytes_of(&u));
    }

    #[allow(clippy::too_many_arguments)]
    fn write_instance(
        &self,
        inst: &Instance,
        view_proj: Mat4,
        model: Mat4,
        light: Option<&Lighting>,
        joints: &[Mat4],
        hide: [u32; 2],
        sub: f32,
        clut_sub: u8,
        floor: Option<([f32; 4], u32)>,
    ) {
        let l = light.copied().unwrap_or_default();
        let v4 = |v: [f32; 3]| [v[0], v[1], v[2], 0.0];
        let (depth, floor_mode) = floor.unwrap_or(([0.0; 4], 0));
        let mut u = InstUniform {
            view_proj: view_proj.to_cols_array_2d(),
            model: model.to_cols_array_2d(),
            params: [if light.is_some() { 1.0 } else { 0.0 }, if self.manual_srgb { 1.0 } else { 0.0 }, sub, clut_sub as f32],
            light: l.dir.map(v4),
            colour: l.colour.map(v4),
            back: v4(l.back),
            floor: depth,
            hide: [hide[0], hide[1], floor_mode, 0],
            joints: [Mat4::IDENTITY.to_cols_array_2d(); MAX_JOINTS],
        };
        for (i, j) in joints.iter().take(MAX_JOINTS).enumerate() {
            u.joints[i] = j.to_cols_array_2d();
        }
        self.queue.write_buffer(&inst.ubuf, 0, bytemuck::bytes_of(&u));
    }
}
