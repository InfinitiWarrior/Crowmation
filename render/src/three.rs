//! 3D scenes: meshes and 2D layers share one camera and one depth buffer, so
//! either can pass through the other. 2D objects live on planes parallel to
//! the canvas (at their `z`); with the default camera the z = 0 plane fills
//! the frame at 1 unit = 1 pixel, so 2D content looks exactly as in 2D scenes.
//!
//! Per frame: vello draws each 2D depth layer into a texture, one render pass
//! draws meshes then the layers (depth tested, back to front), a compute pass
//! adds outlines from object id edges and downsamples, and overlay objects
//! (screen fixed or pinned to a 3D object) are drawn on top.

use crate::draw_2d;
use crowmation_core::{Shape, State, Timeline};
use glam::{Mat4, Quat, Vec2, Vec3, Vec4};
use std::collections::HashMap;
use vello::kurbo::Affine;
use vello::peniko::Color;
use vello::{AaConfig, RenderParams, Renderer, Scene, wgpu};

/// Supersampling factor for the 3D pass and 2D layers.
const SS: u32 = 2;
const DEPTH: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;
const COLOR: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;
const OBJ_BYTES: u64 = 128;

struct Target {
    view: wgpu::TextureView,
    bind: wgpu::BindGroup,
}

pub struct Three {
    width: u32,
    height: u32,
    color: wgpu::TextureView,
    ids: wgpu::TextureView,
    depth: wgpu::TextureView,
    mesh_pipe: wgpu::RenderPipeline,
    layer_pipe: wgpu::RenderPipeline,
    layer_id_pipe: wgpu::RenderPipeline,
    overlay_pipe: wgpu::RenderPipeline,
    resolve_pipe: wgpu::ComputePipeline,
    globals: wgpu::Buffer,
    params: wgpu::Buffer,
    /// Grows with the scene; bind groups are rebuilt with it.
    objs: Option<(wgpu::Buffer, wgpu::BindGroup)>,
    mats: wgpu::Buffer,
    mats_bind: wgpu::BindGroup,
    /// Vello targets, one per 2D layer in use.
    layers: Vec<Target>,
    meshes: HashMap<(String, usize), (wgpu::Buffer, wgpu::Buffer, u32)>,
    scene: Scene,
}

/// Most distinct 2D depths drawn in one frame (plus the overlay).
const MAX_LAYERS: usize = 64;

fn texture(device: &wgpu::Device, w: u32, h: u32, format: wgpu::TextureFormat, usage: wgpu::TextureUsages) -> wgpu::TextureView {
    device
        .create_texture(&wgpu::TextureDescriptor {
            label: None,
            size: wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage,
            view_formats: &[],
        })
        .create_view(&Default::default())
}

fn premul_blend() -> Option<wgpu::BlendState> {
    Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING)
}

impl Three {
    pub fn new(device: &wgpu::Device, width: u32, height: u32) -> Self {
        let (sw, sh) = (width * SS, height * SS);
        use wgpu::TextureUsages as U;
        let color = texture(device, sw, sh, COLOR, U::RENDER_ATTACHMENT | U::TEXTURE_BINDING);
        let ids = texture(device, sw, sh, wgpu::TextureFormat::R32Uint, U::RENDER_ATTACHMENT | U::TEXTURE_BINDING);
        let depth = texture(device, sw, sh, DEPTH, U::RENDER_ATTACHMENT);

        let depth_state = |write: bool, compare| {
            Some(wgpu::DepthStencilState {
                format: DEPTH,
                depth_write_enabled: Some(write),
                depth_compare: Some(compare),
                stencil: Default::default(),
                bias: Default::default(),
            })
        };
        let id_target = |write| {
            Some(wgpu::ColorTargetState {
                format: wgpu::TextureFormat::R32Uint,
                blend: None,
                write_mask: if write { wgpu::ColorWrites::ALL } else { wgpu::ColorWrites::empty() },
            })
        };
        let color_target = |write| {
            Some(wgpu::ColorTargetState {
                format: COLOR,
                blend: premul_blend(),
                write_mask: if write { wgpu::ColorWrites::ALL } else { wgpu::ColorWrites::empty() },
            })
        };

        let mesh = device.create_shader_module(wgpu::include_wgsl!("mesh.wgsl"));
        let mesh_pipe = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("mesh"),
            layout: None,
            vertex: wgpu::VertexState {
                module: &mesh,
                entry_point: Some("vs"),
                compilation_options: Default::default(),
                buffers: &[Some(wgpu::VertexBufferLayout {
                    array_stride: 12,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &wgpu::vertex_attr_array![0 => Float32x3],
                })],
            },
            primitive: Default::default(),
            depth_stencil: depth_state(true, wgpu::CompareFunction::Less),
            multisample: Default::default(),
            fragment: Some(wgpu::FragmentState {
                module: &mesh,
                entry_point: Some("fs"),
                compilation_options: Default::default(),
                targets: &[color_target(true), id_target(true)],
            }),
            multiview_mask: None,
            cache: None,
        });

        let layer = device.create_shader_module(wgpu::include_wgsl!("layer.wgsl"));
        let layer_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: None,
            bind_group_layouts: &[
                Some(&device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                    label: None,
                    entries: &[wgpu::BindGroupLayoutEntry {
                        binding: 0,
                        visibility: wgpu::ShaderStages::VERTEX,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Storage { read_only: true },
                            has_dynamic_offset: false,
                            min_binding_size: None,
                        },
                        count: None,
                    }],
                })),
                Some(&device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                    label: None,
                    entries: &[wgpu::BindGroupLayoutEntry {
                        binding: 0,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Texture {
                            sample_type: wgpu::TextureSampleType::Float { filterable: false },
                            view_dimension: wgpu::TextureViewDimension::D2,
                            multisampled: false,
                        },
                        count: None,
                    }],
                })),
            ],
            immediate_size: 0,
        });
        let layer_pipeline = |entry: &str, targets: &[Option<wgpu::ColorTargetState>], depth: bool| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(entry),
                layout: Some(&layer_layout),
                vertex: wgpu::VertexState { module: &layer, entry_point: Some("vs"), compilation_options: Default::default(), buffers: &[] },
                primitive: wgpu::PrimitiveState { topology: wgpu::PrimitiveTopology::TriangleStrip, ..Default::default() },
                depth_stencil: if depth { depth_state(false, wgpu::CompareFunction::LessEqual) } else { None },
                multisample: Default::default(),
                fragment: Some(wgpu::FragmentState { module: &layer, entry_point: Some(entry), compilation_options: Default::default(), targets }),
                multiview_mask: None,
                cache: None,
            })
        };
        let layer_pipe = layer_pipeline("fs", &[color_target(true), id_target(false)], true);
        let layer_id_pipe = layer_pipeline("fs_id", &[color_target(false), id_target(true)], true);
        let overlay_pipe = layer_pipeline("fs_overlay", &[color_target(true)], false);

        let resolve = device.create_shader_module(wgpu::include_wgsl!("resolve.wgsl"));
        let resolve_pipe = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("resolve"),
            layout: None,
            module: &resolve,
            entry_point: Some("main"),
            compilation_options: Default::default(),
            cache: None,
        });

        let buffer = |size, usage| device.create_buffer(&wgpu::BufferDescriptor { label: None, size, usage, mapped_at_creation: false });
        use wgpu::BufferUsages as B;
        let globals = buffer(96, B::UNIFORM | B::COPY_DST);
        let params = buffer(16, B::UNIFORM | B::COPY_DST);
        let mats = buffer(64 * (MAX_LAYERS as u64 + 1), B::STORAGE | B::COPY_DST);
        let mats_bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout: &layer_pipe.get_bind_group_layout(0),
            entries: &[wgpu::BindGroupEntry { binding: 0, resource: mats.as_entire_binding() }],
        });
        Self {
            width,
            height,
            color,
            ids,
            depth,
            mesh_pipe,
            layer_pipe,
            layer_id_pipe,
            overlay_pipe,
            resolve_pipe,
            globals,
            params,
            objs: None,
            mats,
            mats_bind,
            layers: Vec::new(),
            meshes: HashMap::new(),
            scene: Scene::new(),
        }
    }

    fn target(&mut self, device: &wgpu::Device, i: usize) -> &Target {
        while self.layers.len() <= i {
            use wgpu::TextureUsages as U;
            let view = texture(device, self.width * SS, self.height * SS, COLOR, U::STORAGE_BINDING | U::TEXTURE_BINDING);
            let bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: None,
                layout: &self.layer_pipe.get_bind_group_layout(1),
                entries: &[wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&view) }],
            });
            self.layers.push(Target { view, bind });
        }
        &self.layers[i]
    }

    /// Render frame `t` into `out` (Rgba8Unorm, width x height, usable as a
    /// storage texture and render attachment).
    pub fn render(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        renderer: &mut Renderer,
        tl: &Timeline,
        t: f64,
        out: &wgpu::TextureView,
    ) -> Result<(), String> {
        let states = tl.eval(t);
        let world = world_transforms(tl, &states);
        let cam = camera(tl, &states);

        // Sort 2D objects into depth layers and the overlay.
        let mut planes: Vec<f32> = Vec::new();
        let mut overlay = false;
        let mut pinned = states.clone();
        for (i, o) in tl.objects.iter().enumerate() {
            if matches!(o.shape, Shape::Group | Shape::Model { .. } | Shape::Mesh { .. } | Shape::Camera { .. }) || world[i].1 <= 0.0 {
                continue;
            }
            if let Some(p) = o.pin {
                let w = cam.view_proj * world[p].0.col(3);
                if w.w <= 0.0 {
                    pinned[i].opacity = 0.0;
                    continue;
                }
                let ndc = w.truncate() / w.w;
                pinned[i].pos += Vec2::new((ndc.x + 1.0) / 2.0 * tl.width as f32, (1.0 - ndc.y) / 2.0 * tl.height as f32);
            }
            if o.overlay || o.pin.is_some() {
                overlay = true;
            } else if !planes.contains(&plane_z(&world, i)) {
                planes.push(plane_z(&world, i));
            }
        }
        // ponytail: distinct depths cap; merge nearby depths if scenes need more
        if planes.len() > MAX_LAYERS {
            return Err(format!("more than {MAX_LAYERS} different 2D depths in one frame"));
        }
        // Back to front.
        planes.sort_by(|a, b| cam.distance(*b).total_cmp(&cam.distance(*a)));

        let ss = Affine::scale(SS as f64);
        let params = RenderParams { base_color: Color::TRANSPARENT, width: self.width * SS, height: self.height * SS, antialiasing_method: AaConfig::Area };
        let layer_of = |i: usize| -> Option<usize> {
            let o = &tl.objects[i];
            if o.overlay || o.pin.is_some() { Some(planes.len()) } else { planes.iter().position(|z| *z == plane_z(&world, i)) }
        };
        for k in 0..planes.len() + overlay as usize {
            let mut scene = std::mem::take(&mut self.scene);
            scene.reset();
            let s = if k == planes.len() { &pinned } else { &states };
            draw_2d(tl, s, |i| layer_of(i) == Some(k), ss, &mut scene);
            let view = &self.target(device, k).view;
            let r = renderer.render_to_texture(device, queue, &scene, view, &params).map_err(|e| e.to_string());
            self.scene = scene;
            r?;
        }

        // Per object data: world matrix, color, outline, section clip plane, id.
        let mut objs: Vec<u8> = Vec::with_capacity(tl.objects.len() * OBJ_BYTES as usize);
        for (i, (o, (m, opacity, color))) in tl.objects.iter().zip(&world).enumerate() {
            let outline = o.outline.as_ref().map_or([0.0; 4], |l| [l.color[0], l.color[1], l.color[2], l.color[3] * opacity]);
            let clip = clip_plane(tl, &states, &world, i);
            let floats = m.to_cols_array().into_iter().chain([color[0], color[1], color[2], color[3] * opacity]).chain(outline).chain(clip);
            objs.extend(floats.flat_map(f32::to_le_bytes));
            objs.extend([(i + 1) as u32, 0, 0, 0].into_iter().flat_map(u32::to_le_bytes));
        }
        if self.objs.as_ref().is_none_or(|(b, _)| b.size() < objs.len() as u64) {
            let buf = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("objects"),
                size: (objs.len() as u64).max(OBJ_BYTES),
                usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            let bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: None,
                layout: &self.mesh_pipe.get_bind_group_layout(0),
                entries: &[
                    wgpu::BindGroupEntry { binding: 0, resource: self.globals.as_entire_binding() },
                    wgpu::BindGroupEntry { binding: 1, resource: buf.as_entire_binding() },
                ],
            });
            self.objs = Some((buf, bind));
        }
        let (objs_buf, mesh_bind) = self.objs.as_ref().unwrap();
        queue.write_buffer(objs_buf, 0, &objs);

        let light = Vec3::new(-0.45, -0.8, 0.55).normalize();
        let globals: Vec<f32> = cam.view_proj.to_cols_array().into_iter().chain(cam.eye.extend(1.0).to_array()).chain(light.extend(0.4).to_array()).collect();
        queue.write_buffer(&self.globals, 0, &globals.iter().flat_map(|f| f.to_le_bytes()).collect::<Vec<_>>());

        let (w, h) = (tl.width as f32, tl.height as f32);
        let mut mats: Vec<Mat4> = planes.iter().map(|z| cam.view_proj * Mat4::from_translation(Vec3::new(0.0, 0.0, *z)) * Mat4::from_scale(Vec3::new(w, h, 1.0))).collect();
        // Overlay: unit square to the whole screen.
        mats.push(Mat4::from_cols(Vec4::new(2.0, 0.0, 0.0, 0.0), Vec4::new(0.0, -2.0, 0.0, 0.0), Vec4::ZERO, Vec4::new(-1.0, 1.0, 0.0, 1.0)));
        queue.write_buffer(&self.mats, 0, &mats.iter().flat_map(|m| m.to_cols_array()).flat_map(f32::to_le_bytes).collect::<Vec<_>>());

        let max_outline = tl.objects.iter().filter(|o| matches!(o.shape, Shape::Mesh { .. })).filter_map(|o| o.outline.as_ref()).map(|o| o.width).fold(0.0, f32::max);
        let radius = (max_outline * SS as f32 / 2.0).round() as u32;
        queue.write_buffer(&self.params, 0, &[radius, SS, 0, 0].iter().flat_map(|v| v.to_le_bytes()).collect::<Vec<_>>());

        // Upload meshes on first use.
        for o in &tl.objects {
            if let Shape::Mesh { path, part } = &o.shape
                && !self.meshes.contains_key(&(path.clone(), *part))
            {
                let m = crowmation_mesh::load(path)?;
                let p = m.parts.get(*part).ok_or_else(|| format!("{path}: no part {part}"))?;
                let init = |bytes: Vec<u8>, usage| {
                    let b = device.create_buffer(&wgpu::BufferDescriptor { label: None, size: bytes.len() as u64, usage, mapped_at_creation: true });
                    b.slice(..).get_mapped_range_mut().expect("mapped at creation").copy_from_slice(&bytes);
                    b.unmap();
                    b
                };
                let vb = init(p.positions.iter().flatten().flat_map(|f| f.to_le_bytes()).collect(), wgpu::BufferUsages::VERTEX);
                let ib = init(p.indices.iter().flat_map(|i| i.to_le_bytes()).collect(), wgpu::BufferUsages::INDEX);
                self.meshes.insert((path.clone(), *part), (vb, ib, p.indices.len() as u32));
            }
        }

        let mut enc = device.create_command_encoder(&Default::default());
        {
            let [r, g, b, a] = tl.background;
            let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("3d"),
                color_attachments: &[
                    Some(wgpu::RenderPassColorAttachment {
                        view: &self.color,
                        depth_slice: None,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(wgpu::Color { r: r as f64, g: g as f64, b: b as f64, a: a as f64 }),
                            store: wgpu::StoreOp::Store,
                        },
                    }),
                    Some(wgpu::RenderPassColorAttachment {
                        view: &self.ids,
                        depth_slice: None,
                        resolve_target: None,
                        ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT), store: wgpu::StoreOp::Store },
                    }),
                ],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &self.depth,
                    depth_ops: Some(wgpu::Operations { load: wgpu::LoadOp::Clear(1.0), store: wgpu::StoreOp::Discard }),
                    stencil_ops: None,
                }),
                ..Default::default()
            });
            pass.set_pipeline(&self.mesh_pipe);
            pass.set_bind_group(0, mesh_bind, &[]);
            for (i, o) in tl.objects.iter().enumerate() {
                if let Shape::Mesh { path, part } = &o.shape
                    && world[i].1 > 0.0
                {
                    let (vb, ib, n) = &self.meshes[&(path.clone(), *part)];
                    pass.set_vertex_buffer(0, vb.slice(..));
                    pass.set_index_buffer(ib.slice(..), wgpu::IndexFormat::Uint32);
                    pass.draw_indexed(0..*n, 0, i as u32..i as u32 + 1);
                }
            }
            pass.set_bind_group(0, &self.mats_bind, &[]);
            for k in 0..planes.len() {
                pass.set_bind_group(1, &self.layers[k].bind, &[]);
                pass.set_pipeline(&self.layer_pipe);
                pass.draw(0..4, k as u32..k as u32 + 1);
                pass.set_pipeline(&self.layer_id_pipe);
                pass.draw(0..4, k as u32..k as u32 + 1);
            }
        }
        {
            let bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: None,
                layout: &self.resolve_pipe.get_bind_group_layout(0),
                entries: &[
                    wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&self.color) },
                    wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(&self.ids) },
                    wgpu::BindGroupEntry { binding: 2, resource: objs_buf.as_entire_binding() },
                    wgpu::BindGroupEntry { binding: 3, resource: wgpu::BindingResource::TextureView(out) },
                    wgpu::BindGroupEntry { binding: 4, resource: self.params.as_entire_binding() },
                ],
            });
            let mut pass = enc.begin_compute_pass(&Default::default());
            pass.set_pipeline(&self.resolve_pipe);
            pass.set_bind_group(0, &bind, &[]);
            pass.dispatch_workgroups(self.width.div_ceil(8), self.height.div_ceil(8), 1);
        }
        if overlay {
            let k = planes.len();
            let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("overlay"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: out,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations { load: wgpu::LoadOp::Load, store: wgpu::StoreOp::Store },
                })],
                ..Default::default()
            });
            pass.set_pipeline(&self.overlay_pipe);
            pass.set_bind_group(0, &self.mats_bind, &[]);
            pass.set_bind_group(1, &self.layers[k].bind, &[]);
            pass.draw(0..4, k as u32..k as u32 + 1);
        }
        queue.submit([enc.finish()]);
        Ok(())
    }
}

struct Camera {
    view_proj: Mat4,
    eye: Vec3,
}

impl Camera {
    /// Distance from the eye to the 2D plane at depth `z` (planes are parallel).
    fn distance(&self, z: f32) -> f32 {
        (self.eye.z - z).abs()
    }
}

/// Camera from the scene's camera object, or the default one looking at the frame center.
fn camera(tl: &Timeline, states: &[State]) -> Camera {
    let (w, h) = (tl.width as f32, tl.height as f32);
    let (fov, s) = tl
        .objects
        .iter()
        .zip(states)
        .find_map(|(o, s)| match o.shape {
            Shape::Camera { fov } => Some((fov, *s)),
            _ => None,
        })
        .unwrap_or((30f32.to_radians(), State { pos: Vec2::new(w / 2.0, h / 2.0), ..State::default() }));
    let d = (h / 2.0) / (fov / 2.0).tan() / s.scale.max(1e-3);
    let target = s.pos.extend(s.z);
    let q = Quat::from_rotation_y(s.turn.y) * Quat::from_rotation_x(s.turn.x);
    let eye = target + q * Vec3::new(0.0, 0.0, d);
    let up = Quat::from_axis_angle((target - eye).normalize(), s.rotation) * (q * Vec3::NEG_Y);
    let view = glam::camera::lh::view::look_at_mat4(eye, target, up);
    let proj = glam::camera::lh::proj::directx::perspective(fov, w / h, d * 0.01, d * 100.0);
    Camera { view_proj: proj * view, eye }
}

/// Depth of the plane a 2D object is drawn on.
fn plane_z(world: &[(Mat4, f32, [f32; 4])], i: usize) -> f32 {
    world[i].0.w_axis.z
}

/// Section cut for a model part: (normal, d) in world space; points with
/// dot(normal, p) > d are cut away. Nothing is cut for (0, 0, 0, 1).
fn clip_plane(tl: &Timeline, states: &[State], world: &[(Mat4, f32, [f32; 4])], i: usize) -> [f32; 4] {
    let o = &tl.objects[i];
    let Some(p) = o.parent else { return [0.0, 0.0, 0.0, 1.0] };
    let (Shape::Mesh { .. }, Shape::Model { size, section_axis, .. }) = (&o.shape, &tl.objects[p].shape) else {
        return [0.0, 0.0, 0.0, 1.0];
    };
    let keep = states[p].section;
    if keep >= 1.0 {
        return [0.0, 0.0, 0.0, 1.0];
    }
    // Cut from the right (x), the top (y, which points down) or the front (z).
    let (n, len) = match section_axis {
        0 => (Vec3::X, size.x),
        2 => (Vec3::Z, size.z),
        _ => (Vec3::NEG_Y, size.y),
    };
    let m = world[p].0;
    let point = m.transform_point3(n * (len * keep - len / 2.0));
    let normal = m.transform_vector3(n).normalize_or_zero();
    [normal.x, normal.y, normal.z, normal.dot(point)]
}

/// World matrix, opacity and color of every object, parents applied.
fn world_transforms(tl: &Timeline, states: &[State]) -> Vec<(Mat4, f32, [f32; 4])> {
    let mut out: Vec<(Mat4, f32, [f32; 4])> = Vec::with_capacity(states.len());
    for (o, s) in tl.objects.iter().zip(states) {
        // A model's unit scales its parts' vertices; part offsets are already in pixels.
        let unit = match (&o.shape, o.parent.map(|p| &tl.objects[p].shape)) {
            (Shape::Mesh { .. }, Some(Shape::Model { unit, .. })) => *unit,
            _ => 1.0,
        };
        let local = Mat4::from_translation(s.pos.extend(s.z))
            * Mat4::from_quat(Quat::from_rotation_y(s.turn.y) * Quat::from_rotation_x(s.turn.x) * Quat::from_rotation_z(s.rotation))
            * Mat4::from_scale(Vec3::splat(s.scale * unit));
        out.push(match o.parent {
            Some(p) => {
                let (pm, po, pc) = out[p];
                (pm * local, po * s.opacity, std::array::from_fn(|k| pc[k] * s.color[k]))
            }
            None => (local, s.opacity, s.color),
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_camera_maps_canvas_to_pixels() {
        let tl = Timeline { width: 1920, height: 1080, fps: 30, duration: 1.0, background: [0.0; 4], objects: vec![], tracks: vec![] };
        let cam = camera(&tl, &[]);
        for (x, y) in [(0.0, 0.0), (1920.0, 1080.0), (480.0, 270.0)] {
            let c = cam.view_proj * Vec4::new(x, y, 0.0, 1.0);
            let ndc = c.truncate() / c.w;
            let px = ((ndc.x + 1.0) / 2.0 * 1920.0, (1.0 - ndc.y) / 2.0 * 1080.0);
            assert!((px.0 - x).abs() < 0.05 && (px.1 - y).abs() < 0.05, "{px:?} vs {x},{y}");
        }
        // Positive z is toward the viewer: closer than the canvas.
        let near = cam.view_proj * Vec4::new(960.0, 540.0, 100.0, 1.0);
        let mid = cam.view_proj * Vec4::new(960.0, 540.0, 0.0, 1.0);
        assert!(near.z / near.w < mid.z / mid.w);
    }
}
