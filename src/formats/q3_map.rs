//! A Quake III map as ok_q3_converter exports it (glTF + its shader table): the world's surfaces and how each is
//! drawn.
//!
//! Engine addition: the extra stage Q3DM17, `fight::stage::extra`
//! State: static data (not serialised)
//!
//! ok_q3_converter writes a map as glTF (Y up, 1 Quake unit = 0.0254 m): the world in the node `worldspawn` (the items
//! placed at their spawn points are other nodes, not read here), the BSP lightmap atlas on `TEXCOORD_1` (bound as the
//! material's occlusion texture), vertex colours, one material per Quake shader (its name), the images by name
//! relative to the file. The shader table (`shaders.json`: name → params, stages of directives) says how a shader
//! draws. `parse` reduces each shader to what the engine's lightmapped pipeline draws (`Look`): one base texture, times
//! the lightmap or the vertex colour, plus one additive glow texture, opaque / alpha-tested / blended / added, culled
//! or two-sided, a constant texture scroll. Not reproduced (listed in docs/extra_stage_q3dm17.md): environment
//! mapping, animated colours (`rgbGen wave`), animated maps, `tcMod` other than `scroll`, vertex deforms
//! (autosprite surfaces are left out), fog, and stages beyond the base, the lightmap and one glow.

/// A vertex in the file's space (glTF metres, Y up).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Vertex {
    pub pos: [f32; 3],
    pub uv: [f32; 2],
    /// the lightmap atlas position (`TEXCOORD_1`)
    pub lm: [f32; 2],
    pub color: [f32; 4],
}

/// How a surface is put over what is behind it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Blend {
    Opaque,
    /// texels with alpha under ½ are not drawn (`alphaFunc`)
    Mask,
    /// alpha blended, no depth write
    Alpha,
    /// added, no depth write
    Add,
}

/// What the engine draws of a shader.
#[derive(Clone, Debug, PartialEq)]
pub struct Look {
    /// the base texture: a path under the map's root folder (`baseq3/textures/….png`)
    pub base: String,
    /// an additive glow texture over the lit base (a light's `.blend` / `.glow` stage)
    pub glow: Option<String>,
    /// lit by the lightmap; else by the vertex colours (`rgbGen vertex`, the implicit shader of a surface without
    /// one) or not at all (`rgbGen identity`)
    pub lightmap: bool,
    pub vertex_colour: bool,
    pub blend: Blend,
    /// back faces culled (`cull none` / `disable` / `twosided`: drawn both ways)
    pub cull: bool,
    /// the base texture's `tcMod scroll` (texture units per second)
    pub scroll: [f32; 2],
}

/// A run of triangles drawn with one look.
#[derive(Clone, Debug)]
pub struct Surface {
    pub first: u32,
    pub count: u32,
    pub look: Look,
}

/// The map's world.
#[derive(Clone, Debug, Default)]
pub struct Map {
    pub vertices: Vec<Vertex>,
    pub indices: Vec<u32>,
    pub surfaces: Vec<Surface>,
    /// the lightmap atlas (a path under the map's root folder), None without one
    pub lightmap: Option<String>,
    /// the shaders left out (sky, autosprites, nodraw): (name, why)
    pub skipped: Vec<(String, &'static str)>,
    /// the sky shader of the map's sky surfaces (the first one), None without one
    pub sky: Option<Sky>,
}

/// A sky shader (`skyParms farbox cloudheight -` and its stages): the far box drawn first, then the cloud layers over
/// it (a sphere round the eye, its top at the cloud height: `render::lightmapped`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Sky {
    /// the far box's six faces: their path under the map's root folder without `_<face>.png` (`baseq3/env/space1`),
    /// None for `-`
    pub farbox: Option<String>,
    /// the cloud layer's height (`skyParms`' second word; 512 for `-` or 0, as Quake III's)
    pub cloud_height: f32,
    /// the cloud layers, in order (the shader's texture stages)
    pub clouds: Vec<CloudLayer>,
}

/// One cloud stage: its texture (a path under the map's root folder), its coordinates' `tcMod scale` and `scroll`
/// folded in their order into st · `scale` + t · `scroll`, and whether it is added (`blendFunc add`) or replaces
/// what is there.
#[derive(Clone, Debug, PartialEq)]
pub struct CloudLayer {
    pub texture: String,
    pub scale: [f32; 2],
    pub scroll: [f32; 2],
    pub add: bool,
}

impl Map {
    /// Every texture the surfaces draw (base and glow), each once, in first use order.
    pub fn textures(&self) -> Vec<&str> {
        let mut v: Vec<&str> = Vec::new();
        for s in &self.surfaces {
            for t in std::iter::once(s.look.base.as_str()).chain(s.look.glow.as_deref()) {
                if !v.contains(&t) {
                    v.push(t);
                }
            }
        }
        v
    }
}

/// `rel` (a file's name relative to the file at `from`, both under the map's root folder) as a path under that
/// folder: `.` and `..` resolved; None when it leaves the folder or is absolute.
pub fn resolve(from: &str, rel: &str) -> Option<String> {
    if rel.starts_with('/') || rel.contains(':') || rel.contains('\\') {
        return None;
    }
    let mut parts: Vec<&str> = from.split('/').collect();
    parts.pop();
    for p in rel.split('/') {
        match p {
            "" | "." => {}
            ".." => {
                parts.pop()?;
            }
            p => parts.push(p),
        }
    }
    Some(parts.join("/"))
}

/// A shader table texture name (`textures/x/y.tga`) as the converter wrote it: `<game>/textures/x/y.png`; None when
/// it leaves the map's root folder or is absolute (`resolve`).
fn shader_texture(game: &str, name: &str) -> Option<String> {
    // the extension of the last part only (`a.b/c` has none)
    let stem = name.rsplit_once('.').filter(|(_, ext)| !ext.contains('/')).map_or(name, |(s, _)| s);
    resolve(&format!("{game}/"), &format!("{stem}.png"))
}

/// One stage of a shader: its directives (lower case, as the converter writes them).
type Stage = Vec<Vec<String>>;

fn directive<'a>(st: &'a Stage, name: &str) -> Option<&'a [String]> {
    st.iter().find(|d| d.first().is_some_and(|k| k == name)).map(|d| &d[1..])
}

/// The stage's texture (`map` / `clampMap`, the first frame of `animMap`), `$lightmap` included.
fn stage_map(st: &Stage) -> Option<&str> {
    directive(st, "map")
        .or(directive(st, "clampmap"))
        .and_then(|a| a.first())
        .map(String::as_str)
        .or_else(|| directive(st, "animmap").and_then(|a| a.get(1)).map(String::as_str))
}

/// A shader word as a finite number (`inf`, `nan` and words that are not numbers: None).
fn number(w: Option<&String>) -> Option<f32> {
    w.and_then(|x| x.parse::<f32>().ok()).filter(|x| x.is_finite())
}

/// The stage's blend: None = replaces what is there (no `blendFunc`, or `GL_ONE GL_ZERO`).
fn stage_blend(st: &Stage) -> Option<Blend> {
    let a = directive(st, "blendfunc")?;
    let a: Vec<&str> = a.iter().map(String::as_str).collect();
    match a.as_slice() {
        ["add"] | ["gl_one", "gl_one"] => Some(Blend::Add),
        ["blend"] | ["gl_src_alpha", "gl_one_minus_src_alpha"] => Some(Blend::Alpha),
        ["gl_one", "gl_zero"] => None,
        // filter (gl_dst_color gl_zero …): a multiply, the lightmap's way
        _ => Some(Blend::Opaque),
    }
}

/// What the engine draws of shader `name` (`shaders`: the converter's table; `game`: the folder its textures are
/// under, `baseq3`), given the glTF material's texture `glb_base` and lightmap flag; None: not drawn (with why).
pub fn look(name: &str, shaders: &serde_json::Value, game: &str, glb_base: Option<&str>, glb_lightmap: bool) -> Result<Look, &'static str> {
    let implicit = |base: Option<String>| -> Result<Look, &'static str> {
        let base = base.ok_or("no texture in the map's folder")?;
        Ok(Look { base, glow: None, lightmap: glb_lightmap, vertex_colour: !glb_lightmap, blend: Blend::Opaque, cull: true, scroll: [0.0; 2] })
    };
    let Some(sh) = shaders.get(name) else {
        // no script: the texture of the same name, times the lightmap (or the vertex colours)
        return implicit(glb_base.map(str::to_string).or_else(|| shader_texture(game, name)));
    };
    let words = |v: &serde_json::Value| -> Vec<String> { v.as_array().into_iter().flatten().filter_map(|w| w.as_str()).map(str::to_lowercase).collect() };
    let params: Vec<Vec<String>> = sh["params"].as_array().into_iter().flatten().map(words).collect();
    let param = |k: &str, v: &str| params.iter().any(|p| p.first().is_some_and(|x| x == k) && p.get(1).is_some_and(|x| x == v));
    if param("surfaceparm", "sky") {
        return Err("sky (drawn as the sky box)");
    }
    if param("surfaceparm", "nodraw") {
        return Err("nodraw");
    }
    if params.iter().any(|p| p.first().is_some_and(|x| x == "deformvertexes") && p.get(1).is_some_and(|x| x.starts_with("autosprite"))) {
        return Err("autosprite (not reproduced)");
    }
    let stages: Vec<Stage> = sh["stages"].as_array().into_iter().flatten().map(|s| s.as_array().into_iter().flatten().map(words).collect()).collect();
    if stages.is_empty() {
        return implicit(glb_base.map(str::to_string).or_else(|| shader_texture(game, name)));
    }
    let cull =
        !params.iter().any(|p| p.first().is_some_and(|x| x == "cull") && p.get(1).is_some_and(|x| matches!(x.as_str(), "none" | "disable" | "twosided")));
    let is_lightmap = |st: &Stage| stage_map(st) == Some("$lightmap");
    let is_env = |st: &Stage| directive(st, "tcgen").is_some_and(|a| a.first().is_some_and(|x| x == "environment"));
    let animated = |st: &Stage| directive(st, "tcmod").is_some() || directive(st, "animmap").is_some();
    let textured = |st: &Stage| stage_map(st).is_some_and(|m| m != "$lightmap" && !m.starts_with('$')) && !is_env(st);
    // the base: the first still texture stage, else the first texture stage (an environment map underneath is left
    // out: the base drawn over it stands for the surface)
    let Some(bi) = stages.iter().position(|s| textured(s) && !animated(s)).or_else(|| stages.iter().position(textured)) else {
        return Err("no texture stage");
    };
    let base_stage = &stages[bi];
    let lightmap = stages.iter().any(is_lightmap);
    // the surface's blend is its first drawn stage's: the base's when it comes first, else opaque (a stage under it
    // fills the surface)
    let first = stages.iter().position(|s| textured(s) || is_lightmap(s) || is_env(s)).unwrap_or(bi);
    let mut blend = if first == bi { stage_blend(base_stage).unwrap_or(Blend::Opaque) } else { Blend::Opaque };
    if blend == Blend::Opaque && directive(base_stage, "alphafunc").is_some() {
        blend = Blend::Mask;
    }
    let rgbgen = directive(base_stage, "rgbgen").and_then(|a| a.first()).map(String::as_str);
    let vertex_colour = !lightmap && matches!(rgbgen, Some("vertex" | "exactvertex"));
    // the glow: an added still texture stage after the base and the lightmap (a light's halo)
    let glow = (lightmap && matches!(blend, Blend::Opaque | Blend::Mask))
        .then(|| stages.iter().enumerate().skip(bi + 1).find(|(_, s)| textured(s) && !animated(s) && stage_blend(s) == Some(Blend::Add)).map(|(_, s)| s))
        .flatten()
        .and_then(stage_map)
        .and_then(|m| shader_texture(game, m));
    let scroll = directive(base_stage, "tcmod")
        .filter(|a| a.first().is_some_and(|x| x == "scroll"))
        .map(|a| [1, 2].map(|k| number(a.get(k)).unwrap_or(0.0)))
        .unwrap_or([0.0; 2]);
    let base = shader_texture(game, stage_map(base_stage).unwrap_or(name)).ok_or("a texture outside the map's folder")?;
    Ok(Look { base, glow, lightmap, vertex_colour, blend, cull, scroll })
}

/// The sky shader `name` of `shaders` (`game`: the folder its textures are under); None when it is not a sky
/// (`surfaceparm sky`) or not in the table. Quake III's defaults: no far box, clouds at 512; a stage whose texture
/// lies outside the folder is left out, as are `tcMod`s other than `scale` and `scroll` (the sky's `turb` /
/// `rotate`; not in q3dm6).
pub fn sky(name: &str, shaders: &serde_json::Value, game: &str) -> Option<Sky> {
    let sh = shaders.get(name)?;
    let words = |v: &serde_json::Value| -> Vec<String> { v.as_array().into_iter().flatten().filter_map(|w| w.as_str()).map(str::to_lowercase).collect() };
    let params: Vec<Vec<String>> = sh["params"].as_array().into_iter().flatten().map(words).collect();
    if !params.iter().any(|p| p.first().is_some_and(|x| x == "surfaceparm") && p.get(1).is_some_and(|x| x == "sky")) {
        return None;
    }
    let mut out = Sky { farbox: None, cloud_height: 512.0, clouds: Vec::new() };
    if let Some(p) = params.iter().find(|p| p.first().is_some_and(|x| x == "skyparms")) {
        out.farbox = p.get(1).filter(|b| *b != "-").and_then(|b| resolve(&format!("{game}/"), b));
        out.cloud_height = number(p.get(2)).filter(|&h| h > 0.0).unwrap_or(512.0);
    }
    for st in sh["stages"].as_array().into_iter().flatten() {
        let st: Stage = st.as_array().into_iter().flatten().map(words).collect();
        let Some(texture) = stage_map(&st).filter(|m| !m.starts_with('$')).and_then(|m| shader_texture(game, m)) else { continue };
        let (mut scale, mut scroll) = ([1.0f32; 2], [0.0f32; 2]);
        for d in st.iter().filter(|d| d.first().is_some_and(|k| k == "tcmod")) {
            let v = |k: usize| number(d.get(k)).unwrap_or(0.0);
            match d.get(1).map(String::as_str) {
                Some("scale") => {
                    let m = [v(2), v(3)];
                    scale = [scale[0] * m[0], scale[1] * m[1]];
                    scroll = [scroll[0] * m[0], scroll[1] * m[1]];
                }
                Some("scroll") => scroll = [scroll[0] + v(2), scroll[1] + v(3)],
                _ => {}
            }
        }
        // a product or sum past f32's range (e.g. two `scale 1e30`) counts 0 as a number that is not finite does
        let finite = |v: [f32; 2]| v.map(|x| if x.is_finite() { x } else { 0.0 });
        let (scale, scroll) = (finite(scale), finite(scroll));
        out.clouds.push(CloudLayer { texture, scale, scroll, add: stage_blend(&st) == Some(Blend::Add) });
    }
    Some(out)
}

/// The world of an ok_q3_converter map (`glb`, at `path` under the map's root folder, e.g. `baseq3/maps/q3dm17.glb`)
/// with its shader table: the node `worldspawn`'s triangles grouped by look, in the file's order (the blended ones
/// after the rest is up to the caller).
pub fn parse(glb: &[u8], path: &str, shaders: &serde_json::Value) -> Result<Map, String> {
    // a glb header's length under the header's 12 bytes: the gltf crate subtracts them unchecked
    if glb.starts_with(b"glTF") && glb.get(8..12).is_none_or(|n| u32::from_le_bytes([n[0], n[1], n[2], n[3]]) < 12) {
        return Err("glb header too short".into());
    }
    let g = gltf::Gltf::from_slice(glb).map_err(|e| e.to_string())?;
    let blob = g.blob.as_deref().ok_or("glb without binary chunk")?;
    let get = |b: gltf::Buffer| (b.index() == 0).then_some(blob);
    let game = path.split('/').next().unwrap_or("baseq3");
    let image_path = |img: gltf::Image| match img.source() {
        gltf::image::Source::Uri { uri, .. } => resolve(path, uri),
        gltf::image::Source::View { .. } => None,
    };
    let world = g.nodes().find(|n| n.name() == Some("worldspawn")).ok_or("no worldspawn node")?;
    let mesh = world.mesh().ok_or("worldspawn without a mesh")?;
    let mut map = Map::default();
    for p in mesh.primitives() {
        if p.mode() != gltf::mesh::Mode::Triangles {
            continue;
        }
        let mat = p.material();
        let name = mat.name().unwrap_or("");
        let glb_base = mat.pbr_metallic_roughness().base_color_texture().and_then(|t| image_path(t.texture().source()));
        let lm_tex = mat.occlusion_texture().filter(|t| t.tex_coord() == 1);
        if map.lightmap.is_none() {
            map.lightmap = lm_tex.as_ref().and_then(|t| image_path(t.texture().source()));
        }
        let look = match look(name, shaders, game, glb_base.as_deref(), lm_tex.is_some()) {
            Ok(l) => l,
            Err(why) => {
                if !map.skipped.iter().any(|(n, _)| n == name) {
                    map.skipped.push((name.to_string(), why));
                }
                if map.sky.is_none() {
                    map.sky = sky(name, shaders, game);
                }
                continue;
            }
        };
        let r = p.reader(get);
        let Some(pos) = r.read_positions() else { continue };
        let pos: Vec<[f32; 3]> = pos.collect();
        let n = pos.len();
        let uv: Vec<[f32; 2]> = r.read_tex_coords(0).map(|i| i.into_f32().collect()).unwrap_or_else(|| vec![[0.0; 2]; n]);
        let lm: Vec<[f32; 2]> = r.read_tex_coords(1).map(|i| i.into_f32().collect()).unwrap_or_else(|| vec![[0.0; 2]; n]);
        let col: Vec<[f32; 4]> = r.read_colors(0).map(|i| i.into_rgba_f32().collect()).unwrap_or_else(|| vec![[1.0; 4]; n]);
        if uv.len() != n || lm.len() != n || col.len() != n {
            return Err(format!("{name}: attribute counts differ"));
        }
        let base = map.vertices.len() as u32;
        map.vertices.extend((0..n).map(|i| Vertex { pos: pos[i], uv: uv[i], lm: lm[i], color: col[i] }));
        let first = map.indices.len() as u32;
        match r.read_indices() {
            Some(ix) => {
                let ix: Vec<u32> = ix.into_u32().collect();
                if ix.iter().any(|&i| i as usize >= n) {
                    return Err(format!("{name}: an index past the vertices"));
                }
                map.indices.extend(ix.iter().map(|i| i + base));
            }
            None => map.indices.extend(base..base + n as u32),
        }
        let count = map.indices.len() as u32 - first;
        map.surfaces.push(Surface { first, count: count - count % 3, look });
    }
    if map.indices.is_empty() {
        return Err("no surface drawn".into());
    }
    Ok(map)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A glb header claiming fewer bytes than itself: Err (found by the fuzz target `q3_map_parse`).
    #[test]
    fn short_glb_header() {
        let mut b = b"glTF\x02\0\0\0\x0b\0\0\0".to_vec();
        assert!(parse(&b, "baseq3/maps/q3dm17.glb", &serde_json::Value::Null).is_err());
        b.truncate(10);
        assert!(parse(&b, "baseq3/maps/q3dm17.glb", &serde_json::Value::Null).is_err());
    }

    /// q3dm6's sky (`textures/skies/xtoxicskytim_q3dm5`): no far box, clouds at 512, two layers: the base scaled
    /// 3 × 2 then scrolled 0.15 / 0.15, the added one scaled 3 × 3 then scrolled 0.05 / 0.05; a scroll before a scale is
    /// scaled with it; not a sky: None.
    #[test]
    fn sky_shader() {
        let shaders = serde_json::json!({
            "textures/skies/xtoxicskytim_q3dm5": {
                "params": [["surfaceparm", "sky"], ["skyparms", "-", "512", "-"]],
                "stages": [
                    [["map", "textures/skies/bluedimclouds.tga"], ["tcmod", "scale", "3", "2"], ["tcmod", "scroll", "0.15", "0.15"], ["depthwrite"]],
                    [["map", "textures/skies/topclouds.tga"], ["blendfunc", "gl_one", "gl_one"], ["tcmod", "scale", "3", "3"], ["tcmod", "scroll", "0.05", "0.05"]]
                ]
            },
            "textures/skies/x": { "params": [["surfaceparm", "sky"], ["skyParms", "env/space1", "-", "-"]], "stages": [[["map", "a.tga"], ["tcMod", "scroll", "1", "2"], ["tcMod", "scale", "2", "4"]]] },
            "textures/a/b": { "params": [], "stages": [] }
        });
        let s = sky("textures/skies/xtoxicskytim_q3dm5", &shaders, "baseq3").expect("a sky");
        assert_eq!((s.farbox.as_deref(), s.cloud_height), (None, 512.0));
        let layer = |t: &str, scale, scroll, add| CloudLayer { texture: format!("baseq3/textures/skies/{t}.png"), scale, scroll, add };
        assert_eq!(s.clouds, [layer("bluedimclouds", [3.0, 2.0], [0.15, 0.15], false), layer("topclouds", [3.0, 3.0], [0.05, 0.05], true)]);
        let x = sky("textures/skies/x", &shaders, "baseq3").expect("a sky");
        assert_eq!((x.farbox.as_deref(), x.cloud_height), (Some("baseq3/env/space1"), 512.0));
        assert_eq!((x.clouds[0].scale, x.clouds[0].scroll), ([2.0, 4.0], [2.0, 8.0]));
        assert_eq!(sky("textures/a/b", &shaders, "baseq3"), None);
        // numbers that are not finite: the defaults (clouds at 512, no scale or scroll)
        let odd = serde_json::json!({ "s": { "params": [["surfaceparm", "sky"], ["skyparms", "-", "inf", "-"]],
            "stages": [[["map", "a.tga"], ["tcmod", "scale", "nan", "2"], ["tcmod", "scroll", "1e40", "-inf"]]] } });
        let o = sky("s", &odd, "baseq3").expect("a sky");
        assert_eq!((o.cloud_height, o.clouds[0].scale, o.clouds[0].scroll), (512.0, [0.0, 2.0], [0.0, 0.0]));
        // finite numbers whose product or sum is not: 0 as well
        let big = serde_json::json!({ "s": { "params": [["surfaceparm", "sky"]], "stages": [[["map", "a.tga"], ["tcmod", "scroll", "3e38", "1"],
            ["tcmod", "scroll", "3e38", "1"], ["tcmod", "scale", "1e30", "2"], ["tcmod", "scale", "1e30", "2"]]] } });
        let b = sky("s", &big, "baseq3").expect("a sky");
        assert_eq!((b.clouds[0].scale, b.clouds[0].scroll), ([0.0, 4.0], [0.0, 8.0]));
        assert_eq!(sky("textures/none", &shaders, "baseq3"), None);
    }

    #[test]
    fn resolve_paths() {
        assert_eq!(resolve("baseq3/maps/q3dm17.glb", "../../baseq3/textures/a/b.png").as_deref(), Some("baseq3/textures/a/b.png"));
        assert_eq!(resolve("baseq3/maps/q3dm17.glb", "q3dm17_lightmap.png").as_deref(), Some("baseq3/maps/q3dm17_lightmap.png"));
        assert_eq!(resolve("baseq3/maps/q3dm17.glb", "../../../x.png"), None);
        assert_eq!(resolve("baseq3/maps/q3dm17.glb", "/etc/passwd"), None);
        assert_eq!(resolve("baseq3/maps/q3dm17.glb", "c:/x.png"), None);
    }

    /// The shaders of q3dm17 as the converter's table has them (abridged), each reduced to its look.
    #[test]
    fn looks() {
        let t: serde_json::Value = serde_json::json!({
            "textures/base_trim/pewter_shiney": {"params": [], "stages": [
                [["map", "textures/effects/tinfx.tga"], ["tcgen", "environment"], ["rgbgen", "identity"]],
                [["map", "textures/base_trim/pewter_shiney.tga"], ["blendfunc", "gl_src_alpha", "gl_one_minus_src_alpha"], ["rgbgen", "identity"]],
                [["map", "$lightmap"], ["blendfunc", "gl_dst_color", "gl_one_minus_dst_alpha"], ["rgbgen", "identity"]]]},
            "textures/base_light/baslt4_1_4k": {"params": [["surfaceparm", "nomarks"]], "stages": [
                [["map", "$lightmap"], ["rgbgen", "identity"]],
                [["map", "textures/base_light/baslt4_1.tga"], ["blendfunc", "gl_dst_color", "gl_zero"], ["rgbgen", "identity"]],
                [["map", "textures/base_light/baslt4_1.blend.tga"], ["blendfunc", "gl_one", "gl_one"]]]},
            "textures/skies/blacksky": {"params": [["surfaceparm", "sky"]], "stages": [[["map", "gfx/colors/black.tga"]]]},
            "textures/sfx/beam": {"params": [["cull", "none"]], "stages": [[["map", "textures/sfx/beam.tga"], ["tcmod", "scroll", ".3", "0"], ["blendfunc", "add"]]]},
            "models/mapobjects/lamps/flare03": {"params": [["deformvertexes", "autosprite"]], "stages": [[["map", "models/mapobjects/lamps/flare03.tga"]]]},
            "models/mapobjects/spotlamp/spotlamp": {"params": [["cull", "disable"]], "stages": [
                [["map", "models/mapobjects/spotlamp/spotlamp.tga"], ["alphafunc", "ge128"], ["depthwrite"], ["rgbgen", "vertex"]]]}
        });
        let l = look("textures/base_trim/pewter_shiney", &t, "baseq3", Some("baseq3/textures/effects/tinfx.png"), true).unwrap();
        assert_eq!((l.base.as_str(), l.lightmap, l.blend, l.glow), ("baseq3/textures/base_trim/pewter_shiney.png", true, Blend::Opaque, None));
        let l = look("textures/base_light/baslt4_1_4k", &t, "baseq3", None, true).unwrap();
        assert_eq!(
            (l.base.as_str(), l.blend, l.glow.as_deref()),
            ("baseq3/textures/base_light/baslt4_1.png", Blend::Opaque, Some("baseq3/textures/base_light/baslt4_1.blend.png"))
        );
        assert!(look("textures/skies/blacksky", &t, "baseq3", None, false).is_err());
        assert!(look("models/mapobjects/lamps/flare03", &t, "baseq3", None, false).is_err());
        let l = look("textures/sfx/beam", &t, "baseq3", None, false).unwrap();
        assert_eq!((l.blend, l.cull, l.scroll, l.lightmap, l.vertex_colour), (Blend::Add, false, [0.3, 0.0], false, false));
        let l = look("models/mapobjects/spotlamp/spotlamp", &t, "baseq3", None, false).unwrap();
        assert_eq!((l.blend, l.cull, l.vertex_colour), (Blend::Mask, false, true));
        // no script: the glTF material's texture, times the lightmap (or the vertex colours without one)
        let l = look("textures/base_wall/c_met5_2", &t, "baseq3", Some("baseq3/textures/base_wall/c_met5_2.png"), true).unwrap();
        assert_eq!((l.base.as_str(), l.lightmap, l.vertex_colour, l.blend), ("baseq3/textures/base_wall/c_met5_2.png", true, false, Blend::Opaque));
        let l = look("models/mapobjects/teleporter/pad", &t, "baseq3", None, false).unwrap();
        assert_eq!((l.base.as_str(), l.lightmap, l.vertex_colour), ("baseq3/models/mapobjects/teleporter/pad.png", false, true));
        // a texture name leaving the map's folder or absolute: not drawn (a script's map, or the name without one)
        let bad = serde_json::json!({
            "a": {"params": [], "stages": [[["map", "../../../../Library/x.tga"]]]},
            "b": {"params": [], "stages": [[["clampmap", "/etc/x.tga"]]]},
            "c": {"params": [], "stages": [[["map", "textures/x.tga"]], [["map", "c:/x.tga"], ["blendfunc", "add"]], [["map", "$lightmap"]]]}
        });
        assert!(look("a", &bad, "baseq3", None, false).is_err() && look("b", &bad, "baseq3", None, false).is_err());
        assert!(look("../../x", &bad, "baseq3", None, false).is_err());
        assert_eq!(look("c", &bad, "baseq3", None, true).map(|l| (l.base, l.glow)), Ok(("baseq3/textures/x.png".into(), None)));
    }
}
