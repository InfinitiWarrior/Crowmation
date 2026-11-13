//! Compiled timeline (IR): a flat, fully resolved scene. Every frame is a pure
//! function of time, so frames can be rendered in any order or in parallel.

use glam::{Vec2, Vec3};

fn half() -> f32 {
    0.5
}
fn is_half(v: &f32) -> bool {
    *v == 0.5
}
fn one_u8() -> u8 {
    1
}
use serde::{Deserialize, Serialize};

/// Largest frame width or height. 3D scenes render at 2x internally, which
/// stays within the GPU texture size every wgpu device supports (8192).
pub const MAX_SIZE: u32 = 4096;
pub const MAX_FPS: u32 = 240;
/// Seconds.
pub const MAX_DURATION: f64 = 3600.0;

/// sRGB, straight alpha, components 0..=1.
pub type Color = [f32; 4];

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Timeline {
    pub width: u32,
    pub height: u32,
    pub fps: u32,
    /// Seconds.
    pub duration: f64,
    pub background: Color,
    pub objects: Vec<Object>,
    /// Sorted by `start` after loading; later tracks win on conflicts.
    pub tracks: Vec<Track>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Object {
    pub shape: Shape,
    /// State before any track touches it.
    #[serde(default)]
    pub state: State,
    /// Stroke around the shape. Fades with the object's opacity.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub outline: Option<Outline>,
    /// Transforms and opacity are relative to this object (a `Group` or `Model`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent: Option<usize>,
    /// 3D only: 2D object drawn on top of everything at the screen position of
    /// this object, offset by its own `pos`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pin: Option<usize>,
    /// 3D only: 2D object fixed to the screen, drawn on top, ignoring the camera.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub overlay: bool,
}

impl Object {
    pub fn new(shape: Shape, state: State) -> Self {
        Self { shape, state, outline: None, parent: None, pin: None, overlay: false }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Outline {
    pub color: Color,
    pub width: f32,
}

/// Sizes in pixels, centered on the object's position.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum Shape {
    Rect {
        size: Vec2,
        /// Corner radius.
        #[serde(default)]
        radius: f32,
    },
    Circle { radius: f32 },
    /// Line from the object's position to position + `to`, with an arrow head
    /// of size `head` (0 for a plain line).
    Arrow { to: Vec2, width: f32, head: f32 },
    /// PNG or JPEG file (absolute path), drawn at `size` pixels.
    Image { path: String, size: Vec2 },
    /// Typst markup, laid out at 1pt = 1px and placed on the position: `align`
    /// 0 puts the left edge there, 0.5 the center (default), 1 the right edge.
    Typst {
        source: String,
        #[serde(default = "half", skip_serializing_if = "is_half")]
        align: f32,
    },
    /// Draws nothing; its children (`Object::parent`) are placed, rotated,
    /// scaled and faded with it. Component instances are groups.
    Group,
    /// 3D group for the parts of a glTF file. Child meshes are scaled by
    /// `unit` (pixels per model unit); child positions are in pixels.
    /// `size` is the model's bounding box in pixels; `section_axis` (0 x,
    /// 1 y, 2 z) is the axis `State::section` cuts along.
    Model {
        unit: f32,
        #[serde(default)]
        size: Vec3,
        #[serde(default = "one_u8")]
        section_axis: u8,
    },
    /// One part (`Mesh::parts[part]`) of a .glb file (absolute path), placed
    /// relative to its parent model. Lit flat color: the state's color.
    Mesh { path: String, part: usize },
    /// The 3D camera. Its state: `pos`/`z` is the point it looks at, `turn`
    /// is (elevation, azimuth), `scale` zooms, `rotation` rolls. With the
    /// defaults the z = 0 plane fills the frame at 1 unit = 1 pixel.
    Camera { fov: f32 },
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct State {
    /// Pixels from the top left corner.
    pub pos: Vec2,
    /// Depth in pixels, positive toward the viewer. Used only in 3D scenes.
    pub z: f32,
    /// Radians, clockwise.
    pub rotation: f32,
    /// 3D only: rotation around the x axis (tilt) and then the vertical axis (orbit), radians.
    pub turn: Vec2,
    pub scale: f32,
    pub opacity: f32,
    pub color: Color,
    /// Models only: fraction of the model kept by a section cut, 1 (all) to 0.
    pub section: f32,
}

impl Default for State {
    fn default() -> Self {
        Self { pos: Vec2::ZERO, z: 0.0, rotation: 0.0, turn: Vec2::ZERO, scale: 1.0, opacity: 1.0, color: [1.0; 4], section: 1.0 }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Track {
    /// Index into `Timeline::objects`.
    pub object: usize,
    pub start: f64,
    pub end: f64,
    #[serde(default)]
    pub ease: Ease,
    pub anim: Anim,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum Anim {
    Move { from: Vec2, to: Vec2 },
    Rotate { from: f32, to: f32 },
    Scale { from: f32, to: f32 },
    Fade { from: f32, to: f32 },
    Color { from: Color, to: Color },
    Z { from: f32, to: f32 },
    /// `turn.x`: rotation around the x axis.
    Tilt { from: f32, to: f32 },
    /// `turn.y`: rotation around the vertical axis.
    Orbit { from: f32, to: f32 },
    Section { from: f32, to: f32 },
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub enum Ease {
    #[default]
    Linear,
    In,
    Out,
    InOut,
}

impl Ease {
    /// Cubic easing, `t` in 0..=1.
    pub fn apply(self, t: f32) -> f32 {
        match self {
            Ease::Linear => t,
            Ease::In => t * t * t,
            Ease::Out => 1.0 - (1.0 - t).powi(3),
            Ease::InOut if t < 0.5 => 4.0 * t * t * t,
            Ease::InOut => 1.0 - (-2.0 * t + 2.0).powi(3) / 2.0,
        }
    }
}

#[derive(Debug)]
pub enum LoadError {
    Ron(ron::error::SpannedError),
    Invalid(String),
}

impl std::fmt::Display for LoadError {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        match self {
            LoadError::Ron(e) => write!(f, "{e}"),
            LoadError::Invalid(e) => write!(f, "invalid timeline: {e}"),
        }
    }
}

impl std::error::Error for LoadError {}

impl Timeline {
    pub fn from_ron(src: &str) -> Result<Self, LoadError> {
        let mut tl: Timeline = ron::from_str(src).map_err(LoadError::Ron)?;
        tl.validate().map_err(LoadError::Invalid)?;
        Ok(tl)
    }

    pub fn to_ron(&self) -> String {
        ron::ser::to_string_pretty(self, Default::default()).expect("timeline always serializes")
    }

    /// Checks invariants and sorts tracks by start time (stable).
    pub fn validate(&mut self) -> Result<(), String> {
        if !(1..=MAX_SIZE).contains(&self.width) || !(1..=MAX_SIZE).contains(&self.height) {
            return Err(format!("width and height must be 1 to {MAX_SIZE}, got {}x{}", self.width, self.height));
        }
        if !(1..=MAX_FPS).contains(&self.fps) {
            return Err(format!("fps must be 1 to {MAX_FPS}, got {}", self.fps));
        }
        if !(0.0..=MAX_DURATION).contains(&self.duration) {
            return Err(format!("duration must be 0 to {MAX_DURATION} seconds, got {}", self.duration));
        }
        for (i, t) in self.tracks.iter().enumerate() {
            if t.object >= self.objects.len() {
                return Err(format!("track {i}: object {} does not exist", t.object));
            }
            if !(t.start <= t.end) || !t.end.is_finite() {
                return Err(format!("track {i}: start {} is after end {}", t.start, t.end));
            }
        }
        for (i, o) in self.objects.iter().enumerate() {
            if o.parent.is_some_and(|p| p >= i) || o.pin.is_some_and(|p| p >= self.objects.len()) {
                return Err(format!("object {i}: parent must come before it and pin must exist"));
            }
        }
        if self.objects.iter().filter(|o| matches!(o.shape, Shape::Camera { .. })).count() > 1 {
            return Err("only one camera is supported".into());
        }
        self.tracks.sort_by(|a, b| a.start.total_cmp(&b.start));
        Ok(())
    }

    /// A scene is 3D when it has a camera or a model; otherwise it renders as plain 2D.
    pub fn is_3d(&self) -> bool {
        self.objects.iter().any(|o| matches!(o.shape, Shape::Camera { .. } | Shape::Model { .. } | Shape::Mesh { .. }))
    }

    pub fn frame_count(&self) -> u64 {
        (self.duration * self.fps as f64).ceil() as u64
    }

    /// State of every object at time `t` (seconds). Requires sorted tracks.
    // ponytail: scans every track per frame; index tracks by time or evaluate on GPU (DESIGN 7) when scenes get big
    pub fn eval(&self, t: f64) -> Vec<State> {
        let mut states: Vec<State> = self.objects.iter().map(|o| o.state).collect();
        for tr in self.tracks.iter().take_while(|tr| tr.start <= t) {
            tr.apply(&mut states[tr.object], t);
        }
        states
    }
}

impl Track {
    /// Apply this track's value at time `t` (at or after its start) to `s`.
    pub fn apply(&self, s: &mut State, t: f64) {
        let tr = self;
        let p = if tr.end > tr.start { ((t - tr.start) / (tr.end - tr.start)).min(1.0) } else { 1.0 };
        let p = tr.ease.apply(p as f32);
        {
            match &tr.anim {
                Anim::Move { from, to } => s.pos = from.lerp(*to, p),
                Anim::Rotate { from, to } => s.rotation = lerp(*from, *to, p),
                Anim::Scale { from, to } => s.scale = lerp(*from, *to, p),
                Anim::Fade { from, to } => s.opacity = lerp(*from, *to, p),
                Anim::Color { from, to } => s.color = std::array::from_fn(|i| lerp(from[i], to[i], p)),
                Anim::Z { from, to } => s.z = lerp(*from, *to, p),
                Anim::Tilt { from, to } => s.turn.x = lerp(*from, *to, p),
                Anim::Orbit { from, to } => s.turn.y = lerp(*from, *to, p),
                Anim::Section { from, to } => s.section = lerp(*from, *to, p),
            }
        }
    }
}

fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

#[cfg(test)]
mod tests {
    use super::*;

    const SRC: &str = r#"(
        width: 100, height: 100, fps: 10, duration: 2.0,
        background: (0.0, 0.0, 0.0, 1.0),
        objects: [(shape: Circle(radius: 5.0)), (shape: Rect(size: (1.0, 1.0)), outline: Some((color: (1.0, 0.0, 0.0, 1.0), width: 2.0)))],
        tracks: [
            (object: 0, start: 1.0, end: 2.0, anim: Move(from: (10.0, 0.0), to: (20.0, 0.0))),
            (object: 0, start: 0.0, end: 1.0, anim: Move(from: (0.0, 0.0), to: (10.0, 0.0))),
        ],
    )"#;

    #[test]
    fn eval_over_time() {
        let tl = Timeline::from_ron(SRC).unwrap();
        assert_eq!(tl.tracks[0].start, 0.0, "tracks sorted on load");
        assert_eq!(tl.frame_count(), 20);
        assert_eq!(tl.eval(0.0)[0].pos, Vec2::new(0.0, 0.0));
        assert_eq!(tl.eval(0.5)[0].pos, Vec2::new(5.0, 0.0));
        assert_eq!(tl.eval(1.5)[0].pos, Vec2::new(15.0, 0.0));
        assert_eq!(tl.eval(9.0)[0].pos, Vec2::new(20.0, 0.0), "holds after end");
        assert_eq!(tl.eval(0.0)[0].scale, 1.0, "defaults apply");
    }

    #[test]
    fn ron_round_trip() {
        let tl = Timeline::from_ron(SRC).unwrap();
        assert_eq!(Timeline::from_ron(&tl.to_ron()).unwrap(), tl);
    }

    #[test]
    fn rejects_bad_object() {
        let bad = SRC.replace("object: 0, start: 1.0", "object: 3, start: 1.0");
        assert!(matches!(Timeline::from_ron(&bad), Err(LoadError::Invalid(_))));
    }

    #[test]
    fn rejects_out_of_range_settings() {
        for (a, b) in [("duration: 2.0", "duration: inf"), ("duration: 2.0", "duration: 1e9"), ("width: 100", "width: 100000"), ("fps: 10", "fps: 100000")] {
            assert!(matches!(Timeline::from_ron(&SRC.replace(a, b)), Err(LoadError::Invalid(_))), "{b}");
        }
    }

    #[test]
    fn ease_endpoints() {
        for e in [Ease::Linear, Ease::In, Ease::Out, Ease::InOut] {
            assert_eq!(e.apply(0.0), 0.0);
            assert_eq!(e.apply(1.0), 1.0);
        }
        assert_eq!(Ease::InOut.apply(0.5), 0.5);
    }
}
