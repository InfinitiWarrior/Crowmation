// Outlines from object id edges, then a box downsample of the supersampled frame.

struct Obj {
    model: mat4x4<f32>,
    color: vec4<f32>,
    outline: vec4<f32>,
    id: u32,
    _p0: u32,
    _p1: u32,
    _p2: u32,
}

@group(0) @binding(0) var color: texture_2d<f32>;
@group(0) @binding(1) var ids: texture_2d<u32>;
@group(0) @binding(2) var<storage, read> objs: array<Obj>;
@group(0) @binding(3) var out: texture_storage_2d<rgba8unorm, write>;
// x: outline half width in supersampled pixels, y: supersampling factor.
@group(0) @binding(4) var<uniform> params: vec4<u32>;

fn outline_of(id: u32) -> vec4<f32> {
    if (id == 0u) {
        return vec4(0.0);
    }
    return objs[id - 1u].outline;
}

fn shade(p: vec2<i32>) -> vec3<f32> {
    let m = vec2<i32>(textureDimensions(ids)) - 1;
    let c = textureLoad(color, p, 0).rgb;
    let a = textureLoad(ids, p, 0).x;
    var edge = vec4(0.0);
    let r = i32(params.x);
    for (var k = 1; k <= r; k++) {
        for (var dy = -1; dy <= 1; dy++) {
            for (var dx = -1; dx <= 1; dx++) {
                let b = textureLoad(ids, clamp(p + vec2(dx, dy) * k, vec2(0), m), 0).x;
                if (b != a) {
                    var o = outline_of(a);
                    if (o.a == 0.0) {
                        o = outline_of(b);
                    }
                    if (o.a > edge.a) {
                        edge = o;
                    }
                }
            }
        }
    }
    return mix(c, edge.rgb, edge.a);
}

@compute @workgroup_size(8, 8)
fn main(@builtin(global_invocation_id) id: vec3<u32>) {
    let dims = textureDimensions(out);
    if (id.x >= dims.x || id.y >= dims.y) {
        return;
    }
    let s = i32(params.y);
    var acc = vec3(0.0);
    for (var y = 0; y < s; y++) {
        for (var x = 0; x < s; x++) {
            acc += shade(vec2<i32>(id.xy) * s + vec2(x, y));
        }
    }
    textureStore(out, id.xy, vec4(acc / f32(s * s), 1.0));
}
