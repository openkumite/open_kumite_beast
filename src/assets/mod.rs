//! L1 assets (ARCHITECTURE.md, Layers): the access to the converted export, its install record and checks, and the
//! static data bundles built from it.

pub mod br1_art;
pub mod extras;
pub mod game_tables;
pub mod manifest;
pub mod roster;
pub mod source;
pub mod text;

pub use source::{Assets, BR1_PREFIX};
