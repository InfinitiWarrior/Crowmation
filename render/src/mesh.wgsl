// Flat color x (ambient + Lambert), fixed light. Writes the object id for outlines.

struct Globals {
    view_proj: mat4x4<f32>,
    eye: vec4<f32>,
    // xyz: direction toward the light, w: ambient.
    light: vec4<f32>,
}

struct Obj {
    model: mat4x4<f32>,
    // sRGB, straight alpha (opacity applied).
    color: vec4<f32>,
    outline: vec4<f32>,
    id: u32,
    _p0: u32,
    _p1: u32,
    _p2: u32,
}

@group(0) @binding(0) var<uniform> g: Globals;
@group(0) @binding(1) var<storage, read> objs: array<Obj>;

struct V {
    @builtin(position) pos: vec4<f32>,
    @location(0) world: vec3<f32>,
    @location(1) @interpolate(flat) inst: u32,
}

struct Out {
    @location(0) color: vec4<f32>,
    @location(1) id: u32,
}

@vertex
fn vs(@location(0) p: vec3<f32>, @builtin(instance_index) i: u32) -> V {
    let w = objs[i].model * vec4(p, 1.0);
    return V(g.view_proj * w, w.xyz, i);
}

@fragment
fn fs(v: V) -> Out {
    let o = objs[v.inst];
    // Face normal from screen space derivatives, turned toward the camera.
    var n = normalize(cross(dpdx(v.world), dpdy(v.world)));
    if (dot(n, g.eye.xyz - v.world) < 0.0) {
        n = -n;
    }
    let lit = g.light.w + (1.0 - g.light.w) * max(dot(n, g.light.xyz), 0.0);
    let c = pow(pow(o.color.rgb, vec3(2.2)) * lit, vec3(1.0 / 2.2));
    return Out(vec4(c * o.color.a, o.color.a), o.id);
}
