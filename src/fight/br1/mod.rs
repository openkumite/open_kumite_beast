//! The imported game's fighters (engine addition: the import): their static data, pose, state
//! scripts and fight frame.
//!
//! Stages S3 / S4 / S5 of the import: the fighter file, the skeleton / clip playback, the fighter block, the
//! state-script interpreter, the input, the throws, the move decision and the physics of the fight frame, with
//! the imported game's own integer math, and the reference runners' cores that check them
//! against recorded runs; stage S7: the round-start reset (`init.rs`) and the bridge into a main-game match
//! (`bridge.rs`, run by `fight::sim::br1`). Without the imported game's disc nothing of it is loaded.

pub mod address_map;
pub mod ai;
pub mod bridge;
pub mod decision;
pub mod frame;
#[cfg(test)]
mod frame_tests;
pub mod hit;
pub mod init;
pub mod input;
pub mod object;
pub mod physics;
pub mod pose;
pub mod reference;
pub mod reference_frame;
pub mod res;
pub mod script;
pub mod throw;
