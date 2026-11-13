//! L0 base (ARCHITECTURE.md, Layers): what every layer may use.

pub(crate) mod build;
pub(crate) mod decode_limit;
pub mod env;
pub mod hash;
pub mod libc_rand;
pub mod portable;
pub mod serde_util;
pub mod trace;
