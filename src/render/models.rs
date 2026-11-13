//! Models on the GPU: uploads, palettes, paletted textures.
//!
//! State: presentation (reads the state, never writes it)
//!
//! Models on the GPU: `upload` (a `ModelData`'s buffers, textures and draws), palettes and paletted textures
//! (`palette`, `set_paletted`, `set_palette_row`: a texture drawn from texel indices and a CLUT row).

use super::targets::make_texture;
use super::{Draw, GpuModel, PalTexture, Renderer};
use crate::formats::gltf_model::ModelData;
use wgpu::util::DeviceExt;

/// A palette for `PalTexture`s (Rgba8UnormSrgb, one row per CLUT, as the textures' PNGs hold their colours).
pub struct Palette {
    view: wgpu::TextureView,
}

impl GpuModel {
    /// The model has shadow proxies (`Prim::shadow`).
    pub fn has_shadow_proxy(&self) -> bool {
        self.draws.iter().any(|d| d.shadow)
    }
}

impl Renderer {
    fn texture_bind(&self, w: u32, h: u32, rgba: &[u8]) -> wgpu::BindGroup {
        make_texture(&self.device, &self.queue, &self.tex_layout, &self.sampler, w, h, rgba)
    }

    pub fn upload(&self, m: &ModelData) -> GpuModel {
        let vbuf = self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: None,
            contents: bytemuck::cast_slice(&m.vertices),
            usage: wgpu::BufferUsages::VERTEX,
        });
        let ibuf = self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: None,
            contents: bytemuck::cast_slice(&m.indices),
            usage: wgpu::BufferUsages::INDEX | wgpu::BufferUsages::COPY_DST,
        });
        let textures = m.images.iter().map(|i| self.texture_bind(i.width, i.height, &i.rgba)).collect();
        let draws = m
            .prims
            .iter()
            .map(|p| Draw {
                first: p.first_index,
                count: p.count,
                tex: p.texture.map(|t| t + 1).unwrap_or(0),
                hidden: p.hidden,
                wall: p.wall,
                shadow: p.shadow,
                over: p.over,
                semi: p.semi,
            })
            .collect();
        let tex = m.images.iter().map(|i| 4 * i.width as u64 * i.height as u64).sum();
        GpuModel { bytes: [tex, vbuf.size() + ibuf.size()], vbuf, ibuf, draws, pal: m.images.iter().map(|_| None).collect(), textures, lit: None }
    }

    /// Overwrites `gm`'s indices from index `first` on with `idx` (written before the next submit's draws).
    pub fn write_indices(&self, gm: &GpuModel, first: u32, idx: &[u32]) {
        self.queue.write_buffer(&gm.ibuf, first as u64 * 4, bytemuck::cast_slice(idx));
    }

    /// A palette of `w`×`h` colours (RGBA, alpha 0 = transparent).
    pub fn palette(&self, w: u32, h: u32, rgba: &[u8]) -> Palette {
        let tex = self.device.create_texture_with_data(
            &self.queue,
            &wgpu::TextureDescriptor {
                label: Some("palette"),
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
        Palette { view: tex.create_view(&Default::default()) }
    }

    /// Texture `image` of `gm` is drawn from `index` (one palette index per texel, the image's size) and palette
    /// row (x, y) of `pal` (the colours of indices 0.. at x, x + 1 …) instead of its colours; `set_palette_row`
    /// moves the row.
    pub fn set_paletted(&self, gm: &mut GpuModel, image: usize, w: u32, h: u32, index: &[u8], pal: &Palette, row: [u32; 2]) {
        let tex = self.device.create_texture_with_data(
            &self.queue,
            &wgpu::TextureDescriptor {
                label: Some("palette index"),
                size: wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::R8Uint,
                usage: wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            },
            wgpu::util::TextureDataOrder::LayerMajor,
            index,
        );
        let view = tex.create_view(&Default::default());
        let buf = self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("palette row"),
            contents: bytemuck::cast_slice(&[row[0], row[1], 0, 0]),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });
        let bind = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("palette"),
            layout: &self.pal_layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::TextureView(&view) },
                wgpu::BindGroupEntry { binding: 3, resource: wgpu::BindingResource::TextureView(&pal.view) },
                wgpu::BindGroupEntry { binding: 4, resource: buf.as_entire_binding() },
            ],
        });
        gm.bytes[0] += w as u64 * h as u64;
        // engine addition: the image's RGBA texture is drawn no more (every draw but a semi-transparent one takes the
        // palette), so it is let go
        if !gm.draws.iter().any(|d| d.tex == image + 1 && d.semi)
            && let Some(t) = gm.textures.get_mut(image)
        {
            *t = self.white.clone();
            gm.bytes[0] = gm.bytes[0].saturating_sub(4 * w as u64 * h as u64);
        }
        if let Some(p) = gm.pal.get_mut(image) {
            *p = Some(PalTexture { bind, row: buf });
        }
    }

    /// The palette row a paletted texture of `gm` shows from now on (16 bytes written; the queue applies them before
    /// the next submit's draws, so they hold for the frame being recorded).
    pub fn set_palette_row(&self, gm: &GpuModel, image: usize, row: [u32; 2]) {
        if let Some(Some(p)) = gm.pal.get(image) {
            self.queue.write_buffer(&p.row, 0, bytemuck::cast_slice(&[row[0], row[1], 0, 0]));
        }
    }
}
