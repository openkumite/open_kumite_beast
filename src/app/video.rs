//! `replay --video`: a replay rendered into a video file with its sound, without a window.
//!
//! Engine addition.
//!
//! The replay's steps run as the `replay` check runs them (`session::replay::Headless`: every checkpoint compared,
//! the static data loaded at the steps the recording had it). After each step its output is served as the app
//! serves a playback's at ×1 (`App::step`): the sound queue into the mixer, a movie's music, a fight's view built at
//! `Request::Fight` and dropped or kept for the backdrop at `FightOver`, music tracks loaded; then the state is
//! drawn as `App::draw` draws it, without the overlay (no playback status, input display or key help, as with
//! OKB_NOPLAYUI), by the offscreen renderer (`render::Renderer::offscreen`) at the video's size. One video frame per
//! 60 Hz step (a step that draws nothing repeats the frame before), and per step its share of the sound mixed
//! offline (`RATE / 60` samples; the count accumulated, so any rate stays in step). Nothing of the drawing or the
//! sound reaches the logic. The frames go to ffmpeg through `video_ffmpeg`'s writer thread, the sound into a temp file
//! muxed in at the end into a third one, renamed over the output only when the mux succeeded. Every step of the file
//! is in the video, from its first (a stored match replay's part before its VS screen too, which the app's playback
//! skips).

use super::video_ffmpeg::{self, Codec, Encoder, TempFile};
use crate::audio::Audio;
use crate::frontend::options::{Aspect, Prefs};
use crate::output::layout;
use crate::present::fight_scene::{FightView, Game, TeamGame};
use crate::present::front_vram::VramView;
use crate::render::Renderer;
use crate::session::replay::{Headless, MAX_BYTES, Replay};
use crate::session::{AssetStore, GameState, Request};
use std::io::Write;
use std::path::PathBuf;

/// The mixer's rate: 800 samples per 60 Hz step.
pub const RATE: u32 = 48_000;
/// Frames read back at once: the GPU draws the next ones while the oldest is mapped and copied.
const READBACK: usize = 3;
/// The largest video: 8192 pixels a side, 8192×4320 (DCI 8K) in all (a frame is 142 MB; with the readback ring and
/// the encoder's queue about 1 GB is held, ffmpeg's own memory aside).
pub const MAX_SIDE: u32 = 8192;
pub const MAX_PIXELS: u64 = 8192 * 4320;

/// What `render_video` makes.
pub struct VideoOptions {
    /// the converted export
    pub export: String,
    pub replay: PathBuf,
    pub out: PathBuf,
    /// the video's size in pixels (even)
    pub size: (u32, u32),
    /// SCREEN MODE: None = the stored preferences' (as the app's window)
    pub aspect: Option<Aspect>,
    pub codec: Codec,
    /// H.264's bit rate (bits/s); None: by the size
    pub bitrate: Option<u64>,
    /// the ffmpeg to run; None: the first on PATH
    pub ffmpeg: Option<PathBuf>,
}

/// A video made.
pub struct VideoSummary {
    pub frames: u32,
    pub samples: u64,
    /// the video encoder used (`h264_videotoolbox`, `libx264`, `ffv1`)
    pub encoder: &'static str,
    pub secs: f64,
}

/// The presentation of a playback: what the app holds beside the state (`App::view`, `backdrop_view`,
/// `vram_view`, `audio`, `renderer`).
struct Show {
    r: Renderer,
    audio: Audio,
    view: Option<FightView>,
    backdrop_view: Option<FightView>,
    vram_view: VramView,
    assets: crate::assets::Assets,
}

impl Show {
    /// XA track `track` loaded and played from `secs` in (`App::load_music`, here at once).
    fn music(&self, track: u8, looped: bool, secs: f64) {
        let path = crate::formats::music::track_file(track);
        match pollster::block_on(self.assets.load_music(&path)).ok().and_then(crate::formats::music::decode) {
            Some(m) => self.audio.play_music_from(std::sync::Arc::new(m), looped, secs),
            None => log::warn!("music track {track:#x} ({path}) not found"),
        }
    }

    /// A step's output, as `App::step` serves a playback's at ×1: the sound, a movie's music, the requests the
    /// presentation follows (the logic's data was served by `Headless`).
    fn stepped(&mut self, s: &GameState, store: &AssetStore, out: &mut crate::session::Out) {
        self.audio.apply(&mut out.audio);
        if let Some(f) = &s.front
            && let Some((m, at)) = self.vram_view.step(f, &store.files)
        {
            self.audio.play_music_from(m, false, at);
        }
        for r in &out.requests {
            match *r {
                Request::Fight => {
                    if let (Some(data), true) = (&store.fight, s.fight.is_some() || s.team.is_some()) {
                        self.view = Some(FightView::new(&mut self.r, data, &mut self.audio));
                    }
                }
                // the team battle (engine addition, T4)
                Request::TeamOver { .. } => self.view = None,
                Request::FightOver { backdrop, .. } => {
                    let view = self.view.take();
                    if backdrop {
                        self.backdrop_view = view;
                    }
                }
                Request::Music(Some((track, looped))) => self.music(track, looped, 0.0),
                _ => {}
            }
        }
        if s.backdrop.is_none() {
            self.backdrop_view = None;
        }
    }

    /// The state drawn as `App::draw` draws it without the overlay: the fight, or the front end over the kept demo
    /// stage; false when there is neither (nothing drawn).
    fn draw(&mut self, s: &GameState, store: &AssetStore) -> bool {
        let r = &mut self.r;
        if let Some(g) = Game::of(store.fight.as_deref(), s.fight.as_ref(), self.view.as_ref()) {
            g.draw(r);
            return true;
        }
        if let Some(t) = TeamGame::of(store.fight.as_deref(), s.team.as_deref(), self.view.as_ref(), s.team_config.as_ref().map(|c| &c.menu)) {
            t.draw_capture(r, false);
            return true;
        }
        let Some(f) = &s.front else { return false };
        r.afterimage = 0;
        let backdrop = Game::of(store.backdrop.as_deref(), s.backdrop.as_ref(), self.backdrop_view.as_ref());
        if let (None, Some(data)) = (&s.backdrop, &store.front)
            && self.vram_view.sync(f, data, &store.files)
        {
            r.upload_vram(&self.vram_view.vram.words);
        }
        // `front_prims` without the replay list or the NETWORK menu
        let prims = f.draw();
        r.ambient = layout::ambient_split(&prims, r.viewport());
        let quads = layout::to_quads(&prims, r.viewport());
        match (backdrop, f.backdrop_cam()) {
            (Some(g), Some(cam)) => g.draw_stage(r, cam, f.frame_no() as u64, &quads, false),
            _ => r.frame_capture(&[], &quads, &[], [0.0, 0.0, 0.0], false),
        };
        true
    }
}

/// The video of `o.replay` written to `o.out` (progress on stderr when `progress`).
pub fn render_video(o: &VideoOptions, progress: bool) -> Result<VideoSummary, String> {
    let t0 = std::time::Instant::now();
    let (w, h) = o.size;
    if w == 0 || h == 0 || w % 2 != 0 || h % 2 != 0 {
        return Err(format!("{w}x{h}: the size must be even and not 0"));
    }
    if w > MAX_SIDE || h > MAX_SIDE || w as u64 * h as u64 > MAX_PIXELS {
        return Err(format!("{w}x{h}: at most {MAX_SIDE} pixels a side and 8192x4320 in all"));
    }
    // the output checked before the render (a bad path would otherwise only fail the final mux)
    let format = o.codec.muxer(&o.out)?;
    let out = video_ffmpeg::output_path(&o.out)?;
    let ffmpeg = video_ffmpeg::find(o.ffmpeg.as_deref())?;
    let bytes = crate::platform::storage::read_capped(&o.replay, MAX_BYTES).map_err(|e| format!("{}: {e}", o.replay.display()))?;
    let replay = Replay::decode(&bytes).map_err(|e| format!("{}: {e}", o.replay.display()))?;
    let frames = replay.header.frames;
    // the window's preferences, as the app's (a playback's logic runs with the file's)
    let prefs = Prefs::from_stored(crate::platform::storage::read_prefs());
    let mut r = pollster::block_on(Renderer::offscreen(w, h, READBACK))?;
    r.set_fit(o.aspect.unwrap_or(prefs.aspect).fit());
    r.render_scale = prefs.render_scale();
    r.hud_scale = prefs.hud_scale();
    let assets = crate::assets::Assets::new(o.export.clone());
    let audio = Audio::offline(RATE as f64);
    let (mut hl, mut start_out) = pollster::block_on(Headless::open(&assets, replay, audio.logging()))?;
    let mut show = Show { r, audio, view: None, backdrop_view: None, vram_view: VramView::default(), assets: assets.clone() };
    // the session's start as `App::start_up` → `Ready` → `begin_session` has it for `--replay`
    if let Some(data) = &hl.store().fight {
        show.view = Some(FightView::new(&mut show.r, data, &mut show.audio));
    }
    if let Some(data) = &hl.store().front {
        show.audio.load_front_banks(data);
    }
    show.audio.apply(&mut start_out.audio);

    let (video_tmp, audio_tmp) = (TempFile::beside(&out, "video", o.codec.temp_ext()), TempFile::beside(&out, "sound", "f32"));
    let encoder = video_ffmpeg::video_encoder(&ffmpeg, o.codec, w, h);
    let rate = o.bitrate.unwrap_or_else(|| video_ffmpeg::default_bitrate(w, h));
    let mut enc = Encoder::start(&ffmpeg, encoder, w, h, rate, &video_tmp.0)?;
    let mut pcm = std::io::BufWriter::new(std::fs::File::create(&audio_tmp.0).map_err(|e| format!("{}: {e}", audio_tmp.0.display()))?);
    let (mut sent, mut samples, mut last_report) = (0u32, 0u64, 0.0f64);
    let mut sink = |px: &[u8]| {
        sent += 1;
        enc.frame(px)
    };
    let mut step = 0u64;
    while let Some(mut out) = pollster::block_on(hl.step_out()).map_err(|e| format!("replay out of sync: {e}"))? {
        show.stepped(hl.state(), hl.store(), &mut out);
        if !show.draw(hl.state(), hl.store()) {
            show.r.repeat_frame();
        }
        show.r.take_frames(READBACK - 1, &mut sink)?;
        // this step's samples: up to (step + 1) / 60 s
        step += 1;
        let n = step * RATE as u64 / 60 - samples;
        samples += n;
        for v in show.audio.render_offline(n as usize) {
            pcm.write_all(&v.to_le_bytes()).map_err(|e| e.to_string())?;
        }
        let t = t0.elapsed().as_secs_f64();
        if progress && (t - last_report >= 0.5 || step == frames as u64) {
            last_report = t;
            let fps = step as f64 / t.max(1e-6);
            eprint!("\rframe {step} of {frames} ({}%), {fps:.1} fps, {:.0} s left   ", step * 100 / frames.max(1) as u64, (frames as f64 - step as f64) / fps);
        }
    }
    show.r.take_frames(0, &mut sink)?;
    pcm.flush().map_err(|e| e.to_string())?;
    drop(pcm);
    if progress {
        eprintln!();
    }
    enc.finish()?;
    if sent as u64 != step {
        return Err(format!("{sent} frames encoded for {step} steps"));
    }
    // muxed into a temp file renamed over the output: a file that was there is replaced only by a whole video
    let part = TempFile::beside(&out, "part", out.extension().and_then(|e| e.to_str()).unwrap_or(o.codec.temp_ext()));
    if let Err(e) = video_ffmpeg::mux(&ffmpeg, &video_tmp.0, &audio_tmp.0, RATE, o.codec, format, &part.0) {
        // the render kept for a mux by hand; the partial mux removed (`part` dropped)
        let (v, a) = (video_tmp.keep(), audio_tmp.keep());
        return Err(format!("{e}; the picture is kept in {} and the sound (f32le, 2 channels, {RATE} Hz) in {}", v.display(), a.display()));
    }
    if let Err(e) = std::fs::rename(&part.0, &out) {
        let kept = part.keep();
        return Err(format!("{}: {e}; the video is kept in {}", out.display(), kept.display()));
    }
    Ok(VideoSummary { frames: sent, samples, encoder, secs: t0.elapsed().as_secs_f64() })
}
