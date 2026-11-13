//! Small PS1 libc helpers shared by several subsystems.
//!
//! State: none (the callers own the seeds, which are state)

/// The game's libc rand (the game keeps one global seed). The caller owns the seed: each
/// subsystem keeps its own seed variable.
pub fn rand(seed: &mut u32) -> i32 {
    *seed = seed.wrapping_mul(0x41C6_4E6D).wrapping_add(0x3039);
    ((*seed >> 16) & 0x7FFF) as i32
}
