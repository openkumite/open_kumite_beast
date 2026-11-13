//! wgpu renderer: one skinned, textured pipeline used for stages (static, joint 0 = identity) and fighters.
//!
//! State: presentation (reads the state, never writes it)
//!
//! `mod.rs`: the GPU-side types and `Renderer` with its small API (`upload_vram`, `resize`, `set_vsync`, `set_fit`,
//! `viewport`); `pipelines.rs`: `Renderer::new`; `models.rs`: models and palettes; `instances.rs`: instances and
//! their uniforms; `lightmapped.rs`: lightmapped meshes and sky boxes (the extra stage's map); `frame.rs`: a frame's passes; `targets.rs`: depth, the ambient fill's and the scaled scene's
//! targets; `gpu_timer.rs`: the performance overlay's numbers; `atlas_cache.rs`: the sprite atlas; `offscreen.rs`: the
//! windowless renderer and its frame readback (`replay --video`).

pub mod atlas_cache;
mod frame;
mod gpu_timer;
mod instances;
pub mod lightmapped;
mod models;
#[cfg(not(target_arch = "wasm32"))]
mod offscreen;
mod pipelines;
mod targets;

use crate::output::layout::{Fit, Split, Viewport};
use atlas_cache::Atlas;
use std::sync::Arc;

pub use crate::output::psx_quad::PsxQuad;
pub use gpu_timer::FrameStats;
pub use instances::Lighting;
pub use models::Palette;

pub const MAX_JOINTS: usize = 64;

pub struct GpuModel {
    vbuf: wgpu::Buffer,
    ibuf: wgpu::Buffer,
    draws: Vec<Draw>,
    textures: Vec<wgpu::BindGroup>,
    /// per texture: drawn from texel indices and a palette row instead (`Renderer::set_paletted`)
    pal: Vec<Option<PalTexture>>,
    /// GPU bytes (textures, buffers) for the performance overlay
    pub bytes: [u64; 2],
    /// a lightmapped mesh or a sky box (`lightmapped`; engine addition): drawn by `draw_lit`, `draws` unused
    lit: Option<lightmapped::LitParts>,
}

/// One primitive of a `GpuModel` (a `gltf_model::Prim`'s draw state).
#[derive(Clone, Copy)]
struct Draw {
    first: u32,
    count: u32,
    /// texture slot: 0 = white, else image `tex - 1`
    tex: usize,
    hidden: bool,
    /// the ring wall it belongs to (`Renderer::walls_hidden`)
    wall: Option<u8>,
    /// shadow proxy
    shadow: bool,
    /// drawn over the stage (`Prim::over`)
    over: bool,
    /// semi-transparent (`Prim::semi`)
    semi: bool,
}

/// A texture drawn through a palette (shader.wgsl `fs_pal`): its texel indices, the palette and the uniform with
/// the palette row it shows.
struct PalTexture {
    bind: wgpu::BindGroup,
    row: wgpu::Buffer,
}

/// GPU frame timing (engine addition): two timestamps around the frame's passes, read back through a ring of mapped
/// buffers without waiting (state 0 free, 1 mapping, 2 ready).
struct GpuTimer {
    set: wgpu::QuerySet,
    resolve: wgpu::Buffer,
    read: Vec<(wgpu::Buffer, Arc<std::sync::atomic::AtomicU8>)>,
    period: f32,
}

/// A model placed in the world for one frame.
pub struct Instance {
    ubuf: wgpu::Buffer,
    bind: wgpu::BindGroup,
    /// drawn by the shadow pass (only the model's shadow proxies, flattened)
    shadow: std::cell::Cell<bool>,
    /// the shadow pass flattens all of the model's draws, not only its shadow proxies (an imported fighter: engine
    /// addition, the import S8)
    shadow_all: std::cell::Cell<bool>,
    /// a model of semi-transparent stage polygons (`set_instance_semi`)
    semi: std::cell::Cell<bool>,
    /// part of the stage: on the PS1 linked at the fixed far OT slots 0x80F..0x81E, behind everything sorted
    /// (`frame_once`)
    stage: std::cell::Cell<bool>,
    /// part of the stage drawn in ordering-table order (`set_ot`)
    ot: std::cell::Cell<Option<OtDraw>>,
    /// a lightmapped mesh drawn with the courtyard clip (`set_instance_lit_clip`; engine addition, T6)
    lit_clip: std::cell::Cell<bool>,
}

/// A stage instance drawn in the order of the PS1 ordering table (`Instance::set_ot`): after the stage instances
/// before it, with no depth test (each polygon over what is there, as the GPU draws the OT), the model's draws
/// `range` only (None: all); `late`: after the fighters' shadows (OT slot 0x80F, behind 0x810).
#[derive(Clone, Copy)]
struct OtDraw {
    range: Option<(u32, u32)>,
    late: bool,
}

impl Instance {
    /// Draws this stage instance in ordering-table order (`OtDraw`): the draws `range` of its model (None: all), after
    /// the shadows when `late`. Set after `set_instance_sub` / `set_instance`, which clear it.
    pub fn set_ot(&self, range: Option<(u32, u32)>, late: bool) {
        self.stage.set(true);
        self.ot.set(Some(OtDraw { range, late }));
    }
}

pub struct Renderer {
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
    /// kept to create the surface again for a new native window (Android, `resume_surface`)
    instance: wgpu::Instance,
    /// None while the window has no native surface (Android in the background, `suspend`)
    surface: Option<wgpu::Surface<'static>>,
    pub config: wgpu::SurfaceConfiguration,
    depth: wgpu::TextureView,
    pipeline: wgpu::RenderPipeline,
    /// the model pipeline for paletted textures (`set_paletted`)
    pal_pipe: wgpu::RenderPipeline,
    pal_layout: wgpu::BindGroupLayout,
    /// semi-transparent stage polygons: [depth-tested, drawn over] × [texels without STP (opaque), STP texels (added)]
    semi_pipes: [[wgpu::RenderPipeline; 2]; 2],
    /// fighter shadows: [depth-tested, over the stage drawn in OT order]
    shadow_pipes: [wgpu::RenderPipeline; 2],
    /// the stage in ordering-table order (`OtDraw`): [plain, paletted], no depth test
    ot_pipes: [wgpu::RenderPipeline; 2],
    /// writes the far depth over the viewport, no colour: the stage's depth dropped before the fighters
    depth_reset: wgpu::RenderPipeline,
    flat: wgpu::RenderPipeline,
    /// lightmapped meshes and sky boxes (`lightmapped`; engine addition)
    lit: lightmapped::LitPipes,
    /// PS1 quads: [opaque, blend 0..3]
    psx: Vec<wgpu::RenderPipeline>,
    vram_bind: wgpu::BindGroup,
    vram_tex: wgpu::Texture,
    /// atlas quads: [opaque, blend 0..3] (sprite.wgsl)
    sprite: Vec<wgpu::RenderPipeline>,
    atlas: Atlas,
    atlas_layout: wgpu::BindGroupLayout,
    atlas_tex: wgpu::Texture,
    atlas_bind: wgpu::BindGroup,
    /// OKB_SPRITES=old: atlas quads drawn from the VRAM copy as before; OKB_SPRITES=ab: captured frames are drawn
    /// both ways and the differing pixels logged
    sprites_old: bool,
    sprites_ab: bool,
    inst_layout: wgpu::BindGroupLayout,
    tex_layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    /// the untextured polygons' texture: white, alpha 254 (`shader.wgsl` `shade_texel`: no palette, so no side fade)
    white: wgpu::BindGroup,
    /// the untextured semi-transparent polygons' texture: white with the STP alpha (0.5)
    white_stp: wgpu::BindGroup,
    manual_srgb: bool,
    /// stage ring walls not drawn (bit = wall model index)
    pub walls_hidden: u8,
    /// M.9E: non-zero keeps the image between frames; before the new frame a full-screen (16,16,16)
    /// tile darkens it (1, subtract), whitens it (2, additive) or nothing (3)
    pub afterimage: u8,
    /// the fight backdrop (`present/backdrop.rs`): drawn before the scene, its bottom colour the clear colour. Set by the
    /// caller per frame, taken by the frame
    pub backdrop: Option<crate::output::backdrop::Backdrop>,
    persist: Option<wgpu::Texture>,
    persist_used: bool,
    /// the window layout (`set_fit`, rebuilt on resize)
    vp: Viewport,
    /// Android (engine addition, `set_hud_insets`): the HUD's safe area out to the window's edges less these display
    /// cutout insets (`Viewport::with_insets`); None (desktop, web): the 16:9 one
    hud_insets: Option<[f32; 4]>,
    /// the ambient fill of the next frames (engine addition, `output::layout::Split`; set by the caller per frame)
    pub ambient: Option<Split>,
    amb_blur: wgpu::RenderPipeline,
    amb_pipe: wgpu::RenderPipeline,
    amb_sampler: wgpu::Sampler,
    /// the ambient fill's textures (rebuilt when the size changes)
    amb: Option<AmbTextures>,
    /// the last `acquire` got a surface texture, so the frame is drawn and presented (false: none, e.g. an occluded
    /// or minimised window, and the frame is skipped); OKB_LOCKSTEP steps only for such frames
    pub presented: bool,
    /// the failure of the current run of failed `acquire`s (None: the last one succeeded), logged when it changes
    acquire_failure: Option<&'static str>,
    /// the performance overlay is on: `stats` are filled (engine addition)
    pub perf: bool,
    pub stats: FrameStats,
    uploads: u32,
    /// None without `Features::TIMESTAMP_QUERY`
    timer: Option<GpuTimer>,
    /// RENDER QUALITY (engine addition): the 3D scene's resolution against the window (1 = drawn straight into it)
    pub render_scale: f32,
    /// the quads from this index on are the frame's 2D layer (HUD, text), drawn at the window's resolution over the
    /// scaled 3D scene; None (the front end) draws the whole frame at the window's resolution. Set per frame by the
    /// caller, taken by the frame (engine addition)
    pub scaled_at: Option<usize>,
    /// HUD SIZE (engine addition): the fight HUD's scale (`layout::hud_fit`)
    pub hud_scale: f32,
    /// the scaled 3D scene's target (rebuilt when its size or filter changes)
    lo: Option<LoTarget>,
    /// the scaled scene drawn onto the window: `ambient.wgsl` `copy`
    lo_pipe: wgpu::RenderPipeline,
    /// the next frame's target (`acquire`): the surface's texture, or the offscreen one
    out: Option<Target>,
    /// the frames' vertex streams, kept and grown: effect quads, the afterimage tile, the overlay, the ambient fill,
    /// the screen quads
    streams: [Option<wgpu::Buffer>; 5],
    /// quads drawn last, over the whole window and after the afterimage copy, in clip space of the window: the touch
    /// controls (engine addition, Android; `app::touch`), in the bars of CLASSIC too. Set by the caller per frame,
    /// taken by the frame
    pub screen: Vec<PsxQuad>,
    /// the windowless renderer's target and frame readback (`offscreen`, engine addition); None with a window
    #[cfg(not(target_arch = "wasm32"))]
    offscreen: Option<offscreen::Offscreen>,
}

/// What a frame is drawn into: the surface's texture (presented after the frame) or the windowless renderer's
/// texture (read back, `offscreen`).
enum Target {
    Surface(wgpu::SurfaceTexture),
    #[cfg_attr(target_arch = "wasm32", allow(dead_code))]
    Texture(wgpu::Texture),
}

impl Target {
    fn texture(&self) -> &wgpu::Texture {
        match self {
            Target::Surface(s) => &s.texture,
            Target::Texture(t) => t,
        }
    }
}

impl Renderer {
    /// Copy the game's VRAM (1024×512 halfwords) to the GPU for the effect quads; it is also the sprite atlas's
    /// staging composite.
    /// The same image goes into bank 1 (`upload_vram_bank1` replaces it there).
    pub fn upload_vram(&mut self, words: &[u16]) {
        self.uploads += 1;
        self.atlas.set_source(words);
        self.write_vram_rows(0, words);
        self.write_vram_rows(512, words);
    }

    /// Engine addition (the team battle): VRAM bank 1, the copy the effect quads of fighters 2 / 3 sample (a CLUT word
    /// with bit 15 set, `render/shaders/psx.wgsl`): bank 0 with their `<CHR>EFT` pages at P1's / P2's places.
    pub fn upload_vram_bank1(&mut self, words: &[u16]) {
        self.write_vram_rows(512, words);
    }

    fn write_vram_rows(&mut self, y: u32, words: &[u16]) {
        self.queue.write_texture(
            wgpu::TexelCopyTextureInfo { origin: wgpu::Origin3d { x: 0, y, z: 0 }, ..self.vram_tex.as_image_copy() },
            bytemuck::cast_slice(words),
            wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(2048), rows_per_image: Some(512) },
            wgpu::Extent3d { width: 1024, height: 512, depth_or_array_layers: 1 },
        );
    }

    pub fn resize(&mut self, w: u32, h: u32) {
        if w == 0 || h == 0 {
            return;
        }
        self.config.width = w;
        self.config.height = h;
        self.vp = self.layout([w, h], self.vp.fit);
        self.out = None;
        if let Some(s) = &self.surface {
            s.configure(&self.device, &self.config);
        }
        self.depth = Self::make_depth(&self.device, &self.config);
        self.persist = None;
        #[cfg(not(target_arch = "wasm32"))]
        if let Some(o) = &mut self.offscreen {
            o.resize(&self.device, &self.config);
        }
    }

    /// The native window went (Android in the background, engine addition): the surface is dropped; frames are
    /// skipped (`acquire` fails) until `resume_surface`.
    pub fn suspend(&mut self) {
        self.out = None;
        self.surface = None;
    }

    /// A new native window (Android back in front): the surface made again from it and configured at its size.
    pub fn resume_surface(&mut self, window: Arc<winit::window::Window>) -> Result<(), String> {
        let size = window.inner_size();
        let s = self.instance.create_surface(window).map_err(|e| e.to_string())?;
        self.surface = Some(s);
        let (w, h) = (size.width.max(1), size.height.max(1));
        if (w, h) != (self.config.width, self.config.height) {
            self.resize(w, h);
        } else if let Some(s) = &self.surface {
            s.configure(&self.device, &self.config);
        }
        Ok(())
    }

    /// VSYNC (engine addition): AutoVsync / AutoNoVsync, the surface reconfigured when it changes.
    pub fn set_vsync(&mut self, on: bool) {
        let mode = if on { wgpu::PresentMode::AutoVsync } else { wgpu::PresentMode::AutoNoVsync };
        if self.config.present_mode != mode {
            self.config.present_mode = mode;
            self.out = None;
            if let Some(s) = &self.surface {
                s.configure(&self.device, &self.config);
            }
        }
    }

    /// Classic: every frame is drawn into the centred 4:3 rectangle (viewport + scissor), the bars stay black.
    pub fn set_fit(&mut self, fit: Fit) {
        if self.vp.fit != fit {
            self.vp = self.layout(self.vp.win, fit);
            self.persist = None;
            self.persist_used = false;
        }
    }

    /// Android (engine addition): the fight HUD to the window's edges, within the display cutout's safe `insets`
    /// (window pixels: left, top, right, bottom).
    pub fn set_hud_insets(&mut self, insets: [f32; 4]) {
        if self.hud_insets != Some(insets) {
            self.hud_insets = Some(insets);
            self.vp = self.layout(self.vp.win, self.vp.fit);
        }
    }

    fn layout(&self, win: [u32; 2], fit: Fit) -> Viewport {
        let vp = Viewport::new(win, fit);
        match self.hud_insets {
            Some(i) => vp.with_insets(i),
            None => vp,
        }
    }

    /// The window layout (drawn area, 3D aspect, HUD mapping).
    pub fn viewport(&self) -> &Viewport {
        &self.vp
    }
}

/// The ambient fill's textures (engine addition): the copied picture and its blur at 1/4 size, with their bind groups.
struct AmbTextures {
    copy: wgpu::Texture,
    copy_bind: wgpu::BindGroup,
    small: wgpu::Texture,
    small_view: wgpu::TextureView,
    small_bind: wgpu::BindGroup,
}

/// RENDER QUALITY's scaled scene (engine addition): colour (in the surface format) and depth targets, the colour's
/// bind group and the window quad it is drawn with.
struct LoTarget {
    size: [u32; 2],
    nearest: bool,
    format: wgpu::TextureFormat,
    tex: wgpu::Texture,
    view: wgpu::TextureView,
    depth: wgpu::TextureView,
    bind: wgpu::BindGroup,
    quad: wgpu::Buffer,
}

impl LoTarget {
    /// A window rect (x, y, w, h) of a `w`×`h` window in this target's pixels.
    fn rect(&self, r: [u32; 4], w: u32, h: u32) -> [u32; 4] {
        let (fx, fy) = (self.size[0] as f32 / w.max(1) as f32, self.size[1] as f32 / h.max(1) as f32);
        let x0 = ((r[0] as f32 * fx).round() as u32).min(self.size[0]);
        let y0 = ((r[1] as f32 * fy).round() as u32).min(self.size[1]);
        let x1 = (((r[0] + r[2]) as f32 * fx).round() as u32).clamp(x0, self.size[0]);
        let y1 = (((r[1] + r[3]) as f32 * fy).round() as u32).clamp(y0, self.size[1]);
        [x0, y0, x1 - x0, y1 - y0]
    }
}
