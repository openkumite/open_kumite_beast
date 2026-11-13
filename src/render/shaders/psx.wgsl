// PS1 GPU textured / flat quads drawn from a copy of the game's VRAM (1024x512 16-bit words; two banks, below).
// info.x = tpage (x = bits 0-3 × 64, y = bit 4 × 256, colour mode bits 7-8: 0 = 4-bit, 1 = 8-bit, 2 = 15-bit)
// info.y = clut (x = bits 0-5 × 16, y = bits 6-14; bit 15, engine addition: VRAM bank 1, rows 512..1023 of the copy,
//          the team battle's fighters 2 / 3)
// info.z = flags: bit 0 textured, bit 1 semi-transparent, bits 2-3 pass (0 = all texels, 1 = only STP texels,
//          2 = only non-STP texels), bits 4-5 blend mode (0 = B/2+F/2, 1 = B+F, 2 = B−F, 3 = B+F/4)
@group(0) @binding(0) var vram: texture_2d<u32>;

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

fn fetch(x: u32, y: u32, bank: u32) -> u32 {
    return textureLoad(vram, vec2<i32>(i32(x & 1023u), i32((y & 511u) + bank)), 0).r;
}

@fragment
fn fs(i: VOut) -> @location(0) vec4<f32> {
    let flags = i.info.z;
    var rgb: vec3<f32>;
    var stp = 1u;
    if ((flags & 1u) != 0u) {
        let tp = i.info.x;
        let px = (tp & 15u) * 64u;
        let py = ((tp >> 4u) & 1u) * 256u;
        let mode = (tp >> 7u) & 3u;
        let cx = (i.info.y & 63u) * 16u;
        let cy = (i.info.y >> 6u) & 511u;
        let bank = (i.info.y >> 15u) * 512u;
        let u = u32(max(i.uv.x, 0.0)) & 255u;
        let v = u32(max(i.uv.y, 0.0)) & 255u;
        var c: u32;
        if (mode == 0u) {
            let w = fetch(px + u / 4u, py + v, bank);
            c = fetch(cx + ((w >> ((u & 3u) * 4u)) & 15u), cy, bank);
        } else if (mode == 1u) {
            let w = fetch(px + u / 2u, py + v, bank);
            c = fetch(cx + ((w >> ((u & 1u) * 8u)) & 255u), cy, bank);
        } else {
            c = fetch(px + u, py + v, bank);
        }
        if (c == 0u) {
            discard;
        }
        stp = c >> 15u;
        let t = vec3<f32>(f32(c & 31u), f32((c >> 5u) & 31u), f32((c >> 10u) & 31u)) / 31.0;
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
