//! The fighters' lighting: the stage light record, the light set-up, NCCS / NCCT.
//!
//! State: presentation (reads the state, never writes it)
//!
//! The fighters' lighting: the stage light record (0x44 bytes per stage, copied to
//! M+0x2F8), the frame's light set-up and the per-fighter one with the stage hooks
//! (one shared by ST4 / ST7 / ST8, another for ST6). The GTE then lights each vertex with NCCS / NCCT:
//! colour = vertex code × clamp(BK + LCM · max(0, LLM · n)) / 4096, where LLM = the light matrix (M+0x33C) times the
//! bone's world rotation, so its rows are light directions in world (game) space.
//!
//! Presentation only: read from the match state each drawn frame, nothing is kept.

use crate::fight::fighter::Fighter;
use crate::fight::physics::Trig;
use crate::formats::tables::Tables;

/// A fighter's light set as the GTE has it when its model is drawn.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Lights {
    /// the light matrix (rows: lights 1..3, directions in game space, 4.12; a normal facing a row is lit by it)
    pub dir: [[i32; 3]; 3],
    /// the colour matrix (rows: R, G, B; columns: lights 1..3; 4.12)
    pub colour: [[i32; 3]; 3],
    /// the back (ambient) colour (4.12)
    pub back: [i32; 3],
    /// the shadow matrix (M+0x35C): the x and z shift per unit of height (m01, m21; 4.12)
    pub shadow: [i32; 2],
}

/// The stage light records (`stage.light`: 13 × 0x44 bytes).
fn rec(tables: &Tables, stage: usize, off: u32) -> i32 {
    tables.table("stage.light").i16(0x44 * stage.min(12) + off as usize) as i32
}

/// The unit vector (−sin p · sin q, cos p, −sin p · cos q).
fn dir(t: &Trig, p: i32, q: i32) -> [i32; 3] {
    let s = -t.sin(p);
    [(s * t.sin(q)) >> 12, t.cos(p) as i16 as i32, (s * t.cos(q)) >> 12]
}

/// Tan a (4.12) from the tangent table (`math.tan`, 0x3AF entries), ±0x7FFF from 0x3AF on.
fn tan(tables: &Tables, a: i32) -> i32 {
    let i = if a & 0x400 != 0 { 0x400 - (a & 0x3FF) } else { a & 0x3FF };
    let v = if i >= 0x3AF { 0x7FFF } else { tables.table("math.tan").i16_at(i as usize) as i32 };
    if a & 0x400 != 0 { -v } else { v }
}

/// The shadow matrix [1, tan a · sin b, 0; 0, 0, 0; 0, tan a · cos b, 1]: (m01, m21).
fn shadow(tables: &Tables, t: &Trig, a: i32, b: i32) -> [i32; 2] {
    let k = tan(tables, a);
    [((k * t.sin(b)) >> 12) as i16 as i32, ((k * t.cos(b)) >> 12) as i16 as i32]
}

/// SquareRoot12(v): √v in 4.12 for v in 4.12 (= 64 √v) by the square root table (`math.sqrt`: 192 entries for the normalised 0x40..0xFF; LZCS normalisation).
fn sqrt12(tables: &Tables, v: i32) -> i32 {
    let lz = if v < 0 { (!v).leading_zeros() } else { v.leading_zeros() } as i32;
    if lz == 32 {
        return 0;
    }
    let e = lz & !1;
    let sh = (0x13 - e) >> 1;
    let m = if e >= 0x18 { v << (e - 0x18) } else { v >> (0x18 - e) };
    let r = tables.sqrt(m - 0x40);
    if sh >= 0 { r << sh } else { ((r as u32) >> -sh) as i32 }
}

/// The hit flash's light #3 (M+0x348) aimed by fighter `f`'s angles to the opponent.
pub fn flash_dir(t: &Trig, f: &Fighter) -> [i32; 3] {
    let v0 = crate::fight::fx::hit_effects::atan_xz(f.o_76 as i32, f.o_78 as i32);
    let mut v1 = f.yaw as i32 + f.o_74 as i32;
    if v1 < 0 {
        v1 += 0x1000;
    }
    dir(t, (0x800 - v0 as i16 as i32) & 0xFFF, 0x1000 - v1)
}

/// The light set of fighter `side` (0 = P1, M+0xE8 / 1 = P2, M+0x1F0) on stage `stage`: the frame's set-up, then
/// the fighter's (f, side + 1). `back`: the colour flash's back colour (else the record's); `flash`: the hit
/// flash's light #3 colour, which also aims light #3 by the fighter's angles to the opponent.
pub fn fighter(tables: &Tables, t: &Trig, stage: usize, side: usize, f: &Fighter, back: Option<[i32; 3]>, flash: Option<[i32; 3]>) -> Lights {
    let r = |off: u32| rec(tables, stage, off);
    // the colour matrix (light #3: the flash colour while the flag is set, else record +0x24, 0 on every
    // stage), lights #1 / #2 and the shadow from the record (it has no stage hook)
    let l3 = flash.unwrap_or([r(0x24), r(0x26), r(0x28)]);
    let mut l = Lights {
        dir: [dir(t, r(6), r(4)), dir(t, r(0xA), r(8)), [0; 3]],
        colour: [[r(0x14), r(0x1C), l3[0]], [r(0x16), r(0x1E), l3[1]], [r(0x18), r(0x20), l3[2]]],
        back: [0; 3],
        shadow: shadow(tables, t, r(2), r(0)),
    };
    // the back colour of entry side + 1 (record +0x34 / +0x3C, or what the colour flash put there)
    let k = 8 * (side as u32 + 1);
    l.back = back.unwrap_or([r(0x2C + k), r(0x2E + k), r(0x30 + k)]);
    if flash.is_some() {
        l.dir[2] = flash_dir(t, f);
    }
    let [x, y, z] = f.hips;
    match stage {
        // light #2 and the shadow from a point 0xA00 above the hips' level over the stage centre
        // (Square0: each square >> 12, SquareRoot12: × 64, so r = the horizontal distance)
        4 | 7 | 8 => {
            let yaw = t.atan2(z, x) as i16 as i32;
            let sq = |v: i32| ((v as i16 as i32) * (v as i16 as i32)) >> 12;
            let pitch = t.atan2(-(y + 0xA00), sqrt12(tables, sq(x) + sq(z))) as i16 as i32;
            l.dir[1] = dir(t, pitch, yaw);
            l.shadow = shadow(tables, t, pitch, yaw);
        }
        // light #1 horizontal along the fighter's direction from the centre, its colour scaled by the
        // distance to the centre (full from 0x600)
        6 => {
            let d = (f.o_72 as i32).min(0x600);
            for c in 0..3 {
                l.colour[c][0] = (r(0x14 + 2 * c as u32) * d / 0x600) as i16 as i32;
            }
            let yaw = t.atan2(z, x) as i16 as i32;
            l.dir[0] = dir(t, 0x400, (yaw + 0x800) & 0xFFF);
        }
        _ => {}
    }
    l
}

/// Engine addition (the import S8): a fighter's light set on an imported stage (the imported game's,
/// whose record (0x44 bytes by G.94) has the layout of the main game's): the colour
/// matrix and the shadow as the main game's; light #1's yaw relative to the camera's (`rec+4 − cam.yaw`), light #3
/// aimed by the record (+0xE, +0xC; main-game only aims it for a flash); the back colour of side + 1 as
/// the fighter set-up's. The stage hooks: stage 3 (MITSUKO's) light #2's red, stage 4 (GADO's) light #2
/// pulsing (their oscillators from the match's first frame: `frame`); stage 8's Uriko
/// glow (`glow`: the fighter it watches is in beast form; its demo mode G.0 = 7 lighting is not implemented). `rec`: the stage's record (`Loaded::br1_light`); `tables` / `t`:
/// the main game's tables for the shared math (tan, sine: byte-identical in both games).
#[allow(clippy::too_many_arguments)]
pub fn fighter_br1(tables: &Tables, rec: &[u8; 0x44], t: &Trig, g94: u8, side: usize, cam_yaw: i16, frame: u64, flash: Option<[i32; 3]>, glow: bool) -> Lights {
    let r = |off: u32| i16::from_le_bytes([rec[off as usize], rec[off as usize + 1]]) as i32;
    let mut l = Lights {
        dir: [dir(t, r(6), r(4) - cam_yaw as i32), dir(t, r(0xA), r(8)), dir(t, r(0xE), r(0xC))],
        colour: [[r(0x14), r(0x1C), r(0x24)], [r(0x16), r(0x1E), r(0x26)], [r(0x18), r(0x20), r(0x28)]],
        back: [0; 3],
        shadow: shadow(tables, t, r(2), r(0)),
    };
    // the hit's light flash (light #3's colour from the flash while its flag is set)
    if let Some(c) = flash {
        for k in 0..3 {
            l.colour[k][2] = c[k];
        }
    }
    let k = 8 * (side as u32 + 1);
    l.back = [r(0x2C + k), r(0x2E + k), r(0x30 + k)];
    match g94 {
        3 => {
            let p = osc(0x100, osc3, frame % OSC3_PERIOD);
            l.colour[0][1] = (l.colour[0][1] + p) as i16 as i32;
        }
        4 => {
            let p = osc(-0x80, osc4, frame % OSC4_PERIOD);
            for c in 0..3 {
                l.colour[c][1] = (l.colour[c][1] + p) as i16 as i32;
            }
        }
        8 => {
            // Uriko's glow, light #2 (level · 64, level · 32, level · 32) with the level 0..0x80 by ±4 a frame,
            // while the fighter it watches is in beast form (P2 when P2 is Uriko, else P1; Uriko never reverts)
            let p = osc(4, osc8, frame % OSC8_PERIOD);
            if glow {
                (l.colour[0][1], l.colour[1][1], l.colour[2][1]) = (p << 6, p << 5, p << 5);
            }
        }
        _ => {}
    }
    l
}

/// The stage oscillators' bounds: stage 3's and stage 4's, speed `s` turned at phase `p`.
fn osc3(p: i32, s: i32) -> bool {
    if s >= 0 { p >= 0x1000 } else { p < -0x7FF }
}
fn osc4(p: i32, s: i32) -> bool {
    if s >= 0 { p >= 0 } else { p < -0x9FF }
}
/// Stage 8's: 0..0x80 by 4.
fn osc8(p: i32, s: i32) -> bool {
    if s >= 0 { p >= 0x80 } else { p <= 0 }
}
/// Their periods in frames: back to phase 0 with the first speed (0 → 0x1000 → −0x800 → 0 by 0x100; 0 → −0xA00 → 0
/// by 0x80), so the phase of frame n is the one of n mod the period.
const OSC3_PERIOD: u64 = 48;
const OSC4_PERIOD: u64 = 40;
/// (stage 8: 0 → 0x80 → 0 by 4)
const OSC8_PERIOD: u64 = 64;

/// An oscillator's phase after `n` frames: phase += speed every frame, the speed turned at the bounds.
fn osc(speed: i32, turn: fn(i32, i32) -> bool, n: u64) -> i32 {
    let (mut p, mut s) = (0i32, speed);
    for _ in 0..n {
        p = (p + s) as i16 as i32;
        if turn(p, s) {
            s = -s;
        }
    }
    p
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tables() -> Option<Tables> {
        crate::formats::tables::export_tables()
    }

    fn trig() -> Option<Trig> {
        tables().map(|t| Trig::from_tables(&t))
    }

    #[test]
    fn sta_lights() {
        let (Some(e), Some(t)) = (tables(), trig()) else {
            eprintln!("skipped: no okb_export");
            return;
        };
        let f = Fighter::new(0, 0);
        let l = fighter(&e, &t, 10, 0, &f, None, None);
        // STA: cyan light #1 along −z, orange light #2 along +z, blue back colour (recorded OT walk: fighter vertex
        // colours (0, g, 64 + g) facing the camera at yaw 0)
        assert_eq!(l.dir[0], [0, 0, -4096]);
        assert_eq!(l.dir[1], [0, 0, 4096]);
        assert_eq!(l.colour, [[0, 6144, 0], [2048, 3072, 0], [2048, 0, 0]]);
        assert_eq!(l.back, [0, 0, 2048]);
    }

    #[test]
    fn uriko_glow() {
        let (Some(e), Some(t)) = (tables(), trig()) else {
            eprintln!("skipped: no okb_export");
            return;
        };
        let rec = [0u8; 0x44];
        let l2 = |frame: u64, glow: bool| {
            let l = fighter_br1(&e, &rec, &t, 8, 0, 0, frame, None, glow);
            [l.colour[0][1], l.colour[1][1], l.colour[2][1]]
        };
        // the level 4 a frame up to 0x80, then down to 0, light #2 = (level << 6, level << 5, level << 5)
        assert_eq!(l2(1, true), [4 << 6, 4 << 5, 4 << 5]);
        assert_eq!(l2(32, true), [0x80 << 6, 0x80 << 5, 0x80 << 5]);
        assert_eq!(l2(33, true), [0x7C << 6, 0x7C << 5, 0x7C << 5]);
        assert_eq!(l2(64, true), [0; 3]);
        assert_eq!(l2(65, true), [4 << 6, 4 << 5, 4 << 5]);
        assert_eq!(l2(33, false), [0; 3]);
    }

    #[test]
    fn oscillator_periods() {
        for (speed, turn, period) in [(0x100, osc3 as fn(i32, i32) -> bool, OSC3_PERIOD), (-0x80, osc4, OSC4_PERIOD), (4, osc8, OSC8_PERIOD)] {
            for n in 0..10 * period {
                assert_eq!(osc(speed, turn, n), osc(speed, turn, n % period), "frame {n}");
            }
        }
        assert_eq!(osc(0x100, osc3, 16), 0x1000);
        assert_eq!(osc(-0x80, osc4, 20), -0xA00);
    }

    #[test]
    fn square_root12() {
        let Some(e) = tables() else {
            eprintln!("skipped: no okb_export");
            return;
        };
        for v in [1, 2, 3, 4, 100, 4096, 65535, 1 << 20, 123_456_789] {
            let r = sqrt12(&e, v);
            let f = 64.0 * (v as f64).sqrt();
            assert!((r as f64 - f).abs() <= f / 256.0 + 1.0, "sqrt12({v}) = {r}, {f}");
        }
        assert_eq!(sqrt12(&e, 4096), 4096);
        assert_eq!(sqrt12(&e, 0), 0);
    }
}
