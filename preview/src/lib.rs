//! Preview window: plays a timeline, scrub with mouse or keys, reloads on save.
//!
//! Keys: Space play/pause, Left/Right step one frame, Home/End jump.
//! Mouse: click or drag anywhere to scrub.

use crowmation_core::Timeline;
use crowmation_render::vello::kurbo::{Affine, Rect};
use crowmation_render::vello::peniko::{Blob, Color, Fill, ImageAlphaType, ImageBrush, ImageData, ImageFormat};
use crowmation_render::vello::util::{RenderContext, RenderSurface};
use crowmation_render::vello::{Renderer, Scene, wgpu};
use crowmation_render::{Three, build_scene, render_params, renderer};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Instant, SystemTime};
use winit::application::ApplicationHandler;
use winit::event::{ElementState, KeyEvent, MouseButton, WindowEvent};
use winit::event_loop::{ActiveEventLoop, EventLoop};
use winit::keyboard::{Key, NamedKey};
use winit::window::{Window, WindowId};

pub fn run(path: PathBuf) -> Result<(), String> {
    let (tl, watched) = load(&path);
    let mut app = App {
        path,
        tl: tl?,
        watched,
        t: 0.0,
        playing: true,
        last: Instant::now(),
        drag: false,
        cursor_x: 0.0,
        ctx: RenderContext::new(),
        gpu: None,
        scene: Scene::new(),
    };
    let el = EventLoop::new().map_err(|e| e.to_string())?;
    el.run_app(&mut app).map_err(|e| e.to_string())
}

type Watched = Vec<(PathBuf, Option<SystemTime>)>;

fn mtime(p: &PathBuf) -> Option<SystemTime> {
    std::fs::metadata(p).and_then(|m| m.modified()).ok()
}

/// Load the scene and note the modified time of every file it read.
fn load(path: &PathBuf) -> (Result<Timeline, String>, Watched) {
    use std::io::IsTerminal;
    let (tl, files) = crowmation_lang::load(path, std::io::stderr().is_terminal());
    (tl, files.into_iter().map(|f| (f.clone(), mtime(&f))).collect())
}

struct Gpu {
    window: Arc<Window>,
    surface: RenderSurface<'static>,
    renderer: Renderer,
    three: Option<View3d>,
}

/// 3D frames are rendered offscreen, then shown through a vello image whose
/// pixels come straight from that texture (no readback).
struct View3d {
    three: Three,
    texture: wgpu::Texture,
    view: wgpu::TextureView,
    image: ImageData,
}

impl View3d {
    fn new(device: &wgpu::Device, w: u32, h: u32) -> Self {
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("3d frame"),
            size: wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::STORAGE_BINDING | wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let view = texture.create_view(&Default::default());
        let image = ImageData {
            data: Blob::new(Arc::new(vec![0u8; (w * h * 4) as usize])),
            format: ImageFormat::Rgba8,
            alpha_type: ImageAlphaType::Alpha,
            width: w,
            height: h,
        };
        Self { three: Three::new(device, w, h), texture, view, image }
    }
}

struct App {
    path: PathBuf,
    tl: Timeline,
    watched: Watched,
    t: f64,
    playing: bool,
    last: Instant,
    drag: bool,
    cursor_x: f64,
    ctx: RenderContext,
    gpu: Option<Gpu>,
    scene: Scene,
}

impl App {
    // ponytail: stats every watched file each frame; switch to notify if scenes import many files
    fn hot_reload(&mut self) {
        if self.watched.iter().all(|(p, t)| mtime(p) == *t) {
            return;
        }
        let (tl, watched) = load(&self.path);
        self.watched = watched;
        match tl {
            Ok(tl) => {
                self.tl = tl;
                self.t = self.t.min(self.tl.duration);
                eprintln!("reloaded {}", self.path.display());
            }
            Err(e) => eprintln!("{e}"), // keep showing the last good timeline
        }
    }

    fn scrub_to_cursor(&mut self) {
        if let Some(g) = &self.gpu {
            let w = g.surface.config.width.max(1) as f64;
            self.t = (self.cursor_x / w).clamp(0.0, 1.0) * self.tl.duration;
            self.playing = false;
        }
    }

    fn step(&mut self, frames: f64) {
        self.playing = false;
        let f = (self.t * self.tl.fps as f64).round() + frames;
        self.t = (f / self.tl.fps as f64).clamp(0.0, self.tl.duration);
    }

    fn redraw(&mut self) {
        self.hot_reload();
        let now = Instant::now();
        if self.playing {
            self.t += (now - self.last).as_secs_f64();
            if self.t > self.tl.duration {
                self.t = 0.0;
            }
        }
        self.last = now;

        let Some(g) = &mut self.gpu else { return };
        let (w, h) = (g.surface.config.width, g.surface.config.height);

        // Letterbox the timeline into the window.
        let fit = (w as f64 / self.tl.width as f64).min(h as f64 / self.tl.height as f64);
        let ox = (w as f64 - self.tl.width as f64 * fit) / 2.0;
        let oy = (h as f64 - self.tl.height as f64 * fit) / 2.0;
        let root = Affine::translate((ox, oy)) * Affine::scale(fit);
        let dev = &self.ctx.devices[g.surface.dev_id];
        if self.tl.is_3d() {
            let (tw, th) = (self.tl.width, self.tl.height);
            if g.three.as_ref().is_none_or(|v| (v.image.width, v.image.height) != (tw, th)) {
                g.three = Some(View3d::new(&dev.device, tw, th));
            }
            let v = g.three.as_mut().unwrap();
            if let Err(e) = v.three.render(&dev.device, &dev.queue, &mut g.renderer, &self.tl, self.t, &v.view) {
                eprintln!("render: {e}");
                return;
            }
            g.renderer.override_image(
                &v.image,
                Some(wgpu::TexelCopyTextureInfoBase { texture: v.texture.clone(), mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All }),
            );
            self.scene.reset();
            self.scene.draw_image(&ImageBrush::new(v.image.clone()), root);
        } else {
            build_scene(&self.tl, self.t, root, &mut self.scene);
        }

        // Scrub bar.
        let frac = if self.tl.duration > 0.0 { self.t / self.tl.duration } else { 0.0 };
        let (w, h) = (w as f64, h as f64);
        self.scene.fill(Fill::NonZero, Affine::IDENTITY, Color::new([1.0, 1.0, 1.0, 0.15]), None, &Rect::new(0.0, h - 6.0, w, h));
        self.scene.fill(Fill::NonZero, Affine::IDENTITY, Color::new([1.0, 1.0, 1.0, 0.8]), None, &Rect::new(0.0, h - 6.0, w * frac, h));

        let params = render_params(&self.tl, g.surface.config.width, g.surface.config.height);
        let params = crowmation_render::vello::RenderParams { base_color: Color::new([0.0, 0.0, 0.0, 1.0]), ..params };
        // Background for the timeline area only, so letterbox bars stay black.
        let mut bg = Scene::new();
        bg.fill(Fill::NonZero, root, Color::new(self.tl.background), None, &Rect::new(0.0, 0.0, self.tl.width as f64, self.tl.height as f64));
        bg.append(&self.scene, None);

        if let Err(e) = g.renderer.render_to_texture(&dev.device, &dev.queue, &bg, &g.surface.target_view, &params) {
            eprintln!("render: {e}");
            return;
        }
        let frame = match g.surface.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(f) | wgpu::CurrentSurfaceTexture::Suboptimal(f) => f,
            wgpu::CurrentSurfaceTexture::Outdated | wgpu::CurrentSurfaceTexture::Lost => {
                self.ctx.configure_surface(&g.surface);
                return;
            }
            _ => return,
        };
        let view = frame.texture.create_view(&Default::default());
        let mut enc = dev.device.create_command_encoder(&Default::default());
        g.surface.blitter.copy(&dev.device, &mut enc, &g.surface.target_view, &view);
        dev.queue.submit([enc.finish()]);
        g.window.pre_present_notify();
        dev.queue.present(frame);

        let name = self.path.file_name().unwrap_or_default().to_string_lossy();
        let state = if self.playing { "" } else { "  [paused]" };
        g.window.set_title(&format!("{name}  {:.2}s / {:.2}s{state}", self.t, self.tl.duration));
    }
}

impl ApplicationHandler for App {
    fn resumed(&mut self, el: &ActiveEventLoop) {
        if self.gpu.is_some() {
            return;
        }
        let attrs = Window::default_attributes()
            .with_title("Crowmation")
            .with_inner_size(winit::dpi::LogicalSize::new(self.tl.width, self.tl.height));
        let window = Arc::new(el.create_window(attrs).expect("create window"));
        let size = window.inner_size();
        let surface = pollster::block_on(self.ctx.create_surface(
            window.clone(),
            size.width.max(1),
            size.height.max(1),
            wgpu::PresentMode::AutoVsync,
        ))
        .expect("create surface");
        let renderer = renderer(&self.ctx.devices[surface.dev_id].device);
        self.gpu = Some(Gpu { window, surface, renderer, three: None });
        self.last = Instant::now();
    }

    fn window_event(&mut self, el: &ActiveEventLoop, _: WindowId, event: WindowEvent) {
        match event {
            WindowEvent::CloseRequested => el.exit(),
            WindowEvent::Resized(size) if size.width > 0 && size.height > 0 => {
                if let Some(g) = &mut self.gpu {
                    self.ctx.resize_surface(&mut g.surface, size.width, size.height);
                }
            }
            WindowEvent::RedrawRequested => self.redraw(),
            WindowEvent::KeyboardInput { event: KeyEvent { logical_key, state: ElementState::Pressed, .. }, .. } => {
                match logical_key {
                    Key::Named(NamedKey::Space) => {
                        if !self.playing && self.t >= self.tl.duration {
                            self.t = 0.0;
                        }
                        self.playing = !self.playing;
                    }
                    Key::Named(NamedKey::ArrowLeft) => self.step(-1.0),
                    Key::Named(NamedKey::ArrowRight) => self.step(1.0),
                    Key::Named(NamedKey::Home) => self.t = 0.0,
                    Key::Named(NamedKey::End) => self.t = self.tl.duration,
                    Key::Named(NamedKey::Escape) => el.exit(),
                    _ => {}
                }
            }
            WindowEvent::MouseInput { state, button: MouseButton::Left, .. } => {
                self.drag = state == ElementState::Pressed;
                if self.drag {
                    self.scrub_to_cursor();
                }
            }
            WindowEvent::CursorMoved { position, .. } => {
                self.cursor_x = position.x;
                if self.drag {
                    self.scrub_to_cursor();
                }
            }
            _ => {}
        }
    }

    fn about_to_wait(&mut self, _: &ActiveEventLoop) {
        // Redraw continuously (vsync paced) so playback and hot reload keep running.
        if let Some(g) = &self.gpu {
            g.window.request_redraw();
        }
    }
}
