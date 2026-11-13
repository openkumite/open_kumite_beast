//! The effects' trigonometry: rsin / rcos and the approximate atan2.
//!
//! State: none
//!
//! The effects' trigonometry: rsin / rcos on the 4096-unit circle and the approximate
//! atan2.

/// rsin / rcos on the 4096-unit circle (`libm`'s sine; any accurate sine gives the same
/// integers: `portable::tests::rsin_table`).
pub(crate) fn rsin(a: i32) -> i32 {
    let i = a & 0x3FF;
    let i = if a & 0x400 != 0 { 0x400 - i } else { i };
    let v = (4096.0 * libm::sin(i as f64 * std::f64::consts::PI / 2048.0)).round() as i32;
    if a & 0x800 != 0 { -v } else { v }
}
pub(crate) fn rcos(a: i32) -> i32 {
    rsin(a + 0x400)
}

/// Approximate atan2 (x, z).
pub(crate) fn atan_xz(x: i32, z: i32) -> i32 {
    let x = if x == 0 { 1 } else { x };
    let z = if z == 0 { 1 } else { z };
    let (ax, az) = (x.abs(), z.abs());
    if az < ax {
        let v = ((z << 9) + if z > 0 { ax << 3 } else { -(ax << 3) }) / ax;
        return if x > 0 { v + 0x400 } else { 0xC00 - v };
    }
    let v = (x << 9) + if x > 0 { az << 3 } else { -(az << 3) };
    if z > 0 {
        0x800 - v / az
    } else if x > 0 {
        v / az
    } else {
        v / az + 0xFFF
    }
}
