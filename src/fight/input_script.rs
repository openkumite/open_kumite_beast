//! The scripted pad input of the debug hooks (OKB_SCRIPT / OKB_FESCRIPT, the `sim` and `fetest` binaries).
//!
//! Engine addition.
//! State: none

/// "frames:keys,…" with keys U D L R (the d-pad), J K C I (□ × ○ △), G H (R1 L1), S (START), E (SELECT) → one raw pad
/// word per frame.
pub fn parse_script(s: &str) -> Vec<u16> {
    use crate::fight::input::pad::*;
    let mut out = Vec::new();
    for step in s.split(',').filter(|x| !x.is_empty()) {
        let (n, keys) = step.split_once(':').unwrap_or((step, ""));
        let mut pad = 0u16;
        for c in keys.chars() {
            pad |= match c {
                'U' => UP,
                'D' => DOWN,
                'L' => LEFT,
                'R' => RIGHT,
                'J' => SQUARE,
                'K' => CROSS,
                'C' => CIRCLE,
                'I' => TRIANGLE,
                'G' => R1,
                'H' => L1,
                'S' => 0x800,
                'E' => 0x100, // SELECT
                _ => 0,
            };
        }
        // a step holds at most an hour of frames (the scripts are debug input)
        out.extend(std::iter::repeat_n(pad, n.parse::<usize>().unwrap_or(1).min(216_000)));
    }
    out
}
