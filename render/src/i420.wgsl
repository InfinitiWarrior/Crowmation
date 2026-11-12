// RGBA (sRGB encoded) to I420, BT.709 limited range.
// One invocation converts an 8x2 pixel block, so every write is a whole u32.
// Layout: Y plane (w*h), then U (w/2 * h/2), then V.

@group(0) @binding(0) var src: texture_2d<f32>;
@group(0) @binding(1) var<storage, read_write> dst: array<u32>;

fn luma(c: vec3<f32>) -> f32 {
    return 16.0 + dot(c, vec3(46.559, 156.629, 15.812));
}

fn pack(a: f32, b: f32, c: f32, d: f32) -> u32 {
    let v = vec4<u32>(clamp(round(vec4(a, b, c, d)), vec4(0.0), vec4(255.0)));
    return v.x | (v.y << 8u) | (v.z << 16u) | (v.w << 24u);
}

@compute @workgroup_size(8, 8)
fn main(@builtin(global_invocation_id) id: vec3<u32>) {
    let size = textureDimensions(src);
    let w = size.x;
    let h = size.y;
    let bx = id.x * 8u;
    let by = id.y * 2u;
    if (bx >= w || by >= h) {
        return;
    }

    var px: array<vec3<f32>, 16>;
    for (var r = 0u; r < 2u; r++) {
        for (var i = 0u; i < 8u; i++) {
            px[r * 8u + i] = textureLoad(src, vec2(bx + i, by + r), 0).rgb;
        }
        let row = (by + r) * w + bx;
        let o = r * 8u;
        dst[row / 4u] = pack(luma(px[o]), luma(px[o + 1u]), luma(px[o + 2u]), luma(px[o + 3u]));
        dst[row / 4u + 1u] = pack(luma(px[o + 4u]), luma(px[o + 5u]), luma(px[o + 6u]), luma(px[o + 7u]));
    }

    // Four chroma samples, each the average of a 2x2 block.
    var u: array<f32, 4>;
    var v: array<f32, 4>;
    for (var k = 0u; k < 4u; k++) {
        let c = (px[2u * k] + px[2u * k + 1u] + px[8u + 2u * k] + px[9u + 2u * k]) * 0.25;
        u[k] = 128.0 + dot(c, vec3(-25.664, -86.336, 112.0));
        v[k] = 128.0 + dot(c, vec3(112.0, -101.730, -10.270));
    }
    let cw = w / 2u;
    let crow = (by / 2u) * cw + bx / 2u;
    let uo = w * h;
    let vo = uo + cw * (h / 2u);
    dst[(uo + crow) / 4u] = pack(u[0], u[1], u[2], u[3]);
    dst[(vo + crow) / 4u] = pack(v[0], v[1], v[2], v[3]);
}
