// Engine addition: 2D quads textured from the RGBA sprite atlas (render/atlas_cache.rs), same colour, STP and blend rules as
// psx.wgsl.
// info.x = region origin in the layer (x | y << 16)
// info.y = sheet rect origin (u | v << 8) | layer << 16
// info.z = flags: bit 0 textured, bit 1 semi-transparent, bits 2-3 pass (0 = all texels, 1 = only STP texels,
//          2 = only non-STP texels), bits 4-5 blend mode (0 = B/2+F/2, 1 = B+F, 2 = B−F, 3 = B+F/4),
//          bit 6 5-bit source (texels are c5·255/31 rounded: requantised to c5/31)
// info.w = region size (w | h << 16)
@group(0) @binding(0) var atlas: texture_2d_array<f32>;

struct VIn {
    @location(0) pos: vec4<f32>,
    @location(1) uv: vec2<f32>,
    @location(2) col: vec4<f32>,
    @location(3) info: vec4<u32>,
};
struct VOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) col: vec4<f32>,
    @location(2) @interpolate(flat) info: vec4<u32>,
};

@vertex
fn vs(v: VIn) -> VOut {
    var o: VOut;
    o.clip = v.pos;
    o.uv = v.uv;
    o.col = v.col;
    o.info = v.info;
    return o;
}

@fragment
fn fs(i: VOut) -> @location(0) vec4<f32> {
    let flags = i.info.z;
    var rgb: vec3<f32>;
    var stp = 1u;
    if ((flags & 1u) != 0u) {
        // the page texel (u, v) as the GPU picks it, then its place in the baked rect
        let u = u32(max(i.uv.x, 0.0)) & 255u;
        let v = u32(max(i.uv.y, 0.0)) & 255u;
        let rect = vec2<i32>(i32(i.info.y & 255u), i32((i.info.y >> 8u) & 255u));
        let size = vec2<i32>(i32(i.info.w & 0xFFFFu), i32(i.info.w >> 16u));
        let local = clamp(vec2<i32>(i32(u), i32(v)) - rect, vec2<i32>(0), size - vec2<i32>(1));
        let at = vec2<i32>(i32(i.info.x & 0xFFFFu), i32(i.info.x >> 16u)) + local;
        let c = textureLoad(atlas, at, i32(i.info.y >> 16u), 0);
        if (c.a == 0.0) {
            discard;
        }
        stp = select(0u, 1u, c.a < 0.75);
        var t = c.rgb;
        if ((flags & 64u) != 0u) {
            t = round(t * 31.0) / 31.0;
        }
        rgb = min(t * i.col.rgb / 128.0 * 255.0 / 255.0, vec3<f32>(1.0));
    } else {
        rgb = i.col.rgb / 255.0;
    }
    let pas = (flags >> 2u) & 3u;
    if ((flags & 2u) != 0u) {
        if (pas == 1u && stp == 0u) { discard; }
        if (pas == 2u && stp == 1u) { discard; }
    }
    // colour space of the target (sRGB surfaces want linear values)
    if (i.col.a > 0.5) {
        rgb = pow(rgb, vec3<f32>(2.2));
    }
    let blend = (flags >> 4u) & 3u;
    if ((flags & 2u) != 0u && pas == 1u) {
        if (blend == 0u) { rgb = rgb * 0.5; }
        if (blend == 3u) { rgb = rgb * 0.25; }
    }
    return vec4<f32>(rgb, 1.0);
}
