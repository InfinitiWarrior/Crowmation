//! Writes examples/models/router.glb: a stylized home router built from boxes
//! and cylinders, one named node per part. Units are meters, y up, front is +z.
//!
//! cargo run -p crowmation-mesh --example router

use serde_json::{Value, json};

struct Part {
    name: &'static str,
    /// sRGB.
    color: [f32; 3],
    pos: [f32; 3],
    verts: Vec<[f32; 3]>,
    tris: Vec<u32>,
}

fn cuboid(name: &'static str, size: [f32; 3], pos: [f32; 3], color: [f32; 3]) -> Part {
    let [x, y, z] = size.map(|s| s / 2.0);
    let verts = (0..8).map(|i| [if i & 1 == 0 { -x } else { x }, if i & 2 == 0 { -y } else { y }, if i & 4 == 0 { -z } else { z }]).collect();
    #[rustfmt::skip]
    let tris = vec![0, 2, 1, 1, 2, 3, 4, 5, 6, 5, 7, 6, 0, 1, 4, 1, 5, 4, 2, 6, 3, 3, 6, 7, 0, 4, 2, 2, 4, 6, 1, 3, 5, 3, 7, 5];
    Part { name, color, pos, verts, tris }
}

/// Vertical cylinder.
fn cylinder(name: &'static str, r: f32, h: f32, pos: [f32; 3], color: [f32; 3]) -> Part {
    let n = 20;
    let mut verts = vec![[0.0, -h / 2.0, 0.0], [0.0, h / 2.0, 0.0]];
    for i in 0..n {
        let a = i as f32 / n as f32 * std::f32::consts::TAU;
        verts.push([r * a.cos(), -h / 2.0, r * a.sin()]);
        verts.push([r * a.cos(), h / 2.0, r * a.sin()]);
    }
    let mut tris = vec![];
    for i in 0..n {
        let (b0, t0, b1, t1) = (2 + 2 * i, 3 + 2 * i, 2 + 2 * ((i + 1) % n), 3 + 2 * ((i + 1) % n));
        tris.extend([b0, t0, b1, b1, t0, t1, 0, b0, b1, 1, t1, t0]);
    }
    Part { name, color, pos, verts, tris }
}

fn main() {
    let dark = [0.13, 0.14, 0.16];
    let chip = [0.10, 0.10, 0.12];
    let parts = vec![
        cuboid("base", [0.26, 0.03, 0.18], [0.0, 0.015, 0.0], dark),
        cuboid("pcb", [0.23, 0.003, 0.155], [0.0, 0.0345, 0.0], [0.08, 0.42, 0.22]),
        cuboid("soc", [0.04, 0.006, 0.04], [-0.05, 0.039, 0.005], chip),
        cuboid("ram", [0.032, 0.005, 0.014], [-0.05, 0.0385, -0.045], chip),
        cuboid("flash_chip", [0.024, 0.005, 0.014], [0.06, 0.0385, 0.04], chip),
        cuboid("wifi_chip", [0.026, 0.005, 0.026], [0.06, 0.0385, -0.03], [0.55, 0.57, 0.6]),
        cuboid("uart_header", [0.022, 0.009, 0.006], [-0.095, 0.0405, 0.06], [0.85, 0.68, 0.25]),
        cuboid("ports", [0.15, 0.016, 0.012], [0.0, 0.044, -0.072], [0.62, 0.64, 0.68]),
        cuboid("lid", [0.26, 0.026, 0.18], [0.0, 0.065, 0.0], [0.86, 0.88, 0.92]),
        cuboid("leds", [0.09, 0.004, 0.004], [0.0, 0.065, 0.091], [0.25, 0.85, 0.45]),
        cylinder("antenna_left", 0.0065, 0.15, [-0.11, 0.153, -0.08], dark),
        cylinder("antenna_right", 0.0065, 0.15, [0.11, 0.153, -0.08], dark),
    ];

    let mut bin: Vec<u8> = vec![];
    let (mut views, mut accessors, mut meshes, mut nodes, mut materials) = (vec![], vec![], vec![], vec![], vec![]);
    for (i, p) in parts.iter().enumerate() {
        let (lo, hi) = p.verts.iter().fold(([f32::MAX; 3], [f32::MIN; 3]), |(lo, hi), v| {
            (std::array::from_fn(|k| lo[k].min(v[k])), std::array::from_fn(|k| hi[k].max(v[k])))
        });
        let off = bin.len();
        bin.extend(p.verts.iter().flatten().flat_map(|f| f.to_le_bytes()));
        views.push(json!({"buffer": 0, "byteOffset": off, "byteLength": bin.len() - off, "target": 34962}));
        accessors.push(json!({"bufferView": 2 * i, "componentType": 5126, "count": p.verts.len(), "type": "VEC3", "min": lo, "max": hi}));
        let off = bin.len();
        bin.extend(p.tris.iter().flat_map(|t| t.to_le_bytes()));
        views.push(json!({"buffer": 0, "byteOffset": off, "byteLength": bin.len() - off, "target": 34963}));
        accessors.push(json!({"bufferView": 2 * i + 1, "componentType": 5125, "count": p.tris.len(), "type": "SCALAR"}));
        let linear = p.color.map(|c| c.powf(2.2));
        materials.push(json!({"name": p.name, "pbrMetallicRoughness": {"baseColorFactor": [linear[0], linear[1], linear[2], 1.0], "metallicFactor": 0.0}}));
        meshes.push(json!({"name": p.name, "primitives": [{"attributes": {"POSITION": 2 * i}, "indices": 2 * i + 1, "material": i}]}));
        nodes.push(json!({"name": p.name, "mesh": i, "translation": p.pos}));
    }
    let doc: Value = json!({
        "asset": {"version": "2.0", "generator": "crowmation router example"},
        "scene": 0,
        "scenes": [{"nodes": (0..parts.len()).collect::<Vec<_>>()}],
        "nodes": nodes, "meshes": meshes, "materials": materials,
        "accessors": accessors, "bufferViews": views,
        "buffers": [{"byteLength": bin.len()}],
    });
    let mut js = serde_json::to_vec(&doc).unwrap();
    while js.len() % 4 != 0 {
        js.push(b' ');
    }
    while bin.len() % 4 != 0 {
        bin.push(0);
    }
    let mut out = vec![];
    out.extend(b"glTF");
    out.extend(2u32.to_le_bytes());
    out.extend(((12 + 8 + js.len() + 8 + bin.len()) as u32).to_le_bytes());
    out.extend((js.len() as u32).to_le_bytes());
    out.extend(b"JSON");
    out.extend(js);
    out.extend((bin.len() as u32).to_le_bytes());
    out.extend(b"BIN\0");
    out.extend(bin);
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../examples/models/router.glb");
    std::fs::create_dir_all(std::path::Path::new(path).parent().unwrap()).unwrap();
    std::fs::write(path, out).unwrap();
    println!("wrote {path}");
}
