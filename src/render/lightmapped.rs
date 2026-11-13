//! Lightmapped meshes, a six-face sky box and a cloud sky (engine addition: the extra stages' Quake III maps).
//!
//! State: presentation (reads the state, never writes it)
//!
//! A mesh whose surfaces are a base texture times a lightmap (or the vertex colours) plus an additive glow, opaque,
//! alpha-tested, alpha-blended or added, culled or two-sided (`LitMesh`, `Renderer::upload_lit`), a sky box of six
//! square faces (`Renderer::upload_sky`) and Quake III's cloud layers (`Renderer::upload_clouds`). All come back as a `GpuModel` drawn as a stage instance
//! (`Renderer::set_instance_lit_mesh`): with the stage, before the depth reset, so the fighters stand over it as over
//! every stage. Textures are kept as stored (gamma values) with mipmaps and sampled linearly with repeat
//! (`shaders/lightmapped.wgsl`). The team battle (T6) draws a walled stage's mesh with the courtyard clip
//! (`Renderer::set_instance_lit_clip`: pipelines of their own, so every other draw keeps its shaders bit for bit).

use super::{GpuModel, Instance, Renderer};
use wgpu::util::DeviceExt;

/// A vertex: position (the model's space), texture and lightmap coordinates, colour (gamma, 1 = as the texture), the
/// texture's scroll per second.
#[repr(C)]
#[derive(Clone, Copy, Debug, bytemuck::Pod, bytemuck::Zeroable)]
pub struct LitVertex {
    pub pos: [f32; 3],
    pub uv: [f32; 2],
    pub lm: [f32; 2],
    pub color: [f32; 4],
    pub scroll: [f32; 2],
}

/// How a surface goes over what is behind it (`LitDraw::blend`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LitBlend {
    Opaque,
    /// alpha under ½ not drawn
    Mask,
    /// alpha blended, no depth write
    Alpha,
    /// added, no depth write
    Add,
}

/// A run of triangles: indices `first..first + count`, the base and glow textures (indices into `LitMesh::images`;
/// None: grey / none), lit by the lightmap or not.
#[derive(Clone, Copy, Debug)]
pub struct LitDraw {
    pub first: u32,
    pub count: u32,
    pub base: Option<usize>,
    pub glow: Option<usize>,
    pub lightmap: bool,
    pub blend: LitBlend,
    pub cull: bool,
}

/// An RGBA8 image.
#[derive(Clone, Copy)]
pub struct LitImage<'a> {
    pub width: u32,
    pub height: u32,
    pub rgba: &'a [u8],
}

/// What `upload_lit` takes.
pub struct LitMesh<'a> {
    pub vertices: &'a [LitVertex],
    pub indices: &'a [u32],
    /// drawn in this order within each pass (opaque and masked first, then the blended)
    pub draws: &'a [LitDraw],
    pub images: &'a [Option<LitImage<'a>>],
    pub lightmap: Option<LitImage<'a>>,
}

/// One cloud layer of a sky (`Renderer::upload_clouds`): its image, its coordinates st · `scale` + t · `scroll`
/// (`formats::q3_map::CloudLayer`), added or replacing what is under it.
pub struct CloudImage<'a> {
    pub image: LitImage<'a>,
    pub scale: [f32; 2],
    pub scroll: [f32; 2],
    pub add: bool,
}

/// The GPU side of a `LitMesh`, a sky box or a cloud sky (`GpuModel::lit`); the clouds' first layer added or not.
pub(super) enum LitParts {
    Mesh { draws: Vec<LitDraw>, textures: Vec<wgpu::BindGroup>, lightmap: wgpu::BindGroup },
    Sky(wgpu::BindGroup),
    Clouds { bind: wgpu::BindGroup, add: bool },
}

/// The pipelines: per `LitBlend` × [culled, two-sided], the sky box's, the clouds' [replacing, added]; the 1×1 grey,
/// white and black textures; the samplers. `mesh_clip`: the mesh's with the courtyard clip (T6, the `*_clip` entry
/// points).
pub(super) struct LitPipes {
    mesh: Vec<[wgpu::RenderPipeline; 2]>,
    mesh_clip: Vec<[wgpu::RenderPipeline; 2]>,
    sky: wgpu::RenderPipeline,
    sky_layout: wgpu::BindGroupLayout,
    clouds: [wgpu::RenderPipeline; 2],
    clouds_layout: wgpu::BindGroupLayout,
    grey: wgpu::BindGroup,
    white: wgpu::BindGroup,
    black: wgpu::BindGroup,
    repeat: wgpu::Sampler,
    clamp: wgpu::Sampler,
}

/// The four blends' order in `LitPipes::mesh`.
fn blend_index(b: LitBlend) -> usize {
    match b {
        LitBlend::Opaque => 0,
        LitBlend::Mask => 1,
        LitBlend::Alpha => 2,
        LitBlend::Add => 3,
    }
}

impl LitPipes {
    pub(super) fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        format: wgpu::TextureFormat,
        inst_layout: &wgpu::BindGroupLayout,
        tex_layout: &wgpu::BindGroupLayout,
    ) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("lightmapped"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shaders/lightmapped.wgsl").into()),
        });
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("lightmapped"),
            bind_group_layouts: &[Some(inst_layout), Some(tex_layout), Some(tex_layout), Some(tex_layout)],
            immediate_size: 0,
        });
        let attrs = wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x2, 2 => Float32x2, 3 => Float32x4, 4 => Float32x2];
        let alpha = Some(wgpu::BlendState::ALPHA_BLENDING);
        let one = wgpu::BlendComponent { src_factor: wgpu::BlendFactor::One, dst_factor: wgpu::BlendFactor::One, operation: wgpu::BlendOperation::Add };
        let add = Some(wgpu::BlendState { color: one, alpha: one });
        let pipe = |vs: &str, fs: &str, blend: Option<wgpu::BlendState>, write: bool, cull: bool| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(fs),
                layout: Some(&layout),
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: Some(vs),
                    compilation_options: Default::default(),
                    buffers: &[Some(wgpu::VertexBufferLayout {
                        array_stride: std::mem::size_of::<LitVertex>() as u64,
                        step_mode: wgpu::VertexStepMode::Vertex,
                        attributes: &attrs,
                    })],
                },
                primitive: wgpu::PrimitiveState { cull_mode: cull.then_some(wgpu::Face::Back), ..Default::default() },
                depth_stencil: Some(wgpu::DepthStencilState {
                    format: wgpu::TextureFormat::Depth32Float,
                    depth_write_enabled: Some(write),
                    depth_compare: Some(wgpu::CompareFunction::LessEqual),
                    stencil: Default::default(),
                    bias: Default::default(),
                }),
                multisample: Default::default(),
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some(fs),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState { format, blend, write_mask: wgpu::ColorWrites::ALL })],
                }),
                multiview_mask: None,
                cache: None,
            })
        };
        let mesh = [("fs_opaque", None, true), ("fs_mask", None, true), ("fs_alpha", alpha, false), ("fs_add", add, false)]
            .map(|(fs, blend, write)| [pipe("vs", fs, blend, write, true), pipe("vs", fs, blend, write, false)])
            .into();
        let mesh_clip = [("fs_opaque_clip", None, true), ("fs_mask_clip", None, true), ("fs_alpha_clip", alpha, false), ("fs_add_clip", add, false)]
            .map(|(fs, blend, write)| [pipe("vs_clip", fs, blend, write, true), pipe("vs_clip", fs, blend, write, false)])
            .into();
        let sky_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("sky"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2Array,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });
        let sky_pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("sky"),
            bind_group_layouts: &[Some(inst_layout), Some(&sky_layout)],
            immediate_size: 0,
        });
        let tex_entry = |binding| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Texture {
                sample_type: wgpu::TextureSampleType::Float { filterable: true },
                view_dimension: wgpu::TextureViewDimension::D2,
                multisampled: false,
            },
            count: None,
        };
        let clouds_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("clouds"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 4,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None },
                    count: None,
                },
                tex_entry(5),
                tex_entry(6),
                wgpu::BindGroupLayoutEntry {
                    binding: 7,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });
        let clouds_pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("clouds"),
            bind_group_layouts: &[Some(inst_layout), Some(&clouds_layout)],
            immediate_size: 0,
        });
        // the sky box and the clouds: round the eye at the far plane, no depth
        let sky_pipe = |label: &str, layout: &wgpu::PipelineLayout, fs: &str, blend: Option<wgpu::BlendState>| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(label),
                layout: Some(layout),
                vertex: wgpu::VertexState { module: &shader, entry_point: Some("vs_sky"), compilation_options: Default::default(), buffers: &[] },
                primitive: wgpu::PrimitiveState { cull_mode: None, ..Default::default() },
                depth_stencil: Some(wgpu::DepthStencilState {
                    format: wgpu::TextureFormat::Depth32Float,
                    depth_write_enabled: Some(false),
                    depth_compare: Some(wgpu::CompareFunction::Always),
                    stencil: Default::default(),
                    bias: Default::default(),
                }),
                multisample: Default::default(),
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some(fs),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState { format, blend, write_mask: wgpu::ColorWrites::ALL })],
                }),
                multiview_mask: None,
                cache: None,
            })
        };
        let sky = sky_pipe("sky", &sky_pl, "fs_sky", None);
        let clouds = [sky_pipe("clouds", &clouds_pl, "fs_clouds", None), sky_pipe("clouds added", &clouds_pl, "fs_clouds", add)];
        let sampler = |mode| {
            device.create_sampler(&wgpu::SamplerDescriptor {
                address_mode_u: mode,
                address_mode_v: mode,
                mag_filter: wgpu::FilterMode::Linear,
                min_filter: wgpu::FilterMode::Linear,
                mipmap_filter: wgpu::MipmapFilterMode::Linear,
                ..Default::default()
            })
        };
        let (repeat, clamp) = (sampler(wgpu::AddressMode::Repeat), sampler(wgpu::AddressMode::ClampToEdge));
        let one_texel = |c: [u8; 4]| bind(device, tex_layout, &texture(device, queue, &LitImage { width: 1, height: 1, rgba: &c }, 1), &repeat);
        Self {
            mesh,
            mesh_clip,
            sky,
            sky_layout,
            clouds,
            clouds_layout,
            grey: one_texel([128, 128, 128, 255]),
            white: one_texel([255; 4]),
            black: one_texel([0, 0, 0, 255]),
            repeat,
            clamp,
        }
    }
}

/// `im` and its mipmaps (box-filtered in its gamma values) in `layers` layers of a texture (`im` holds them one
/// after the other).
fn texture(device: &wgpu::Device, queue: &wgpu::Queue, im: &LitImage, layers: u32) -> wgpu::Texture {
    let (w, h) = (im.width.max(1), im.height.max(1));
    let levels = 32 - w.max(h).leading_zeros();
    let tex = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("lightmapped"),
        size: wgpu::Extent3d { width: w, height: h, depth_or_array_layers: layers },
        mip_level_count: levels,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    let layer_bytes = 4 * (w * h) as usize;
    for layer in 0..layers {
        let mut px =
            im.rgba.get(layer as usize * layer_bytes..(layer as usize + 1) * layer_bytes).map(<[u8]>::to_vec).unwrap_or_else(|| vec![128; layer_bytes]);
        let (mut lw, mut lh) = (w, h);
        for level in 0..levels {
            queue.write_texture(
                wgpu::TexelCopyTextureInfo {
                    texture: &tex,
                    mip_level: level,
                    origin: wgpu::Origin3d { x: 0, y: 0, z: layer },
                    aspect: wgpu::TextureAspect::All,
                },
                &px,
                wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(4 * lw), rows_per_image: Some(lh) },
                wgpu::Extent3d { width: lw, height: lh, depth_or_array_layers: 1 },
            );
            let (nw, nh) = ((lw / 2).max(1), (lh / 2).max(1));
            let mut next = vec![0u8; 4 * (nw * nh) as usize];
            for y in 0..nh {
                for x in 0..nw {
                    for c in 0..4 {
                        let at = |xx: u32, yy: u32| px[(4 * (yy.min(lh - 1) * lw + xx.min(lw - 1)) + c) as usize] as u32;
                        let s = at(2 * x, 2 * y) + at(2 * x + 1, 2 * y) + at(2 * x, 2 * y + 1) + at(2 * x + 1, 2 * y + 1);
                        next[(4 * (y * nw + x) + c) as usize] = ((s + 2) / 4) as u8;
                    }
                }
            }
            px = next;
            (lw, lh) = (nw, nh);
        }
    }
    tex
}

fn bind(device: &wgpu::Device, layout: &wgpu::BindGroupLayout, tex: &wgpu::Texture, sampler: &wgpu::Sampler) -> wgpu::BindGroup {
    let view = tex.create_view(&Default::default());
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("lightmapped"),
        layout,
        entries: &[
            wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&view) },
            wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(sampler) },
        ],
    })
}

/// A whole image as `w`×`h` RGBA (a damaged one: grey).
fn fitted(im: &LitImage) -> Vec<u8> {
    let n = 4 * (im.width.max(1) * im.height.max(1)) as usize;
    if im.rgba.len() >= n { im.rgba[..n].to_vec() } else { vec![128; n] }
}

impl Renderer {
    /// A lightmapped mesh on the GPU (drawn by `draw_lit` as a stage instance).
    pub fn upload_lit(&self, m: &LitMesh) -> GpuModel {
        let p = &self.lit;
        let vbuf = self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("lightmapped"),
            contents: bytemuck::cast_slice(m.vertices),
            usage: wgpu::BufferUsages::VERTEX,
        });
        let ibuf = self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("lightmapped"),
            contents: bytemuck::cast_slice(m.indices),
            usage: wgpu::BufferUsages::INDEX,
        });
        let mut bytes = 0u64;
        // an image larger than the device takes: as a missing one (grey; the lightmap white)
        let max = self.device.limits().max_texture_dimension_2d;
        let mut upload = |im: &LitImage, sampler: &wgpu::Sampler| {
            if im.width > max || im.height > max {
                log::warn!("lightmapped: a {}x{} image is over the device's {max}; not drawn", im.width, im.height);
                return None;
            }
            let px = fitted(im);
            bytes += 4 * px.len() as u64 / 3;
            Some(bind(&self.device, &self.tex_layout, &texture(&self.device, &self.queue, &LitImage { rgba: &px, ..*im }, 1), sampler))
        };
        let textures: Vec<wgpu::BindGroup> =
            m.images.iter().map(|im| im.as_ref().and_then(|im| upload(im, &p.repeat)).unwrap_or_else(|| p.grey.clone())).collect();
        let lightmap = m.lightmap.as_ref().and_then(|im| upload(im, &p.clamp)).unwrap_or_else(|| p.white.clone());
        // the draws whose indices lie in the buffer, each texture index in range
        let n = m.indices.len() as u32;
        let draws: Vec<LitDraw> = m
            .draws
            .iter()
            .filter(|d| d.first.checked_add(d.count).is_some_and(|e| e <= n) && d.count > 0)
            .map(|d| LitDraw { base: d.base.filter(|&k| k < textures.len()), glow: d.glow.filter(|&k| k < textures.len()), ..*d })
            .collect();
        GpuModel {
            bytes: [bytes, vbuf.size() + ibuf.size()],
            vbuf,
            ibuf,
            draws: Vec::new(),
            textures: Vec::new(),
            pal: Vec::new(),
            lit: Some(LitParts::Mesh { draws, textures, lightmap }),
        }
    }

    /// A sky box of six square faces of one size (rt bk lf ft up dn: +X +Y −X −Y +Z −Z of the sky's space, Quake III's
    /// layout); None when they are not square and of one size, or larger than the device takes.
    pub fn upload_sky(&self, faces: &[LitImage; 6]) -> Option<GpuModel> {
        let s = faces[0].width;
        if s == 0
            || s > self.device.limits().max_texture_dimension_2d
            || faces.iter().any(|f| f.width != s || f.height != s || f.rgba.len() < 4 * (s * s) as usize)
        {
            return None;
        }
        let px: Vec<u8> = faces.iter().flat_map(|f| f.rgba[..4 * (s * s) as usize].iter().copied()).collect();
        // 7 layers (the last unused): the GL backend makes a square texture of 6 layers a cube map (wgpu-hal's
        // `get_info_from_desc`), which the shader's 2D array view does not sample
        let tex = texture(&self.device, &self.queue, &LitImage { width: s, height: s, rgba: &px }, 7);
        let view = tex.create_view(&wgpu::TextureViewDescriptor { dimension: Some(wgpu::TextureViewDimension::D2Array), ..Default::default() });
        let bind = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("sky"),
            layout: &self.lit.sky_layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&view) },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(&self.lit.clamp) },
            ],
        });
        let empty = |usage| self.device.create_buffer(&wgpu::BufferDescriptor { label: Some("sky"), size: 16, usage, mapped_at_creation: false });
        Some(GpuModel {
            bytes: [4 * px.len() as u64 / 3, 0],
            vbuf: empty(wgpu::BufferUsages::VERTEX),
            ibuf: empty(wgpu::BufferUsages::INDEX),
            draws: Vec::new(),
            textures: Vec::new(),
            pal: Vec::new(),
            lit: Some(LitParts::Sky(bind)),
        })
    }

    /// A sky's cloud layers (one or two `layers`, drawn in one pass round the eye; the first
    /// replaces what is under it unless added) at `height` (the shader's cloud height, Quake units); None without a
    /// layer or with an image larger than the device takes.
    pub fn upload_clouds(&self, layers: &[CloudImage], height: f32) -> Option<GpuModel> {
        // the first two layers (the shader's room); a third is left out before its size is looked at
        let layers = &layers[..layers.len().min(2)];
        let max = self.device.limits().max_texture_dimension_2d;
        if layers.is_empty() || layers.iter().any(|l| l.image.width > max || l.image.height > max) {
            return None;
        }
        let mut bytes = 0u64;
        let views: Vec<wgpu::TextureView> = layers
            .iter()
            .map(|l| {
                let px = fitted(&l.image);
                bytes += 4 * px.len() as u64 / 3;
                texture(&self.device, &self.queue, &LitImage { rgba: &px, ..l.image }, 1).create_view(&Default::default())
            })
            .collect();
        // the uniform (`Clouds` in the shader): per layer (scale, scroll), then (height, layers, second added, 0)
        let mut u = [0f32; 12];
        for (k, l) in layers.iter().enumerate() {
            u[4 * k..4 * k + 4].copy_from_slice(&[l.scale[0], l.scale[1], l.scroll[0], l.scroll[1]]);
        }
        u[8..12].copy_from_slice(&[height, layers.len() as f32, layers.get(1).is_some_and(|l| l.add) as u32 as f32, 0.0]);
        let ubuf = self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("clouds"),
            contents: bytemuck::cast_slice(&u),
            usage: wgpu::BufferUsages::UNIFORM,
        });
        let bind = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("clouds"),
            layout: &self.lit.clouds_layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 4, resource: ubuf.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 5, resource: wgpu::BindingResource::TextureView(&views[0]) },
                wgpu::BindGroupEntry { binding: 6, resource: wgpu::BindingResource::TextureView(views.last().expect("a layer")) },
                wgpu::BindGroupEntry { binding: 7, resource: wgpu::BindingResource::Sampler(&self.lit.repeat) },
            ],
        });
        let empty = |usage| self.device.create_buffer(&wgpu::BufferDescriptor { label: Some("clouds"), size: 16, usage, mapped_at_creation: false });
        Some(GpuModel {
            bytes: [bytes, 0],
            vbuf: empty(wgpu::BufferUsages::VERTEX),
            ibuf: empty(wgpu::BufferUsages::INDEX),
            draws: Vec::new(),
            textures: Vec::new(),
            pal: Vec::new(),
            lit: Some(LitParts::Clouds { bind, add: layers[0].add }),
        })
    }

    /// A stage instance of a lightmapped mesh, a sky box or a cloud sky: `view_proj` × `model` (the sky: `view_proj` without the
    /// eye's position, `model` the world's directions into the sky's), the stage darkening `sub` (0..1, as
    /// `set_instance_sub`), the time `t` (s) its textures scroll by.
    pub fn set_instance_lit_mesh(&self, inst: &Instance, view_proj: glam::Mat4, model: glam::Mat4, sub: f32, t: f32) {
        self.set_instance_sub(inst, view_proj, model, sub);
        inst.lit_clip.set(false);
        // params.w (after the two matrices and params.x..z): the time
        self.queue.write_buffer(&inst.ubuf, (32 + 3) * 4, bytemuck::bytes_of(&t));
    }

    /// A lightmapped mesh instance drawn with the courtyard clip (engine addition, T6; after `set_instance_lit_mesh`,
    /// which turns it off): `eye` and the box `lo`..`hi` in the mesh's model space; a fragment outside the box that
    /// stands between the eye and it is not drawn (the `*_clip` entry points of `shaders/lightmapped.wgsl`).
    pub fn set_instance_lit_clip(&self, inst: &Instance, eye: glam::Vec3, lo: glam::Vec3, hi: glam::Vec3) {
        inst.lit_clip.set(true);
        let u = [eye.x, eye.y, eye.z, 0.0, lo.x, lo.y, lo.z, 0.0, hi.x, hi.y, hi.z, 0.0];
        // the light rows (after the two matrices and params): `Inst::clip`
        self.queue.write_buffer(&inst.ubuf, (32 + 4) * 4, bytemuck::cast_slice(&u));
    }

    /// Draws `m` (a lightmapped mesh: opaque and masked draws, then the blended ones; a sky box; a cloud sky) for
    /// `inst`.
    pub(super) fn draw_lit(&self, pass: &mut wgpu::RenderPass<'_>, m: &GpuModel, inst: &Instance) {
        let p = &self.lit;
        match &m.lit {
            Some(LitParts::Sky(b)) => {
                pass.set_pipeline(&p.sky);
                pass.set_bind_group(0, &inst.bind, &[]);
                pass.set_bind_group(1, b, &[]);
                pass.draw(0..36, 0..1);
            }
            Some(LitParts::Clouds { bind, add }) => {
                pass.set_pipeline(&p.clouds[*add as usize]);
                pass.set_bind_group(0, &inst.bind, &[]);
                pass.set_bind_group(1, bind, &[]);
                pass.draw(0..36, 0..1);
            }
            Some(LitParts::Mesh { draws, textures, lightmap }) => {
                pass.set_bind_group(0, &inst.bind, &[]);
                pass.set_vertex_buffer(0, m.vbuf.slice(..));
                pass.set_index_buffer(m.ibuf.slice(..), wgpu::IndexFormat::Uint32);
                let pipes = if inst.lit_clip.get() { &p.mesh_clip } else { &p.mesh };
                for blended in [false, true] {
                    for d in draws.iter().filter(|d| matches!(d.blend, LitBlend::Alpha | LitBlend::Add) == blended) {
                        pass.set_pipeline(&pipes[blend_index(d.blend)][!d.cull as usize]);
                        pass.set_bind_group(1, d.base.map_or(&p.grey, |k| &textures[k]), &[]);
                        pass.set_bind_group(2, if d.lightmap { lightmap } else { &p.white }, &[]);
                        pass.set_bind_group(3, d.glow.map_or(&p.black, |k| &textures[k]), &[]);
                        pass.draw_indexed(d.first..d.first + d.count, 0, 0..1);
                    }
                }
            }
            None => {}
        }
    }
}
