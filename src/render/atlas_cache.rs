//! The 2D sprite atlas: sprite rects decoded once and packed into a texture array.
//!
//! Engine addition.
//! State: presentation (reads the state, never writes it)
//!
//! Engine addition (not in the console game): the 2D sprite atlas. Textured 2D quads name a sheet (a texture page of
//! the loaded TIMs decoded with one palette) and a texel rect of it; the atlas decodes that rect once into RGBA
//! (lazily, on first use) and packs it into a shelf-packed 1024² texture array the sprite shader samples.
//!
//! The source is a private staging composite of the loaded TIM data (1024×512 halfwords, filled in the game's load
//! order, so overlapping loads resolve as they did); draw code never sees it. Texels: halfword 0x0000 →
//! transparent (alpha 0); otherwise rgb = round(c5·255/31), alpha 255, or 128 when the STP bit is set.
//!
//! The renderer's side: `flush_atlas` copies the rects baked since the last frame to the GPU, growing the texture
//! array (`make_atlas`).

use super::Renderer;
use crate::output::sprite::{Depth, Sheet};
use std::collections::HashMap;

pub const SIZE: u32 = 1024;
/// layers the atlas may grow to (4 MB each)
pub const MAX_LAYERS: u32 = 4;

/// Where a baked rect lives in the atlas: layer, top-left texel, and the sheet rect (u, v, w, h) it holds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Region {
    pub layer: u32,
    pub origin: [u16; 2],
    pub rect: [u16; 4],
}

#[derive(Clone, Copy, Debug)]
struct Shelf {
    layer: u32,
    y: u32,
    h: u32,
    x: u32,
}

pub struct Atlas {
    /// the staging composite (1024×512 halfwords)
    src: Vec<u16>,
    map: HashMap<(Sheet, [u16; 4]), Option<Region>>,
    shelves: Vec<Shelf>,
    /// next free row of the newest layer
    top: (u32, u32),
    /// layers in use
    pub layers: u32,
    /// rects baked since the last `take_uploads`: region and its RGBA texels
    uploads: Vec<(Region, Vec<u8>)>,
    overflowed: bool,
}

impl Default for Atlas {
    fn default() -> Self {
        Self { src: vec![0; 1024 * 512], map: HashMap::new(), shelves: Vec::new(), top: (0, 0), layers: 1, uploads: Vec::new(), overflowed: false }
    }
}

/// The texel rect a quad with these texel coordinates samples (one texel of margin each side for the rounding
/// of the interpolated coordinates), or None when it leaves the page (texel wrap: not bakeable as one rect).
pub fn rect_for(uv: &[[f32; 2]; 4]) -> Option<[u16; 4]> {
    let (mut lo, mut hi) = ([f32::MAX; 2], [f32::MIN; 2]);
    for p in uv {
        for k in 0..2 {
            lo[k] = lo[k].min(p[k]);
            hi[k] = hi[k].max(p[k]);
        }
    }
    if lo[0] < 0.0 || lo[1] < 0.0 || hi[0] > 256.0 || hi[1] > 256.0 {
        return None;
    }
    let a = |k: usize| (lo[k].floor() as i32 - 1).max(0) as u16;
    let b = |k: usize| (hi[k].ceil() as i32 + 1).min(256) as u16;
    Some([a(0), a(1), b(0) - a(0), b(1) - a(1)])
}

impl Atlas {
    /// Replace the staging composite (a load changed the loaded data); every baked rect is dropped when it differs.
    pub fn set_source(&mut self, words: &[u16]) {
        if self.src[..] == words[..words.len().min(self.src.len())] {
            return;
        }
        let n = words.len().min(self.src.len());
        self.src[..n].copy_from_slice(&words[..n]);
        self.map.clear();
        self.shelves.clear();
        self.top = (0, 0);
        self.layers = 1;
        self.uploads.clear();
        self.overflowed = false;
    }

    fn word(&self, x: u32, y: u32) -> u16 {
        self.src[((y & 511) * 1024 + (x & 1023)) as usize]
    }

    /// The halfword colour of texel (u, v) of a sheet, as the GPU fetches it.
    fn texel(&self, s: &Sheet, u: u32, v: u32) -> u16 {
        let (px, py) = (s.page.x as u32, s.page.y as u32);
        let (cx, cy) = (s.pal.x as u32, s.pal.y as u32);
        match s.page.depth {
            Depth::P4 => {
                let w = self.word(px + u / 4, py + v);
                self.word(cx + ((w >> ((u & 3) * 4)) & 15) as u32, cy)
            }
            Depth::P8 => {
                let w = self.word(px + u / 2, py + v);
                self.word(cx + ((w >> ((u & 1) * 8)) & 255) as u32, cy)
            }
            Depth::Direct => self.word(px + u, py + v),
        }
    }

    /// RGBA texels of a sheet rect.
    fn decode(&self, s: &Sheet, rect: [u16; 4]) -> Vec<u8> {
        let [u0, v0, w, h] = rect.map(u32::from);
        let mut out = Vec::with_capacity((w * h * 4) as usize);
        for v in v0..v0 + h {
            for u in u0..u0 + w {
                out.extend_from_slice(&rgba(self.texel(s, u & 255, v & 255)));
            }
        }
        out
    }

    /// The region holding `rect` of `sheet`, baked on first use; None when the atlas is full.
    pub fn get(&mut self, sheet: Sheet, rect: [u16; 4]) -> Option<Region> {
        if let Some(r) = self.map.get(&(sheet, rect)) {
            return *r;
        }
        let r = self.alloc(rect[2] as u32, rect[3] as u32).map(|(layer, x, y)| Region { layer, origin: [x as u16, y as u16], rect });
        match r {
            Some(r) => self.uploads.push((r, self.decode(&sheet, rect))),
            None if !self.overflowed => {
                self.overflowed = true;
                log::warn!("sprite atlas full ({MAX_LAYERS} layers): further sprites use the old path");
            }
            None => {}
        }
        self.map.insert((sheet, rect), r);
        r
    }

    /// Shelf packing: the first shelf of this height class with room, else a new shelf (a new layer when full).
    fn alloc(&mut self, w: u32, h: u32) -> Option<(u32, u32, u32)> {
        if let Some(s) = self.shelves.iter_mut().find(|s| s.h >= h && s.h <= h + h / 4 + 1 && s.x + w <= SIZE) {
            s.x += w;
            return Some((s.layer, s.x - w, s.y));
        }
        let (mut layer, mut y) = self.top;
        if y + h > SIZE {
            (layer, y) = (layer + 1, 0);
        }
        if layer >= MAX_LAYERS || w > SIZE {
            return None;
        }
        self.shelves.push(Shelf { layer, y, h, x: w });
        self.top = (layer, y + h);
        self.layers = self.layers.max(layer + 1);
        Some((layer, 0, y))
    }

    /// The rects baked since the last call, for the GPU copy.
    pub fn take_uploads(&mut self) -> Vec<(Region, Vec<u8>)> {
        std::mem::take(&mut self.uploads)
    }

    /// Baked variants and the texels they use.
    pub fn stats(&self) -> (usize, u64) {
        let regions = self.map.values().flatten();
        (self.map.len(), regions.map(|r| r.rect[2] as u64 * r.rect[3] as u64).sum())
    }

    /// One layer as an RGBA image (OKB_ATLASDUMP), decoded again from the composite.
    pub fn layer_image(&self, layer: u32) -> Vec<u8> {
        let mut img = vec![0u8; (SIZE * SIZE * 4) as usize];
        for ((sheet, rect), r) in &self.map {
            let Some(r) = r.filter(|r| r.layer == layer) else { continue };
            let px = self.decode(sheet, *rect);
            let w = rect[2] as usize * 4;
            for (k, row) in px.chunks_exact(w).enumerate() {
                let o = ((r.origin[1] as usize + k) * SIZE as usize + r.origin[0] as usize) * 4;
                img[o..o + w].copy_from_slice(row);
            }
        }
        img
    }
}

/// The texel rule: 0x0000 transparent, STP → alpha 128, else 255; 5-bit channels widened with rounding.
pub fn rgba(c: u16) -> [u8; 4] {
    if c == 0 {
        return [0; 4];
    }
    let ch = |s: u16| (((c >> s) & 31) as u32 * 255 + 15) / 31;
    [ch(0) as u8, ch(5) as u8, ch(10) as u8, if c & 0x8000 != 0 { 128 } else { 255 }]
}

impl Renderer {
    /// Copy the atlas rects baked since the last frame to the GPU, growing the texture array when needed.
    pub(super) fn flush_atlas(&mut self) {
        let have = self.atlas_tex.size().depth_or_array_layers;
        if self.atlas.layers > have {
            // keep at least 2 layers: WebGL2 makes a 1-layer texture a plain 2D one
            let (tex, bind) = make_atlas(&self.device, &self.atlas_layout, self.atlas.layers.max(2));
            let mut enc = self.device.create_command_encoder(&Default::default());
            enc.copy_texture_to_texture(
                self.atlas_tex.as_image_copy(),
                tex.as_image_copy(),
                wgpu::Extent3d { width: SIZE, height: SIZE, depth_or_array_layers: have },
            );
            self.queue.submit([enc.finish()]);
            (self.atlas_tex, self.atlas_bind) = (tex, bind);
        }
        let ups = self.atlas.take_uploads();
        self.uploads += ups.len() as u32;
        for (r, px) in &ups {
            self.queue.write_texture(
                wgpu::TexelCopyTextureInfo {
                    texture: &self.atlas_tex,
                    mip_level: 0,
                    origin: wgpu::Origin3d { x: r.origin[0] as u32, y: r.origin[1] as u32, z: r.layer },
                    aspect: wgpu::TextureAspect::All,
                },
                px,
                wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(4 * r.rect[2] as u32), rows_per_image: None },
                wgpu::Extent3d { width: r.rect[2] as u32, height: r.rect[3] as u32, depth_or_array_layers: 1 },
            );
        }
        #[cfg(not(target_arch = "wasm32"))]
        if !ups.is_empty() {
            let (n, texels) = self.atlas.stats();
            log::debug!("sprite atlas: {n} variants, {texels} texels, {} layers", self.atlas.layers);
            // OKB_ATLASDUMP=prefix: the layers as prefix_<layer>.png whenever something was baked
            if let Ok(prefix) = crate::base::env::var("ATLASDUMP") {
                for l in 0..self.atlas.layers {
                    let s = SIZE;
                    let _ = image::save_buffer(format!("{prefix}_{l}.png"), &self.atlas.layer_image(l), s, s, image::ExtendedColorType::Rgba8);
                }
            }
        }
    }
}

/// The atlas texture array (Rgba8Unorm, `layers` × 1024²) and its bind group.
pub(super) fn make_atlas(device: &wgpu::Device, layout: &wgpu::BindGroupLayout, layers: u32) -> (wgpu::Texture, wgpu::BindGroup) {
    let tex = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("atlas"),
        size: wgpu::Extent3d { width: SIZE, height: SIZE, depth_or_array_layers: layers },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let view = tex.create_view(&wgpu::TextureViewDescriptor { dimension: Some(wgpu::TextureViewDimension::D2Array), ..Default::default() });
    let bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("atlas"),
        layout,
        entries: &[wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&view) }],
    });
    (tex, bind)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::output::sprite::{Page, Pal};

    #[test]
    fn texel_rule() {
        assert_eq!(rgba(0), [0; 4]);
        assert_eq!(rgba(0x8000), [0, 0, 0, 128]);
        assert_eq!(rgba(0x7FFF), [255, 255, 255, 255]);
        // the 8-bit value recovers c5 exactly (the shader's round(t·31))
        for c5 in 0..32u16 {
            let t = rgba(c5 | 0x400)[0] as f32 / 255.0;
            assert_eq!((t * 31.0).round() as u16, c5);
        }
    }

    #[test]
    fn decode_p4_and_pack() {
        let mut words = vec![0u16; 1024 * 512];
        words[0] = 0x3210; // texels 0..3 of page (0,0) = palette entries 0..3
        for k in 0..4 {
            words[480 * 1024 + 16 + k] = [0, 0x001F, 0x83E0, 0x7C00][k];
        }
        let mut a = Atlas::default();
        a.set_source(&words);
        let sheet = Sheet { page: Page::p4(0, 0), pal: Pal { x: 16, y: 480 } };
        let r = a.get(sheet, [0, 0, 4, 1]).unwrap();
        let up = a.take_uploads();
        assert_eq!(up.len(), 1);
        assert_eq!(up[0].1, [[0; 4], [255, 0, 0, 255], [0, 255, 0, 128], [0, 0, 255, 255]].concat());
        assert_eq!(a.get(sheet, [0, 0, 4, 1]), Some(r));
        let r2 = a.get(sheet, [0, 0, 4, 2]).unwrap();
        assert_ne!(r.origin, r2.origin);
        assert_eq!(rect_for(&[[64.0, 160.0], [104.0, 160.0], [64.0, 192.0], [104.0, 192.0]]), Some([63, 159, 42, 34]));
        assert_eq!(Page::p4(832, 256).tpage() | 0x40, 0x5D);
        assert_eq!(Page::p4(896, 0).tpage(), 0x0E);
        assert_eq!(Pal { x: 480, y: 480 }.clut(), 0x781E);
    }
}
