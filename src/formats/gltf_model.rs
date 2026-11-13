//! glTF (.glb) loading: geometry, textures, node hierarchy, skin and animation clips.
//!
//! State: static data (not serialised; re-attached by `rebind`)

use glam::{Mat4, Quat, Vec3};
use std::collections::HashMap;

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub struct Vertex {
    pub pos: [f32; 3],
    pub normal: [f32; 3],
    pub uv: [f32; 2],
    pub color: [f32; 4],
    pub joints: [u32; 4],
    pub weights: [f32; 4],
}

#[derive(Clone)]
pub struct Prim {
    pub first_index: u32,
    pub count: u32,
    pub texture: Option<usize>,
    pub hidden: bool,
    pub lit: bool,
    /// stage ring wall 0..3 (node `…_wallK`): hidden while that wall model is gone (broken / absent)
    pub wall: Option<u8>,
    /// the fighter's shadow stand-in (HMD slot 1 / 3), drawn flattened by the shadow pass
    pub shadow: bool,
    /// semi-transparent stage polygons of the object the stage routine draws last (node `…_semi_over`): over the
    /// rest of the stage whatever the depth
    pub over: bool,
    /// a unit of a stage in ordering-table order (`<ST>_ot.glb`, node `…_ot_obj<k>` / `…_ot_floor`): slot-2 object k
    /// or `FLOOR`
    pub ot: Option<u8>,
    /// a semi-transparent polygon that blends (material `…_stp`, `<ST>_ot.glb`): texels without the STP bit opaque,
    /// STP texels and untextured polygons added
    pub semi: bool,
    /// the glTF node it came from (its index in `ModelData::nodes`; 0 for a model built in code)
    pub node: u32,
}

/// `gltf_model::Prim::ot` of the floor (the floor object W+0).
pub const FLOOR: u8 = 0xFF;

#[derive(Clone)]
pub struct Node {
    pub name: String,
    pub parent: Option<usize>,
    pub t: Vec3,
    pub r: Quat,
    pub s: Vec3,
}

#[derive(Clone)]
pub enum Track {
    T(Vec<Vec3>),
    R(Vec<Quat>),
    S(Vec<Vec3>),
}

#[derive(Clone)]
pub struct Channel {
    pub node: usize,
    pub times: Vec<f32>,
    pub track: Track,
}

#[derive(Clone)]
pub struct Clip {
    pub duration: f32,
    pub channels: Vec<Channel>,
}

#[derive(Clone)]
pub struct Image {
    /// the glTF image name (stages: `<ST>_tp<tpage>_cl<clut>`, the page and CLUT it was decoded with)
    pub name: String,
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

#[derive(Clone)]
pub struct ModelData {
    pub vertices: Vec<Vertex>,
    pub indices: Vec<u32>,
    pub prims: Vec<Prim>,
    pub images: Vec<Image>,
    pub nodes: Vec<Node>,
    pub joints: Vec<usize>,
    pub inverse_bind: Vec<Mat4>,
    pub clips: HashMap<String, Clip>,
}

/// Meshes whose name ends in one of these start hidden (shadow proxies, wall debris).
const HIDDEN_SUFFIXES: [&str; 2] = ["_shadow_proxy", "_wall_debris"];
/// The open-hand mesh: drawn, its vertices tagged with joint group 1 (bit 8 of the first joint index) so the
/// per-instance hide masks pick fists or open hands per side (objects 11 / 14 / 15 / 16 on joints 13 / 16).
const ALT_HANDS: &str = "_alt_hands";
pub const JOINT_GROUP_ALT: u32 = 0x100;

impl ModelData {
    /// A model with nothing in it.
    pub fn empty() -> Self {
        Self {
            vertices: Vec::new(),
            indices: Vec::new(),
            prims: Vec::new(),
            images: Vec::new(),
            nodes: Vec::new(),
            joints: Vec::new(),
            inverse_bind: Vec::new(),
            clips: HashMap::new(),
        }
    }

    pub fn from_glb(bytes: &[u8]) -> Result<Self, String> {
        Self::from_glb_variant(bytes, None)
    }

    /// The number of material variants of a glb (`KHR_materials_variants`: the imported game's costumes); 0 without.
    pub fn variant_count(bytes: &[u8]) -> usize {
        gltf::Gltf::from_slice(bytes).ok().and_then(|g| g.variants().map(|v| v.count())).unwrap_or(0)
    }

    /// `from_glb` with the materials of variant `variant` (`KHR_materials_variants`, engine addition for the imported game's
    /// costumes: ok_br1_converter's fighter models hold every costume as a variant) where a primitive maps one; None
    /// or a primitive without a mapping for it: its own material.
    pub fn from_glb_variant(bytes: &[u8], variant: Option<u32>) -> Result<Self, String> {
        Self::from_glb_with(bytes, variant, &|_| true)
    }

    /// `from_glb_variant` with only the meshes of the nodes whose name `mesh_node` keeps (engine addition: an imported
    /// stage's scene without the models its code places, `fight_data::load_br1_stage`).
    pub fn from_glb_with(bytes: &[u8], variant: Option<u32>, mesh_node: &dyn Fn(&str) -> bool) -> Result<Self, String> {
        let g = gltf::Gltf::from_slice(bytes).map_err(|e| e.to_string())?;
        let blob = g.blob.as_deref().ok_or("glb without binary chunk")?;
        let buffers: Vec<&[u8]> = vec![blob];
        let get = |b: gltf::Buffer| Some(buffers[b.index()]);

        let mut nodes: Vec<Node> = g
            .nodes()
            .map(|n| {
                let (t, r, s) = n.transform().decomposed();
                Node { name: n.name().unwrap_or("").to_string(), parent: None, t: t.into(), r: Quat::from_array(r), s: s.into() }
            })
            .collect();
        for n in g.nodes() {
            for c in n.children() {
                nodes[c.index()].parent = Some(n.index());
            }
        }

        let mut images = Vec::new();
        for img in g.images() {
            let data = match img.source() {
                gltf::image::Source::View { view, .. } => {
                    let b = buffers[view.buffer().index()];
                    &b[view.offset()..view.offset() + view.length()]
                }
                gltf::image::Source::Uri { .. } => return Err("external image URIs are not supported".into()),
            };
            let im = image::load_from_memory_with_format(data, image::ImageFormat::Png).map_err(|e| e.to_string())?.to_rgba8();
            images.push(Image { name: img.name().unwrap_or("").to_string(), width: im.width(), height: im.height(), rgba: im.into_raw() });
        }
        let tex_of_image: Vec<usize> = g.textures().map(|t| t.source().index()).collect();

        let (joints, inverse_bind) = match g.skins().next() {
            Some(skin) => {
                let j: Vec<usize> = skin.joints().map(|n| n.index()).collect();
                let ib = skin
                    .reader(get)
                    .read_inverse_bind_matrices()
                    .map(|it| it.map(|m| Mat4::from_cols_array_2d(&m)).collect())
                    .unwrap_or_else(|| vec![Mat4::IDENTITY; j.len()]);
                (j, ib)
            }
            None => (vec![], vec![]),
        };

        let globals = rest_globals(&nodes);
        let mut vertices = Vec::new();
        let mut indices = Vec::new();
        let mut prims = Vec::new();
        for n in g.nodes() {
            let Some(mesh) = n.mesh() else { continue };
            if !mesh_node(n.name().unwrap_or("")) {
                continue;
            }
            let skinned = n.skin().is_some();
            let xf = if skinned { Mat4::IDENTITY } else { globals[n.index()] };
            let wall = n.name().and_then(|nm| nm.rsplit_once("_wall")).and_then(|(_, k)| k.parse::<u8>().ok());
            let hidden = HIDDEN_SUFFIXES.iter().any(|s| mesh.name().unwrap_or("").ends_with(s) || n.name().unwrap_or("").ends_with(s));
            let shadow = mesh.name().unwrap_or("").ends_with("_shadow_proxy") || n.name().unwrap_or("").ends_with("_shadow_proxy");
            let alt = mesh.name().unwrap_or("").ends_with(ALT_HANDS) || n.name().unwrap_or("").ends_with(ALT_HANDS);
            let over = n.name().unwrap_or("").ends_with("_semi_over");
            let ot = n.name().and_then(|nm| nm.rsplit_once("_ot_")).and_then(|(_, u)| match u {
                "floor" => Some(FLOOR),
                _ => u.strip_prefix("obj").and_then(|k| k.parse::<u8>().ok()),
            });
            for p in mesh.primitives() {
                let r = p.reader(get);
                let Some(pos) = r.read_positions() else { continue };
                let base = vertices.len() as u32;
                let pos: Vec<[f32; 3]> = pos.collect();
                let nrm: Vec<[f32; 3]> = r.read_normals().map(|i| i.collect()).unwrap_or_else(|| vec![[0.0, 1.0, 0.0]; pos.len()]);
                let uv: Vec<[f32; 2]> = r.read_tex_coords(0).map(|i| i.into_f32().collect()).unwrap_or_else(|| vec![[0.0; 2]; pos.len()]);
                let col: Vec<[f32; 4]> = r.read_colors(0).map(|i| i.into_rgba_f32().collect()).unwrap_or_else(|| vec![[1.0; 4]; pos.len()]);
                let jn: Vec<[u16; 4]> = r.read_joints(0).map(|i| i.into_u16().collect()).unwrap_or_else(|| vec![[0; 4]; pos.len()]);
                let wt: Vec<[f32; 4]> = r.read_weights(0).map(|i| i.into_f32().collect()).unwrap_or_else(|| vec![[1.0, 0.0, 0.0, 0.0]; pos.len()]);
                for i in 0..pos.len() {
                    let p = xf.transform_point3(Vec3::from(pos[i]));
                    let nn = xf.transform_vector3(Vec3::from(nrm[i])).normalize_or_zero();
                    vertices.push(Vertex {
                        pos: p.into(),
                        normal: nn.into(),
                        uv: uv[i],
                        color: col[i],
                        joints: if skinned {
                            let mut j = jn[i].map(|x| x as u32);
                            if alt {
                                j[0] |= JOINT_GROUP_ALT;
                            }
                            j
                        } else {
                            [0; 4]
                        },
                        weights: if skinned { wt[i] } else { [1.0, 0.0, 0.0, 0.0] },
                    });
                }
                let first = indices.len() as u32;
                match r.read_indices() {
                    Some(ix) => indices.extend(ix.into_u32().map(|i| i + base)),
                    None => indices.extend(base..base + pos.len() as u32),
                }
                let mat = variant.and_then(|v| p.mappings().find(|m| m.variants().contains(&v)).map(|m| m.material())).unwrap_or_else(|| p.material());
                let texture = mat.pbr_metallic_roughness().base_color_texture().map(|t| tex_of_image[t.texture().index()]);
                prims.push(Prim {
                    first_index: first,
                    count: indices.len() as u32 - first,
                    texture,
                    hidden,
                    wall,
                    shadow,
                    over,
                    ot,
                    semi: mat.name().is_some_and(|nm| nm.ends_with("_stp")),
                    node: n.index() as u32,
                    lit: !mat.unlit() && !p.attributes().any(|(s, _)| s == gltf::Semantic::Colors(0)),
                });
            }
        }

        let mut clips = HashMap::new();
        for a in g.animations() {
            let mut channels = Vec::new();
            let mut duration = 0.0f32;
            for ch in a.channels() {
                let r = ch.reader(get);
                let Some(times) = r.read_inputs() else { continue };
                let times: Vec<f32> = times.collect();
                duration = duration.max(*times.last().unwrap_or(&0.0));
                use gltf::animation::util::ReadOutputs;
                let track = match r.read_outputs() {
                    Some(ReadOutputs::Translations(v)) => Track::T(v.map(Vec3::from).collect()),
                    Some(ReadOutputs::Rotations(v)) => Track::R(v.into_f32().map(Quat::from_array).collect()),
                    Some(ReadOutputs::Scales(v)) => Track::S(v.map(Vec3::from).collect()),
                    _ => continue,
                };
                channels.push(Channel { node: ch.target().node().index(), times, track });
            }
            clips.insert(a.name().unwrap_or("").to_string(), Clip { duration, channels });
        }

        Ok(Self { vertices, indices, prims, images, nodes, joints, inverse_bind, clips })
    }

    /// The floor unit (`Prim::ot` = `FLOOR`, a stage in OT order): every tile (two triangles in a row,
    /// `ok_br2_converter::stages` / `model::add_floor`) gets vertices of its own whose normal is the tile's centre,
    /// for its CLUT row by depth (shader.wgsl `floor_row`; the floor is unlit, the normal is not used otherwise). Err
    /// (nothing changed) when a floor index is past the vertices (a malformed export).
    pub fn floor_tiles(&mut self) -> Result<(), String> {
        let floor = || self.prims.iter().filter(|p| p.ot == Some(FLOOR));
        let range = |p: &Prim| p.first_index as usize..p.first_index as usize + p.count as usize;
        let n = self.vertices.len();
        if floor().any(|p| self.indices.get(range(p)).is_none_or(|ix| ix.iter().any(|&i| i as usize >= n))) {
            return Err("floor: an index past the vertices".into());
        }
        for p in self.prims.iter().filter(|p| p.ot == Some(FLOOR)) {
            for tile in self.indices[range(p)].as_chunks_mut::<6>().0 {
                let mut corners: Vec<u32> = tile.to_vec();
                corners.sort_unstable();
                corners.dedup();
                let centre = corners.iter().map(|&i| Vec3::from(self.vertices[i as usize].pos)).sum::<Vec3>() / corners.len() as f32;
                for i in tile.iter_mut() {
                    let mut v = self.vertices[*i as usize];
                    v.normal = centre.into();
                    *i = self.vertices.len() as u32;
                    self.vertices.push(v);
                }
            }
        }
        Ok(())
    }

    /// The name of the glTF node prim `p` came from ("" past the nodes).
    pub fn prim_node(&self, p: &Prim) -> &str {
        self.nodes.get(p.node as usize).map_or("", |n| n.name.as_str())
    }

    /// A model of the prims of the nodes `keep` names (engine addition: one decode of an imported stage split into its
    /// parts): their vertices and indices compacted, the images all kept (drop the unused ones after), no nodes' pose or
    /// clips beyond the node list. Err when a kept prim's index is past the vertices or its range past the indices (a
    /// malformed export).
    pub fn extract(&self, keep: &dyn Fn(&str) -> bool) -> Result<ModelData, String> {
        let mut out = ModelData { images: self.images.clone(), nodes: self.nodes.clone(), ..ModelData::empty() };
        let mut map: HashMap<u32, u32> = HashMap::new();
        for p in self.prims.iter().filter(|p| keep(self.prim_node(p))) {
            let past = || format!("{}: a prim past the indices", self.prim_node(p));
            let end = (p.first_index as usize).checked_add(p.count as usize).ok_or_else(past)?;
            let ix = self.indices.get(p.first_index as usize..end).ok_or_else(past)?;
            let first = out.indices.len() as u32;
            for &i in ix {
                let j = match map.get(&i) {
                    Some(&j) => j,
                    None => {
                        let v = *self.vertices.get(i as usize).ok_or_else(|| format!("{}: an index past the vertices", self.prim_node(p)))?;
                        out.vertices.push(v);
                        map.insert(i, out.vertices.len() as u32 - 1);
                        out.vertices.len() as u32 - 1
                    }
                };
                out.indices.push(j);
            }
            out.prims.push(Prim { first_index: first, ..p.clone() });
        }
        Ok(out)
    }

    pub fn node_index(&self, name: &str) -> Option<usize> {
        self.nodes.iter().position(|n| n.name == name)
    }
}

/// Global transforms of the rest pose.
pub fn rest_globals(nodes: &[Node]) -> Vec<Mat4> {
    let locals: Vec<Mat4> = nodes.iter().map(|n| Mat4::from_scale_rotation_translation(n.s, n.r, n.t)).collect();
    globals_from_locals(nodes, &locals)
}

pub fn globals_from_locals(nodes: &[Node], locals: &[Mat4]) -> Vec<Mat4> {
    let mut out: Vec<Option<Mat4>> = vec![None; nodes.len()];
    fn resolve(i: usize, nodes: &[Node], locals: &[Mat4], out: &mut Vec<Option<Mat4>>) -> Mat4 {
        if let Some(m) = out[i] {
            return m;
        }
        let m = match nodes[i].parent {
            Some(p) => resolve(p, nodes, locals, out) * locals[i],
            None => locals[i],
        };
        out[i] = Some(m);
        m
    }
    (0..nodes.len()).map(|i| resolve(i, nodes, locals, &mut out)).collect()
}

pub(crate) fn sample_index(times: &[f32], t: f32) -> (usize, usize, f32) {
    if times.len() < 2 || t <= times[0] {
        return (0, 0, 0.0);
    }
    if t >= *times.last().unwrap() {
        let l = times.len() - 1;
        return (l, l, 0.0);
    }
    let k = times.partition_point(|&x| x <= t) - 1;
    let f = (t - times[k]) / (times[k + 1] - times[k]).max(1e-6);
    (k, k + 1, f)
}

/// Joint matrices for `clip` at time `t` seconds (rest pose when no clip).
/// `mirror` (state +0x100): left and right limbs exchange their rotations and the pose is reflected in X.
pub fn pose(model: &ModelData, clip: Option<&Clip>, t: f32, mirror: bool) -> Vec<Mat4> {
    pose_scaled(model, clip, t, mirror, &Shape::default())
}

/// MODEL TYPE shaping: per game node (skeleton part) scale of the node matrix (op 0x1D) and a
/// root height factor (node 1 translation y).
#[derive(Clone, Debug, PartialEq)]
pub struct Shape {
    pub scale: Vec<(usize, Vec3)>,
    pub root_y: f32,
    /// extra local X rotations (radians) of skeleton nodes, applied before the node's own rotation
    pub rot_x: Vec<(usize, f32)>,
    /// spring chains: skeleton node → direction (model space) its child joint must lie in, chain order
    pub aims: Vec<(usize, Vec3)>,
}

impl Default for Shape {
    fn default() -> Self {
        Self { scale: Vec::new(), root_y: 1.0, rot_x: Vec::new(), aims: Vec::new() }
    }
}

pub fn pose_scaled(model: &ModelData, clip: Option<&Clip>, t: f32, mirror: bool, shape: &Shape) -> Vec<Mat4> {
    let globals = pose_nodes(model, clip, t, mirror, shape);
    model.joints.iter().zip(&model.inverse_bind).map(|(&j, ib)| globals[j] * *ib).collect()
}

fn pose_nodes(model: &ModelData, clip: Option<&Clip>, t: f32, mirror: bool, shape: &Shape) -> Vec<Mat4> {
    let mut nodes = model.nodes.clone();
    if let Some(clip) = clip {
        for ch in &clip.channels {
            let (a, b, f) = sample_index(&ch.times, t);
            let n = &mut nodes[ch.node];
            match &ch.track {
                Track::T(v) => n.t = v[a].lerp(v[b], f),
                Track::R(v) => n.r = v[a].slerp(v[b], f),
                Track::S(v) => n.s = v[a].lerp(v[b], f),
            }
        }
    }
    if mirror {
        let sampled = nodes.clone();
        for (i, n) in nodes.iter_mut().enumerate() {
            let src = counterpart(model, i).unwrap_or(i);
            let s = &sampled[src];
            let rest_t = model.nodes[i].t;
            n.r = Quat::from_xyzw(s.r.x, -s.r.y, -s.r.z, s.r.w);
            // limb offsets stay the node's own; only the animated root translation is reflected
            n.t = if src == i { Vec3::new(-s.t.x, s.t.y, s.t.z) } else { rest_t };
        }
    }
    shaped(model, nodes, shape)
}

/// The pose from per-game-node local transforms (`pose::locals`, mirroring already in the data), shaped.
pub fn pose_locals_scaled(model: &ModelData, locals: &[(Option<Quat>, Option<Vec3>)], shape: &Shape) -> Vec<Mat4> {
    let mut nodes = model.nodes.clone();
    for (n, (r, t)) in locals.iter().enumerate() {
        let Some(&j) = model.joints.get(n) else { continue };
        if let Some(r) = r {
            nodes[j].r = *r;
        }
        if let Some(t) = t {
            nodes[j].t = *t;
        }
    }
    let globals = shaped(model, nodes, shape);
    model.joints.iter().zip(&model.inverse_bind).map(|(&j, ib)| globals[j] * *ib).collect()
}

/// Global transforms (model space) of all nodes for `pose_locals_scaled`'s pose.
pub fn pose_locals_globals(model: &ModelData, locals: &[(Option<Quat>, Option<Vec3>)], shape: &Shape) -> Vec<Mat4> {
    let joints = pose_locals_scaled(model, locals, shape);
    model.joints.iter().zip(&model.inverse_bind).zip(&joints).fold(vec![Mat4::IDENTITY; model.nodes.len()], |mut g, ((&j, ib), m)| {
        g[j] = *m * ib.inverse();
        g
    })
}

/// MODEL TYPE shaping and the spring aims on the sampled local transforms, then the globals.
fn shaped(model: &ModelData, mut nodes: Vec<Node>, shape: &Shape) -> Vec<Mat4> {
    for (part, s) in &shape.scale {
        if let Some(&j) = model.joints.get(*part) {
            nodes[j].s *= *s;
        }
    }
    for (part, a) in &shape.rot_x {
        if let Some(&j) = model.joints.get(*part) {
            nodes[j].r = Quat::from_rotation_x(*a) * nodes[j].r;
        }
    }
    if shape.root_y != 1.0
        && let Some(&j) = model.joints.get(1)
    {
        nodes[j].t.y *= shape.root_y;
    }
    // spring chains: turn each node so that its child joint lies along the aim (root to tip, globals refreshed)
    for (part, dir) in &shape.aims {
        let Some(&j) = model.joints.get(*part) else { continue };
        let Some(c) = nodes.iter().position(|n| n.parent == Some(j)) else { continue };
        let g = rest_globals(&nodes);
        let cur = (g[c].w_axis - g[j].w_axis).truncate();
        if cur.length_squared() < 1e-12 || dir.length_squared() < 1e-12 {
            continue;
        }
        let delta = Quat::from_rotation_arc(cur.normalize(), dir.normalize());
        let pr = nodes[j].parent.map(|p| Quat::from_mat4(&g[p]).normalize()).unwrap_or(Quat::IDENTITY);
        nodes[j].r = (pr.inverse() * delta * pr * nodes[j].r).normalize();
    }
    rest_globals(&nodes)
}

/// Global transforms of all nodes for the same pose as `pose` (model space).
pub fn pose_globals(model: &ModelData, clip: Option<&Clip>, t: f32, mirror: bool) -> Vec<Mat4> {
    pose_globals_scaled(model, clip, t, mirror, &Shape::default())
}

pub fn pose_globals_scaled(model: &ModelData, clip: Option<&Clip>, t: f32, mirror: bool, shape: &Shape) -> Vec<Mat4> {
    let joints = pose_scaled(model, clip, t, mirror, shape);
    model.joints.iter().zip(&model.inverse_bind).zip(&joints).fold(vec![Mat4::IDENTITY; model.nodes.len()], |mut g, ((&j, ib), m)| {
        g[j] = *m * ib.inverse();
        g
    })
}

/// Node of the opposite side ("thigh.L" ↔ "thigh.R").
pub(crate) fn counterpart(model: &ModelData, i: usize) -> Option<usize> {
    let name = &model.nodes[i].name;
    let other = if let Some(b) = name.strip_suffix(".L") {
        format!("{b}.R")
    } else {
        let b = name.strip_suffix(".R")?;
        format!("{b}.L")
    };
    model.node_index(&other)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn floor(vertices: usize, indices: Vec<u32>) -> ModelData {
        let v = |k: usize| Vertex {
            pos: [k as f32, 0.0, 0.0],
            normal: [0.0, 1.0, 0.0],
            uv: [0.0; 2],
            color: [1.0; 4],
            joints: [0; 4],
            weights: [1.0, 0.0, 0.0, 0.0],
        };
        let prim = Prim {
            first_index: 0,
            count: indices.len() as u32,
            texture: None,
            hidden: false,
            lit: false,
            wall: None,
            shadow: false,
            over: false,
            ot: Some(FLOOR),
            semi: false,
            node: 0,
        };
        ModelData {
            vertices: (0..vertices).map(v).collect(),
            indices,
            prims: vec![prim],
            images: Vec::new(),
            nodes: Vec::new(),
            joints: Vec::new(),
            inverse_bind: Vec::new(),
            clips: HashMap::new(),
        }
    }

    #[test]
    fn floor_tiles_own_vertices_with_the_centre() {
        let mut m = floor(4, vec![0, 1, 2, 2, 1, 3]);
        m.floor_tiles().unwrap();
        assert_eq!((m.vertices.len(), m.indices.as_slice()), (10, &[4, 5, 6, 7, 8, 9][..]));
        assert!(m.vertices[4..].iter().all(|v| v.normal == [1.5, 0.0, 0.0]));
    }

    #[test]
    fn floor_tiles_index_past_the_vertices() {
        let mut m = floor(4, vec![0, 1, 2, 2, 1, 4]);
        assert!(m.floor_tiles().is_err());
        assert_eq!((m.vertices.len(), m.indices.as_slice()), (4, &[0, 1, 2, 2, 1, 4][..]));
        // a prim past the indices
        m.prims[0].count = 7;
        assert!(m.floor_tiles().is_err());
    }

    #[test]
    fn extract_keeps_the_named_nodes() {
        let node = |name: &str| Node { name: name.into(), parent: None, t: Vec3::ZERO, r: Quat::IDENTITY, s: Vec3::ONE };
        let mut m = floor(6, vec![0, 1, 2, 3, 4, 5, 5, 4, 1]);
        m.nodes = vec![node("a"), node("b")];
        m.prims[0].count = 3;
        m.prims.push(Prim { first_index: 3, count: 6, node: 1, ..m.prims[0].clone() });
        let b = m.extract(&|n| n == "b").unwrap();
        assert_eq!((b.prims.len(), b.prims[0].first_index, b.prims[0].count, b.prims[0].node), (1, 0, 6, 1));
        assert_eq!(b.indices, [0, 1, 2, 2, 1, 3]);
        assert_eq!(b.vertices.iter().map(|v| v.pos[0]).collect::<Vec<_>>(), [3.0, 4.0, 5.0, 1.0]);
        // an index past the vertices, a prim past the indices
        m.indices[8] = 6;
        assert!(m.extract(&|n| n == "b").is_err());
        assert!(m.extract(&|n| n == "a").is_ok());
        m.prims[0].count = 10;
        assert!(m.extract(&|n| n == "a").is_err());
    }
}
