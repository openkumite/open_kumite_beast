//! The performance overlay's numbers: frame stats, the GPU timer, GPU memory.
//!
//! Engine addition.
//! State: presentation (reads the state, never writes it)
//!
//! The performance overlay's numbers (engine addition): a frame's counts and times (`FrameStats`), the GPU timer's
//! readback (`poll_timer`) and the GPU memory the renderer holds (`gpu_bytes`).

use super::Renderer;

/// One frame's counts and times for the performance overlay (engine addition, `app::perf::Perf`); filled while
/// `Renderer::perf` is on.
#[derive(Clone, Copy, Debug, Default)]
pub struct FrameStats {
    pub draws: u32,
    pub quads: u32,
    pub tris: u32,
    /// texture uploads (VRAM copies, atlas rects)
    pub uploads: u32,
    /// CPU time of `frame_capture` (vertex build, encode, submit, present), without the wait for a surface texture
    pub cpu_ms: f64,
    /// the wait for a surface texture (vsync)
    pub wait_ms: f64,
    /// GPU time of the frame's passes (timestamp queries), the latest one read back
    pub gpu_ms: Option<f64>,
}

impl Renderer {
    /// The GPU times whose readback finished (into `stats.gpu_ms`) and a free readback slot, if any.
    pub(super) fn poll_timer(&mut self) -> Option<usize> {
        let t = self.timer.as_ref()?;
        let _ = self.device.poll(wgpu::PollType::Poll);
        let mut free = None;
        for (i, (b, st)) in t.read.iter().enumerate() {
            match st.load(std::sync::atomic::Ordering::Acquire) {
                2 => {
                    if let Ok(v) = b.slice(..).get_mapped_range() {
                        let ts: [u64; 2] = bytemuck::pod_read_unaligned(&v[..16]);
                        if ts[1] > ts[0] {
                            self.stats.gpu_ms = Some((ts[1] - ts[0]) as f64 * t.period as f64 / 1e6);
                        }
                    }
                    b.unmap();
                    st.store(0, std::sync::atomic::Ordering::Release);
                    free = free.or(Some(i));
                }
                0 => free = free.or(Some(i)),
                _ => {}
            }
        }
        free
    }

    /// GPU memory the renderer holds (bytes of textures, buffers): VRAM copy, atlas, depth, kept image, ambient copy and
    /// blur; timer readbacks and vertex streams.
    pub fn gpu_bytes(&self) -> [u64; 2] {
        let px = |t: &wgpu::Texture| t.width() as u64 * t.height() as u64 * t.depth_or_array_layers() as u64;
        let screen = self.config.width as u64 * self.config.height as u64 * 4;
        let tex = px(&self.vram_tex) * 2
            + px(&self.atlas_tex) * 4
            + screen
            + self.persist.as_ref().map_or(0, |t| px(t) * 4)
            + self.amb.as_ref().map_or(0, |a| (px(&a.copy) + px(&a.small)) * 4)
            + self.lo.as_ref().map_or(0, |t| px(&t.tex) * 8);
        let streams: u64 = self.streams.iter().flatten().map(|b| b.size()).sum();
        [tex, self.timer.as_ref().map_or(0, |t| 16 * (1 + t.read.len() as u64)) + streams]
    }
}
