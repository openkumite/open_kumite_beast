//! `Renderer::new`: the device, the surface and every pipeline.
//!
//! State: presentation (reads the state, never writes it)
//!
//! `Renderer::new`: the adapter and surface; `build` (also `offscreen`'s): the device, then every pipeline (the model and paletted ones, the semi-transparent,
//! shadow and ordering-table families, the depth reset, flat, PS1 quads, atlas sprites, the ambient fill and the scaled
//! scene's copy), the VRAM copy, the atlas, the samplers and the white textures, and the GPU timer.

use super::atlas_cache::{Atlas, make_atlas};
use super::targets::make_texture;
use super::{FrameStats, GpuTimer, Renderer};
use crate::formats::gltf_model::Vertex;
use crate::output::layout::{Fit, Viewport};
use std::sync::Arc;

impl Renderer {
    pub async fn new(window: Arc<winit::window::Window>) -> Result<Self, String> {
        let size = window.inner_size();
        // Android (engine addition): Vulkan first, GLES when no Vulkan adapter drives the window
        #[cfg(target_os = "android")]
        let tries = [wgpu::Backends::VULKAN, wgpu::Backends::GL];
        #[cfg(not(target_os = "android"))]
        let tries = [wgpu::InstanceDescriptor::new_without_display_handle().backends];
        let mut failed = String::new();
        let mut found = None;
        for backends in tries {
            let instance = wgpu::Instance::new(wgpu::InstanceDescriptor { backends, ..wgpu::InstanceDescriptor::new_without_display_handle() });
            let surface = match instance.create_surface(window.clone()) {
                Ok(s) => s,
                Err(e) => {
                    failed = e.to_string();
                    continue;
                }
            };
            match instance
                .request_adapter(&wgpu::RequestAdapterOptions {
                    power_preference: wgpu::PowerPreference::HighPerformance,
                    force_fallback_adapter: false,
                    compatible_surface: Some(&surface),
                    apply_limit_buckets: false,
                })
                .await
            {
                Ok(a) => {
                    found = Some((instance, surface, a));
                    break;
                }
                Err(e) => failed = e.to_string(),
            }
        }
        let (instance, surface, adapter) = found.ok_or(failed)?;
        // Android: which GPU and backend (Vulkan or the GLES fallback) the device gave, for the logcat
        #[cfg(target_os = "android")]
        log::info!("GPU: {} ({:?})", adapter.get_info().name, adapter.get_info().backend);
        let caps = surface.get_capabilities(&adapter);
        // a non-sRGB surface (colours written in gamma space by the shaders): the PS1 blends semi-transparent
        // prims (average, add, subtract) in gamma space, which an sRGB target would do in linear light
        let format = caps.formats.iter().copied().find(|f| !f.is_srgb()).unwrap_or(caps.formats[0]);
        let mut config = surface.get_default_config(&adapter, size.width.max(1), size.height.max(1)).ok_or("surface not supported by adapter")?;
        config.format = format;
        config.present_mode = wgpu::PresentMode::AutoVsync;
        if caps.usages.contains(wgpu::TextureUsages::COPY_DST) {
            config.usage |= wgpu::TextureUsages::COPY_DST; // AFTERIMAGE: the kept image is copied in
        }
        if caps.usages.contains(wgpu::TextureUsages::COPY_SRC) {
            config.usage |= wgpu::TextureUsages::COPY_SRC; // frame capture (OKB_SHOTS)
        }
        Self::build(instance, Some(surface), adapter, config).await
    }

    /// The device and every pipeline for `config`'s target, drawn into `surface` (None: an offscreen target,
    /// `offscreen`).
    pub(super) async fn build(
        instance: wgpu::Instance,
        surface: Option<wgpu::Surface<'static>>,
        adapter: wgpu::Adapter,
        config: wgpu::SurfaceConfiguration,
    ) -> Result<Self, String> {
        let format = config.format;
        let (device, queue) = adapter
            .request_device(&wgpu::DeviceDescriptor {
                label: Some("okb"),
                // GPU frame times for the performance overlay where the adapter has them
                required_features: adapter.features() & wgpu::Features::TIMESTAMP_QUERY,
                required_limits: wgpu::Limits::downlevel_webgl2_defaults().using_resolution(adapter.limits()),
                ..Default::default()
            })
            .await
            .map_err(|e| e.to_string())?;
        if let Some(s) = &surface {
            s.configure(&device, &config);
        }

        let inst_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("instance"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None },
                count: None,
            }],
        });
        let tex_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("texture"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
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
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shaders/model.wgsl").into()),
        });
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: None,
            bind_group_layouts: &[Some(&inst_layout), Some(&tex_layout)],
            immediate_size: 0,
        });
        let attrs = wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x3, 2 => Float32x2, 3 => Float32x4, 4 => Uint32x4, 5 => Float32x4];
        // stage textures whose CLUT the stage animates (`stage_anim`): texel indices, the stage's CLUT block and the
        // row the texture shows this frame (shader.wgsl `fs_pal`)
        let pal_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("palette"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Uint,
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: false },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 4,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None },
                    count: None,
                },
            ],
        });
        let pal_pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: None,
            bind_group_layouts: &[Some(&inst_layout), Some(&pal_layout)],
            immediate_size: 0,
        });
        let model_pipe = |label: &str, layout: &wgpu::PipelineLayout, fs: &str, compare: wgpu::CompareFunction| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(label),
                layout: Some(layout),
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: Some("vs"),
                    compilation_options: Default::default(),
                    buffers: &[Some(wgpu::VertexBufferLayout {
                        array_stride: std::mem::size_of::<Vertex>() as u64,
                        step_mode: wgpu::VertexStepMode::Vertex,
                        attributes: &attrs,
                    })],
                },
                primitive: wgpu::PrimitiveState { cull_mode: Some(wgpu::Face::Back), ..Default::default() },
                depth_stencil: Some(wgpu::DepthStencilState {
                    format: wgpu::TextureFormat::Depth32Float,
                    depth_write_enabled: Some(true),
                    depth_compare: Some(compare),
                    stencil: Default::default(),
                    bias: Default::default(),
                }),
                multisample: Default::default(),
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some(fs),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState { format, blend: None, write_mask: wgpu::ColorWrites::ALL })],
                }),
                multiview_mask: None,
                cache: None,
            })
        };
        let pipeline = model_pipe("main", &layout, "fs", wgpu::CompareFunction::Less);
        let pal_pipe = model_pipe("palette", &pal_pl, "fs_pal", wgpu::CompareFunction::Less);
        // the stage in ordering-table order (`OtDraw`): every polygon over what is there
        let ot_pipes = [
            model_pipe("stage ot", &layout, "fs", wgpu::CompareFunction::Always),
            model_pipe("stage ot palette", &pal_pl, "fs_pal", wgpu::CompareFunction::Always),
        ];
        // semi-transparent stage polygons (drawn after the opaque stage, before the fighters), in ordering-table order
        // over the stage polygons they lie on (STC's decal on the end wall): they win depth ties, and those of the
        // object the stage routine draws last (`Prim::over`) are not depth-tested at all; the STP texels are added
        // (B + F) without writing depth
        let semi_pipe = |fs: &str, blend: Option<wgpu::BlendState>, over: bool| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(fs),
                layout: Some(&layout),
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: Some("vs"),
                    compilation_options: Default::default(),
                    buffers: &[Some(wgpu::VertexBufferLayout {
                        array_stride: std::mem::size_of::<Vertex>() as u64,
                        step_mode: wgpu::VertexStepMode::Vertex,
                        attributes: &attrs,
                    })],
                },
                primitive: wgpu::PrimitiveState { cull_mode: Some(wgpu::Face::Back), ..Default::default() },
                depth_stencil: Some(wgpu::DepthStencilState {
                    format: wgpu::TextureFormat::Depth32Float,
                    depth_write_enabled: Some(blend.is_none()),
                    depth_compare: Some(if over { wgpu::CompareFunction::Always } else { wgpu::CompareFunction::LessEqual }),
                    stencil: Default::default(),
                    bias: wgpu::DepthBiasState { constant: -8, slope_scale: -1.0, clamp: 0.0 },
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
        let add = wgpu::BlendComponent { src_factor: wgpu::BlendFactor::One, dst_factor: wgpu::BlendFactor::One, operation: wgpu::BlendOperation::Add };
        let add = Some(wgpu::BlendState { color: add, alpha: add });
        let semi_pipes = [false, true].map(|over| [semi_pipe("fs_stp_opaque", None, over), semi_pipe("fs_stp_add", add, over)]);
        // fighter shadows: opaque (source + destination × (1 − alpha), alpha 1): black, or an imported stage's
        // shadow colour (`fs_shadow`), depth-tested, no depth write
        let darken_rgb =
            wgpu::BlendComponent { src_factor: wgpu::BlendFactor::One, dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha, operation: wgpu::BlendOperation::Add };
        let darken =
            wgpu::BlendComponent { src_factor: wgpu::BlendFactor::Zero, dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha, operation: wgpu::BlendOperation::Add };
        let shadow_pipe = |compare: wgpu::CompareFunction| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("shadow"),
                layout: Some(&layout),
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: Some("vs_shadow"),
                    compilation_options: Default::default(),
                    buffers: &[Some(wgpu::VertexBufferLayout {
                        array_stride: std::mem::size_of::<Vertex>() as u64,
                        step_mode: wgpu::VertexStepMode::Vertex,
                        attributes: &attrs,
                    })],
                },
                primitive: wgpu::PrimitiveState { cull_mode: None, ..Default::default() },
                depth_stencil: Some(wgpu::DepthStencilState {
                    format: wgpu::TextureFormat::Depth32Float,
                    depth_write_enabled: Some(false),
                    depth_compare: Some(compare),
                    stencil: Default::default(),
                    bias: Default::default(),
                }),
                multisample: Default::default(),
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some("fs_shadow"),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format,
                        blend: Some(wgpu::BlendState { color: darken_rgb, alpha: darken }),
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                multiview_mask: None,
                cache: None,
            })
        };
        // depth-tested against the stage; over all of it (slot 0x810) when the stage is drawn in OT order
        let shadow_pipes = [shadow_pipe(wgpu::CompareFunction::LessEqual), shadow_pipe(wgpu::CompareFunction::Always)];
        // 2D overlay (HUD): screen-space coloured triangles, alpha blended, no depth
        let flat_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("flat"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shaders/flat.wgsl").into()),
        });
        let flat_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: None, bind_group_layouts: &[], immediate_size: 0 });
        let flat_attrs = wgpu::vertex_attr_array![0 => Float32x2, 1 => Float32x4];
        let flat = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("flat"),
            layout: Some(&flat_layout),
            vertex: wgpu::VertexState {
                module: &flat_shader,
                entry_point: Some("vs"),
                compilation_options: Default::default(),
                buffers: &[Some(wgpu::VertexBufferLayout { array_stride: 24, step_mode: wgpu::VertexStepMode::Vertex, attributes: &flat_attrs })],
            },
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
                module: &flat_shader,
                entry_point: Some("fs"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState { format, blend: Some(wgpu::BlendState::ALPHA_BLENDING), write_mask: wgpu::ColorWrites::ALL })],
            }),
            multiview_mask: None,
            cache: None,
        });
        let depth_reset = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("depth reset"),
            layout: Some(&flat_layout),
            vertex: wgpu::VertexState { module: &flat_shader, entry_point: Some("vs_far"), compilation_options: Default::default(), buffers: &[] },
            primitive: Default::default(),
            depth_stencil: Some(wgpu::DepthStencilState {
                format: wgpu::TextureFormat::Depth32Float,
                depth_write_enabled: Some(true),
                depth_compare: Some(wgpu::CompareFunction::Always),
                stencil: Default::default(),
                bias: Default::default(),
            }),
            multisample: Default::default(),
            fragment: Some(wgpu::FragmentState {
                module: &flat_shader,
                entry_point: Some("fs"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState { format, blend: None, write_mask: wgpu::ColorWrites::empty() })],
            }),
            multiview_mask: None,
            cache: None,
        });
        // the ambient fill (engine addition): a copied picture, blurred and darkened, no depth
        let amb_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("ambient"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shaders/ambient.wgsl").into()),
        });
        let amb_layout =
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: None, bind_group_layouts: &[Some(&tex_layout)], immediate_size: 0 });
        let amb_attrs = wgpu::vertex_attr_array![0 => Float32x2, 1 => Float32x2];
        // the 3/8 tint in gamma space: on an sRGB target the shader's output is linear
        let dim = [("dim", if format.is_srgb() { 0.375f64.powf(2.2) } else { 0.375 })];
        // `blur` into the small texture (no depth), `up` into the bars
        let amb_pipeline = |fs: &str, depth: bool| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("ambient"),
                layout: Some(&amb_layout),
                vertex: wgpu::VertexState {
                    module: &amb_shader,
                    entry_point: Some("vs"),
                    compilation_options: Default::default(),
                    buffers: &[Some(wgpu::VertexBufferLayout { array_stride: 16, step_mode: wgpu::VertexStepMode::Vertex, attributes: &amb_attrs })],
                },
                primitive: wgpu::PrimitiveState { cull_mode: None, ..Default::default() },
                depth_stencil: depth.then(|| wgpu::DepthStencilState {
                    format: wgpu::TextureFormat::Depth32Float,
                    depth_write_enabled: Some(false),
                    depth_compare: Some(wgpu::CompareFunction::Always),
                    stencil: Default::default(),
                    bias: Default::default(),
                }),
                multisample: Default::default(),
                fragment: Some(wgpu::FragmentState {
                    module: &amb_shader,
                    entry_point: Some(fs),
                    compilation_options: wgpu::PipelineCompilationOptions { constants: if depth { &dim } else { &[] }, ..Default::default() },
                    targets: &[Some(wgpu::ColorTargetState { format, blend: None, write_mask: wgpu::ColorWrites::ALL })],
                }),
                multiview_mask: None,
                cache: None,
            })
        };
        let (amb_blur, amb_pipe) = (amb_pipeline("blur", false), amb_pipeline("up", true));
        let lo_pipe = amb_pipeline("copy", false);
        let amb_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        // PS1 effect quads sampling a copy of VRAM
        let vram_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("vram"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Uint,
                    view_dimension: wgpu::TextureViewDimension::D2,
                    multisampled: false,
                },
                count: None,
            }],
        });
        // the VRAM copy: bank 0 in rows 0..511, bank 1 (engine addition, the team battle's fighters 2 / 3: a CLUT word with
        // bit 15 set) in rows 512..1023
        let vram_tex = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("vram"),
            size: wgpu::Extent3d { width: 1024, height: 1024, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::R16Uint,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let vram_view = vram_tex.create_view(&Default::default());
        let vram_bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("vram"),
            layout: &vram_layout,
            entries: &[wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&vram_view) }],
        });
        let psx_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("psx"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shaders/psx.wgsl").into()),
        });
        let psx_layout =
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: None, bind_group_layouts: &[Some(&vram_layout)], immediate_size: 0 });
        let psx_attrs = wgpu::vertex_attr_array![0 => Float32x4, 1 => Float32x2, 2 => Float32x4, 3 => Uint32x4];
        let add = |src, dst, op| {
            Some(wgpu::BlendState { color: wgpu::BlendComponent { src_factor: src, dst_factor: dst, operation: op }, alpha: wgpu::BlendComponent::REPLACE })
        };
        use wgpu::{BlendFactor as F, BlendOperation as O};
        let blends =
            [None, add(F::One, F::Constant, O::Add), add(F::One, F::One, O::Add), add(F::One, F::One, O::ReverseSubtract), add(F::One, F::One, O::Add)];
        let quad_pipes = |shader: &wgpu::ShaderModule, layout: &wgpu::PipelineLayout| -> Vec<wgpu::RenderPipeline> {
            blends
                .iter()
                .map(|b| {
                    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                        label: Some("psx"),
                        layout: Some(layout),
                        vertex: wgpu::VertexState {
                            module: shader,
                            entry_point: Some("vs"),
                            compilation_options: Default::default(),
                            buffers: &[Some(wgpu::VertexBufferLayout { array_stride: 56, step_mode: wgpu::VertexStepMode::Vertex, attributes: &psx_attrs })],
                        },
                        primitive: wgpu::PrimitiveState { cull_mode: None, ..Default::default() },
                        depth_stencil: Some(wgpu::DepthStencilState {
                            format: wgpu::TextureFormat::Depth32Float,
                            depth_write_enabled: Some(false),
                            depth_compare: Some(wgpu::CompareFunction::LessEqual),
                            stencil: Default::default(),
                            bias: Default::default(),
                        }),
                        multisample: Default::default(),
                        fragment: Some(wgpu::FragmentState {
                            module: shader,
                            entry_point: Some("fs"),
                            compilation_options: Default::default(),
                            targets: &[Some(wgpu::ColorTargetState { format, blend: *b, write_mask: wgpu::ColorWrites::ALL })],
                        }),
                        multiview_mask: None,
                        cache: None,
                    })
                })
                .collect()
        };
        let psx = quad_pipes(&psx_shader, &psx_layout);
        // atlas quads (engine addition): the same five pipelines over the RGBA sprite atlas
        let atlas_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("atlas"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Float { filterable: false },
                    view_dimension: wgpu::TextureViewDimension::D2Array,
                    multisampled: false,
                },
                count: None,
            }],
        });
        let sprite_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("sprite"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shaders/sprite.wgsl").into()),
        });
        let sprite_layout =
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: None, bind_group_layouts: &[Some(&atlas_layout)], immediate_size: 0 });
        let sprite = quad_pipes(&sprite_shader, &sprite_layout);
        let (atlas_tex, atlas_bind) = make_atlas(&device, &atlas_layout, 2);
        #[cfg(not(target_arch = "wasm32"))]
        let (sprites_old, sprites_ab) = match crate::base::env::var("SPRITES").as_deref() {
            Ok("old") => (true, false),
            Ok("ab") => (false, true),
            _ => (false, false),
        };
        #[cfg(target_arch = "wasm32")]
        let (sprites_old, sprites_ab) = (false, false);
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Nearest,
            min_filter: wgpu::FilterMode::Nearest,
            ..Default::default()
        });
        let depth = Self::make_depth(&device, &config);
        let lit = super::lightmapped::LitPipes::new(&device, &queue, format, &inst_layout, &tex_layout);
        let white = make_texture(&device, &queue, &tex_layout, &sampler, 1, 1, &[255, 255, 255, 254]);
        let white_stp = make_texture(&device, &queue, &tex_layout, &sampler, 1, 1, &[255, 255, 255, 128]);
        let vp = Viewport::new([config.width, config.height], Fit::Fill);
        let timer = device.features().contains(wgpu::Features::TIMESTAMP_QUERY).then(|| GpuTimer {
            set: device.create_query_set(&wgpu::QuerySetDescriptor { label: Some("frame time"), ty: wgpu::QueryType::Timestamp, count: 2 }),
            resolve: device.create_buffer(&wgpu::BufferDescriptor {
                label: None,
                size: 16,
                usage: wgpu::BufferUsages::QUERY_RESOLVE | wgpu::BufferUsages::COPY_SRC,
                mapped_at_creation: false,
            }),
            read: (0..3)
                .map(|_| {
                    let b = device.create_buffer(&wgpu::BufferDescriptor {
                        label: None,
                        size: 16,
                        usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
                        mapped_at_creation: false,
                    });
                    (b, Arc::new(std::sync::atomic::AtomicU8::new(0)))
                })
                .collect(),
            period: queue.get_timestamp_period(),
        });
        Ok(Self {
            device,
            queue,
            instance,
            surface,
            config,
            depth,
            pipeline,
            pal_pipe,
            semi_pipes,
            pal_layout,
            shadow_pipes,
            ot_pipes,
            depth_reset,
            flat,
            lit,
            psx,
            vram_bind,
            vram_tex,
            sprite,
            atlas: Atlas::default(),
            atlas_layout,
            atlas_tex,
            atlas_bind,
            sprites_old,
            sprites_ab,
            inst_layout,
            tex_layout,
            sampler,
            white,
            white_stp,
            manual_srgb: !format.is_srgb(),
            walls_hidden: 0,
            afterimage: 0,
            backdrop: None,
            persist: None,
            persist_used: false,
            vp,
            hud_insets: None,
            ambient: None,
            amb_blur,
            amb_pipe,
            amb_sampler,
            amb: None,
            presented: true,
            acquire_failure: None,
            perf: false,
            stats: FrameStats::default(),
            uploads: 0,
            timer,
            render_scale: 1.0,
            scaled_at: None,
            hud_scale: 1.0,
            lo: None,
            lo_pipe,
            out: None,
            streams: [None, None, None, None, None],
            screen: Vec::new(),
            #[cfg(not(target_arch = "wasm32"))]
            offscreen: None,
        })
    }
}
