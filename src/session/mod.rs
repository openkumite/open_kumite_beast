//! The session: what a run starts from, the whole logic state and the one function that steps it.
//!
//! The session: what a run starts from (`SessionStart`), the whole logic state (`GameState`) and the one function
//! that advances it by a 60 Hz frame (`step`). The app builds the start once from the command line, the debug env
//! hooks, the clock and the stored save; then only `step` changes the state, from the frame's input
//! (`FrameInput`: the pads, the scripted pads, the loads and the card store that completed) and the debug
//! `Cheat`s. Static data (`Loaded`, `FrontData`, fetched files) comes in through `Assets`; what the app must do
//! (sound, loads, the card store, rumble) goes out through `Out`. The logic never reads the environment, the
//! clock or the disk.

pub mod debug_history;
pub mod determinism_trace;
pub mod fight;
mod hash;
pub mod replay;
pub mod rollback;
mod snapshot;
pub mod start;
mod state;
mod step;
#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests;

pub use hash::{HashTrace, hash, section_hashes};
pub use snapshot::{FILE_FORMAT, FILE_MAGIC, MAX_STATE_BYTES, STATE_VERSION, decode_file, load_file, load_state, rebind, save_file, save_state};
pub use start::{Cached, CustomTest, Entry, FightDebug, FileCache, SIM_VERSION, SessionStart, parse_sim_version, sim_version_env};
pub(crate) use start::{browser_build, sim_version_now};
pub use state::{AssetStore, Assets, Cheat, FrameInput, GameState, Music, Out, Request, TeamConfig};
pub use step::{end_match, end_team, inputs, new_state, new_team_state, step};
