//! The head turn toward the opponent at the end of the per-fighter script step (simulation version 13).
//!
//! State: none of its own: updates the serialised `Fighter` (`head_yaw`, `head_pitch` = state +0x106 / +0x108)
//!
//! While F5C & 0x10000 is set (stance, walk) and the fighter is not knocked out, two accumulators step toward the
//! opponent: the yaw (±0x31C, ±0x2D a frame) from the roots' angle, the pitch (−0x271..0x1C7, ±0x22 a frame) from
//! the heads' (node 2's world positions of the last world walk); otherwise both are 0. The game then turns node 2's
//! local matrix by RotMatrix(pitch, −yaw, 0) (`anim::pose`, applied where the pose's matrices are built).

use crate::fight::fighter::{F5C, Fighter};
use crate::fight::physics::{Trig, wrap12};
use crate::formats::tables::Tables;

/// SquareRoot0(v): √v by table (LZCS normalisation); `table(k)` = entry k (clamped to 0..=191) of the
/// table (the main game's; the imported game's own SquareRoot0 is the same code on its own table).
pub fn square_root0(table: &dyn Fn(i32) -> i32, v: i32) -> i32 {
    let lz = if v < 0 { (!v).leading_zeros() } else { v.leading_zeros() } as i32;
    if lz == 32 {
        return 0;
    }
    let e = lz & !1;
    let sh = (31 - e) >> 1;
    let m = if e >= 24 { v << (e - 24) } else { v >> (24 - e) };
    (((table(m - 0x40) as u32) << sh) >> 12) as i32
}

/// Fighter `me`'s head-turn accumulators for this frame, from both fighters' node 2 of
/// the last world walk (`Fighter::head_w`): its head turns toward `opp`.
pub fn head_turn(fs: &mut [Fighter], me: usize, opp: usize, t: &Trig, tables: &Tables) {
    let (o_t, oh) = (fs[opp].m_t, fs[opp].head_w);
    let f = &mut fs[me];
    if f.flags[F5C] & 0x1_0000 == 0 || (f.o_00 as i8) < 0 {
        f.head_yaw = 0;
        f.head_pitch = 0;
        return;
    }
    // yaw: the angle to the opponent's root relative to the facing, plus half node 2's twist record (P+0x3E)
    let tw = (f.pose[2].twist as i32) >> 1;
    let mut a = wrap12(t.atan2(f.m_t[2] - o_t[2], o_t[0] - f.m_t[0]) - f.yaw as i32 + tw) as i32;
    if a.abs() >= 0x31D {
        a = if a < 0 { -0x31C } else { 0x31C };
    }
    // the pitch's bias for an opponent behind (never taken after the clamp above)
    let aa = a.abs();
    let bias = if aa >= 0x401 { (0x800 - aa) / 8 } else { 0 };
    let mut d = a - wrap12(tw + f.head_yaw as i32) as i32;
    if d.abs() >= 0x2E {
        d = if d < 0 { -0x2D } else { 0x2D };
    }
    f.head_yaw = f.head_yaw.wrapping_add(d as i16);
    // pitch: the heads' height difference over their horizontal distance (Square0: the GTE's IR take the
    // low 16 bits), plus node 2's direction record (P+0x3A / +0x3C)
    let h = f.head_w;
    let (dx, dz) = (oh[0].wrapping_sub(h[0]) as i16 as i32, oh[2].wrapping_sub(h[2]) as i16 as i32);
    let r = square_root0(&tables.sqrt_fn(), (dx * dx).wrapping_add(dz * dz));
    // the height difference kept within atan2's range (engine guard: real heads differ by a few thousand; a corrupt
    // save file could hold anything)
    let p = t.atan2(r, oh[1].wrapping_sub(h[1]).clamp(-(1 << 19), 1 << 19));
    let q = t.atan2(f.pose[2].dir[1] as i32, f.pose[2].dir[2] as i32);
    let target = (wrap12(p + q + bias) as i32).clamp(-0x271, 0x1C7);
    let mut d = target - wrap12(q + f.head_pitch as i32) as i32;
    if d.abs() >= 0x23 {
        d = if d < 0 { -0x22 } else { 0x22 };
    }
    f.head_pitch = f.head_pitch.wrapping_add(d as i16);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn square_root0_by_table() {
        let Some(tables) = crate::formats::tables::export_tables() else {
            eprintln!("skipped: no okb_export");
            return;
        };
        let t = tables.sqrt_fn();
        assert_eq!(square_root0(&t, 0), 0);
        for v in [1, 2, 3, 4, 100, 255, 256, 1000, 65535, 1 << 20, 123_456_789, i32::MAX] {
            let (r, f) = (square_root0(&t, v), (v as f64).sqrt());
            assert!((r as f64 - f).abs() <= f / 128.0 + 1.0, "SquareRoot0({v}) = {r}, {f}");
        }
    }

    /// One recorded frame (Yugo vs Bakuryu from vssel.sst with off-axis base positions, raw frame
    /// 1552: P1 just put next to P2): both steps clamped (yaw −0x2D, pitch ∓0x22) to the game's S+0x106 / +0x108.
    #[test]
    fn head_turn_as_recorded() {
        let Some(tables) = crate::formats::tables::export_tables() else {
            eprintln!("skipped: no okb_export");
            return;
        };
        let t = Trig::from_tables(&tables);
        let mut fs = [Fighter::new(0, 0), Fighter::new(1, 1)];
        // the frame before: roots, node 2's world positions, accumulators
        for (f, m_t, h, acc) in
            fs.iter_mut().zip([([100, 0, 250], [114, -372, 250], (274, 76)), ([-171, 0, 11], [-176, -351, -3], (455, -76))]).map(|(f, (a, b, c))| (f, a, b, c))
        {
            (f.m_t, f.head_w, f.head_yaw, f.head_pitch) = (m_t, h, acc.0, acc.1);
            f.flags[F5C] = 0x1051_4400;
        }
        // this frame: the yaw and node 2's pose record
        (fs[0].yaw, fs[0].pose[2].dir, fs[0].pose[2].twist) = (1650, [-1932, 15735, -2004], 1054);
        (fs[1].yaw, fs[1].pose[2].dir, fs[1].pose[2].twist) = (-296, [-715, 15536, 3623], 682);
        head_turn(&mut fs, 0, 1, &t, &tables);
        head_turn(&mut fs, 1, 0, &t, &tables);
        assert_eq!([(fs[0].head_yaw, fs[0].head_pitch), (fs[1].head_yaw, fs[1].head_pitch)], [(229, 42), (410, -42)]);
        // the gate: F5C & 0x10000 clear (an attack) or knocked out resets both
        fs[0].flags[F5C] = 0x20;
        fs[1].o_00 = 0x81;
        head_turn(&mut fs, 0, 1, &t, &tables);
        head_turn(&mut fs, 1, 0, &t, &tables);
        assert_eq!([(fs[0].head_yaw, fs[0].head_pitch), (fs[1].head_yaw, fs[1].head_pitch)], [(0, 0), (0, 0)]);
    }
}
