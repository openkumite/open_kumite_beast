//! L0 platform services (ARCHITECTURE.md, Layers): the clocks, the storage, the game data's folder and the support
//! tools; Android's activity.

#[cfg(target_os = "android")]
pub mod android;
pub mod clock;
pub mod game_data;
pub mod storage;
pub mod support;
