//! The build stamp: compile-time data (build.rs sets OKB_GIT), re-exported by `platform::support`.

/// Version and git commit of this build (build.rs).
pub const VERSION: &str = concat!(env!("CARGO_PKG_VERSION"), " (", env!("OKB_GIT"), ")");
