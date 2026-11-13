//! Turns a timeline frame into pixels with vello.

mod three;
pub use three::Three;

use crowmation_core::{Shape, State, Timeline};
use crowmation_text as text;
use std::collections::HashMap;
use std::sync::{Arc, LazyLock, Mutex};
use vello::kurbo::{Affine, BezPath, Circle, Line, Point, Rect, RoundedRect, Stroke, Vec2};
use vello::peniko::{Blob, Color, Fill, ImageAlphaType, ImageBrush, ImageData, ImageFormat};

/// Decoded images kept before the cache is cleared.
const IMAGES_MAX: usize = 64;
/// Keyed by path and modified time, so an edited image reloads in preview.
static IMAGES: LazyLock<Mutex<HashMap<(String, Option<std::time::SystemTime>), Result<ImageData, String>>>> =
    LazyLock::new(Default::default);

fn load_image(path: &str) -> Result<ImageData, String> {
    // The compiler checked this canonical path; refuse it if it now resolves elsewhere (a swapped in symlink).
    if std::fs::canonicalize(path).ok().as_deref() != Some(std::path::Path::new(path)) {
        return Err(format!("{path}: moved or replaced since the scene was compiled"));
    }
    let mtime = std::fs::metadata(path).and_then(|m| m.modified()).ok();
    let mut cache = IMAGES.lock().unwrap();
    if cache.len() >= IMAGES_MAX {
        cache.clear();
    }
    cache
        .entry((path.to_string(), mtime))
        .or_insert_with(|| {
            let img = image::open(path).map_err(|e| format!("{path}: {e}"))?.into_rgba8();
            let (width, height) = img.dimensions();
            Ok(ImageData {
                data: Blob::new(Arc::new(img.into_raw())),
                format: ImageFormat::Rgba8,
                alpha_type: ImageAlphaType::Alpha,
                width,
                height,
            })
        })
        .clone()
}

/// Line with an optional filled triangle head at the end.
fn arrow(scene: &mut Scene, xf: Affine, to: Vec2, width: f64, head: f64, color: Color) {
    let len = to.hypot();
    if len == 0.0 {
        return;
    }
    let dir = to / len;
    let head = head.min(len);
    let shaft_end = Point::ZERO + dir * (len - head * 0.8);
    scene.stroke(&Stroke::new(width), xf, color, None, &Line::new(Point::ZERO, shaft_end));
    if head > 0.0 {
        let tip = Point::ZERO + to;
        let back = tip - dir * head;
        let side = Vec2::new(-dir.y, dir.x) * head * 0.5;
        let mut p = BezPath::new();
        p.move_to(tip);
        p.line_to(back + side);
        p.line_to(back - side);
        p.close_path();
        scene.fill(Fill::NonZero, xf, color, None, &p);
    }
}
use vello::util::RenderContext;
use vello::{AaConfig, RenderParams, Renderer, RendererOptions, Scene, wgpu};

/// Build the vello scene for time `t` of a 2D timeline. `root` maps timeline
/// pixels to target pixels.
pub fn build_scene(tl: &Timeline, t: f64, root: Affine, scene: &mut Scene) {
    scene.reset();
    draw_2d(tl, &tl.eval(t), |_| true, root, scene);
}

/// Draw the 2D objects for which `keep(index)` holds. 3D objects are skipped.
pub(crate) fn draw_2d(tl: &Timeline, states: &[State], keep: impl Fn(usize) -> bool, root: Affine, scene: &mut Scene) {
    // Each object's transform and opacity with its groups applied (parents come first).
    let mut world: Vec<(Affine, f32)> = Vec::with_capacity(states.len());
    for (i, (obj, s)) in tl.objects.iter().zip(states).enumerate() {
        let local = Affine::translate((s.pos.x as f64, s.pos.y as f64)) * Affine::rotate(s.rotation as f64) * Affine::scale(s.scale as f64);
        let (xf, opacity) = match obj.parent {
            Some(p) => (world[p].0 * local, world[p].1 * s.opacity),
            None => (local, s.opacity),
        };
        world.push((xf, opacity));
        if opacity <= 0.0 || !keep(i) {
            continue;
        }
        let xf = root * xf;
        let [r, g, b, a] = s.color;
        let color = Color::new([r, g, b, a * opacity]);
        let outline = obj.outline.as_ref().map(|o| {
            let [r, g, b, a] = o.color;
            (Stroke::new(o.width as f64), Color::new([r, g, b, a * opacity]))
        });
        match &obj.shape {
            Shape::Rect { size, radius } => {
                let (w, h) = (size.x as f64 / 2.0, size.y as f64 / 2.0);
                let shape = RoundedRect::new(-w, -h, w, h, *radius as f64);
                scene.fill(Fill::NonZero, xf, color, None, &shape);
                if let Some((st, c)) = &outline {
                    scene.stroke(st, xf, *c, None, &shape);
                }
            }
            Shape::Circle { radius } => {
                let shape = Circle::new((0.0, 0.0), *radius as f64);
                scene.fill(Fill::NonZero, xf, color, None, &shape);
                if let Some((st, c)) = &outline {
                    scene.stroke(st, xf, *c, None, &shape);
                }
            }
            Shape::Arrow { to, width, head } => {
                arrow(scene, xf, Vec2::new(to.x as f64, to.y as f64), *width as f64, *head as f64, color);
            }
            Shape::Image { path, size } => match load_image(path) {
                Ok(img) => {
                    let (iw, ih) = (img.width as f64, img.height as f64);
                    let (w, h) = (size.x as f64, size.y as f64);
                    let ixf = xf * Affine::translate((-w / 2.0, -h / 2.0)) * Affine::scale_non_uniform(w / iw, h / ih);
                    scene.draw_image(&ImageBrush::new(img).multiply_alpha(opacity), ixf);
                    if let Some((st, c)) = &outline {
                        scene.stroke(st, xf, *c, None, &Rect::new(-w / 2.0, -h / 2.0, w / 2.0, h / 2.0));
                    }
                }
                Err(e) => eprintln!("image: {e}"),
            },
            Shape::Typst { source, align } => {
                // The compiler already checked the source, so errors here mean a hand written timeline.
                let layout = match crowmation_text::layout(source) {
                    Ok(l) => l,
                    Err(e) => {
                        eprintln!("text: {e}");
                        continue;
                    }
                };
                let xf = xf * Affine::translate((-layout.width * *align as f64, -layout.height / 2.0));
                let pick = |c: &Option<Color>| c.map_or(color, |c| c.multiply_alpha(a * opacity));
                for item in &layout.items {
                    match item {
                        text::Item::Glyphs { font, size, color: c, xf: ixf, glyphs } => scene
                            .draw_glyphs(font)
                            .font_size(*size)
                            .transform(xf * *ixf)
                            .brush(pick(c))
                            .draw(Fill::NonZero, glyphs.iter().copied()),
                        text::Item::Fill { path, color: c, xf: ixf } => {
                            scene.fill(Fill::NonZero, xf * *ixf, pick(c), None, path)
                        }
                        text::Item::Stroke { path, width, color: c, xf: ixf } => {
                            scene.stroke(&Stroke::new(*width), xf * *ixf, pick(c), None, path)
                        }
                    }
                }
            }
            Shape::Group | Shape::Model { .. } | Shape::Mesh { .. } | Shape::Camera { .. } => {}
        }
    }
}

pub fn render_params(tl: &Timeline, width: u32, height: u32) -> RenderParams {
    RenderParams {
        base_color: Color::new(tl.background),
        width,
        height,
        antialiasing_method: AaConfig::Area,
    }
}

pub use vello;

pub fn renderer(device: &wgpu::Device) -> Renderer {
    Renderer::new(
        device,
        RendererOptions { antialiasing_support: vello::AaSupport::area_only(), ..Default::default() },
    )
    .expect("vello renderer")
}

/// Offscreen renderer: timeline frames in, RGBA8 or I420 pixels out.
pub struct Headless {
    device: wgpu::Device,
    queue: wgpu::Queue,
    renderer: Renderer,
    scene: Scene,
    texture: wgpu::Texture,
    view: wgpu::TextureView,
    width: u32,
    height: u32,
    i420: Option<I420>,
    three: Option<Three>,
}

/// GPU RGBA to I420 conversion plus a ring of readback buffers.
struct I420 {
    pipeline: wgpu::ComputePipeline,
    bind: wgpu::BindGroup,
    out: wgpu::Buffer,
    ring: Vec<wgpu::Buffer>,
}

/// Readback buffers in flight; the CPU works on frame N-2 while the GPU renders frame N.
const RING: usize = 3;

impl Headless {
    pub fn new(width: u32, height: u32) -> Result<Self, String> {
        let mut ctx = RenderContext::new();
        let id = pollster::block_on(ctx.device(None)).ok_or("no compatible GPU found")?;
        let handle = ctx.devices.swap_remove(id);
        let renderer = renderer(&handle.device);
        let (device, queue) = (handle.device, handle.queue);
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("frame"),
            size: wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::STORAGE_BINDING
                | wgpu::TextureUsages::COPY_SRC
                | wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        });
        let view = texture.create_view(&Default::default());
        Ok(Self { device, queue, renderer, scene: Scene::new(), texture, view, width, height, i420: None, three: None })
    }

    fn draw(&mut self, tl: &Timeline, t: f64) -> Result<(), String> {
        if tl.is_3d() {
            let three = self.three.get_or_insert_with(|| Three::new(&self.device, self.width, self.height));
            return three.render(&self.device, &self.queue, &mut self.renderer, tl, t, &self.view);
        }
        build_scene(tl, t, Affine::IDENTITY, &mut self.scene);
        let params = render_params(tl, self.width, self.height);
        self.renderer
            .render_to_texture(&self.device, &self.queue, &self.scene, &self.view, &params)
            .map_err(|e| e.to_string())
    }

    fn map_wait(&self, buf: &wgpu::Buffer, idx: wgpu::SubmissionIndex) -> Result<(), String> {
        let (tx, rx) = std::sync::mpsc::channel();
        buf.map_async(wgpu::MapMode::Read, .., move |r| tx.send(r).unwrap());
        self.device
            .poll(wgpu::PollType::Wait { submission_index: Some(idx), timeout: None })
            .map_err(|e| e.to_string())?;
        rx.recv().unwrap().map_err(|e| e.to_string())
    }

    /// Render one frame at time `t` as tightly packed RGBA8 rows.
    pub fn render(&mut self, tl: &Timeline, t: f64) -> Result<Vec<u8>, String> {
        self.draw(tl, t)?;
        let padded = (self.width * 4).next_multiple_of(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT);
        let buf = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("readback"),
            size: (padded * self.height) as u64,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut enc = self.device.create_command_encoder(&Default::default());
        enc.copy_texture_to_buffer(
            self.texture.as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer: &buf,
                layout: wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(padded), rows_per_image: None },
            },
            self.texture.size(),
        );
        let idx = self.queue.submit([enc.finish()]);
        self.map_wait(&buf, idx)?;
        let row = (self.width * 4) as usize;
        let out = buf.get_mapped_range(..).map_err(|e| e.to_string())?.chunks(padded as usize).flat_map(|c| &c[..row]).copied().collect();
        Ok(out)
    }

    fn i420_setup(&mut self) -> Result<(), String> {
        if self.width % 8 != 0 || self.height % 2 != 0 {
            return Err(format!("export needs width divisible by 8 and even height, got {}x{}", self.width, self.height));
        }
        let size = (self.width * self.height * 3 / 2) as u64;
        let module = self.device.create_shader_module(wgpu::include_wgsl!("i420.wgsl"));
        let pipeline = self.device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("i420"),
            layout: None,
            module: &module,
            entry_point: Some("main"),
            compilation_options: Default::default(),
            cache: None,
        });
        let out = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("i420"),
            size,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });
        let bind = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout: &pipeline.get_bind_group_layout(0),
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&self.view) },
                wgpu::BindGroupEntry { binding: 1, resource: out.as_entire_binding() },
            ],
        });
        let ring = (0..RING)
            .map(|_| {
                self.device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("i420 readback"),
                    size,
                    usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                    mapped_at_creation: false,
                })
            })
            .collect();
        self.i420 = Some(I420 { pipeline, bind, out, ring });
        Ok(())
    }

    /// Render the frames in `order` (frame i at time i/fps) as I420 (BT.709,
    /// limited range), calling `sink(i, bytes)` in that order. GPU work for
    /// later frames overlaps with `sink` for earlier ones.
    ///
    /// `order` may interleave `lanes` independent runs of frames (lane =
    /// position % lanes, see crowmation_encode::frame_order). A frame whose
    /// state equals the previous frame of its lane is not rendered again.
    pub fn render_i420(
        &mut self,
        tl: &Timeline,
        order: &[u64],
        lanes: usize,
        mut sink: impl FnMut(u64, &[u8]) -> Result<(), String>,
    ) -> Result<(), String> {
        if self.i420.is_none() {
            self.i420_setup()?;
        }
        let lanes = lanes.max(1);
        // (frame, lane, submission); no submission marks a hold that reuses the lane's last frame.
        let mut pending: std::collections::VecDeque<(u64, usize, Option<(usize, wgpu::SubmissionIndex)>)> = Default::default();
        let mut prev_states: Vec<Option<Vec<crowmation_core::State>>> = vec![None; lanes];
        let mut last: Vec<Vec<u8>> = vec![Vec::new(); lanes];
        let mut slot_next = 0;
        for pos in 0..=order.len() {
            if let Some(&i) = order.get(pos) {
                let lane = pos % lanes;
                let t = i as f64 / tl.fps as f64;
                let states = tl.eval(t);
                if prev_states[lane].as_ref() == Some(&states) {
                    pending.push_back((i, lane, None));
                } else {
                    self.draw(tl, t)?;
                    let c = self.i420.as_ref().unwrap();
                    let slot = slot_next % RING;
                    slot_next += 1;
                    let mut enc = self.device.create_command_encoder(&Default::default());
                    {
                        let mut pass = enc.begin_compute_pass(&Default::default());
                        pass.set_pipeline(&c.pipeline);
                        pass.set_bind_group(0, &c.bind, &[]);
                        pass.dispatch_workgroups((self.width / 8).div_ceil(8), (self.height / 2).div_ceil(8), 1);
                    }
                    enc.copy_buffer_to_buffer(&c.out, 0, &c.ring[slot], 0, c.out.size());
                    pending.push_back((i, lane, Some((slot, self.queue.submit([enc.finish()])))));
                    prev_states[lane] = Some(states);
                }
            }
            // Hand over frames up to the oldest submission once the ring is full; at the end, all of them.
            let done = pos == order.len();
            let in_flight = pending.iter().filter(|p| p.2.is_some()).count();
            if in_flight == RING || pending.len() > 4 * RING || done {
                while let Some((f, lane, sub)) = pending.pop_front() {
                    let real = sub.is_some();
                    if let Some((slot, idx)) = sub {
                        let c = self.i420.as_ref().unwrap();
                        let buf = &c.ring[slot];
                        self.map_wait(buf, idx)?;
                        last[lane].clear();
                        last[lane].extend_from_slice(&buf.get_mapped_range(..).map_err(|e| e.to_string())?);
                        buf.unmap();
                    }
                    sink(f, &last[lane])?;
                    if real && !done {
                        break;
                    }
                }
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Needs a GPU.
    #[test]
    fn holds_reuse_frames_and_motion_does_not() {
        let tl = Timeline::from_ron(
            "(width: 64, height: 32, fps: 10, duration: 0.5, background: (0.0, 0.0, 0.0, 1.0),
              objects: [(shape: Rect(size: (8.0, 8.0)))],
              tracks: [(object: 0, start: 0.0, end: 0.2, anim: Move(from: (8.0, 16.0), to: (40.0, 16.0)))])",
        )
        .unwrap();
        let mut frames = Vec::new();
        Headless::new(64, 32).unwrap().render_i420(&tl, &[0, 1, 2, 3, 4], 1, |i, f| Ok(frames.push((i, f.to_vec())))).unwrap();
        assert_eq!(frames.iter().map(|f| f.0).collect::<Vec<_>>(), [0, 1, 2, 3, 4]);
        assert_ne!(frames[0].1, frames[1].1, "moving");
        assert_eq!(frames[2].1, frames[4].1, "held after the move ends");
        assert_eq!(frames[0].1.len(), 64 * 32 * 3 / 2);
    }

    /// Needs a GPU.
    #[test]
    fn interleaved_lanes_match_in_order_frames() {
        let tl = Timeline::from_ron(
            "(width: 64, height: 32, fps: 10, duration: 0.8, background: (0.0, 0.0, 0.0, 1.0),
              objects: [(shape: Rect(size: (8.0, 8.0)))],
              tracks: [(object: 0, start: 0.1, end: 0.3, anim: Move(from: (8.0, 16.0), to: (40.0, 16.0))),
                       (object: 0, start: 0.5, end: 0.7, anim: Move(from: (40.0, 16.0), to: (8.0, 16.0)))])",
        )
        .unwrap();
        let mut r = Headless::new(64, 32).unwrap();
        let mut seq = vec![Vec::new(); 8];
        r.render_i420(&tl, &(0..8).collect::<Vec<_>>(), 1, |i, f| Ok(seq[i as usize] = f.to_vec())).unwrap();
        let mut lanes = vec![Vec::new(); 8];
        let mut order = Vec::new();
        r.render_i420(&tl, &[0, 4, 1, 5, 2, 6, 3, 7], 2, |i, f| Ok({ order.push(i); lanes[i as usize] = f.to_vec() })).unwrap();
        assert_eq!(order, [0, 4, 1, 5, 2, 6, 3, 7], "sink gets frames in the given order");
        assert_eq!(seq, lanes);
    }
}
