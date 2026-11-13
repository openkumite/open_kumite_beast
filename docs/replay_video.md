# Replay to video (`replay --video`)

An engine addition: the `replay` tool renders a replay file (`.okbr`) into a video file with the game's sound, without
a window, through the system's ffmpeg. Paths are relative to `src/`.

## Usage

```sh
cargo build --release --bins
OKB_SAVE_DIR=$(mktemp -d) target/release/replay --video out.mp4 FILE.okbr                        # 4K, ../okb_export
target/release/replay --video out.mp4 --size 1080p --aspect 4:3 ../okb_export FILE.okbr
target/release/replay --video check.mkv --size 1080p --codec ffv1 ../okb_export FILE.okbr        # lossless
```

| Option | Default | |
|---|---|---|
| `--size 4k\|1440p\|1080p\|720p\|WxH` | `4k` (3840×2160) | the video's size in pixels (even numbers, at most 8192 a side and 8192×4320 in all, and what the GPU's textures and buffers hold) |
| `--aspect 4:3\|wide` | the stored SCREEN MODE (`prefs.json` in the save folder, `OKB_ASPECT`; else WIDE) | CLASSIC is the 4:3 picture with black bars in the frame |
| `--codec h264\|ffv1` | `h264` | H.264 + AAC (`.mp4` / `.m4v`, `.mov` or `.mkv`; another extension or none: MP4); FFV1 + FLAC is lossless RGB (`.mkv`, or no extension: Matroska; about 0.75 MB a frame at 1080p) |
| `--bitrate MBIT/S` | 0.12 bit per pixel: 60 at 4K, 26.5 at 1440p, 15 at 1080p, 6.6 at 720p | H.264's average bit rate (above 0, at most 1000) |
| `--ffmpeg PATH` | the first `ffmpeg` on `PATH` | a missing ffmpeg is an error that says so |
| `EXPORT_DIR` | `../okb_export` | the converted export, as for the other tools |

The options may come anywhere; the other arguments are, in order, OUT, EXPORT_DIR (optional) and FILE. OUT is
checked before the render (its folder must exist and the file be writable; it is passed to ffmpeg as an absolute
path, so a name starting with `-` or holding a `:` is a plain file name; a symlink stands for the file it points to).
A file already at OUT is replaced only when the whole video is made: on any error it is left as it was. Progress (frame n of m, frames per second,
time left) goes to stderr; at the end one line on stdout: frames, sound
samples, the encoder, the time taken. Exit 0 when the video was made, 1 on an error (out of sync, ffmpeg failed,
a file missing), 2 on a usage error. `OKB_LOG=info` shows the engine's log. RENDER QUALITY and HUD SIZE follow the
stored preferences too, as in the app's window.

## What is in it

- **Every step of the file, one 60 fps frame each**: from the session's start (boot logos or title, menus, the VS
  screens) through every fight to the end. The video's frame count equals the replay's step count; frame *n* (from 0)
  shows the state after step *n* + 1 (from 1), as the app draws it after that step. A stored match replay is watched in the app from
  its VS screen (`Header::from`, the part before is skipped silently); the video has the part before too.
- **No overlay**: no playback status, input display or key help (as the app with `OKB_NOPLAYUI`), and the fight HUD
  shown.
- **The sound**: effects, voices, music and the movies' music, mixed offline at 48 kHz, 800 samples per step, so the
  sound stays sample-exact with the picture (the audio stream lasts as long as the video, to within AAC's last
  frame). A music track starts at the step that asked for it (the app starts it once its load is delivered, a few
  milliseconds later).
- Loading is not in the video: the app's playback waits on its loads without steps, the video has no frames for
  them.

## How it works

`app/video.rs` (`render_video`), `app/video_ffmpeg.rs` (the encoder), `render/offscreen.rs` (the windowless
renderer), `audio::Audio::offline`, `session::replay::Headless`.

1. **The steps** run as the `replay` check runs them: `session::replay::Headless` (`session/replay/headless.rs`) gives
   each step's input, loads the match and the fetched files at the steps the recording had them, compares every
   checkpoint (a file out of sync stops the video with an error) and returns the step's `Out`.
2. **The output** of each step is served as the app serves a playback's at ×1 (`App::step`, `App::request`): the
   sound queue into the mixer, a movie's music (`VramView::step`), a fight's `FightView` built at `Request::Fight`
   and dropped (or kept for the attract ranking's backdrop) at `FightOver`, music tracks loaded at `Request::Music`.
   The session's start is set up as `App::start_up` / `Ready` / `begin_session` do for `--replay` (a direct fight's
   view, the front end's banks, the start's sound queue).
3. **The picture** is drawn as `App::draw` draws it, with the same `present` and `render` code: `Game::draw` for a
   fight, `Front::draw` → `layout::to_quads` → `frame_capture` (or `Game::draw_stage` over the kept demo stage) for the
   front end, the VRAM image kept by `VramView::sync`. The renderer is `Renderer::offscreen`: the device without a
   surface; the frame goes into a texture in the format a desktop window's surface takes (BGRA8, not sRGB), then is
   copied into one of three readback buffers and mapped without waiting. `take_frames` hands the frames over in
   order and waits only for the oldest when all three are in flight, so the GPU draws ahead while a frame is copied
   out. A step that draws nothing (none in the fixtures) repeats the frame before (`repeat_frame`), as a window keeps
   showing it.
4. **The sound** is mixed per step by `Audio::render_offline` from an `Audio::offline(48000)` mixer (no device): after
   step *k* the samples up to (*k* + 1) / 60 s, the count accumulated, so a rate that is not a multiple of 60 would
   stay in step too. Raw f32 stereo goes into a temp file beside OUT.
5. **The encoder**: ffmpeg reads raw BGRA frames from a pipe. A writer thread does the blocking writes; the frames go
   to it through a queue of three recycled buffers, so a slower encoder holds the renderer back instead of piling up
   frames (at 4K the whole run stays near 1 GB). H.264 uses VideoToolbox (`h264_videotoolbox`, High profile) when
   it encodes test frames of the video's size on this machine (it has a size limit), else libx264 (`veryfast`); the RGB is converted to BT.709 limited-range
   4:2:0 and tagged so. The picture is written to a temp file; at the end a second ffmpeg run copies it and encodes
   the sound (AAC 256 kbit/s; FLAC with FFV1) into a third temp file, its container named to ffmpeg (`-f`;
   `+faststart` for MP4 / MOV), which is then renamed over OUT. The temp files are beside OUT, in its folder (the
   same disk, so the rename replaces OUT in one step and the GBs of a long render do not go into a RAM-backed temp
   folder): `.NAME.video.PID.mp4` (`.mkv` with FFV1), `.NAME.sound.PID.f32` and `.NAME.part.PID.EXT` for an OUT
   named `NAME`. They are removed when done or on an error, except when that last pass fails: then the error names
   the two kept files (the picture, and the raw f32le stereo 48 kHz sound) for a mux by hand, and the partial mux is
   removed; if the rename fails, the error names the kept, finished video.

Nothing of the drawing or the sound reaches the logic: the state is only read after each step, and the steps are the
check's own (`Headless`).

## Speed and size (Apple silicon laptop, VideoToolbox, 2026-11)

| Replay | Size | Made in | Frames/s | File | Peak memory (tool + ffmpeg) |
|---|---|---|---|---|---|
| fight, 3417 steps (57 s) | 4K | 63 s | 54 | 430 MB (60 Mbit/s) | 0.96 GB |
| fight, 3417 steps (57 s) | 1080p | 19 s | 181 | 109 MB (15 Mbit/s) | 0.36 GB |
| arcade session, 20275 steps (5 min 38 s) | 1080p | 109 s | 186 | 643 MB | 0.56 GB |
| menus, 507 steps (8.5 s) | 4K | 10 s | 50 | 32 MB | 0.94 GB |

At 4K the H.264 encoder sets the pace (VideoToolbox's 4K H.264 alone does about 50–55 frames a second here), so
4K runs at about real time; 1080p about three times faster. libx264 `veryfast` (the fallback) encodes 4K about as
fast on this machine. FFV1 at 1080p is about 0.75 MB a frame (2.6 GB for the fight).

## Checking a video

- Frame count and durations: `ffprobe -show_entries stream=codec_name,width,height,r_frame_rate,nb_frames,duration out.mp4`
  (video `nb_frames` = the replay's steps, both durations = steps / 60 s).
- Picture identity with the app: render with `--codec ffv1` at the window's size and compare a frame
  (`ffmpeg -i v.mkv -map 0:v -vf "select=eq(n\,N)" -fps_mode passthrough -pix_fmt rgb24 -f framemd5 -`) with the app's
  screenshot of the same replay (`OKB_WINDOW=1920x1080 OKB_LOCKSTEP=1 OKB_NOPLAYUI=1 OKB_SHOTS=F okbeast --replay FILE`:
  video frame `F - 1` of a direct fight; `OKB_FESHOTS` for the front end), both as rgb24.
- Sound: `OKB_SOUNDLOG=file` logs the mixer's calls in both the app and `replay --video`.

## Not supported

- The browser and Android builds (the tool runs on the desktop only; ffmpeg is a separate program).
- Trimming, an input display, the app's playback speeds; one replay per run.
- A video's own frame rate other than 60 fps.
- An interrupted run (Ctrl+C) leaves its temp files (`.NAME.video.PID.mp4` / `.NAME.sound.PID.f32`, and
  `.NAME.part.PID.EXT` during the mux) in OUT's folder; ffmpeg stops with it. OUT itself is untouched.
