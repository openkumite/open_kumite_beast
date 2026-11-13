//! The logic's debug-trace mute for re-simulations and its debug-trace switches.
//!
//! Engine addition.
//!
//! The logic's debug-trace switch (engine addition): `muted` turns the log and the logic's debug traces (HITDBG …) off
//! while the rollback test re-simulates frames that were already traced; `flag` tells whether a debug trace is on.
// clippy flags the `const` initialiser below as missing when it checks the Android target (its thread_local is the
// OS-key kind there)
#![cfg_attr(target_os = "android", allow(clippy::missing_const_for_thread_local))]

thread_local! {
    /// a re-simulation runs (the rollback test): its log lines and debug traces are not output again
    static MUTED: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// True while `muted` runs `f`: the debug traces made inside the logic (HITDBG …) and the log stay quiet.
pub fn is_muted() -> bool {
    MUTED.with(|m| m.get())
}

/// Whether the debug-trace variable `name` (HITDBG, GUARDDBG …) is set: the environment's variable names as the
/// process started, read once (the logic asks per fighter and frame; an `env::var` each time takes the environment's
/// lock and allocates). A variable set later is not seen. Names compare as the system's own environment does: exactly,
/// but on Windows without regard to case (`set hitdbg=1` turns HITDBG on, as `env::var` would). The browser has no
/// environment (`env::vars_os` panics on wasm32-unknown-unknown), so there every trace is off; a WASI build has one.
pub fn flag(name: &str) -> bool {
    if cfg!(all(target_arch = "wasm32", target_os = "unknown")) {
        return false;
    }
    // Windows' variable names are case-insensitive: compared without regard to case there (the few names scanned, no
    // allocation per call)
    static SET: std::sync::OnceLock<std::collections::BTreeSet<String>> = std::sync::OnceLock::new();
    let set = SET.get_or_init(|| std::env::vars_os().filter_map(|(k, _)| k.into_string().ok()).collect());
    if cfg!(windows) { set.iter().any(|k| k.eq_ignore_ascii_case(name)) } else { set.contains(name) }
}

/// Runs `f` with the log and the logic's debug traces off (the rollback test's re-simulation, whose frames were
/// already traced when they first ran).
/// The flag is put back by a drop guard, so a panic inside `f` (caught further up) does not leave the log muted.
pub fn muted<R>(f: impl FnOnce() -> R) -> R {
    struct Restore(bool);
    impl Drop for Restore {
        fn drop(&mut self) {
            MUTED.with(|m| m.set(self.0));
        }
    }
    let _restore = Restore(MUTED.with(|m| m.replace(true)));
    f()
}

#[cfg(test)]
mod tests {
    /// `muted` puts the flag back when its closure unwinds (and nested calls keep the outer one's). The unwinds
    /// are `resume_unwind`s: no panic hook (no crash report).
    #[test]
    fn muted_unwind_safe() {
        let unwind = || std::panic::resume_unwind(Box::new("inside"));
        assert!(!super::is_muted());
        let r = std::panic::catch_unwind(|| super::muted(unwind));
        assert!(r.is_err());
        assert!(!super::is_muted());
        super::muted(|| {
            let _ = std::panic::catch_unwind(|| super::muted(unwind));
            assert!(super::is_muted());
        });
        assert!(!super::is_muted());
    }
}
