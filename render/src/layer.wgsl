// A 2D layer (vello output, straight alpha) drawn as a quad. `mats[i]` maps
// the unit square to clip space: a plane in the 3D world, or the screen.

@group(0) @binding(0) var<storage, read> mats: array<mat4x4<f32>>;
@group(1) @binding(0) var tex: texture_2d<f32>;

struct V {
    @builtin(position) pos: vec4<f32>,
    @location(0) uv: vec2<f32>,
}

@vertex
fn vs(@builtin(vertex_index) vi: u32, @builtin(instance_index) i: u32) -> V {
    let c = vec2(f32(vi & 1u), f32(vi >> 1u));
    return V(mats[i] * vec4(c, 0.0, 1.0), c);
}

fn texel(p: vec2<i32>) -> vec4<f32> {
    let m = vec2<i32>(textureDimensions(tex)) - 1;
    let t = textureLoad(tex, clamp(p, vec2(0), m), 0);
    return vec4(t.rgb * t.a, t.a);
}

// Bilinear on premultiplied texels, so transparent edges do not darken.
fn sample(uv: vec2<f32>) -> vec4<f32> {
    let p = uv * vec2<f32>(textureDimensions(tex)) - 0.5;
    let f = fract(p);
    let b = vec2<i32>(floor(p));
    let top = mix(texel(b), texel(b + vec2(1, 0)), f.x);
    let bottom = mix(texel(b + vec2(0, 1)), texel(b + vec2(1, 1)), f.x);
    return mix(top, bottom, f.y);
}

struct Out {
    @location(0) color: vec4<f32>,
    @location(1) id: u32,
}

@fragment
fn fs(v: V) -> Out {
    return Out(sample(v.uv), 0u);
}

// Solid parts of the layer hide what is behind them from the outline pass.
@fragment
fn fs_id(v: V) -> Out {
    let c = sample(v.uv);
    if (c.a < 0.5) {
        discard;
    }
    return Out(c, 0u);
}

@fragment
fn fs_overlay(v: V) -> @location(0) vec4<f32> {
    return sample(v.uv);
}
