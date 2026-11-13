//! The Beast Drive background: the wireframe ring walls and floor.
//!
//! State: presentation (reads the state, never writes it)
//!
//! The Beast Drive background (M.9F, stage darkening at 255): the stage is not drawn and the stage draw sorts five
//! line-grid objects instead — four green ring walls and the blue floor — built once at stage load (at stage load;
//! from the grid builder). Stages >= 9 have none.

use crate::fight::fx::effect_models::ModelPrim;
use crate::fight::fx::hit_effects::Gte;
use crate::formats::tables::Tables;

/// One grid object: polylines of 2 (LineF2) or 3 (LineF3) world vertices, one flat colour.
#[derive(Clone, Debug, Default)]
pub struct Grid {
    pub lines: Vec<Vec<[i16; 3]>>,
    pub rgb: [u8; 3],
}

const GREEN: [u8; 3] = [0x40, 0x80, 0x40];
const BLUE: [u8; 3] = [0x40, 0x40, 0x80];

fn add(a: [i16; 3], b: [i16; 3]) -> [i16; 3] {
    [0, 1, 2].map(|k| a[k].wrapping_add(b[k]))
}

/// The nU × nV cell grid from `o` along `u` and `v` (steps u / nU, v / nV truncated, int16).
fn grid(o: [i16; 3], u: [i16; 3], v: [i16; 3], nu: i32, nv: i32, rgb: [u8; 3]) -> Grid {
    if nu <= 0 || nv <= 0 {
        return Grid { lines: Vec::new(), rgb }; // engine addition: no division by zero on bad table data
    }
    let du = u.map(|x| (x as i32 / nu) as i16);
    let dv = v.map(|x| (x as i32 / nv) as i16);
    let mut lines = Vec::new();
    let mut row = o;
    for r in 0..nv {
        let mut p = row;
        for c in 0..nu {
            let (pu, pv, puv) = (add(p, du), add(p, dv), add(add(p, du), dv));
            if c >= r {
                lines.push(vec![p, pu, puv]);
            } else if nu < nv && c == nu - 1 {
                lines.push(vec![p, pu]); // the game's duplicate top edge (never reached: no grid has nU < nV)
            }
            if c <= r {
                lines.push(vec![puv, pv, p]);
            } else if nu > nv && r == nv - 1 {
                lines.push(vec![puv, pv]); // bottom border of the upper-right cells
            }
            p = pu;
        }
        row = add(row, dv);
    }
    Grid { lines, rgb }
}

/// The grids for ring half sizes `hx` / `hz` and wall height `h` (negative, y down):
/// objects 0 +X, 1 −X, 2 −Z, 3 +Z walls, 4 the floor.
pub fn build_ring(hx: i16, hz: i16, h: i32) -> [Grid; 5] {
    let rows = -h / 256;
    let (nx, nz) = ((hx >> 7) as i32, (hz >> 7) as i32);
    let (h, up) = (h as i16, (-h) as i16);
    let (x2, z2) = (hx.wrapping_mul(2), hz.wrapping_mul(2));
    [
        grid([hx, h, hz], [0, 0, z2.wrapping_neg()], [0, up, 0], nz, rows, GREEN),
        grid([-hx, h, -hz], [0, 0, z2], [0, up, 0], nz, rows, GREEN),
        grid([hx, h, -hz], [x2.wrapping_neg(), 0, 0], [0, up, 0], nx, rows, GREEN),
        grid([-hx, h, hz], [x2, 0, 0], [0, up, 0], nx, rows, GREEN),
        grid([-hx, 0, hz], [x2, 0, 0], [0, 0, z2.wrapping_neg()], nx, nz, BLUE),
    ]
}

/// The grids of stage `stage` (ring `stage.grid_ring`, height `stage.drive_grid`); `None` from stage 9 on (objects hidden).
pub fn build(tables: &Tables, stage: usize) -> Option<[Grid; 5]> {
    build_scaled(tables, stage, 100)
}

/// `build` over the ring scaled by `pct` percent in x / z (engine addition: the team battle's enlarged rings).
pub fn build_scaled(tables: &Tables, stage: usize, pct: u16) -> Option<[Grid; 5]> {
    if stage >= 9 {
        return None;
    }
    let (ring, o) = (tables.table("stage.grid_ring"), 8 * stage);
    let s = |v: i16| (v as i32 * pct as i32 / 100) as i16;
    Some(build_ring(s(ring.i16(o + 4)), s(ring.i16(o + 6)), tables.table("stage.drive_grid").u32(4 * stage) as i32))
}

/// Wall `side` (0 +X, 1 −X, 2 +Z, 3 −Z) is behind the fighters for camera yaw `yaw`.
pub fn far(side: usize, yaw: i16) -> bool {
    let s12 = |v: i32| (v << 20) >> 20;
    let yaw = s12(yaw as i32);
    match side {
        0 => s12(yaw + 0x400).abs() >= 0x31C,
        1 => s12(yaw - 0x400).abs() >= 0x31C,
        2 => yaw.abs() < 0x4E4,
        _ => yaw.abs() >= 0x31C,
    }
}

/// RTPT as the line handlers see it: frame-buffer SX / SY (OFX 256, OFY 240), `None` when SZ < 0 (FLAG bit 18).
/// The division saturates at 2 (H / SZ with H = 360: SZ < 180 projects as 180) and SX / SY at −1024..1023.
fn rtps(gte: &Gte, p: [i32; 3]) -> Option<[i32; 2]> {
    let v = gte.view * glam::Vec4::new(p[0] as f32, p[1] as f32, p[2] as f32, 1.0);
    if v.z < 0.0 {
        return None;
    }
    let z = v.z.max(gte.h / 4.0);
    let sx = (256.0 + 0.8 * gte.h * v.x / z).floor().clamp(-1024.0, 1023.0) as i32;
    let sy = (240.0 + gte.h * v.y / z).floor().clamp(-1024.0, 1023.0) as i32;
    Some([sx, sy])
}

/// A 1-pixel GPU line from frame-buffer pixel `a` to `b` as a quad in display units (one pixel across the minor
/// axis, both end pixels included).
fn line(a: [i32; 2], b: [i32; 2], rgb: [u8; 3], otz: i32) -> ModelPrim {
    let (ax, ay, bx, by) = (a[0] as f32 + 0.5, a[1] as f32 + 0.5, b[0] as f32 + 0.5, b[1] as f32 + 0.5);
    let xy = if (b[0] - a[0]).abs() >= (b[1] - a[1]).abs() {
        let e = if bx >= ax { 0.5 } else { -0.5 };
        [[ax - e, ay - 0.5], [bx + e, by - 0.5], [ax - e, ay + 0.5], [bx + e, by + 0.5]]
    } else {
        let e = if by >= ay { 0.5 } else { -0.5 };
        [[ax - 0.5, ay - e], [bx - 0.5, by + e], [ax + 0.5, ay - e], [bx + 0.5, by + e]]
    };
    ModelPrim { xy: xy.map(|p| [(p[0] - 256.0) * 1.25, p[1] - 240.0]), otz, rgb: [rgb; 4], uv: [[0; 2]; 4], tpage: 0, clut: 0, textured: false, semi: false }
}

/// libgs LineF2 / LineF3 handlers for one object shifted by `off` (game units): a
/// polyline is dropped when a vertex is behind the camera or when no vertex is on screen in x or in y; the GPU
/// skips segments with |dx| >= 1024 or |dy| >= 512. The on-screen range grows with the view's field (`Gte::fov`: Hor+ x, Vert+ y).
pub fn prims(g: &Grid, off: [i32; 3], gte: &Gte, otz: i32, out: &mut Vec<ModelPrim>) {
    let half = 0.8 * 240.0 * gte.fov[0];
    let yh = 240.0 / gte.fov[1];
    let (y0, y1) = ((240.0 - yh) as i32, (240.0 + yh) as i32);
    let (x0, x1) = ((256.0 - half) as i32, (256.0 + half) as i32);
    for l in &g.lines {
        let mut buf = [[0i32; 2]; 3];
        let n = l.len().min(3);
        if l[..n].iter().zip(&mut buf).any(|(p, b)| rtps(gte, [0, 1, 2].map(|k| p[k] as i32 + off[k])).map(|v| *b = v).is_none()) {
            continue;
        }
        let s = &buf[..n];
        if !s.iter().any(|p| (x0..x1).contains(&p[0])) || !s.iter().any(|p| (y0..y1).contains(&p[1])) {
            continue;
        }
        for w in s.windows(2) {
            if (w[1][0] - w[0][0]).abs() < 1024 && (w[1][1] - w[0][1]).abs() < 512 {
                out.push(line(w[0], w[1], g.rgb, otz));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stage7_grid() {
        let g = build_ring(1536, 1536, -768);
        let count = |g: &Grid, n: usize| g.lines.iter().filter(|l| l.len() == n).count();
        assert_eq!((count(&g[4], 3), count(&g[4], 2)), (156, 0));
        for w in &g[..4] {
            assert_eq!((count(w, 3), count(w, 2)), (39, 9));
            assert_eq!(w.rgb, GREEN);
        }
        assert_eq!(g[4].lines[0], vec![[-1536, 0, 1536], [-1280, 0, 1536], [-1280, 0, 1280]]);
        assert_eq!(g[0].lines[0], vec![[1536, -768, 1536], [1536, -768, 1280], [1536, -512, 1280]]);
        // every cell edge once: 13·12·2 floor edges, 13·3 + 4·12 per wall
        let edges = |g: &Grid| g.lines.iter().map(|l| l.len() - 1).sum::<usize>();
        assert_eq!(edges(&g[4]), 2 * 13 * 12);
        assert_eq!(edges(&g[2]), 13 * 3 + 4 * 12);
    }

    #[test]
    fn stage4_rows() {
        let g = build_ring(1536, 1536, -1536);
        assert_eq!(g[1].lines.iter().filter(|l| l.len() == 2).count(), 12 - 6);
        assert_eq!(g[1].lines.iter().map(|l| l.len() - 1).sum::<usize>(), 13 * 6 + 7 * 12);
    }

    #[test]
    fn degenerate_table() {
        let g = build_ring(100, 1536, -100);
        assert!(g.iter().all(|w| w.lines.is_empty()));
    }

    #[test]
    fn far_rule() {
        // raw frame 1289, yaw 4093 (−3): +X, −X, +Z behind the fighters, −Z near the camera
        assert_eq!([0, 1, 2, 3].map(|s| far(s, 4093)), [true, true, true, false]);
        assert_eq!([0, 1, 2, 3].map(|s| far(s, 0x800)), [true, true, false, true]);
    }
}
