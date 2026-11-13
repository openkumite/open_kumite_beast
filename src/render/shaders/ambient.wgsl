// Engine addition: the ambient fill (output::layout::Split). The bars around a fixed picture or the capped 3D view show a
// blurred, darkened copy of it: `blur` draws the copied picture `img` blurred into a texture at 1/4 of its size (32
// taps of a Vogel disk, linear filtering), `up` draws that scaled up into the bars (`uv`: its placement) at 3/8
// brightness (the PS1 tint 0x30 of 0x80; `dim`, set by the renderer: 0.375^2.2 on an sRGB target, which encodes
// the linear output).
override dim: f32 = 0.375;

@group(0) @binding(0) var img: texture_2d<f32>;
@group(0) @binding(1) var smp: sampler;

struct VIn {
    @location(0) pos: vec2<f32>,
    @location(1) uv: vec2<f32>,
};
struct VOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@vertex
fn vs(v: VIn) -> VOut {
    var o: VOut;
    o.clip = vec4<f32>(v.pos, 0.0, 1.0);
    o.uv = v.uv;
    return o;
}

@fragment
fn blur(i: VOut) -> @location(0) vec4<f32> {
    let n = 32;
    let radius = 0.06;
    var sum = vec3<f32>(0.0);
    for (var k = 0; k < n; k++) {
        let r = radius * sqrt((f32(k) + 0.5) / f32(n));
        let a = f32(k) * 2.39996323;
        sum += textureSampleLevel(img, smp, clamp(i.uv + r * vec2<f32>(cos(a), sin(a)), vec2<f32>(0.0), vec2<f32>(1.0)), 0.0).rgb;
    }
    return vec4<f32>(sum / f32(n), 1.0);
}

@fragment
fn up(i: VOut) -> @location(0) vec4<f32> {
    return vec4<f32>(textureSampleLevel(img, smp, i.uv, 0.0).rgb * dim, 1.0);
}

// RENDER QUALITY (engine addition): the scene drawn at a scaled resolution, copied onto the window.
@fragment
fn copy(i: VOut) -> @location(0) vec4<f32> {
    return vec4<f32>(textureSampleLevel(img, smp, i.uv, 0.0).rgb, 1.0);
}
