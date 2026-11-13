//! The main game's engine (Rust + wgpu). Reads the converted game data (`okb_export/` in development).
//!
//! The layers, lowest first (a module uses only its own layer and the ones below; `tests/layering.rs` checks it):
//!
//! - L0 `base` (portable math, rand, hashes, serde helpers), `platform` (clock, storage, support);
//! - L1 `formats` (file readers), `assets` (the export, the roster), `output` (draw and sound vocabulary);
//! - L2, the deterministic logic: `session` (`GameState`, `step`, replays) → `frontend` (menus, selects) → `fight`
//!   (the match, the effects);
//! - L3 `render` (wgpu), `audio` (the mixer, cpal), `net` (netplay);
//! - L4 `present` (the state into draw lists and GPU objects);
//! - L5 `app` (the window, the 60 Hz loop, the debug tools), private: `run`, `Config`, `session_start`, `web_main`
//!   and `replay --video`'s `render_video` are re-exported here, Android's `android_main` is defined here.
//!
//! The map, the frame step by step and where to find what: `ARCHITECTURE.md` at the repository root.

mod app;
pub mod assets;
pub mod audio;
pub mod base;
pub mod fight;
pub mod formats;
pub mod frontend;
pub mod net;
pub mod output;
pub mod platform;
pub mod present;
pub mod render;
pub mod session;

/// The browser's conversion worker's entry (`bin/convert_worker.rs`).
#[cfg(all(target_arch = "wasm32", feature = "converter"))]
pub use app::convert_worker_main;
#[cfg(target_arch = "wasm32")]
pub use app::web_main;

/// The Android app's entry, run by android-activity on its own thread (engine addition, `docs/android.md`).
#[cfg(target_os = "android")]
#[allow(unsafe_code)] // the exported symbol android-activity looks up
#[unsafe(no_mangle)]
fn android_main(app: winit::platform::android::activity::AndroidApp) {
    app::android_main(app);
}
pub use app::{Config, run, session_start};
#[cfg(not(target_arch = "wasm32"))]
pub use app::{
    video::{RATE as VIDEO_RATE, VideoOptions, VideoSummary, render_video},
    video_ffmpeg::Codec as VideoCodec,
};
pub use platform::clock::clock_seed;
pub(crate) use platform::clock::{now, perf_now};
