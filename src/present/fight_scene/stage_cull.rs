//! The stage polygons the PS1 does not draw: the stage primitive drivers' tests and the GPU's size limit.
//!
//! State: presentation (reads the state, never writes it)
//!
//! The stage sorter hands every polygon of a stage object to its type's driver, which projects it on the GTE
//! and links it only if: RTPT v0..v2 leaves no FLAG bit of 0x7FC40000 (IR1..3 saturated or SZ3 below 0: a corner
//! behind the camera, or 32768 or more ahead or aside), NCLIP > 0, and one corner's SX is in [0, 512) and one's SY
//! in [0, 480) (all corners, v3 from RTPS without a FLAG test). The GPU then drops each triangle of a linked polygon
//! (a quad: (v0, v1, v2) and (v1, v2, v3)) whose corners lie more than 1023 pixels apart across or 511 down; SX / SY
//! saturate at −1024..1023. A large polygon reaching behind or close past the camera (STB's dome floor, ST9's
//! walls, STC's corridor seen from outside) is so dropped whole and the backdrop shows. The engine draws the stage
//! with the true projection (clipped at the near plane), so `StageCull::cull` writes the stage model's indices each
//! frame with the dropped triangles made degenerate. The polygons that are drawn keep the true projection (the
//! game's saturated corners are not reproduced); the GTE is stood in for in floats (a polygon at a test's
//! boundary can differ).

use crate::formats::gltf_model::{FLOOR, ModelData};
use crate::render::{GpuModel, Renderer};
use glam::{Mat4, Vec3};

/// A stage polygon: its corners v0..v3 in the HMD's order (model space, glTF units; v3 unused for a triangle) and
/// the index positions of its triangles in the model's index buffer, (v0, v1, v2) and for a quad (v1, v2, v3).
struct Poly {
    v: [Vec3; 4],
    tri: [usize; 2],
    quad: bool,
}

/// The GTE and the frame buffer the stage is projected on: H and the visible area's half size round
/// the screen centre (256, 240) in frame-buffer pixels: (256, 240) at the native 4:3 (a wider or taller view of
/// the engine shows more: its polygons there are kept in the same way).
pub struct Screen {
    h: f32,
    half: [f32; 2],
}

impl Screen {
    pub fn new(cam: &crate::fight::camera::Camera, layout: &crate::output::layout::Viewport) -> Self {
        let h = cam.h as f32;
        let [kx, ky] = layout.proj_scale(h);
        // the 4:3 view's scale is (h / 160, h / 120)
        Self { h, half: [256.0 * h / 160.0 / kx, 240.0 * h / 120.0 / ky] }
    }
}

/// The screen-scale matrix on the view (M.20 = 512, M.22 = 480 over the logical 320×240): x · (512 << 12) /
/// 320 >> 12, y · 2.
const SCALE: [f32; 2] = [((512 << 12) / 320) as f32 / 4096.0, 2.0];

/// The stage model's polygons by OT unit (not the floor, which its own handler draws) and its index buffer.
pub struct StageCull {
    /// per unit: the unit, its polygons and the span of index positions they cover
    units: Vec<(u8, Vec<Poly>, std::ops::Range<usize>)>,
    indices: Vec<u32>,
    /// the indices as last written to the GPU
    written: std::cell::RefCell<Vec<u32>>,
}

impl StageCull {
    /// The polygons of `m`'s OT units. `ok_br2_converter::stages` writes a quad (v0, v1, v2, v3) as the triangles
    /// (v0, v2, v1), (v1, v2, v3) (or, for the winding of a stage with lit polygons, (v0, v1, v2), (v1, v3, v2)), the
    /// two in a row in one primitive: they are paired by their corners' positions, the winding the one most pairs
    /// fit.
    pub fn new(m: &ModelData) -> Self {
        let pos = |i: u32| Vec3::from(m.vertices.get(i as usize).map_or([0.0; 3], |v| v.pos));
        let tri = |k: usize| [0, 1, 2].map(|j| pos(m.indices[k + j]));
        let ranges: Vec<(u8, usize, usize)> = (m.prims.iter())
            .filter_map(|p| Some((p.ot?, p.first_index as usize, (p.first_index + p.count) as usize)))
            .filter(|&(u, a, b)| u != FLOOR && a <= b && b <= m.indices.len())
            .collect();
        // a = (v0, v2, v1), b = (v1, v2, v3) / a = (v0, v1, v2), b = (v1, v3, v2)
        let pair: [fn([Vec3; 3], [Vec3; 3]) -> bool; 2] = [|a, b| b[0] == a[2] && b[1] == a[1], |a, b| b[0] == a[1] && b[2] == a[2]];
        let fits = |w: usize| {
            let pairs = |&(_, a, b): &(u8, usize, usize)| (a..b.saturating_sub(5)).step_by(3).filter(|&k| pair[w](tri(k), tri(k + 3))).count();
            ranges.iter().map(pairs).sum::<usize>()
        };
        let w = (fits(1) >= fits(0)) as usize; // a tie (no quads) takes the winding every current export uses
        let mut units: Vec<(u8, Vec<Poly>, std::ops::Range<usize>)> = Vec::new();
        for &(u, a, b) in &ranges {
            if units.last().is_none_or(|l| l.0 != u) {
                units.push((u, Vec::new(), a..b));
            }
            let Some(unit) = units.last_mut() else { continue };
            unit.2 = unit.2.start.min(a)..unit.2.end.max(b);
            let mut k = a;
            while k + 3 <= b {
                let t = tri(k);
                if k + 6 <= b && pair[w](t, tri(k + 3)) {
                    let n = tri(k + 3);
                    let v = if w == 0 { [t[0], t[2], t[1], n[2]] } else { [t[0], t[1], t[2], n[1]] };
                    unit.1.push(Poly { v, tri: [k, k + 3], quad: true });
                    k += 6;
                } else {
                    let v = if w == 0 { [t[0], t[2], t[1], t[1]] } else { [t[0], t[1], t[2], t[2]] };
                    unit.1.push(Poly { v, tri: [k, k], quad: false });
                    k += 3;
                }
            }
        }
        Self { units, written: std::cell::RefCell::new(m.indices.clone()), indices: m.indices.clone() }
    }

    /// Writes `gm`'s indices for this frame's `drawn` units (unit, model matrix): the triangles the PS1 drops under
    /// `view` (glTF space → the game camera's view, game units) made degenerate.
    pub fn cull(&self, r: &Renderer, gm: &GpuModel, drawn: &[(u8, Mat4)], view: Mat4, scr: &Screen) {
        let mut written = self.written.borrow_mut();
        for &(u, xf) in drawn {
            let Some((_, polys, span)) = self.units.iter().find(|(k, ..)| *k == u) else { continue };
            let mut idx = self.indices[span.clone()].to_vec();
            let m = view * xf;
            for p in polys {
                let keep = drawn_tris(p.v.map(|c| m.transform_point3(c)), p.quad, scr);
                for (t, keep) in p.tri.iter().zip(keep).take(1 + p.quad as usize) {
                    if !keep {
                        let k = t - span.start;
                        idx[k + 1] = idx[k];
                        idx[k + 2] = idx[k];
                    }
                }
            }
            if written[span.clone()] != idx[..] {
                r.write_indices(gm, span.start as u32, &idx);
                written[span.clone()].copy_from_slice(&idx);
            }
        }
    }
}

/// Which triangles of a polygon with corners `c` (view space, game units: x right, y down, z ahead) the PS1 draws:
/// the driver's tests on the whole polygon, then the GPU's size limit per triangle.
fn drawn_tris(c: [Vec3; 4], quad: bool, scr: &Screen) -> [bool; 2] {
    // the GTE's IR = the screen-scaled view, MAC = (R · v >> 12) + T
    let ir = c.map(|v| [(v.x * SCALE[0]).floor(), (v.y * SCALE[1]).floor(), v.z.floor()]);
    // RTPT v0..v2: FLAG & 0x7FC40000 (IR1..3 outside −0x8000..0x7FFF, SZ3 below 0)
    if ir[..3].iter().any(|v| v.iter().any(|&x| !(-32768.0..=32767.0).contains(&x)) || v[2] < 0.0) {
        return [false; 2];
    }
    let s = ir.map(|v| project(v, scr.h));
    // NCLIP of v0..v2 > 0
    let nclip = (s[1][0] - s[0][0]) as i64 * (s[2][1] - s[0][1]) as i64 - (s[2][0] - s[0][0]) as i64 * (s[1][1] - s[0][1]) as i64;
    let n = 3 + quad as usize;
    // a corner's SX in [0, 512) and one's SY in [0, 480) (the engine's visible area)
    let inside = |k: usize, centre: f32| s[..n].iter().any(|v| (centre - scr.half[k]..centre + scr.half[k]).contains(&(v[k] as f32)));
    if nclip <= 0 || !inside(0, 256.0) || !inside(1, 240.0) {
        return [false; 2];
    }
    // the GPU: a triangle more than 1023 pixels across or 511 down is not drawn
    let fits = |t: [usize; 3]| {
        let span = |k: usize| t.iter().map(|&i| s[i][k]).max().unwrap_or(0) - t.iter().map(|&i| s[i][k]).min().unwrap_or(0);
        span(0) <= 1023 && span(1) <= 511
    };
    [fits([0, 1, 2]), quad && fits([1, 2, 3])]
}

/// RTPS / RTPT: SX, SY of the IR vector `v` (OFX 256, OFY 240): the division saturates at 0x1FFFF / 0x10000 (SZ ≤
/// H / 2), the result at −0x400..0x3FF.
fn project(v: [f32; 3], h: f32) -> [i32; 2] {
    let sz = v[2].clamp(0.0, 65535.0);
    let q = if 2.0 * sz <= h { 0x1FFFF as f32 / 65536.0 } else { (h / sz).min(0x1FFFF as f32 / 65536.0) };
    [256.0 + v[0] * q, 240.0 + v[1] * q].map(|x| (x.floor() as i32).clamp(-1024, 1023))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::formats::gltf_model::{Prim, Vertex};

    /// The view-space point that projects to frame-buffer pixel (sx, sy) at depth 360 (= H: q = 1).
    fn at(sx: f32, sy: f32) -> Vec3 {
        Vec3::new((sx - 256.0) / SCALE[0], (sy - 240.0) / SCALE[1], 360.0)
    }

    fn native() -> Screen {
        Screen { h: 360.0, half: [256.0, 240.0] }
    }

    #[test]
    fn the_drivers_tests_and_the_gpu_size_limit() {
        let quad = |s: [[f32; 2]; 4]| s.map(|[x, y]| at(x, y));
        // on screen, facing the camera
        assert_eq!(drawn_tris(quad([[10.0, 10.0], [500.0, 10.0], [10.0, 400.0], [500.0, 400.0]]), true, &native()), [true, true]);
        // facing away (NCLIP ≤ 0)
        assert_eq!(drawn_tris(quad([[10.0, 10.0], [10.0, 400.0], [500.0, 10.0], [500.0, 400.0]]), true, &native()), [false, false]);
        // a corner of v0..v2 behind the camera (SZ3 below 0); v3 is not tested
        let mut c = quad([[10.0, 10.0], [500.0, 10.0], [10.0, 400.0], [500.0, 400.0]]);
        c[2].z = -1.0;
        assert_eq!(drawn_tris(c, true, &native()), [false, false]);
        let mut c = quad([[10.0, 10.0], [500.0, 10.0], [10.0, 400.0], [500.0, 400.0]]);
        c[3].z = -1.0;
        assert!(drawn_tris(c, true, &native())[0]);
        // over the whole screen but no corner's SX in [0, 512)
        assert_eq!(drawn_tris(quad([[-100.0, 10.0], [600.0, 10.0], [-100.0, 400.0], [600.0, 400.0]]), true, &native()), [false, false]);
        // a wider view (engine addition) keeps it
        let wide = Screen { h: 360.0, half: [400.0, 240.0] };
        assert_eq!(drawn_tris(quad([[-100.0, 10.0], [600.0, 10.0], [-100.0, 400.0], [600.0, 400.0]]), true, &wide), [true, true]);
        // the second triangle (v1, v2, v3) more than 1023 pixels across (v3 saturated at 1023)
        assert_eq!(drawn_tris(quad([[10.0, 10.0], [500.0, 10.0], [-5.0, 400.0], [1500.0, 400.0]]), true, &native()), [true, false]);
        // more than 511 down
        assert_eq!(drawn_tris(quad([[10.0, 10.0], [500.0, 10.0], [10.0, 530.0], [500.0, 530.0]]), true, &native()), [false, false]);
    }

    #[test]
    fn quads_paired_from_the_export() {
        let p: Vec<[f32; 3]> = (0..7).map(|k| [k as f32, (k * k) as f32, 0.0]).collect();
        let vertex = |pos: [f32; 3]| Vertex { pos, normal: [0.0; 3], uv: [0.0; 2], color: [1.0; 4], joints: [0; 4], weights: [1.0, 0.0, 0.0, 0.0] };
        // a quad v0..v3 as (v0, v2, v1), (v1, v2, v3), then a triangle v4..v6 as (v4, v6, v5)
        let indices = vec![0, 2, 1, 1, 2, 3, 4, 6, 5];
        let prim = Prim {
            first_index: 0,
            count: 9,
            texture: None,
            hidden: false,
            lit: false,
            wall: None,
            shadow: false,
            over: false,
            ot: Some(5),
            semi: false,
            node: 0,
        };
        let m = ModelData {
            vertices: p.iter().map(|&q| vertex(q)).collect(),
            indices,
            prims: vec![prim],
            images: Vec::new(),
            nodes: Vec::new(),
            joints: Vec::new(),
            inverse_bind: Vec::new(),
            clips: Default::default(),
        };
        let c = StageCull::new(&m);
        assert_eq!(c.units.len(), 1);
        let (u, polys, span) = &c.units[0];
        assert_eq!((*u, span.clone(), polys.len()), (5, 0..9, 2));
        let v = |k: usize| Vec3::from(p[k]);
        assert!(polys[0].quad && polys[0].tri == [0, 3] && polys[0].v == [v(0), v(1), v(2), v(3)]);
        assert!(!polys[1].quad && polys[1].tri[0] == 6 && polys[1].v[..3] == [v(4), v(5), v(6)]);
    }
}
