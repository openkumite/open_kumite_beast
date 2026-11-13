//! Render targets and textures: depth, the ambient fill, the scaled scene.
//!
//! State: presentation (reads the state, never writes it)
//!
//! Render targets and textures: the depth buffer (`make_depth`), the ambient fill's textures (`amb_texture`,
//! `ambient_verts`; engine addition), RENDER QUALITY's scaled scene (`lo_target`; engine addition), a texture's bind group
//! (`make_texture`).

use super::{AmbTextures, LoTarget, Renderer};
use wgpu::util::DeviceExt;

impl Renderer {
    pub(super) fn make_depth(device: &wgpu::Device, config: &wgpu::SurfaceConfiguration) -> wgpu::TextureView {
        device
            .create_texture(&wgpu::TextureDescriptor {
                label: Some("depth"),
                size: wgpu::Extent3d { width: config.width, height: config.height, depth_or_array_layers: 1 },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Depth32Float,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
                view_formats: &[],
            })
            .create_view(&Default::default())
    }

    /// The ambient fill's textures for a copied picture of `w`×`h`: the copy and its blur at 1/4 size, both in the
    /// surface format.
    pub(super) fn amb_texture(&mut self, w: u32, h: u32) {
        if self.amb.as_ref().is_some_and(|a| a.copy.width() == w && a.copy.height() == h) {
            return;
        }
        let texture = |label, w: u32, h: u32, usage| {
            let tex = self.device.create_texture(&wgpu::TextureDescriptor {
                label: Some(label),
                size: wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: self.config.format,
                usage: wgpu::TextureUsages::TEXTURE_BINDING | usage,
                view_formats: &[],
            });
            let view = tex.create_view(&Default::default());
            let bind = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some(label),
                layout: &self.tex_layout,
                entries: &[
                    wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&view) },
                    wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(&self.amb_sampler) },
                ],
            });
            (tex, view, bind)
        };
        let (copy, _, copy_bind) = texture("ambient", w, h, wgpu::TextureUsages::COPY_DST);
        let (small, small_view, small_bind) = texture("ambient blur", w.div_ceil(4), h.div_ceil(4), wgpu::TextureUsages::RENDER_ATTACHMENT);
        self.amb = Some(AmbTextures { copy, copy_bind, small, small_view, small_bind });
    }

    /// RENDER QUALITY (engine addition): the scaled scene's colour and depth targets for this window size and scale
    /// (at most the device's texture size), sampled nearest when scaled down (whole low-resolution pixels), linearly
    /// when scaled up (the supersampled scene averaged).
    pub(super) fn lo_target(&mut self) {
        let max = self.device.limits().max_texture_dimension_2d;
        let dim = |v: u32| ((v as f32 * self.render_scale).round() as u32).clamp(1, max);
        let size = [dim(self.config.width), dim(self.config.height)];
        let nearest = self.render_scale < 1.0;
        if self.lo.as_ref().is_some_and(|t| t.size == size && t.nearest == nearest && t.format == self.config.format) {
            return;
        }
        let target = |label, format, usage| {
            self.device.create_texture(&wgpu::TextureDescriptor {
                label: Some(label),
                size: wgpu::Extent3d { width: size[0], height: size[1], depth_or_array_layers: 1 },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT | usage,
                view_formats: &[],
            })
        };
        let tex = target("scene", self.config.format, wgpu::TextureUsages::TEXTURE_BINDING);
        let view = tex.create_view(&Default::default());
        let depth = target("scene depth", wgpu::TextureFormat::Depth32Float, wgpu::TextureUsages::empty()).create_view(&Default::default());
        let bind = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("scene"),
            layout: &self.tex_layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&view) },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(if nearest { &self.sampler } else { &self.amb_sampler }) },
            ],
        });
        // the whole window (clip x, y, u, v), as the first six vertices of `ambient_verts`
        let mut q: Vec<f32> = Vec::new();
        for (px, py) in [(0.0f32, 0.0f32), (1.0, 0.0), (1.0, 1.0), (0.0, 0.0), (1.0, 1.0), (0.0, 1.0)] {
            q.extend_from_slice(&[px * 2.0 - 1.0, 1.0 - py * 2.0, px, py]);
        }
        let quad = self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("scene quad"),
            contents: bytemuck::cast_slice(&q),
            usage: wgpu::BufferUsages::VERTEX,
        });
        self.lo = Some(LoTarget { size, nearest, format: self.config.format, tex, view, depth, bind, quad });
    }

    /// The ambient fill's triangles (clip x, y, u, v): first the whole small texture (the blur), then the window
    /// outside `rect` as up to four rectangles, the copied rect scaled about its centre until it covers the window.
    pub(super) fn ambient_verts(&self, rect: [u32; 4]) -> Vec<f32> {
        let (w, h) = (self.config.width as f32, self.config.height as f32);
        let [x, y, rw, rh] = rect.map(|v| v as f32);
        let (cx, cy) = (x + rw / 2.0, y + rh / 2.0);
        let s = [cx / (rw / 2.0), (w - cx) / (rw / 2.0), cy / (rh / 2.0), (h - cy) / (rh / 2.0)].into_iter().fold(1.0f32, f32::max);
        let mut v = Vec::new();
        for (px, py) in [(0.0f32, 0.0f32), (1.0, 0.0), (1.0, 1.0), (0.0, 0.0), (1.0, 1.0), (0.0, 1.0)] {
            v.extend_from_slice(&[px * 2.0 - 1.0, 1.0 - py * 2.0, px, py]);
        }
        let (x1, y1) = (x + rw, y + rh);
        for [a, b, c, d] in [[0.0, 0.0, x, h], [x1, 0.0, w, h], [x, 0.0, x1, y], [x, y1, x1, h]] {
            if c <= a || d <= b {
                continue;
            }
            for (px, py) in [(a, b), (c, b), (c, d), (a, b), (c, d), (a, d)] {
                v.extend_from_slice(&[px / w * 2.0 - 1.0, 1.0 - py / h * 2.0, 0.5 + (px - cx) / (s * rw), 0.5 + (py - cy) / (s * rh)]);
            }
        }
        v
    }
}

pub(super) fn make_texture(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    layout: &wgpu::BindGroupLayout,
    sampler: &wgpu::Sampler,
    w: u32,
    h: u32,
    rgba: &[u8],
) -> wgpu::BindGroup {
    let tex = device.create_texture_with_data(
        queue,
        &wgpu::TextureDescriptor {
            label: None,
            size: wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8UnormSrgb,
            usage: wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        },
        wgpu::util::TextureDataOrder::LayerMajor,
        rgba,
    );
    let view = tex.create_view(&Default::default());
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: None,
        layout,
        entries: &[
            wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&view) },
            wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(sampler) },
        ],
    })
}
