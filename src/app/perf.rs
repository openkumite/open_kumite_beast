//! The frame counter and the performance overlay of the debug tools (engine addition; `support::HELP`'s ` key).

use std::collections::VecDeque;

/// Frames-per-second counter for the overlay.
#[derive(Default)]
pub struct Fps {
    frames: u32,
    since: f64,
    pub value: f32,
}

impl Fps {
    pub fn frame(&mut self, now: f64) {
        self.frames += 1;
        if self.since == 0.0 {
            self.since = now;
        }
        if now - self.since >= 1.0 {
            self.value = self.frames as f32 / (now - self.since) as f32;
            self.frames = 0;
            self.since = now;
        }
    }
}

/// The performance overlay (engine addition; OKB_PERF=1 at start, the ` key toggles it): frame times over the last
/// 2 s, the CPU time per frame of the game step, the draw-list build and the renderer, the GPU time, the renderer's
/// counts and memory. The panel text is refreshed every 0.5 s; nothing is timed while it is off.
#[derive(Default)]
pub struct Perf {
    pub on: bool,
    /// (time, frame ms) over the last 2 s
    frames: VecDeque<(f64, f64)>,
    last: Option<f64>,
    /// since the last refresh: step, build, renderer CPU, GPU, vsync wait (ms sums), frames, frames with a GPU time
    sum: [f64; 5],
    n: u32,
    gpu_n: u32,
    shown_at: f64,
    pub lines: Vec<String>,
}

impl Perf {
    /// A new frame starts at `t` (seconds, `crate::perf_now`).
    pub fn frame(&mut self, t: f64) {
        if let Some(l) = self.last {
            self.frames.push_back((t, (t - l) * 1000.0));
        }
        self.last = Some(t);
        while self.frames.front().is_some_and(|f| t - f.0 > 2.0) {
            self.frames.pop_front();
        }
    }

    /// This frame's CPU times (ms) and the renderer's counts; `mem` = (process RSS, renderer textures, buffers) in
    /// bytes. The text is rebuilt every 0.5 s.
    pub fn record(&mut self, t: f64, step: f64, build: f64, s: &crate::render::FrameStats, mem: impl FnOnce() -> (Option<u64>, u64, u64)) {
        self.sum[0] += step;
        self.sum[1] += build;
        self.sum[2] += s.cpu_ms;
        self.sum[4] += s.wait_ms;
        if let Some(g) = s.gpu_ms {
            self.sum[3] += g;
            self.gpu_n += 1;
        }
        self.n += 1;
        if t - self.shown_at < 0.5 && !self.lines.is_empty() {
            return;
        }
        self.shown_at = t;
        let mem = mem();
        let mut ms: Vec<f64> = self.frames.iter().map(|f| f.1).collect();
        let avg = if ms.is_empty() { 0.0 } else { ms.iter().sum::<f64>() / ms.len() as f64 };
        ms.sort_by(|a, b| b.total_cmp(a));
        let worst = &ms[..ms.len().div_ceil(100).min(ms.len())];
        let low = if worst.is_empty() { 0.0 } else { worst.iter().sum::<f64>() / worst.len() as f64 };
        let n = self.n.max(1) as f64;
        let gpu = if self.gpu_n > 0 { format!("{:.2} MS", self.sum[3] / self.gpu_n as f64) } else { "N/A".to_string() };
        let mb = |b: u64| b as f64 / (1024.0 * 1024.0);
        let rss = mem.0.map_or("N/A".to_string(), |b| format!("{:.0} MB", mb(b)));
        self.lines = vec![
            format!("FPS {:.1}", if avg > 0.0 { 1000.0 / avg } else { 0.0 }),
            format!("FRAME AVG {avg:.2} MAX {:.2} 1% {low:.2} MS", ms.first().copied().unwrap_or(0.0)),
            format!("CPU STEP {:.2} BUILD {:.2} RENDER {:.2} MS", self.sum[0] / n, self.sum[1] / n, self.sum[2] / n),
            format!("GPU {gpu}  VSYNC WAIT {:.2} MS", self.sum[4] / n),
            format!("DRAWS {} QUADS {} TRIS {} UPLOADS {}", s.draws, s.quads, s.tris, s.uploads),
            format!("RSS {rss} GPU TEX {:.1} BUF {:.1} MB", mb(mem.1), mb(mem.2)),
        ];
        self.sum = [0.0; 5];
        (self.n, self.gpu_n) = (0, 0);
    }
}
