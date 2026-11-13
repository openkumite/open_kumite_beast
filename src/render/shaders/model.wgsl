struct Inst {
    view_proj: mat4x4<f32>,
    model: mat4x4<f32>,
    params: vec4<f32>,          // x: lit (1) / unlit (0), y: encode sRGB in the shader (non-sRGB surface), z: subtract,
                                // w: palette subtract level 0..255 (a fighter's side fade)
    light: array<vec4<f32>, 3>, // lit: the light matrix, rows = lights 1..3 (world directions, 1.0 = 0x1000);
                                // the shadow pass: light[0] = shear (xy) and floor height (w)
    colour: array<vec4<f32>, 3>,// lit: the colour matrix, rows R, G, B, columns lights 1..3 (1.0 = 0x1000)
    back: vec4<f32>,            // lit: the back (ambient) colour (1.0 = 0x1000)
    floor: vec4<f32>,           // floor tiles: the view-space depth SZ (game units) of a point p as dot(xyz, p) + w
    hide: vec4<u32>,            // hidden-joint masks: x main mesh, y open hands (vertex group = bit 8 of j.x);
                                // z: floor CLUT rows by depth (1: ST0..ST8; 2: ST9..STC)
    joints: array<mat4x4<f32>, 64>,
};
@group(0) @binding(0) var<uniform> inst: Inst;
@group(1) @binding(0) var tex: texture_2d<f32>;
@group(1) @binding(1) var smp: sampler;

struct VIn {
    @location(0) pos: vec3<f32>,
    @location(1) nrm: vec3<f32>,
    @location(2) uv: vec2<f32>,
    @location(3) col: vec4<f32>,
    @location(4) j: vec4<u32>,
    @location(5) w: vec4<f32>,
};
struct VOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) uv: vec2<f32>,
    // the vertex colour as the GPU gets it (a factor in gamma space: texel × colour / 128, or the flat / Gouraud
    // colour / 255), lighting included
    @location(1) col: vec4<f32>,
    // a floor tile's CLUT row offset (`fs_pal`)
    @location(2) @interpolate(flat) row: u32,
};

// The GTE's NCCS / NCCT (fighters): colour = code × clamp(BK + LCM · clamp(LLM · n)) / 4096 per channel,
// the vertex colour saturating at 255 (texel × 255 / 128 at most)
fn gte_light(n: vec3<f32>) -> vec3<f32> {
    let llv = clamp(vec3<f32>(dot(n, inst.light[0].xyz), dot(n, inst.light[1].xyz), dot(n, inst.light[2].xyz)), vec3<f32>(0.0), vec3<f32>(32767.0 / 4096.0));
    let c = inst.back.rgb + vec3<f32>(dot(inst.colour[0].xyz, llv), dot(inst.colour[1].xyz, llv), dot(inst.colour[2].xyz, llv));
    return clamp(c, vec3<f32>(0.0), vec3<f32>(32767.0 / 4096.0));
}

// the floor tiles' draw handlers (ST0..ST8 / ST9..STC): OTZ = (SZ0 + … + SZ3) / 16 (ZSF4
// 0x100), i.e. the tile centre's SZ / 4 (the floor's vertex normals hold the tile centre, `ModelData::floor_tiles`);
// from OTZ 0x100 on the CLUT row offset is (OTZ − 0x100) >> 6, from 0x500 on row 15 (ST0..ST8) or not drawn
// (ST9..STC). Returns the row, or 16 for a tile not drawn.
fn floor_row(centre: vec3<f32>) -> u32 {
    let sz = clamp(dot(inst.floor.xyz, centre) + inst.floor.w, 0.0, 65535.0);
    let otz = u32(sz) >> 2u;
    if (otz < 0x100u) { return 0u; }
    let t = otz - 0x100u;
    if (t < 0x400u) { return t >> 6u; }
    return select(15u, 16u, inst.hide.z == 2u);
}

@vertex
fn vs(v: VIn) -> VOut {
    let jx = v.j.x & 0xFFu;
    let group = v.j.x >> 8u;
    let mask = select(inst.hide.x, inst.hide.y, group == 1u);
    let skin = inst.joints[jx] * v.w.x + inst.joints[v.j.y] * v.w.y
             + inst.joints[v.j.z] * v.w.z + inst.joints[v.j.w] * v.w.w;
    let world = inst.model * skin * vec4<f32>(v.pos, 1.0);
    let n = normalize((inst.model * skin * vec4<f32>(v.nrm, 0.0)).xyz);
    var o: VOut;
    o.clip = inst.view_proj * world;
    if (((mask >> jx) & 1u) == 1u) {
        o.clip = vec4<f32>(0.0, 0.0, 2.0, 1.0); // hidden object: outside the depth range
    }
    o.uv = v.uv;
    let shade = select(vec3<f32>(1.0), gte_light(n), inst.params.x > 0.5);
    o.col = vec4<f32>(min(max(v.col.rgb, vec3<f32>(0.0)) * shade, vec3<f32>(255.0 / 128.0)), v.col.a);
    o.row = 0u;
    if (inst.hide.z != 0u) {
        o.row = floor_row(v.nrm);
        if (o.row == 16u) {
            o.clip = vec4<f32>(0.0, 0.0, 2.0, 1.0); // a far tile of ST9..STC: not drawn
        }
    }
    return o;
}

// a paletted stage texture (`Renderer::set_paletted`): texel indices, the stage's CLUT block (as the PNGs hold the
// colours) and the CLUT row it shows this frame (x, y in the palette; the stage animation moves it)
@group(1) @binding(2) var pal_index: texture_2d<u32>;
@group(1) @binding(3) var pal: texture_2d<f32>;
@group(1) @binding(4) var<uniform> pal_row: vec4<u32>;

// sRGB transfer functions (the textures are sRGB: their bytes are the PS1 colours, (c5 << 3) | (c5 >> 2))
fn to_srgb(c: vec3<f32>) -> vec3<f32> {
    return select(1.055 * pow(c, vec3<f32>(1.0 / 2.4)) - 0.055, c * 12.92, c <= vec3<f32>(0.0031308));
}
fn from_srgb(c: vec3<f32>) -> vec3<f32> {
    return select(pow((c + 0.055) / 1.055, vec3<f32>(2.4)), c / 12.92, c <= vec3<f32>(0.04045));
}

fn shade_texel(texel: vec4<f32>, i: VOut) -> vec4<f32> {
    var t = texel;
    if (t.a < 0.5) { discard; }
    // a fighter's side fade: a subtractive tile of the level over its palettes in VRAM, 5 bits per
    // channel; an entry that reaches 0 becomes the transparent colour. The untextured polygons (flat / Gouraud, e.g.
    // ALI's skin) read no palette and keep their colour (until BKOUT skips the body): the white texture they are
    // drawn with has alpha 254 (`Renderer::white`), every fighter texel 0 or 255
    if (inst.params.w > 0.0 && t.a > 254.5 / 255.0) {
        let c5 = floor(floor(to_srgb(t.rgb) * 255.0 + 0.5) / 8.0);
        let s5 = max(c5 - floor(inst.params.w / 8.0), vec3<f32>(0.0));
        if (all(s5 == vec3<f32>(0.0))) { discard; }
        t = vec4<f32>(from_srgb((s5 * 8.0 + floor(s5 / 4.0)) / 255.0), t.a);
    }
    // the vertex colour (texel × colour / 128, or the flat / Gouraud colour) is a PS1 frame-buffer value: the GPU
    // multiplies in gamma space, so in linear light the factor is colour^2.2 (texels are linear from the sRGB texture).
    // Every model: the stages' polygons and the fighters' untextured ones (flat colours, e.g. ALI's skin (224, 160,
    // 120) shows as 224, not 241); textured fighter polygons have no colour (factor 1), only their lighting
    let m = pow(i.col.rgb, vec3<f32>(2.2));
    var c = max(t.rgb * m - vec3<f32>(inst.params.z), vec3<f32>(0.0));   // z: stage darkening
    if (inst.params.y > 0.5) { c = pow(max(c, vec3<f32>(0.0)), vec3<f32>(1.0 / 2.2)); }
    return vec4<f32>(c, 1.0);
}

@fragment
fn fs(i: VOut) -> @location(0) vec4<f32> {
    return shade_texel(textureSample(tex, smp, i.uv), i);
}

// the semi-transparent stage polygons (`<ST>_semi.glb`): alpha 1 = texel without the STP bit, drawn opaque; 0.5 =
// STP texel (and the untextured polygons' white), added to the frame
@fragment
fn fs_stp_opaque(i: VOut) -> @location(0) vec4<f32> {
    let t = textureSample(tex, smp, i.uv);
    if (t.a < 0.75) { discard; }
    return shade_texel(t, i);
}

@fragment
fn fs_stp_add(i: VOut) -> @location(0) vec4<f32> {
    let t = textureSample(tex, smp, i.uv);
    if (t.a < 0.25 || t.a > 0.75) { discard; }
    return shade_texel(vec4<f32>(t.rgb, 1.0), i);
}

// the texel the nearest sampler of `fs` picks, its index, and that colour of the palette row. floor(uv × size) is
// the WebGPU spec's nearest rule; a GPU that samples with fewer sub-texel bits may pick the neighbour where uv lies
// within that precision of a texel edge, so at such edges this and the sampled texture may differ by one texel
// (neither is the PS1's own rasteriser rule; left as is)
@fragment
fn fs_pal(i: VOut) -> @location(0) vec4<f32> {
    let size = vec2<f32>(textureDimensions(pal_index));
    let p = vec2<i32>(clamp(floor(i.uv * size), vec2<f32>(0.0), size - 1.0));
    let k = textureLoad(pal_index, p, 0).r;
    return shade_texel(textureLoad(pal, vec2<i32>(i32(pal_row.x + k), i32(pal_row.y + i.row)), 0), i);
}

// fighter shadow: the skinned shadow proxy sheared along the stage light (light[0].xy per unit of
// height, the shadow matrix at M+0x35C) and flattened onto the floor (light[0].w), drawn as
// solid black (as the game renders it); light[1].rgb: an imported stage's shadow colour (0 = black)
@vertex
fn vs_shadow(v: VIn) -> VOut {
    let jx = v.j.x & 0xFFu;
    let group = v.j.x >> 8u;
    let mask = select(inst.hide.x, inst.hide.y, group == 1u);
    let skin = inst.joints[jx] * v.w.x + inst.joints[v.j.y] * v.w.y
             + inst.joints[v.j.z] * v.w.z + inst.joints[v.j.w] * v.w.w;
    var world = inst.model * skin * vec4<f32>(v.pos, 1.0);
    world.x = world.x + inst.light[0].x * world.y;
    world.z = world.z + inst.light[0].y * world.y;
    world.y = inst.light[0].w + 0.004;
    var o: VOut;
    o.clip = inst.view_proj * world;
    if (((mask >> jx) & 1u) == 1u) {
        o.clip = vec4<f32>(0.0, 0.0, 2.0, 1.0);
    }
    o.uv = v.uv;
    o.col = v.col;
    o.row = 0u;
    return o;
}

@fragment
fn fs_shadow(i: VOut) -> @location(0) vec4<f32> {
    // a frame-buffer colour (0..1), linear in light as `shade_texel`'s vertex colour
    var c = pow(inst.light[1].xyz, vec3<f32>(2.2));
    if (inst.params.y > 0.5) { c = inst.light[1].xyz; }
    return vec4<f32>(c, 1.0);
}
