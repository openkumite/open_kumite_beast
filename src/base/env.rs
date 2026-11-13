//! The engine's environment switches by name: `var("SAVE_DIR")` reads `OKB_SAVE_DIR`, else `BR2_SAVE_DIR` (the
//! prefix of earlier builds, still read). Unprefixed switches (the `sim` tool's `STAGE`, `HITDBG` …) are read as they
//! are.
//!
//! State: none

use std::ffi::OsString;

/// The prefix of the engine's switches.
pub const PREFIX: &str = "OKB_";
/// The prefix earlier builds read, still read when the new name is not set.
pub const OLD_PREFIX: &str = "BR2_";

/// Switch `name` (without its prefix): `OKB_<name>`, else `BR2_<name>`.
pub fn var(name: &str) -> Result<String, std::env::VarError> {
    match std::env::var(format!("{PREFIX}{name}")) {
        Err(std::env::VarError::NotPresent) => std::env::var(format!("{OLD_PREFIX}{name}")),
        r => r,
    }
}

/// Switch `name` as an `OsString`: `OKB_<name>`, else `BR2_<name>`.
pub fn var_os(name: &str) -> Option<OsString> {
    pick(name, |k| std::env::var_os(k)).map(|(_, v)| v)
}

/// The full name switch `name` is set under (`OKB_<name>` when neither is set), for readers that derive more names
/// from it (`<name>_LOG`, `<name>_DUMP`).
pub fn name(name: &str) -> String {
    pick(name, |k| std::env::var_os(k)).map_or_else(|| format!("{PREFIX}{name}"), |(n, _)| n)
}

/// The first of `OKB_<name>`, `BR2_<name>` that `get` finds, with its full name.
pub fn pick<T>(name: &str, get: impl Fn(&str) -> Option<T>) -> Option<(String, T)> {
    [PREFIX, OLD_PREFIX].iter().find_map(|p| {
        let full = format!("{p}{name}");
        get(&full).map(|v| (full, v))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_prefix_first_then_old() {
        let env = |set: &'static [(&'static str, &'static str)]| move |k: &str| set.iter().find(|(n, _)| *n == k).map(|(_, v)| *v);
        assert_eq!(pick("SAVE_DIR", env(&[])), None);
        assert_eq!(pick("SAVE_DIR", env(&[("BR2_SAVE_DIR", "old")])), Some(("BR2_SAVE_DIR".into(), "old")));
        assert_eq!(pick("SAVE_DIR", env(&[("BR2_SAVE_DIR", "old"), ("OKB_SAVE_DIR", "new")])), Some(("OKB_SAVE_DIR".into(), "new")));
        assert_eq!(pick("SAVE_DIR", env(&[("SAVE_DIR", "bare")])), None);
    }
}
