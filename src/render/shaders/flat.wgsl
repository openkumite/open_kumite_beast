struct VIn {
    @location(0) pos: vec2<f32>,
    @location(1) col: vec4<f32>,
};
struct VOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) col: vec4<f32>,
};
@vertex
fn vs(v: VIn) -> VOut {
    var o: VOut;
    o.clip = vec4<f32>(v.pos, 0.0, 1.0);
    o.col = v.col;
    return o;
}
// the depth reset (render/pipelines.rs `depth_reset` (used in frame.rs)): one triangle over the whole viewport at the far plane
@vertex
fn vs_far(@builtin(vertex_index) i: u32) -> VOut {
    var o: VOut;
    o.clip = vec4<f32>(f32(i & 1u) * 4.0 - 1.0, f32(i >> 1u) * 4.0 - 1.0, 1.0, 1.0);
    o.col = vec4<f32>(0.0);
    return o;
}
@fragment
fn fs(i: VOut) -> @location(0) vec4<f32> {
    return i.col;
}
