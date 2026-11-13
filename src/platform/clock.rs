//! The clocks: wall time, a monotonic timer, the clock seed.
//!
//! Engine addition.
//!
//! The clocks (engine addition): wall time, a monotonic high-resolution timer, the clock seed. Re-exported at the
//! crate root (`crate::now`, `crate::perf_now`, `crate::clock_seed`).

/// A seed from the clock (the front end's ladder shuffle; `SessionStart::seed`, built by the app).
pub fn clock_seed() -> u64 {
    (now() * 1000.0) as u64
}

/// A monotonic high-resolution clock in seconds for the performance overlay (engine addition): `Instant` on the
/// desktop, `performance.now()` in the browser.
pub(crate) fn perf_now() -> f64 {
    #[cfg(not(target_arch = "wasm32"))]
    {
        static START: std::sync::OnceLock<std::time::Instant> = std::sync::OnceLock::new();
        START.get_or_init(std::time::Instant::now).elapsed().as_secs_f64()
    }
    #[cfg(target_arch = "wasm32")]
    {
        web_sys::window().and_then(|w| w.performance()).map_or(0.0, |p| p.now() / 1000.0)
    }
}

pub(crate) fn now() -> f64 {
    #[cfg(not(target_arch = "wasm32"))]
    {
        use std::time::{SystemTime, UNIX_EPOCH};
        SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs_f64()
    }
    #[cfg(target_arch = "wasm32")]
    {
        js_sys::Date::now() / 1000.0
    }
}
