//! .glb loading. Every glTF node with a triangle mesh becomes one named part,
//! baked into model space. Only self contained .glb files are read: external
//! buffers or images would mean file access the scene did not declare.

use glam::{Mat4, Vec3};
use std::collections::HashMap;
use std::sync::{Arc, LazyLock, Mutex};

/// Guards against huge or hostile files; scenes stay fast to load.
pub const MAX_VERTICES: usize = 4_000_000;

pub struct Part {
    /// Node name made into an identifier (`flash_chip`), unique in the model.
    pub name: String,
    /// Relative to `center`. Model units, x right, y down, z toward the viewer
    /// (glTF y up is flipped).
    pub positions: Vec<[f32; 3]>,
    pub indices: Vec<u32>,
    /// sRGB base color of the first primitive's material.
    pub color: [f32; 4],
    /// Center of the part's bounding box, relative to the model's bounding box center.
    pub center: Vec3,
}

pub struct Mesh {
    pub parts: Vec<Part>,
    /// Bounding box size of the whole model.
    pub extent: Vec3,
}

/// Models kept before the cache is cleared.
const CACHE_MAX: usize = 32;
/// Keyed by path and modified time, so an edited model reloads in preview.
static CACHE: LazyLock<Mutex<HashMap<(String, Option<std::time::SystemTime>), Result<Arc<Mesh>, String>>>> =
    LazyLock::new(Default::default);

/// Load and cache a .glb file. `path` must be canonical (the compiler checked
/// it is inside the scene folder); a path that now resolves elsewhere is refused.
pub fn load(path: &str) -> Result<Arc<Mesh>, String> {
    if std::fs::canonicalize(path).ok().as_deref() != Some(std::path::Path::new(path)) {
        return Err(format!("{path}: moved or replaced since the scene was compiled"));
    }
    let mtime = std::fs::metadata(path).and_then(|m| m.modified()).ok();
    let mut cache = CACHE.lock().unwrap();
    if cache.len() >= CACHE_MAX {
        cache.clear();
    }
    cache
        .entry((path.to_string(), mtime))
        .or_insert_with(|| {
            let bytes = std::fs::read(path).map_err(|e| format!("{path}: {e}"))?;
            parse(&bytes).map(Arc::new).map_err(|e| format!("{path}: {e}"))
        })
        .clone()
}

pub fn parse(bytes: &[u8]) -> Result<Mesh, String> {
    let g = gltf::Gltf::from_slice(bytes).map_err(|e| e.to_string())?;
    let blob = g.blob.as_deref();
    let buffers = g
        .buffers()
        .map(|b| match b.source() {
            gltf::buffer::Source::Bin => blob.ok_or("the binary chunk is missing"),
            gltf::buffer::Source::Uri(_) => Err("external buffers are not supported, export a self contained .glb"),
        })
        .collect::<Result<Vec<&[u8]>, _>>()?;
    let scene = g.default_scene().or_else(|| g.scenes().next()).ok_or("the file has no scene")?;

    let mut parts = Vec::new();
    let (mut total, mut total_indices) = (0, 0);
    let mut stack: Vec<(gltf::Node, Mat4)> = scene.nodes().map(|n| (n, Mat4::IDENTITY)).collect();
    while let Some((node, parent)) = stack.pop() {
        let m = parent * Mat4::from_cols_array_2d(&node.transform().matrix());
        stack.extend(node.children().map(|c| (c, m)));
        let Some(mesh) = node.mesh() else { continue };
        let mut part = Part { name: ident(node.name().or(mesh.name()).unwrap_or("")), positions: vec![], indices: vec![], color: [0.8, 0.8, 0.8, 1.0], center: Vec3::ZERO };
        for (k, prim) in mesh.primitives().enumerate() {
            if prim.mode() != gltf::mesh::Mode::Triangles {
                continue;
            }
            // Check sizes before reading, so a huge accessor is refused without allocating.
            let Some(acc) = prim.get(&gltf::Semantic::Positions) else { continue };
            let n = acc.count();
            if n == 0 {
                continue;
            }
            total += n;
            total_indices += prim.indices().map_or(n, |a| a.count());
            if total > MAX_VERTICES || total_indices > 3 * MAX_VERTICES {
                return Err(format!("more than {MAX_VERTICES} vertices or {} indices", 3 * MAX_VERTICES));
            }
            let reader = prim.reader(|b| buffers.get(b.index()).copied());
            let Some(pos) = reader.read_positions() else { continue };
            let base = part.positions.len() as u32;
            part.positions.extend(pos.map(|p| {
                let w = m.transform_point3(Vec3::from(p));
                [w.x, -w.y, w.z]
            }));
            let n = part.positions.len() as u32 - base;
            // Indices with no vertices to point at would land on another primitive's.
            if n == 0 {
                continue;
            }
            match reader.read_indices() {
                Some(ix) => part.indices.extend(ix.into_u32().map(|i| base + i.min(n.saturating_sub(1)))),
                None => part.indices.extend(base..base + n),
            }
            if k == 0 {
                let c = prim.material().pbr_metallic_roughness().base_color_factor();
                part.color = [srgb(c[0]), srgb(c[1]), srgb(c[2]), c[3]];
            }
        }
        if !part.positions.is_empty() {
            parts.push(part);
        }
    }
    if parts.is_empty() {
        return Err("no triangle meshes found".into());
    }
    // glTF order is stable and matches what modeling tools show.
    parts.reverse();

    let bounds = |pts: &mut dyn Iterator<Item = Vec3>| pts.fold((Vec3::MAX, Vec3::MIN), |(lo, hi), p| (lo.min(p), hi.max(p)));
    let (lo, hi) = bounds(&mut parts.iter().flat_map(|p| p.positions.iter().map(|&q| Vec3::from(q))));
    let mid = (lo + hi) / 2.0;
    let mut seen: HashMap<String, usize> = HashMap::new();
    for (i, p) in parts.iter_mut().enumerate() {
        let (plo, phi) = bounds(&mut p.positions.iter().map(|&q| Vec3::from(q)));
        let c = (plo + phi) / 2.0;
        for q in &mut p.positions {
            *q = (Vec3::from(*q) - c).into();
        }
        p.center = c - mid;
        if p.name.is_empty() {
            p.name = format!("part{i}");
        }
        let n = seen.entry(p.name.clone()).or_default();
        *n += 1;
        if *n > 1 {
            p.name = format!("{}_{n}", p.name);
        }
    }
    Ok(Mesh { parts, extent: hi - lo })
}

/// `Flash Chip.001` → `Flash_Chip_001`, usable as a scene name.
fn ident(s: &str) -> String {
    let mut out: String = s.chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '_' }).collect();
    if out.starts_with(|c: char| c.is_ascii_digit()) {
        out.insert(0, '_');
    }
    out
}

fn srgb(linear: f32) -> f32 {
    let l = linear.clamp(0.0, 1.0);
    if l <= 0.0031308 { l * 12.92 } else { 1.055 * l.powf(1.0 / 2.4) - 0.055 }
}

#[cfg(test)]
mod tests {
    #[test]
    fn loads_example_router() {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../examples/models/router.glb");
        let m = super::parse(&std::fs::read(path).unwrap()).unwrap();
        let names: Vec<&str> = m.parts.iter().map(|p| p.name.as_str()).collect();
        assert!(names.contains(&"flash_chip") && names.contains(&"lid"), "{names:?}");
        let lid = m.parts.iter().find(|p| p.name == "lid").unwrap();
        let base = m.parts.iter().find(|p| p.name == "base").unwrap();
        assert!(lid.center.y < base.center.y, "y flipped: the lid is above the base on screen");
        assert!(super::parse(b"not a glb").is_err());
    }
}
