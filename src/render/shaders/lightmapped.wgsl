// Lightmapped meshes, the six-face sky box and the cloud sky (engine addition: the extra stages, `render::lightmapped`).
// The textures are sampled as stored (gamma values, as Quake III blends them in its frame buffer): base × lightmap
// (or × vertex colour) + glow, then the stage darkening subtracted in linear light as `model.wgsl` does.

struct Inst {
    view_proj: mat4x4<f32>,
    model: mat4x4<f32>,
    params: vec4<f32>,          // y: encode sRGB in the shader (non-sRGB surface), z: subtract (stage darkening),
                                // w: time (s, the texture scroll)
    // (T6: the courtyard clip's entry points `*_clip` only) row 0: the eye (model space); rows 1 / 2: the box's corners
    clip: array<vec4<f32>, 3>,
};
@group(0) @binding(0) var<uniform> inst: Inst;
@group(1) @binding(0) var base_tex: texture_2d<f32>;
@group(1) @binding(1) var base_smp: sampler;
@group(2) @binding(0) var lm_tex: texture_2d<f32>;
@group(2) @binding(1) var lm_smp: sampler;
@group(3) @binding(0) var glow_tex: texture_2d<f32>;
@group(3) @binding(1) var glow_smp: sampler;

struct VIn {
    @location(0) pos: vec3<f32>,
    @location(1) uv: vec2<f32>,
    @location(2) lm: vec2<f32>,
    @location(3) col: vec4<f32>,
    @location(4) scroll: vec2<f32>,
};
struct VOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) lm: vec2<f32>,
    @location(2) col: vec4<f32>,
};

@vertex
fn vs(v: VIn) -> VOut {
    var o: VOut;
    o.clip = inst.view_proj * inst.model * vec4<f32>(v.pos, 1.0);
    o.uv = v.uv + v.scroll * inst.params.w;
    o.lm = v.lm;
    o.col = v.col;
    return o;
}

// gamma-space colour → the target (darkened in linear light; encoded again on a non-sRGB target)
fn out_colour(g: vec3<f32>, a: f32) -> vec4<f32> {
    var c = max(pow(max(g, vec3<f32>(0.0)), vec3<f32>(2.2)) - vec3<f32>(inst.params.z), vec3<f32>(0.0));
    if (inst.params.y > 0.5) { c = pow(c, vec3<f32>(1.0 / 2.2)); }
    return vec4<f32>(c, a);
}

fn lit(i: VOut) -> vec4<f32> {
    let t = textureSample(base_tex, base_smp, i.uv);
    let l = textureSample(lm_tex, lm_smp, i.lm).rgb;
    let g = textureSample(glow_tex, glow_smp, i.uv).rgb;
    return vec4<f32>(min(t.rgb * l * i.col.rgb + g, vec3<f32>(1.0)), t.a * i.col.a);
}

@fragment
fn fs_opaque(i: VOut) -> @location(0) vec4<f32> {
    let c = lit(i);
    return out_colour(c.rgb, 1.0);
}

@fragment
fn fs_mask(i: VOut) -> @location(0) vec4<f32> {
    let c = lit(i);
    if (c.a < 0.5) { discard; }
    return out_colour(c.rgb, 1.0);
}

@fragment
fn fs_alpha(i: VOut) -> @location(0) vec4<f32> {
    let c = lit(i);
    return out_colour(c.rgb, c.a);
}

// added: the darkening subtracted from what is added would brighten nothing, so it scales it instead
@fragment
fn fs_add(i: VOut) -> @location(0) vec4<f32> {
    let c = lit(i);
    var l = pow(c.rgb, vec3<f32>(2.2)) * (1.0 - inst.params.z);
    if (inst.params.y > 0.5) { l = pow(l, vec3<f32>(1.0 / 2.2)); }
    return vec4<f32>(l, 1.0);
}

// The courtyard clip (engine addition, T6: the team battle on a walled extra stage; `Renderer::set_instance_lit_clip`
// picks these entry points, every other mesh keeps the ones above): as `vs` / `fs_*`, but a point outside the box that
// stands between the eye and the box (the ray from the eye through it meets the box further on) is not drawn, so the
// camera, which knows no stage geometry, sees the fighters' courtyard through pillars and walls behind it.
struct VOutClip {
    @builtin(position) clip: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) lm: vec2<f32>,
    @location(2) col: vec4<f32>,
    @location(3) pos: vec3<f32>,
};

@vertex
fn vs_clip(v: VIn) -> VOutClip {
    var o: VOutClip;
    o.clip = inst.view_proj * inst.model * vec4<f32>(v.pos, 1.0);
    o.uv = v.uv + v.scroll * inst.params.w;
    o.lm = v.lm;
    o.col = v.col;
    o.pos = v.pos;
    return o;
}

fn clipped(p: vec3<f32>) -> bool {
    // the box shrunk by 0.05 (15 game units): a face in its sides' planes (the pillars' fronts, the walls' insides,
    // the floor) is outside it, hidden only when the box lies beyond it from the eye
    let lo = inst.clip[1].xyz + vec3<f32>(0.05);
    let hi = inst.clip[2].xyz - vec3<f32>(0.05);
    if (all(p > lo) && all(p < hi)) { return false; }
    let o = inst.clip[0].xyz;
    // the slabs: the ray o + t·d inside the box for t in [t0, t1]; it meets it past the point when t1 >= max(t0, 1)
    // (a component of d of about 0 divides to a huge t: that slab is all or nothing, as wanted; not 0, whose 0/0 for a
    // point in the slab's plane would be NaN, which min / max treat differently on each backend)
    let d0 = p - o;
    let d = select(d0, vec3<f32>(1e-6), abs(d0) < vec3<f32>(1e-6));
    let a = (lo - o) / d;
    let b = (hi - o) / d;
    let n = min(a, b);
    let f = max(a, b);
    let t0 = max(max(n.x, n.y), max(n.z, 1.0));
    let t1 = min(min(f.x, f.y), f.z);
    return t1 >= t0;
}

fn unclip(i: VOutClip) -> VOut {
    return VOut(i.clip, i.uv, i.lm, i.col);
}

@fragment
fn fs_opaque_clip(i: VOutClip) -> @location(0) vec4<f32> {
    let c = lit(unclip(i));
    if (clipped(i.pos)) { discard; }
    return out_colour(c.rgb, 1.0);
}

@fragment
fn fs_mask_clip(i: VOutClip) -> @location(0) vec4<f32> {
    let c = lit(unclip(i));
    if (c.a < 0.5 || clipped(i.pos)) { discard; }
    return out_colour(c.rgb, 1.0);
}

@fragment
fn fs_alpha_clip(i: VOutClip) -> @location(0) vec4<f32> {
    let c = lit(unclip(i));
    if (clipped(i.pos)) { discard; }
    return out_colour(c.rgb, c.a);
}

@fragment
fn fs_add_clip(i: VOutClip) -> @location(0) vec4<f32> {
    let c = lit(unclip(i));
    if (clipped(i.pos)) { discard; }
    var l = pow(c.rgb, vec3<f32>(2.2)) * (1.0 - inst.params.z);
    if (inst.params.y > 0.5) { l = pow(l, vec3<f32>(1.0 / 2.2)); }
    return vec4<f32>(l, 1.0);
}

// The sky box: a cube round the eye (view_proj without the eye's position), each face of the 2D array texture
// (rt bk lf ft up dn = +X +Y −X −Y +Z −Z) picked by the direction;
// `model` turns the world's directions into the sky's. Drawn first, at the far plane, no depth.
@group(1) @binding(0) var sky_tex: texture_2d_array<f32>;
@group(1) @binding(1) var sky_smp: sampler;

struct SkyOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) dir: vec3<f32>,
};

@vertex
fn vs_sky(@builtin(vertex_index) k: u32) -> SkyOut {
    // 36 vertices: 6 faces × 2 triangles of the unit cube
    let face = k / 6u;
    let corner = array<vec2<f32>, 6>(vec2(-1.0, -1.0), vec2(1.0, -1.0), vec2(1.0, 1.0), vec2(-1.0, -1.0), vec2(1.0, 1.0), vec2(-1.0, 1.0))[k % 6u];
    var p: vec3<f32>;
    switch face {
        case 0u: { p = vec3(1.0, corner.x, corner.y); }
        case 1u: { p = vec3(-1.0, corner.y, corner.x); }
        case 2u: { p = vec3(corner.y, 1.0, corner.x); }
        case 3u: { p = vec3(corner.x, -1.0, corner.y); }
        case 4u: { p = vec3(corner.x, corner.y, 1.0); }
        default: { p = vec3(corner.y, corner.x, -1.0); }
    }
    var o: SkyOut;
    let c = inst.view_proj * vec4<f32>(p, 1.0);
    o.clip = vec4<f32>(c.xy, c.w * 0.999999, c.w);
    o.dir = (inst.model * vec4<f32>(p, 0.0)).xyz;
    return o;
}

@fragment
fn fs_sky(i: SkyOut) -> @location(0) vec4<f32> {
    let d = i.dir;
    let a = abs(d);
    var uv: vec2<f32>;
    var f: i32;
    if (a.x >= a.y && a.x >= a.z) {
        if (d.x > 0.0) { f = 0; uv = vec2(-d.y, d.z) / a.x; } else { f = 2; uv = vec2(d.y, d.z) / a.x; }
    } else if (a.y >= a.z) {
        if (d.y > 0.0) { f = 1; uv = vec2(d.x, d.z) / a.y; } else { f = 3; uv = vec2(-d.x, d.z) / a.y; }
    } else {
        if (d.z > 0.0) { f = 4; uv = vec2(-d.y, -d.x) / a.z; } else { f = 5; uv = vec2(-d.y, d.x) / a.z; }
    }
    let st = clamp(vec2(uv.x + 1.0, 1.0 - uv.y) * 0.5, vec2(0.002), vec2(0.998));
    let t = textureSampleLevel(sky_tex, sky_smp, st, f, 0.0);
    return out_colour(t.rgb, 1.0);
}

// The cloud sky (`skyParms - <height> -` and its stages): the direction from the eye meets a
// sphere of radius 4096 whose top is `height` above the eye (its centre 4096 below); the texture coordinates are the
// angles acos(x), acos(y) of that point from the sphere's centre, then each layer's st · scale + t · scroll. Below the
// horizon black (Quake III skips only the box's bottom face).
struct Clouds {
    layer0: vec4<f32>,          // scale.xy, scroll.xy
    layer1: vec4<f32>,
    misc: vec4<f32>,            // x: the cloud height, y: layers (1 or 2), z: the second added (else over the first)
};
@group(1) @binding(4) var<uniform> clouds: Clouds;
@group(1) @binding(5) var cloud0_tex: texture_2d<f32>;
@group(1) @binding(6) var cloud1_tex: texture_2d<f32>;
@group(1) @binding(7) var cloud_smp: sampler;

@fragment
fn fs_clouds(i: SkyOut) -> @location(0) vec4<f32> {
    let d = normalize(i.dir);
    let r = 4096.0;
    let h = clouds.misc.x;
    let p = -d.z * r + sqrt(d.z * d.z * r * r + 2.0 * r * h + h * h);
    let v = normalize(d * p + vec3<f32>(0.0, 0.0, r));
    let st = acos(clamp(v.xy, vec2<f32>(-1.0), vec2<f32>(1.0)));
    let t = inst.params.w;
    let st0 = st * clouds.layer0.xy + clouds.layer0.zw * t;
    let st1 = st * clouds.layer1.xy + clouds.layer1.zw * t;
    // both sampled in uniform control flow (the derivatives), then chosen
    var c = textureSample(cloud0_tex, cloud_smp, st0).rgb;
    let c1 = textureSample(cloud1_tex, cloud_smp, st1);
    if (clouds.misc.y > 1.5) {
        if (clouds.misc.z > 0.5) { c = c + c1.rgb; } else { c = mix(c, c1.rgb, c1.a); }
    }
    // below the horizon black (engine: Quake III draws the clouds down the box's sides; the main game's camera may stand
    // outside a courtyard's walls, where the void under the floor's edge would show clouds)
    if (d.z < 0.0) { c = vec3<f32>(0.0); }
    return out_colour(min(c, vec3<f32>(1.0)), 1.0);
}
