//! The fight backdrop: two flat colours behind the stage model, split at the horizon.
//!
//! State: presentation (reads the state, never writes it)
//!
//! At stage load the stage's 26-byte backdrop record is copied and the packets
//! are built (two buffers of 0x68 bytes). Every drawn frame the backdrop projects a far point on the
//! camera's heading to get the horizon row and copies the bottom colour into the DRAWENV clear colour; the frame flip
//! clears rows 0..h of the 512×480 frame buffer with the top colour (ClearImage) after the DRAWENV clear
//! and before the ordering table is drawn. The darkening sets both colours to base − L.
//!
//! Presentation only: a function of the camera and the darkening level.

use crate::fight::physics::Trig;
use crate::formats::tables::Tables;

pub use crate::output::backdrop::Backdrop;

/// `stage.backdrop`: per stage 26 bytes, +0 top colour, +4/+8 the colours of two POLY_G4 that are built but never drawn, +0xC bottom
/// colour, +0x14 distance R and +0x16 height Y of the horizon point.
const TABLE: &str = "stage.backdrop";

/// `stage`'s backdrop for a camera of heading `yaw` (cam +0x2A) at darkening level `level`; `rtps` projects a game
/// point to the frame-buffer row (0..480 on screen; None behind the camera).
pub fn backdrop(tables: &Tables, trig: &Trig, stage: usize, yaw: i16, level: i32, rtps: impl Fn([i32; 3]) -> Option<f32>) -> Backdrop {
    let (t, rec) = (tables.table(TABLE), 26 * stage);
    // each byte − L, below 0 → 0
    let colour = |o: usize| [0, 1, 2].map(|k| (t.u8(rec + o + k) as i32 - level).max(0) as u8);
    // P = (rsin(yaw)·R / 4096, Y, rcos(yaw)·R / 4096) (C division), RTPS with the view matrix; the
    // top rows = SY clamped to 0..M.22 (480)
    let (r, y) = (t.i16(rec + 0x14) as i32, t.i16(rec + 0x16) as i32);
    let yaw = yaw as i32;
    let p = [trig.sin(yaw) * r / 4096, y, trig.cos(yaw) * r / 4096];
    let rows = rtps(p).map_or(0, |sy| (sy.floor() as i32).clamp(0, 480));
    Backdrop { top: colour(0), bottom: colour(0xC), rows }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::formats::tables::Elem;

    /// Tables holding ST3's backdrop record and a zero sine table.
    fn st3() -> (Tables, Trig) {
        let mut b = vec![0; 26 * 4];
        b[26 * 3..26 * 4].copy_from_slice(&[0x70, 0, 0, 0, 0x70, 0, 0, 0, 0xA8, 0x78, 0x68, 0, 0xA8, 0x78, 0x68, 0, 0x80, 0x80, 0x80, 0, 0, 0x64, 0, 2, 0, 0]);
        (Tables::build([(TABLE.into(), Elem::Rec(26), 4, 0, b)]).unwrap(), Trig::parse(&[0; 0x1004]))
    }

    #[test]
    fn colours_and_rows() {
        let (tables, trig) = st3();
        // RAM at frame 1300 (default camera): tile0.h = 0xA4
        let b = backdrop(&tables, &trig, 3, 0, 0, |p| {
            assert_eq!(p, [0, 512, 0]);
            Some(164.6)
        });
        assert_eq!(b, Backdrop { top: [0x70, 0, 0], bottom: [0xA8, 0x78, 0x68], rows: 164 });
        // base − L floored at 0
        assert_eq!(backdrop(&tables, &trig, 3, 0, 0x78, |_| Some(0.0)).bottom, [0x30, 0, 0]);
        assert_eq!(backdrop(&tables, &trig, 3, 0, 255, |_| Some(0.0)), Backdrop { top: [0; 3], bottom: [0; 3], rows: 0 });
        // SY clamped to 0..480
        assert_eq!(backdrop(&tables, &trig, 3, 0, 0, |_| Some(-30.0)).rows, 0);
        assert_eq!(backdrop(&tables, &trig, 3, 0, 0, |_| Some(900.0)).rows, 480);
        assert_eq!(backdrop(&tables, &trig, 3, 0, 0, |_| None).rows, 0);
    }
}
