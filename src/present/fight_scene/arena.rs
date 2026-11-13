//! An extra stage on screen: the map as a lightmapped mesh and the sky round it.
//!
//! Engine addition: `fight::stage::extra`
//! State: presentation (reads the state, never writes it)
//!
//! The map's world (`fight_data::Arena`, `formats::q3_map`) moved into the fight's space (`fight::stage::extra`: the
//! stage's `centre` and `floor` at the origin, turned by its `turn`, `scale` game units per Quake unit, the fight's
//! simulation version's; glTF space = game (x, −y, −z) / 300) and drawn as a stage (`render::lightmapped`): the sky
//! (a far box, then cloud layers) first, then the world, all before the depth reset, so the fighters, their shadows
//! and the effects go over it as over every stage.

use crate::fight::fight_data::Arena;
use crate::fight::stage::extra::Extra;
use crate::fight::version::Gates;
use crate::formats::q3_map::{Blend, Look, Map, Vertex};
use crate::render::lightmapped::{CloudImage, LitBlend, LitDraw, LitImage, LitMesh, LitVertex};
use crate::render::{GpuModel, Instance, Renderer};
use glam::{Mat3, Mat4, Vec3};

/// Metres per Quake unit in the converter's glTF.
const METRES: f32 = 0.0254;

/// The map's point `p` (the file's glTF metres: Quake (x, z, −y) · 0.0254) in the fight's glTF space of stage `e`
/// (game (x, −y, −z) / 300, `extra_stage`: game (x, z) = turn · (Quake (x, y) − centre), y = −(Quake z − floor),
/// × `scale` game units per Quake unit, `version::Gates::q3_scale`).
pub(super) fn to_fight(e: &Extra, p: [f32; 3], scale: i32) -> [f32; 3] {
    let q = Vec3::new(p[0], -p[2], p[1]) / METRES - Vec3::new(e.centre[0] as f32, e.centre[1] as f32, e.floor as f32);
    let [[a, b], [c, d]] = e.turn.map(|r| r.map(|k| k as f32));
    (Vec3::new(a * q.x + b * q.y, q.z, -(c * q.x + d * q.y)) * scale as f32 / 300.0).into()
}

/// How high the courtyard clip's box reaches over the floor (Quake units: past the courtyard's walls, which rise past
/// 500).
const CLIP_HEIGHT_Q: i32 = 512;

/// The team battle's courtyard clip on a walled extra stage (T6, engine addition; Q3DM6): with the zoomed-out camera
/// standing outside the courtyard (it knows no stage geometry), the map between it and the ring (the south arcade's
/// pillars, the tops of the walls) is not drawn (`Renderer::set_instance_lit_clip`). The eye of the game-space view
/// `view_game` and the box over the ring (half sizes `half`, game units) from the floor up, in the fight's glTF
/// space (game (x, −y, −z) / 300). None on an open stage. A match does not clip (its frames stay as they were).
pub(super) fn courtyard_clip(e: &Extra, view_game: Mat4, half: [i16; 2], g: Gates) -> Option<[Vec3; 3]> {
    if !e.walled {
        return None;
    }
    let to_gltf = |p: Vec3| Vec3::new(p.x, -p.y, -p.z) / 300.0;
    let eye = to_gltf(view_game.inverse().transform_point3(Vec3::ZERO));
    let (hx, hz) = (half[0] as f32, half[1] as f32);
    let h = (CLIP_HEIGHT_Q * g.q3_scale()) as f32;
    Some([eye, Vec3::new(-hx, 0.0, -hz) / 300.0, Vec3::new(hx, h, hz) / 300.0])
}

/// The directions of stage `e`'s fight glTF space as Quake's (x, y, z): the inverse of `to_fight`'s turn (columns:
/// glTF x, y, z).
fn to_quake(e: &Extra) -> Mat4 {
    let [[a, b], [c, d]] = e.turn.map(|r| r.map(|k| k as f32));
    Mat4::from_cols(glam::Vec4::new(a, b, 0.0, 0.0), glam::Vec4::Z, glam::Vec4::new(-c, -d, 0.0, 0.0), glam::Vec4::W)
}

/// Surface `s`'s indices of `m` (none when its range does not lie in the index list, an overflowing one too).
fn surface_indices<'a>(m: &'a Map, s: &crate::formats::q3_map::Surface) -> &'a [u32] {
    s.first.checked_add(s.count).and_then(|end| m.indices.get(s.first as usize..end as usize)).unwrap_or(&[])
}

/// Stage `e`'s ground (`Extra::ground`): a square 16384 Quake units round the ring's centre, 1 under its floor (so
/// the map's own floors at that height stay over it), in the file's space, mapped as the ground shader's floor
/// triangles are (their texture coordinates' plane fitted to one of them at the floor's height) and lit by that
/// triangle's middle lightmap texel; with its look. None without a ground or such a triangle.
fn ground(e: &Extra, m: &Map) -> Option<([Vertex; 4], Look)> {
    let name = format!("baseq3/{}.png", e.ground?);
    let quake = |v: &Vertex| Vec3::new(v.pos[0], -v.pos[2], v.pos[1]) / METRES;
    for s in m.surfaces.iter().filter(|s| s.look.base == name) {
        for t in surface_indices(m, s).as_chunks::<3>().0 {
            let Some(v) = t.iter().map(|&i| m.vertices.get(i as usize)).collect::<Option<Vec<_>>>() else { continue };
            let q: Vec<Vec3> = v.iter().map(|v| quake(v)).collect();
            if q.iter().any(|q| (q.z - e.floor as f32).abs() > 0.5) {
                continue;
            }
            // uv = (x, y, 1) · coefficients, solved from the three corners
            let rows = Mat3::from_cols(Vec3::new(q[0].x, q[1].x, q[2].x), Vec3::new(q[0].y, q[1].y, q[2].y), Vec3::ONE);
            if rows.determinant().abs() < 1.0 {
                continue;
            }
            let inv = rows.inverse();
            let cu = inv * Vec3::new(v[0].uv[0], v[1].uv[0], v[2].uv[0]);
            let cv = inv * Vec3::new(v[0].uv[1], v[1].uv[1], v[2].uv[1]);
            let lm = [(v[0].lm[0] + v[1].lm[0] + v[2].lm[0]) / 3.0, (v[0].lm[1] + v[1].lm[1] + v[2].lm[1]) / 3.0];
            let corner = |dx: f32, dy: f32| {
                let p = Vec3::new(e.centre[0] as f32 + dx, e.centre[1] as f32 + dy, 1.0);
                let pos = [p.x * METRES, (e.floor - 1) as f32 * METRES, -p.y * METRES];
                Vertex { pos, uv: [cu.dot(p), cv.dot(p)], lm, color: [1.0; 4] }
            };
            const R: f32 = 16384.0;
            return Some(([corner(-R, -R), corner(R, -R), corner(R, R), corner(-R, R)], s.look.clone()));
        }
    }
    None
}

/// The GPU objects of an extra stage: the world and its instance, the sky box and its, the cloud layers and theirs;
/// the directions into Quake's (`to_quake`).
pub(super) struct ArenaView {
    world: GpuModel,
    world_inst: Instance,
    sky: Option<(GpuModel, Instance)>,
    clouds: Option<(GpuModel, Instance)>,
    to_sky: Mat4,
}

impl ArenaView {
    /// Stage `e`'s map at the scale of the rules `g` (the ring's, `Gates::q3_scale`).
    pub(super) fn new(r: &Renderer, e: &Extra, a: &Arena, g: Gates) -> Self {
        let m = &a.map;
        let scale = g.q3_scale();
        let mut vertices: Vec<LitVertex> =
            m.vertices.iter().map(|v| LitVertex { pos: to_fight(e, v.pos, scale), uv: v.uv, lm: v.lm, color: v.color, scroll: [0.0; 2] }).collect();
        let texture = |name: &str| a.textures.iter().position(|(t, im)| t == name && im.is_some());
        let mut draws = Vec::new();
        for s in &m.surfaces {
            let l = &s.look;
            // the scroll and the colour (vertex colours, or none) go into the surface's vertices
            for &i in surface_indices(m, s) {
                if let Some(v) = vertices.get_mut(i as usize) {
                    v.scroll = l.scroll;
                    if !l.vertex_colour {
                        v.color = [1.0; 4];
                    }
                }
            }
            draws.push(LitDraw {
                first: s.first,
                count: s.count,
                base: texture(&l.base),
                glow: l.glow.as_deref().and_then(texture),
                lightmap: l.lightmap,
                blend: match l.blend {
                    Blend::Opaque => LitBlend::Opaque,
                    Blend::Mask => LitBlend::Mask,
                    Blend::Alpha => LitBlend::Alpha,
                    Blend::Add => LitBlend::Add,
                },
                cull: l.cull,
            });
        }
        // the ground under the floor's edge (`Extra::ground`)
        let mut indices = m.indices.clone();
        if let Some((corners, look)) = ground(e, m) {
            let base = vertices.len() as u32;
            vertices.extend(corners.map(|v| LitVertex { pos: to_fight(e, v.pos, scale), uv: v.uv, lm: v.lm, color: [1.0; 4], scroll: [0.0; 2] }));
            draws.push(LitDraw {
                first: indices.len() as u32,
                count: 6,
                base: texture(&look.base),
                glow: None,
                lightmap: look.lightmap,
                blend: LitBlend::Opaque,
                cull: false,
            });
            indices.extend([0, 1, 2, 0, 2, 3].map(|k| base + k));
        }
        fn image(im: &crate::formats::gltf_model::Image) -> LitImage<'_> {
            LitImage { width: im.width, height: im.height, rgba: &im.rgba }
        }
        let images: Vec<Option<LitImage>> = a.textures.iter().map(|(_, im)| im.as_ref().map(image)).collect();
        let mesh = LitMesh { vertices: &vertices, indices: &indices, draws: &draws, images: &images, lightmap: a.lightmap.as_ref().map(image) };
        let sky = a.sky.as_ref().and_then(|f| r.upload_sky(&[0, 1, 2, 3, 4, 5].map(|k| image(&f[k])))).map(|g| (g, r.instance()));
        let layers: Vec<CloudImage> = a.clouds.iter().map(|(l, im)| CloudImage { image: image(im), scale: l.scale, scroll: l.scroll, add: l.add }).collect();
        let clouds = r.upload_clouds(&layers, a.cloud_height).map(|g| (g, r.instance()));
        Self { world: r.upload_lit(&mesh), world_inst: r.instance(), sky, clouds, to_sky: to_quake(e) }
    }

    /// The frame's stage items (the sky, then the world) under the game's camera: `view` / `proj` in game units
    /// (y down), `vp` the glTF space's view-projection, the stage darkening `dim` (0..1), `t` the time (s) the
    /// textures scroll by.
    /// `clip`: the team battle's courtyard clip (T6, `courtyard_clip`), None in a match.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn items(&self, r: &Renderer, view: Mat4, proj: Mat4, vp: Mat4, dim: f32, t: f32, clip: Option<[Vec3; 3]>) -> Vec<(&GpuModel, &Instance)> {
        let mut items = Vec::new();
        // the directions of the glTF space as the camera turns them (no eye position), and as the sky's (Quake's:
        // `to_quake`)
        let sky_vp = proj * Mat4::from_mat3(Mat3::from_mat4(view)) * Mat4::from_scale(Vec3::new(1.0, -1.0, -1.0));
        for (g, inst) in self.sky.iter().chain(&self.clouds) {
            r.set_instance_lit_mesh(inst, sky_vp, self.to_sky, dim, t);
            items.push((g, inst));
        }
        r.set_instance_lit_mesh(&self.world_inst, vp, Mat4::IDENTITY, dim, t);
        if let Some([eye, lo, hi]) = clip {
            r.set_instance_lit_clip(&self.world_inst, eye, lo, hi);
        }
        items.push((&self.world, &self.world_inst));
        items
    }

    /// GPU bytes (textures, buffers) for the performance overlay.
    pub(super) fn bytes(&self) -> [u64; 2] {
        self.sky.iter().chain(&self.clouds).map(|(g, _)| g.bytes).fold(self.world.bytes, |a, b| [a[0] + b[0], a[1] + b[1]])
    }
}
