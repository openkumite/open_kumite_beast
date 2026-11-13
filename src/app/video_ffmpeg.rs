//! `replay --video`'s encoder: the system's ffmpeg, fed raw frames through a pipe, then the sound muxed in.
//!
//! Engine addition.
//!
//! The encoder (`Encoder`): ffmpeg reads the frames as raw BGRA from its stdin (`render::offscreen::FORMAT`) and
//! writes the picture alone into a temp file: H.264 through VideoToolbox when this ffmpeg has it and it starts
//! (`video_encoder`), else libx264, BT.709 4:2:0; or, for checks, FFV1 (lossless, the RGB as drawn). A writer thread
//! does the pipe's blocking writes; the frames go to it through a bounded queue of recycled buffers (`BUFFERS`),
//! so a slow encoder holds the renderer back instead of piling up frames. `mux` then copies the picture and
//! encodes the mixed sound (raw f32 stereo) as AAC (FLAC with FFV1) into a temp file renamed over the output when
//! done. Temp files live beside the output (the same disk: the rename is in place, and no GBs land in a RAM-backed
//! temp folder) and go when dropped (`TempFile`).

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::{Receiver, SyncSender, sync_channel};

/// Frame buffers in all: the one being written, the ones queued (a 4K frame is 33 MB).
const BUFFERS: usize = 3;

/// The video codec.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Codec {
    /// H.264 in MP4 (VideoToolbox, else libx264), AAC sound
    H264,
    /// FFV1 in Matroska: lossless RGB, FLAC sound (checks against screenshots)
    Ffv1,
}

impl Codec {
    /// The picture's temp file's extension.
    pub fn temp_ext(self) -> &'static str {
        match self {
            Codec::H264 => "mp4",
            Codec::Ffv1 => "mkv",
        }
    }

    /// The output's ffmpeg muxer: by `out`'s extension (`.mp4` / `.m4v`, `.mov`, `.mkv`), else the codec's own (MP4
    /// for H.264, Matroska for FFV1). Err: an extension this codec does not go into.
    pub fn muxer(self, out: &Path) -> Result<&'static str, String> {
        let ext = out.extension().and_then(|e| e.to_str()).map(|e| e.to_ascii_lowercase());
        match (self, ext.as_deref()) {
            (Codec::H264, Some("mp4" | "m4v")) => Ok("mp4"),
            (Codec::H264, Some("mov")) => Ok("mov"),
            (_, Some("mkv")) => Ok("matroska"),
            (Codec::H264, _) => Ok("mp4"),
            (Codec::Ffv1, None) => Ok("matroska"),
            (Codec::Ffv1, Some(e)) => Err(format!("{}: FFV1 goes into a .mkv, not a .{e}", out.display())),
        }
    }
}

/// The output file checked before the render: an absolute path (so ffmpeg reads no `-…` as an option and no `x:…`
/// as a protocol; a symlink's target, so the video replaces the file it points to), its folder there, the file
/// creatable (one that was not there is removed again).
pub fn output_path(out: &Path) -> Result<PathBuf, String> {
    let abs = std::path::absolute(out).map_err(|e| format!("{}: {e}", out.display()))?;
    let abs = if abs.is_symlink() { std::fs::canonicalize(&abs).unwrap_or(abs) } else { abs };
    if abs.is_dir() {
        return Err(format!("{}: is a folder", out.display()));
    }
    let existed = abs.exists();
    std::fs::OpenOptions::new().write(true).create(true).truncate(false).open(&abs).map_err(|e| format!("{}: {e}", out.display()))?;
    if !existed {
        let _ = std::fs::remove_file(&abs);
    }
    Ok(abs)
}

/// A file beside the output, removed when dropped.
pub struct TempFile(pub PathBuf);

impl TempFile {
    /// `.NAME.KIND.PID.EXT` in the folder of `out` (absolute, `output_path`) named `NAME`.
    pub fn beside(out: &Path, kind: &str, ext: &str) -> Self {
        let name = out.file_name().unwrap_or_default().to_string_lossy();
        Self(out.with_file_name(format!(".{name}.{kind}.{}.{ext}", std::process::id())))
    }

    /// The file kept (not removed when dropped).
    pub fn keep(mut self) -> PathBuf {
        std::mem::take(&mut self.0)
    }
}

impl Drop for TempFile {
    fn drop(&mut self) {
        if !self.0.as_os_str().is_empty() {
            let _ = std::fs::remove_file(&self.0);
        }
    }
}

/// The ffmpeg to run: `over` (`--ffmpeg PATH`), else the first `ffmpeg` on PATH.
pub fn find(over: Option<&Path>) -> Result<PathBuf, String> {
    if let Some(p) = over {
        return if p.is_file() { Ok(p.to_path_buf()) } else { Err(format!("ffmpeg not found at {}", p.display())) };
    }
    let path = std::env::var_os("PATH").unwrap_or_default();
    std::env::split_paths(&path)
        .map(|d| d.join(if cfg!(windows) { "ffmpeg.exe" } else { "ffmpeg" }))
        .find(|p| p.is_file())
        .ok_or_else(|| "ffmpeg not found on PATH: install it (e.g. `brew install ffmpeg`) or give its path with --ffmpeg PATH".into())
}

/// The video encoder for `codec`: VideoToolbox's H.264 when it encodes test frames of the video's `w`×`h` here (it
/// has a size limit), else libx264.
pub fn video_encoder(ffmpeg: &Path, codec: Codec, w: u32, h: u32) -> &'static str {
    if codec == Codec::Ffv1 {
        return "ffv1";
    }
    let vt = Command::new(ffmpeg)
        .args(["-hide_banner", "-loglevel", "error", "-f", "lavfi", "-i", &format!("color=c=black:s={w}x{h}:r=60"), "-frames:v", "2"])
        .args(["-c:v", "h264_videotoolbox", "-f", "null", "-"])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|s| s.success());
    if vt { "h264_videotoolbox" } else { "libx264" }
}

/// H.264's bit rate for a `w`×`h` 60 fps video: 0.12 bits per pixel (60 Mbit/s at 4K, 15 at 1080p).
pub fn default_bitrate(w: u32, h: u32) -> u64 {
    (w as u64 * h as u64 * 60 * 12 / 100).max(1_000_000)
}

/// ffmpeg encoding the picture, its stdin fed by the writer thread.
pub struct Encoder {
    child: Child,
    tx: Option<SyncSender<Vec<u8>>>,
    /// the writer's used buffers back
    free: Receiver<Vec<u8>>,
    made: usize,
    writer: Option<std::thread::JoinHandle<Result<(), String>>>,
}

impl Encoder {
    /// ffmpeg started on `w`×`h` BGRA frames at 60 fps, the picture into `out` with `encoder` (`bitrate` for H.264).
    pub fn start(ffmpeg: &Path, encoder: &str, w: u32, h: u32, bitrate: u64, out: &Path) -> Result<Self, String> {
        let mut c = Command::new(ffmpeg);
        c.args(["-hide_banner", "-loglevel", "error", "-y", "-f", "rawvideo", "-pix_fmt", "bgra"]);
        c.args(["-s", &format!("{w}x{h}"), "-framerate", "60", "-i", "pipe:0"]);
        let rate = bitrate.to_string();
        match encoder {
            "ffv1" => {
                c.args(["-c:v", "ffv1", "-level", "3", "-pix_fmt", "bgr0"]);
            }
            _ => {
                // RGB to BT.709 limited-range 4:2:0, tagged as such
                c.args(["-vf", "scale=out_color_matrix=bt709:out_range=tv,format=yuv420p", "-c:v", encoder]);
                if encoder == "libx264" {
                    c.args(["-preset", "veryfast", "-b:v", &rate]);
                } else {
                    c.args(["-b:v", &rate, "-profile:v", "high"]);
                }
                c.args(["-colorspace", "bt709", "-color_primaries", "bt709", "-color_trc", "bt709", "-color_range", "tv"]);
            }
        }
        c.arg("-an").arg(out).stdin(Stdio::piped()).stdout(Stdio::null()).stderr(Stdio::inherit());
        let mut child = c.spawn().map_err(|e| format!("{}: {e}", ffmpeg.display()))?;
        let mut stdin = child.stdin.take().ok_or("ffmpeg: no stdin")?;
        let (tx, rx) = sync_channel::<Vec<u8>>(BUFFERS);
        let (back, free) = sync_channel::<Vec<u8>>(BUFFERS);
        let writer = std::thread::Builder::new()
            .name("video writer".into())
            .spawn(move || {
                for b in rx {
                    stdin.write_all(&b).map_err(|e| format!("ffmpeg stopped reading frames: {e}"))?;
                    let _ = back.send(b);
                }
                Ok(())
            })
            .map_err(|e| e.to_string())?;
        Ok(Self { child, tx: Some(tx), free, made: 0, writer: Some(writer) })
    }

    /// A frame (tightly packed BGRA) queued for ffmpeg: copied into a free buffer, waiting for one while all are in
    /// use.
    pub fn frame(&mut self, px: &[u8]) -> Result<(), String> {
        let mut b = match self.free.try_recv() {
            Ok(b) => b,
            Err(_) if self.made < BUFFERS => {
                self.made += 1;
                Vec::with_capacity(px.len())
            }
            Err(_) => self.free.recv().map_err(|_| self.failed())?,
        };
        b.clear();
        b.extend_from_slice(px);
        self.tx.as_ref().ok_or("encoder finished")?.send(b).map_err(|_| self.failed())
    }

    /// Why the writer stopped (its error, or ffmpeg's exit).
    fn failed(&mut self) -> String {
        self.tx = None;
        match self.writer.take().map(|w| w.join()) {
            Some(Ok(Err(e))) => e,
            _ => "the video writer stopped".into(),
        }
    }

    /// The last frames written, ffmpeg's stdin closed and its end waited for.
    pub fn finish(mut self) -> Result<(), String> {
        self.tx = None;
        if let Some(w) = self.writer.take() {
            w.join().map_err(|_| "the video writer panicked".to_string())??;
        }
        let st = self.child.wait().map_err(|e| e.to_string())?;
        if st.success() { Ok(()) } else { Err(format!("ffmpeg (video) failed: {st}")) }
    }
}

impl Drop for Encoder {
    /// An encoder given up (an error before `finish`): ffmpeg stopped.
    fn drop(&mut self) {
        if self.writer.is_some() || self.child.try_wait().ok().flatten().is_none() {
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
    }
}

/// The picture of `video` and the raw sound of `pcm` (f32 stereo at `rate`) into `out` (absolute, `output_path`)
/// with the muxer `format` (`Codec::muxer`): the picture copied, the sound encoded (AAC; FLAC with FFV1).
pub fn mux(ffmpeg: &Path, video: &Path, pcm: &Path, rate: u32, codec: Codec, format: &str, out: &Path) -> Result<(), String> {
    let mut c = Command::new(ffmpeg);
    c.args(["-hide_banner", "-loglevel", "error", "-y", "-i"]).arg(video);
    c.args(["-f", "f32le", "-ar", &rate.to_string(), "-ac", "2", "-i"]).arg(pcm);
    c.args(["-map", "0:v:0", "-map", "1:a:0", "-c:v", "copy"]);
    match codec {
        Codec::H264 => c.args(["-c:a", "aac", "-b:a", "256k"]),
        Codec::Ffv1 => c.args(["-c:a", "flac"]),
    };
    // MP4 / MOV: the index first (playable while it downloads)
    if format != "matroska" {
        c.args(["-movflags", "+faststart"]);
    }
    let st = c.args(["-f", format]).arg(out).stdin(Stdio::null()).status().map_err(|e| format!("{}: {e}", ffmpeg.display()))?;
    if st.success() { Ok(()) } else { Err(format!("ffmpeg (mux) failed: {st}")) }
}
