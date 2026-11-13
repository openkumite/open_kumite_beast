//! The windowless renderer and its frame readback (engine addition, `replay --video`).
//!
//! State: presentation (reads the state, never writes it)
//!
//! `Renderer::offscreen`: the device without a surface; every frame is drawn into a texture of the video's size in
//! `FORMAT` (the non-sRGB BGRA that `Renderer::new` takes for a desktop window), the same passes as on a window, then
//! copied into the next buffer of a small ring and mapped without waiting (`frame_once` → `Offscreen::copy`,
//! `submitted`). `take_frames` hands the mapped frames over in order, waiting only for the oldest when more than
//! `keep` are in flight, so the GPU is not stalled and at most the ring's frames are held; `repeat_frame` reads the
//! last frame again (a step that drew nothing shows the frame before, as a window does).

use super::Renderer;
use std::collections::VecDeque;
use std::sync::Arc;
use std::sync::atomic::{AtomicU8, Ordering};

/// The offscreen target's format: what a desktop surface gives `Renderer::new` (the first non-sRGB format; BGRA8 on
/// Metal), so that the shaders' gamma-space blending and the picture are the window's.
pub const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Bgra8Unorm;

/// A readback slot's state.
const FREE: u8 = 0;
const MAPPING: u8 = 1;
const MAPPED: u8 = 2;
const FAILED: u8 = 3;

struct Slot {
    buf: wgpu::Buffer,
    state: Arc<AtomicU8>,
    /// the submission that copies the frame in
    sub: Option<wgpu::SubmissionIndex>,
}

/// The target texture and the ring of readback buffers.
pub(super) struct Offscreen {
    tex: wgpu::Texture,
    slots: Vec<Slot>,
    /// the slots in flight, oldest first
    queue: VecDeque<usize>,
    /// a readback row's bytes (256-aligned)
    row: u32,
    size: [u32; 2],
    /// a frame's rows without their padding (sizes whose rows are not 256-aligned)
    scratch: Vec<u8>,
    /// a frame found no free slot (`take_frames` stops with an error)
    dropped: bool,
}

impl Offscreen {
    fn new(device: &wgpu::Device, config: &wgpu::SurfaceConfiguration, n: usize) -> Self {
        let (w, h) = (config.width, config.height);
        let tex = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("offscreen"),
            size: wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: config.format,
            usage: config.usage,
            view_formats: &[],
        });
        let row = row_bytes(w);
        let slots = (0..n.max(1))
            .map(|_| Slot {
                buf: device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("frame readback"),
                    size: row as u64 * h as u64,
                    usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                    mapped_at_creation: false,
                }),
                state: Arc::new(AtomicU8::new(FREE)),
                sub: None,
            })
            .collect();
        Self { tex, slots, queue: VecDeque::new(), row, size: [w, h], scratch: Vec::new(), dropped: false }
    }

    /// A new size: the target and the ring made again (the frames in flight are dropped).
    pub(super) fn resize(&mut self, device: &wgpu::Device, config: &wgpu::SurfaceConfiguration) {
        if self.size != [config.width, config.height] {
            *self = Self::new(device, config, self.slots.len());
        }
    }

    pub(super) fn target(&self) -> &wgpu::Texture {
        &self.tex
    }

    /// The frame in `tex` copied into a free slot (None: none free, the caller did not take its frames; the frame is
    /// dropped and the next `take_frames` fails).
    pub(super) fn copy(&mut self, enc: &mut wgpu::CommandEncoder, tex: &wgpu::Texture) -> Option<usize> {
        let Some(k) = (0..self.slots.len()).find(|&k| self.slots[k].state.load(Ordering::Acquire) == FREE) else {
            self.dropped = true;
            return None;
        };
        let [w, h] = self.size;
        enc.copy_texture_to_buffer(
            tex.as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer: &self.slots[k].buf,
                layout: wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(self.row), rows_per_image: None },
            },
            wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
        );
        self.slots[k].state.store(MAPPING, Ordering::Release);
        self.queue.push_back(k);
        Some(k)
    }

    /// Slot `k`'s copy was submitted as `sub`: its mapping starts (it completes in a later device poll).
    pub(super) fn submitted(&mut self, k: usize, sub: wgpu::SubmissionIndex) {
        let s = &mut self.slots[k];
        s.sub = Some(sub);
        let st = s.state.clone();
        s.buf.slice(..).map_async(wgpu::MapMode::Read, move |r| st.store(if r.is_ok() { MAPPED } else { FAILED }, Ordering::Release));
    }
}

/// A readback row's bytes for `w` pixels: BGRA, padded to wgpu's 256-byte copy alignment.
fn row_bytes(w: u32) -> u32 {
    (w * 4).div_ceil(256) * 256
}

impl Renderer {
    /// A renderer without a window (engine addition, `replay --video`): `w`×`h` frames drawn into a texture and read back
    /// through a ring of `frames` buffers (`take_frames`).
    pub async fn offscreen(w: u32, h: u32, frames: usize) -> Result<Self, String> {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::HighPerformance,
                force_fallback_adapter: false,
                compatible_surface: None,
                apply_limit_buckets: false,
            })
            .await
            .map_err(|e| e.to_string())?;
        let max = adapter.limits().max_texture_dimension_2d;
        if w == 0 || h == 0 || w > max || h > max {
            return Err(format!("{w}x{h}: the GPU's textures are at most {max}x{max}"));
        }
        // as a desktop surface: copies in (AFTERIMAGE) and out (the readback, the ambient fill)
        let config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC | wgpu::TextureUsages::COPY_DST,
            format: FORMAT,
            width: w,
            height: h,
            present_mode: wgpu::PresentMode::AutoVsync,
            desired_maximum_frame_latency: 2,
            alpha_mode: wgpu::CompositeAlphaMode::Opaque,
            view_formats: Vec::new(),
            color_space: wgpu::SurfaceColorSpace::Auto,
        };
        let mut r = Self::build(instance, None, adapter, config).await?;
        // a readback buffer holds a whole frame
        let (bytes, max) = (row_bytes(w) as u64 * h as u64, r.device.limits().max_buffer_size);
        if bytes > max {
            return Err(format!("{w}x{h}: a frame's {bytes} bytes are more than the GPU's buffers hold ({max})"));
        }
        r.offscreen = Some(Offscreen::new(&r.device, &r.config, frames));
        Ok(r)
    }

    /// The windowless renderer's frames read back so far, in order, to `sink` as tightly packed `FORMAT` rows (BGRA,
    /// `w * 4` bytes each): every mapped one, and while more than `keep` are in flight the oldest waited for. Err:
    /// a readback failed, a frame was dropped (`Offscreen::copy`), or `sink`'s error.
    pub fn take_frames(&mut self, keep: usize, sink: &mut dyn FnMut(&[u8]) -> Result<(), String>) -> Result<(), String> {
        let Some(o) = &mut self.offscreen else { return Ok(()) };
        if o.dropped {
            return Err("a frame was dropped: no free readback buffer (a step drew more than one frame)".into());
        }
        let _ = self.device.poll(wgpu::PollType::Poll);
        while let Some(&k) = o.queue.front() {
            let s = &o.slots[k];
            match s.state.load(Ordering::Acquire) {
                MAPPED => {}
                FAILED => return Err("a frame's readback failed".into()),
                _ if o.queue.len() > keep => {
                    // the oldest frame's submission done (its map callback runs in this poll)
                    self.device.poll(wgpu::PollType::Wait { submission_index: s.sub.clone(), timeout: None }).map_err(|e| e.to_string())?;
                    continue;
                }
                _ => break,
            }
            {
                let data = s.buf.slice(..).get_mapped_range().map_err(|e| e.to_string())?;
                let [w, h] = o.size;
                let line = (w * 4) as usize;
                if o.row as usize == line {
                    sink(&data[..line * h as usize])?;
                } else {
                    o.scratch.clear();
                    for y in 0..h as usize {
                        o.scratch.extend_from_slice(&data[y * o.row as usize..y * o.row as usize + line]);
                    }
                    sink(&o.scratch)?;
                }
            }
            s.buf.unmap();
            s.state.store(FREE, Ordering::Release);
            o.queue.pop_front();
        }
        Ok(())
    }

    /// The windowless renderer: the last frame drawn read back once more (a step that drew nothing; black before the
    /// first).
    pub fn repeat_frame(&mut self) {
        let Some(o) = &mut self.offscreen else { return };
        let mut enc = self.device.create_command_encoder(&Default::default());
        let tex = o.tex.clone();
        if let Some(k) = o.copy(&mut enc, &tex) {
            let sub = self.queue.submit([enc.finish()]);
            o.submitted(k, sub);
        }
    }
}
