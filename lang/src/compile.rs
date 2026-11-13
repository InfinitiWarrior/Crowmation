//! AST to compiled timeline: evaluates expressions with unit checks, expands
//! components and repeats, resolves timing, then fills in each animation's
//! start value.

use crate::parse::{Expr, Item, Op, Path, Prop, Stmt, Timing, Word};
use crate::{Diag, Span};
use crowmation_core::{Anim, Ease, MAX_DURATION, MAX_FPS, MAX_SIZE, Object, Shape, State, Timeline, Track};
use glam::{Vec2, Vec3};
use std::collections::HashMap;

type R<T> = Result<T, Diag>;

/// Guards against typos like `repeat 1000000000`; scenes stay fast to compile.
pub const MAX_REPEAT: f64 = 10_000.0;
/// Bounds on text objects, so Typst layout stays fast.
pub const MAX_TEXT_SIZE: f64 = 1000.0;
pub const MAX_TEXT_LEN: usize = 100_000;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Dim {
    None,
    Time,
    Angle,
}

#[derive(Clone, Debug)]
pub enum Val {
    Num(f64, Dim),
    Vec2(f64, f64),
    Vec3(f64, f64, f64),
    Color([f32; 4]),
    Word(String),
    Str(String),
    List(Vec<Val>),
}

fn describe(v: &Val) -> String {
    match v {
        Val::Num(_, Dim::None) => "a number".into(),
        Val::Num(_, Dim::Time) => "a time".into(),
        Val::Num(_, Dim::Angle) => "an angle".into(),
        Val::Vec2(..) => "a point".into(),
        Val::Vec3(..) => "a 3D point".into(),
        Val::Color(_) => "a color".into(),
        Val::Word(w) => format!("`{w}`"),
        Val::Str(s) => format!("the string \"{s}\""),
        Val::List(l) => format!("a list of {}", l.len()),
    }
}

fn want(d: Dim) -> &'static str {
    match d {
        Dim::None => "a number",
        Dim::Time => "a time like `2s` or `500ms`",
        Dim::Angle => "an angle like `45deg`",
    }
}

fn type_err(span: &Span, expected: &str, got: &Val) -> Diag {
    match got {
        Val::Word(w) => Diag::new(span.clone(), format!("unknown name `{w}`, expected {expected}")),
        _ => Diag::new(span.clone(), format!("expected {expected}, found {}", describe(got))),
    }
}

const COMMON_PROPS: &[&str] = &["at", "color", "opacity", "rotation", "scale"];
const ACTION_PROPS: &[&str] = &["to", "by", "duration", "ease", "stagger"];
/// Placing an object next to another one instead of at a point.
const LAYOUT: &[&str] = &["below", "above", "left_of", "right_of", "gap"];
/// Props every 2D shape takes in addition to its own.
const PLACE_2D: &[&str] = &["z", "layer", "pin", "below", "above", "left_of", "right_of", "gap"];
pub(crate) const SHAPES: &[&str] = &["rect", "circle", "arrow", "line", "image", "text", "code", "math", "model", "camera"];
pub(crate) const ACTIONS: &[&str] =
    &["move", "rotate", "scale", "fade", "fade_in", "fade_out", "color", "orbit", "tilt", "explode", "highlight", "shake", "section"];
/// Functions usable in expressions, like `len(xs)` or `bytes("dump.bin")`.
pub(crate) const FUNCTIONS: &[&str] =
    &["len", "floor", "ceil", "round", "abs", "min", "max", "mod", "str", "hex", "join", "slice", "ascii", "bytes", "file", "table"];
/// Largest data file `bytes`, `file` and `table` read.
pub const MAX_DATA_FILE: u64 = 1 << 20;
/// Most items a list made by a function can have.
pub const MAX_LIST: usize = 65_536;
pub(crate) const KEYWORDS: &[&str] = &["let", "wait", "repeat", "at", "after", "with", "scene", "component", "import"];

#[derive(Clone)]
enum Change {
    Move { to: Option<Vec2>, by: Option<Vec2> },
    Rotate { to: Option<f32>, by: Option<f32> },
    Scale { to: Option<f32>, by: Option<f32> },
    Fade(f32),
    FadeIn,
    Color([f32; 4]),
    Z { to: Option<f32>, by: Option<f32> },
    /// Axis 0 tilts, axis 1 orbits.
    Turn { axis: usize, to: Option<f32>, by: Option<f32> },
    /// Scale up by the factor and back.
    Pulse(f32),
    /// Color to this and back.
    Flash([f32; 4]),
    /// Side to side by `amp` pixels, `times` times, back to the start.
    Shake { amp: f32, times: u32 },
    Section { to: Option<f32>, by: Option<f32> },
}

struct Pending {
    object: usize,
    start: f64,
    end: f64,
    ease: Ease,
    change: Change,
}

struct Props(Vec<(String, Val, Span)>);

impl Props {
    fn get(&self, k: &str) -> Option<&(String, Val, Span)> {
        self.0.iter().find(|p| p.0 == k)
    }
    fn num(&self, k: &str, d: Dim) -> R<Option<f64>> {
        match self.get(k) {
            None => Ok(None),
            Some((_, Val::Num(n, dd), _)) if *dd == d => Ok(Some(*n)),
            Some((_, v, s)) => Err(type_err(s, want(d), v)),
        }
    }
    fn vec2(&self, k: &str) -> R<Option<Vec2>> {
        match self.get(k) {
            None => Ok(None),
            Some((_, Val::Vec2(x, y), _)) => Ok(Some(Vec2::new(*x as f32, *y as f32))),
            Some((_, v, s)) => Err(type_err(s, "a point like `(x, y)`", v)),
        }
    }
    fn color(&self, k: &str) -> R<Option<[f32; 4]>> {
        match self.get(k) {
            None => Ok(None),
            Some((_, Val::Color(c), _)) => Ok(Some(*c)),
            Some((_, v, s)) => Err(type_err(s, "a color like `#ff0000`", v)),
        }
    }
    fn str(&self, k: &str) -> R<Option<(String, Span)>> {
        match self.get(k) {
            None => Ok(None),
            Some((_, Val::Str(t), s)) => Ok(Some((t.clone(), s.clone()))),
            Some((_, v, s)) => Err(type_err(s, "a string in quotes", v)),
        }
    }
    fn ease(&self) -> R<Ease> {
        match self.get("ease") {
            None => Ok(Ease::InOut),
            Some((_, Val::Word(w), s)) => match w.as_str() {
                "linear" => Ok(Ease::Linear),
                "in" => Ok(Ease::In),
                "out" => Ok(Ease::Out),
                "in_out" => Ok(Ease::InOut),
                _ => Err(Diag::new(s.clone(), format!("unknown easing `{w}`, expected linear, in, out or in_out"))),
            },
            Some((_, v, s)) => Err(type_err(s, "an easing like `in_out`", v)),
        }
    }
}

struct Component<'a> {
    file: usize,
    params: &'a [Word],
    body: &'a [Stmt],
}

struct Compiler<'a> {
    diags: Vec<Diag>,
    /// Path of each source file (None for in-memory source) and the scene folder.
    paths: &'a [Option<std::path::PathBuf>],
    root: Option<std::path::PathBuf>,
    comps: HashMap<String, Component<'a>>,
    /// File the statements being compiled come from.
    file: usize,
    env: HashMap<String, Val>,
    /// Inside a component instance: name prefix like `row.`, the instance's
    /// group object (new objects are its children), when its actions start
    /// (`at` inside a component is relative to it) and the call chain.
    prefix: String,
    parent: Option<usize>,
    base: f64,
    stack: Vec<String>,
    names: HashMap<String, usize>,
    objects: Vec<Object>,
    pending: Vec<Pending>,
    /// Start and end of the most recent group (an action plus its `with`s).
    group: (f64, f64),
    size: Vec2,
    fps: u32,
    background: [f32; 4],
    duration: Option<f64>,
    /// Model object -> its parts and their centers (pixels), for `explode`.
    models: HashMap<usize, Vec<(usize, Vec3)>>,
    /// Camera declared without `at`: it looks at the frame center.
    camera_centered: Option<usize>,
    /// Data files read by `bytes`, `file` and `table`, for hot reload.
    deps: std::cell::RefCell<Vec<std::path::PathBuf>>,
}

/// `files[0]` is the scene file; the rest are imports. Data files the scene
/// reads are added to `deps`.
pub fn compile(
    files: &[Vec<Item>],
    paths: &[Option<std::path::PathBuf>],
    diags: &mut Vec<Diag>,
    deps: &mut Vec<std::path::PathBuf>,
) -> Option<Timeline> {
    let mut c = Compiler {
        diags: Vec::new(),
        paths,
        root: paths.first().and_then(|p| p.as_ref()).and_then(|p| p.parent()).map(|p| p.to_path_buf()),
        comps: HashMap::new(),
        file: 0,
        env: HashMap::new(),
        prefix: String::new(),
        parent: None,
        base: 0.0,
        stack: Vec::new(),
        names: HashMap::new(),
        objects: Vec::new(),
        pending: Vec::new(),
        group: (0.0, 0.0),
        size: Vec2::new(1920.0, 1080.0),
        fps: 60,
        background: [0.0, 0.0, 0.0, 1.0],
        duration: None,
        models: HashMap::new(),
        camera_centered: None,
        deps: Default::default(),
    };
    let mut scene = None;
    for (file, items) in files.iter().enumerate() {
        c.file = file;
        for item in items {
            match item {
                Item::Scene { body, span } if file == 0 => match scene {
                    None => scene = Some(body),
                    Some(_) => c.push(Diag::new(span.clone(), "only one scene per file is supported")),
                },
                Item::Scene { span, .. } => {
                    c.push(Diag::new(span.clone(), "imported files can only contain components and imports"))
                }
                Item::Component { name, params, body } => c.define(file, name, params, body),
                Item::Import { .. } => {}
            }
        }
    }
    c.file = 0;
    match scene {
        Some(body) => c.block(body),
        None => c.push(Diag::new(0..0, "no `scene { ... }` found")),
    }
    deps.extend(c.deps.borrow_mut().drain(..));
    if !c.diags.is_empty() {
        diags.append(&mut c.diags);
        return None;
    }
    // Catches limits that depend on the whole scene, like a total length over the cap.
    let mut tl = c.finish();
    if let Err(e) = tl.validate() {
        diags.push(Diag::new(0..0, e));
        return None;
    }
    Some(tl)
}

impl<'a> Compiler<'a> {
    fn push(&mut self, mut d: Diag) {
        d.file = self.file;
        // Errors inside a repeat would otherwise show once per iteration.
        if !self.diags.contains(&d) {
            self.diags.push(d);
        }
    }

    fn define(&mut self, file: usize, name: &Word, params: &'a [Word], body: &'a [Stmt]) {
        let n = name.name.as_str();
        if SHAPES.contains(&n) || ACTIONS.contains(&n) || KEYWORDS.contains(&n) || crate::parse::SETTINGS.contains(&n) {
            return self.push(Diag::new(name.span.clone(), format!("`{n}` is a built in name, pick another component name")));
        }
        if self.comps.contains_key(n) {
            return self.push(Diag::new(name.span.clone(), format!("component `{n}` is defined twice")));
        }
        if let Some(p) = params.iter().find(|p| COMMON_PROPS.contains(&p.name.as_str()) || LAYOUT.contains(&p.name.as_str())) {
            return self.push(Diag::new(p.span.clone(), format!("`{}` is reserved for placing the component", p.name)));
        }
        self.comps.insert(n.to_string(), Component { file, params, body });
    }

    fn block(&mut self, stmts: &'a [Stmt]) {
        for s in stmts {
            if let Err(d) = self.stmt(s) {
                self.push(d);
            }
        }
    }

    fn in_component(&self) -> bool {
        !self.stack.is_empty()
    }

    fn stmt(&mut self, stmt: &'a Stmt) -> R<()> {
        match stmt {
            Stmt::Let { name, value } => {
                let v = self.eval(value)?;
                self.env.insert(name.name.clone(), v);
            }
            Stmt::Wait(e) => {
                let d = self.num(e, Dim::Time)?;
                self.group.1 += d;
                self.group.0 = self.group.1;
            }
            Stmt::Setting { key, .. } if self.in_component() => {
                return Err(Diag::new(key.span.clone(), format!("`{}` can only be set in the scene", key.name)));
            }
            Stmt::Setting { key, value } => self.setting(key, value)?,
            Stmt::Repeat { count, var, body } => {
                let n = self.num(count, Dim::None)?;
                if n < 0.0 || n.fract() != 0.0 || n > MAX_REPEAT {
                    return Err(Diag::new(count.span(), format!("repeat count must be a whole number from 0 to {MAX_REPEAT}")));
                }
                let saved = self.env.clone();
                for i in 0..n as u32 {
                    self.env.insert(var.name.clone(), Val::Num(i as f64, Dim::None));
                    self.block(body);
                }
                self.env = saved;
            }
            Stmt::Cmd { timing, head, target, props } => {
                let h = head.name.as_str();
                let is_action = ACTIONS.contains(&h);
                if let Some((_, span)) = timing.as_ref().filter(|_| SHAPES.contains(&h)) {
                    return Err(Diag::new(span.clone(), "only actions and component instances take timing; declare objects without `at`, `after` or `with`"));
                }
                if SHAPES.contains(&h) {
                    let name = self.decl_name(target)?;
                    if let Err(d) = self.object(head, &name, props) {
                        // Keep the name so later actions get checked instead of all saying "no object named".
                        if !self.names.contains_key(&name) {
                            self.names.insert(name, self.objects.len());
                            self.objects.push(Object::new(Shape::Circle { radius: 0.0 }, State::default()));
                        }
                        return Err(d);
                    }
                } else if is_action {
                    self.action(timing.as_ref().map(|t| &t.0), head, target, props)?;
                } else if self.comps.contains_key(h) {
                    self.instance(timing.as_ref().map(|t| &t.0), head, target, props)?;
                } else {
                    let mut all: Vec<&str> = SHAPES.iter().chain(ACTIONS).copied().collect();
                    all.extend(self.comps.keys().map(String::as_str));
                    let msg = match all.iter().min_by_key(|w| edit_distance(h, w)).filter(|w| edit_distance(h, w) <= 2) {
                        Some(w) => format!("unknown command `{h}`, did you mean `{w}`?"),
                        None => format!("unknown command `{h}`, expected one of: {}", all.join(", ")),
                    };
                    return Err(Diag::new(head.span.clone(), msg));
                }
            }
        }
        Ok(())
    }

    fn setting(&mut self, key: &Word, value: &Expr) -> R<()> {
        let v = self.eval(value)?;
        let s = value.span();
        match (key.name.as_str(), &v) {
            ("size", Val::Vec2(w, h)) if (1.0..=MAX_SIZE as f64).contains(w) && (1.0..=MAX_SIZE as f64).contains(h) => {
                self.size = Vec2::new(w.round() as f32, h.round() as f32)
            }
            ("size", Val::Vec2(..)) => return Err(Diag::new(s, format!("width and height must be 1 to {MAX_SIZE}"))),
            ("size", _) => return Err(type_err(&s, "a size like `(1920, 1080)`", &v)),
            ("fps", Val::Num(n, Dim::None)) if (1.0..=MAX_FPS as f64).contains(n) && n.fract() == 0.0 => self.fps = *n as u32,
            ("fps", _) => return Err(type_err(&s, &format!("a whole number of frames per second, 1 to {MAX_FPS}"), &v)),
            ("background", Val::Color(c)) => self.background = *c,
            ("background", _) => return Err(type_err(&s, "a color like `#101218`", &v)),
            ("duration", Val::Num(n, Dim::Time)) if (0.0..=MAX_DURATION).contains(n) => self.duration = Some(*n),
            ("duration", Val::Num(_, Dim::Time)) => return Err(Diag::new(s, format!("duration must be 0s to {MAX_DURATION}s"))),
            ("duration", _) => return Err(type_err(&s, want(Dim::Time), &v)),
            _ => unreachable!("parser only produces known settings"),
        }
        Ok(())
    }

    /// `c[2]` → `c[2]`, `row.c[i]` → `row.c[3]`.
    fn path_name(&self, p: &Path) -> R<String> {
        let mut out = String::new();
        for (i, (w, idx)) in p.segs.iter().enumerate() {
            if i > 0 {
                out.push('.');
            }
            out.push_str(&w.name);
            if let Some(e) = idx {
                let n = self.num(e, Dim::None)?;
                if n < 0.0 || n.fract() != 0.0 {
                    return Err(Diag::new(e.span(), format!("index must be a whole number, got {n}")));
                }
                out.push_str(&format!("[{n}]"));
            }
        }
        Ok(out)
    }

    /// Full name for a new object or instance, with the component prefix.
    fn decl_name(&self, p: &Path) -> R<String> {
        if p.segs.len() > 1 {
            return Err(Diag::new(p.span.clone(), "declared names cannot contain `.`"));
        }
        let name = format!("{}{}", self.prefix, self.path_name(p)?);
        if self.names.contains_key(&name) {
            return Err(Diag::new(p.span.clone(), format!("`{name}` is already declared")));
        }
        Ok(name)
    }

    fn props(&self, list: &[Prop], allowed: &[&str], what: &str) -> R<Props> {
        let mut out: Vec<(String, Val, Span)> = Vec::new();
        for p in list {
            if !allowed.contains(&p.key.name.as_str()) {
                let k = &p.key.name;
                let msg = match allowed.iter().min_by_key(|w| edit_distance(k, w)).filter(|w| edit_distance(k, w) <= 2) {
                    Some(w) => format!("`{what}` has no property `{k}`, did you mean `{w}`?"),
                    None => format!("`{what}` has no property `{k}`, expected one of: {}", allowed.join(", ")),
                };
                return Err(Diag::new(p.key.span.clone(), msg));
            }
            if out.iter().any(|o| o.0 == p.key.name) {
                return Err(Diag::new(p.key.span.clone(), format!("`{}` is set twice", p.key.name)));
            }
            out.push((p.key.name.clone(), self.eval(&p.value)?, p.value.span()));
        }
        Ok(Props(out))
    }

    fn object(&mut self, head: &Word, name: &str, list: &[Prop]) -> R<()> {
        match head.name.as_str() {
            "model" => return self.model(head, name, list),
            "camera" => return self.camera(head, name, list),
            _ => {}
        }
        let p = self.props(list, &props_of(&head.name), &head.name)?;
        let missing = |k: &str| Diag::new(head.span.clone(), format!("`{}` needs `{k}`", head.name));
        let mut at = p.vec2("at")?;
        let shape = match head.name.as_str() {
            "rect" => Shape::Rect {
                size: p.vec2("size")?.ok_or_else(|| missing("size"))?,
                radius: p.num("corner", Dim::None)?.unwrap_or(0.0) as f32,
            },
            "arrow" | "line" => {
                let from = p.vec2("from")?.ok_or_else(|| missing("from"))?;
                let to = p.vec2("to")?.ok_or_else(|| missing("to"))?;
                at = Some(from);
                let default_head = if head.name == "arrow" { 16.0 } else { 0.0 };
                Shape::Arrow {
                    to: to - from,
                    width: p.num("width", Dim::None)?.unwrap_or(4.0) as f32,
                    head: p.num("head", Dim::None)?.unwrap_or(default_head) as f32,
                }
            }
            "image" => {
                let (rel, span) = p.str("content")?.ok_or_else(|| missing("a file path"))?;
                let path = self.asset(&rel, &["png", "jpg", "jpeg"]).map_err(|m| Diag::new(span.clone(), m))?;
                let (w, h) = image::image_dimensions(&path).map_err(|e| Diag::new(span, format!("`{rel}`: {e}")))?;
                let dims = Vec2::new(w as f32, h as f32);
                let size = match (p.vec2("size")?, p.num("width", Dim::None)?) {
                    (Some(s), None) => s,
                    (None, Some(w)) => Vec2::new(w as f32, w as f32 * dims.y / dims.x),
                    (None, None) => dims,
                    (Some(_), Some(_)) => return Err(Diag::new(head.span.clone(), "`image` takes `size` or `width`, not both")),
                };
                Shape::Image { path, size }
            }
            "circle" => Shape::Circle { radius: p.num("radius", Dim::None)?.ok_or_else(|| missing("radius"))? as f32 },
            kind => {
                let (text, span) = p.str("content")?.ok_or_else(|| missing("content"))?;
                let lang = p.str("lang")?;
                let size = p.num("size", Dim::None)?.unwrap_or(if kind == "math" { 40.0 } else { 32.0 });
                if !(1.0..=MAX_TEXT_SIZE).contains(&size) {
                    return Err(Diag::new(p.get("size").unwrap().2.clone(), format!("text size must be 1 to {MAX_TEXT_SIZE}")));
                }
                if text.len() > MAX_TEXT_LEN {
                    return Err(Diag::new(span, format!("text is longer than {MAX_TEXT_LEN} bytes")));
                }
                let source = typst_source(kind, &text, size, lang.as_ref().map(|l| l.0.as_str()))
                    .map_err(|m| Diag::new(span.clone(), m))?;
                crowmation_text::layout(&source).map_err(|e| Diag::new(span, format!("typst: {e}")))?;
                let align = match p.get("align") {
                    None => 0.5,
                    Some((_, Val::Word(w), _)) if w == "left" => 0.0,
                    Some((_, Val::Word(w), _)) if w == "center" => 0.5,
                    Some((_, Val::Word(w), _)) if w == "right" => 1.0,
                    Some((_, v, s)) => return Err(type_err(s, "`left`, `center` or `right`", v)),
                };
                Shape::Typst { source, align }
            }
        };
        let d = State::default();
        let outline = match p.color("outline")? {
            Some(color) => Some(crowmation_core::Outline { color, width: p.num("outline_width", Dim::None)?.unwrap_or(2.0) as f32 }),
            None if p.get("outline_width").is_some() => return Err(missing("outline")),
            None => None,
        };
        let state = State { pos: at.unwrap_or(d.pos), ..self.common_state(&p)? };
        let pin = match p.get("pin") {
            None => None,
            Some((_, Val::Word(w), s)) => Some(self.lookup(w).ok_or_else(|| Diag::new(s.clone(), format!("no object named `{w}` to pin to")))?),
            Some((_, v, s)) => return Err(type_err(s, "an object name like `router.flash_chip`", v)),
        };
        let overlay = match p.get("layer") {
            None => false,
            Some((_, Val::Word(w), _)) if w == "overlay" => true,
            Some((_, Val::Word(w), _)) if w == "world" => false,
            Some((_, v, s)) => return Err(type_err(s, "`overlay` or `world`", v)),
        };
        let i = self.objects.len();
        self.names.insert(name.to_string(), i);
        self.objects.push(Object { outline, pin, overlay, parent: self.parent, ..Object::new(shape, state) });
        self.place(i, &p, at.is_some() && !matches!(head.name.as_str(), "arrow" | "line"))
    }

    /// An object by name, preferring the current component's own parts.
    fn lookup(&self, name: &str) -> Option<usize> {
        self.names.get(&format!("{}{name}", self.prefix)).or_else(|| self.names.get(name)).copied()
    }

    /// Bounding box (center, half size) of object `i` in its parent's frame.
    /// Rotation is ignored.
    fn local_box(&self, i: usize) -> Option<(Vec2, Vec2)> {
        let o = &self.objects[i];
        let s = &o.state;
        let (c, h) = match &o.shape {
            Shape::Rect { size, .. } | Shape::Image { size, .. } => (Vec2::ZERO, *size / 2.0),
            Shape::Circle { radius } => (Vec2::ZERO, Vec2::splat(*radius)),
            Shape::Arrow { to, width, .. } => (*to / 2.0, to.abs() / 2.0 + Vec2::splat(width / 2.0)),
            Shape::Typst { source, align } => {
                let l = crowmation_text::layout(source).ok()?;
                let (w, h) = (l.width as f32, l.height as f32);
                (Vec2::new((0.5 - align) * w, 0.0), Vec2::new(w, h) / 2.0)
            }
            Shape::Model { size, .. } => (Vec2::ZERO, size.truncate() / 2.0),
            Shape::Group => {
                let boxes: Vec<_> = (i + 1..self.objects.len()).filter(|&j| self.objects[j].parent == Some(i)).filter_map(|j| self.local_box(j)).collect();
                let lo = boxes.iter().map(|(c, h)| c - h).reduce(Vec2::min)?;
                let hi = boxes.iter().map(|(c, h)| c + h).reduce(Vec2::max)?;
                ((lo + hi) / 2.0, (hi - lo) / 2.0)
            }
            Shape::Mesh { .. } | Shape::Camera { .. } => return None,
        };
        Some((s.pos + c * s.scale, h * s.scale))
    }

    /// Origin and scale of the frame the children of `parent` live in.
    fn frame(&self, parent: Option<usize>) -> (Vec2, f32) {
        match parent {
            None => (Vec2::ZERO, 1.0),
            Some(p) => {
                let (o, k) = self.frame(self.objects[p].parent);
                let s = &self.objects[p].state;
                (o + s.pos * k, k * s.scale)
            }
        }
    }

    /// Put object `i` next to another one: `below title gap 20`.
    fn place(&mut self, i: usize, p: &Props, at_given: bool) -> R<()> {
        let set: Vec<&(String, Val, Span)> = ["below", "above", "left_of", "right_of"].iter().filter_map(|d| p.get(d)).collect();
        let Some(first) = set.first() else {
            return match p.get("gap") {
                Some((_, _, s)) => Err(Diag::new(s.clone(), "`gap` goes with `below`, `above`, `left_of` or `right_of`")),
                None => Ok(()),
            };
        };
        let (dir, val, span) = (first.0.as_str(), &first.1, &first.2);
        if set.len() > 1 {
            return Err(Diag::new(set[1].2.clone(), "use only one of `below`, `above`, `left_of` and `right_of`"));
        }
        if at_given {
            return Err(Diag::new(span.clone(), format!("`at` and `{dir}` both place the object; use one")));
        }
        let Val::Word(target) = val else { return Err(type_err(span, "an object name", val)) };
        let t = self.lookup(target).ok_or_else(|| Diag::new(span.clone(), format!("no object named `{target}`")))?;
        let no_size = |n: &str| Diag::new(span.clone(), format!("`{n}` has no size to place against"));
        let (tc, th) = self.local_box(t).ok_or_else(|| no_size(target))?;
        let (to, tk) = self.frame(self.objects[t].parent);
        let (tc, th) = (to + tc * tk, th * tk);
        let gap = p.num("gap", Dim::None)?.unwrap_or(24.0) as f32;
        let saved = std::mem::take(&mut self.objects[i].state.pos);
        let own = self.local_box(i);
        self.objects[i].state.pos = saved;
        let (oc, oh) = own.ok_or_else(|| no_size("this object"))?;
        let (fo, fk) = self.frame(self.objects[i].parent);
        let wh = oh * fk;
        let want = match dir {
            "below" => Vec2::new(tc.x, tc.y + th.y + gap + wh.y),
            "above" => Vec2::new(tc.x, tc.y - th.y - gap - wh.y),
            "right_of" => Vec2::new(tc.x + th.x + gap + wh.x, tc.y),
            _ => Vec2::new(tc.x - th.x - gap - wh.x, tc.y),
        };
        self.objects[i].state.pos = (want - fo) / fk - oc;
        Ok(())
    }

    /// Rotation, scale, opacity, color and z from props, defaults otherwise.
    fn common_state(&self, p: &Props) -> R<State> {
        let d = State::default();
        Ok(State {
            z: p.num("z", Dim::None)?.map_or(d.z, |v| v as f32),
            rotation: p.num("rotation", Dim::Angle)?.map_or(d.rotation, |v| v as f32),
            scale: p.num("scale", Dim::None)?.map_or(d.scale, |v| v as f32),
            opacity: p.num("opacity", Dim::None)?.map_or(d.opacity, |v| v as f32),
            color: p.color("color")?.unwrap_or(d.color),
            ..d
        })
    }

    /// `model router "models/router.glb" size 600`: a group object plus one
    /// object per glTF part, named `router.<part>`.
    fn model(&mut self, head: &Word, name: &str, list: &[Prop]) -> R<()> {
        let p = self.props(list, &props_of("model"), "model")?;
        let (rel, span) = p.str("content")?.ok_or_else(|| Diag::new(head.span.clone(), "`model` needs a .glb file path"))?;
        let path = self.asset(&rel, &["glb"]).map_err(|m| Diag::new(span.clone(), m))?;
        let mesh = crowmation_mesh::load(&path).map_err(|m| Diag::new(span, m))?;
        let size = p.num("size", Dim::None)?.unwrap_or(400.0) as f32;
        let outline = crowmation_core::Outline {
            color: p.color("outline")?.unwrap_or([0.05, 0.06, 0.08, 1.0]),
            width: p.num("outline_width", Dim::None)?.unwrap_or(2.0) as f32,
        };
        let state = State { pos: p.vec2("at")?.unwrap_or_default(), ..self.common_state(&p)? };
        let unit = size / mesh.extent.max_element().max(1e-9);
        let model = self.objects.len();
        self.names.insert(name.to_string(), model);
        let shape = Shape::Model { unit, size: mesh.extent * unit, section_axis: 1 };
        self.objects.push(Object { parent: self.parent, ..Object::new(shape, state) });
        let mut parts = Vec::new();
        for (i, part) in mesh.parts.iter().enumerate() {
            let c = part.center * unit;
            let st = State { pos: c.truncate(), z: c.z, color: part.color, ..State::default() };
            parts.push((self.objects.len(), c));
            self.names.insert(format!("{name}.{}", part.name), self.objects.len());
            self.objects.push(Object {
                outline: Some(outline.clone()),
                parent: Some(model),
                ..Object::new(Shape::Mesh { path: path.clone(), part: i }, st)
            });
        }
        self.models.insert(model, parts);
        Ok(())
    }

    fn camera(&mut self, head: &Word, name: &str, list: &[Prop]) -> R<()> {
        if self.objects.iter().any(|o| matches!(o.shape, Shape::Camera { .. })) {
            return Err(Diag::new(head.span.clone(), "a scene can only have one camera"));
        }
        let p = self.props(list, &props_of("camera"), "camera")?;
        let fov = p.num("fov", Dim::Angle)?.unwrap_or(30f64.to_radians());
        if !(0.01..3.0).contains(&fov) {
            return Err(Diag::new(p.get("fov").unwrap().2.clone(), "fov must be between 1deg and 170deg"));
        }
        let at = p.vec2("at")?;
        if at.is_none() {
            self.camera_centered = Some(self.objects.len());
        }
        let state = State { pos: at.unwrap_or_default(), ..self.common_state(&p)? };
        self.names.insert(name.to_string(), self.objects.len());
        self.objects.push(Object::new(Shape::Camera { fov: fov as f32 }, state));
        Ok(())
    }

    /// Resolve an asset path relative to the current file; it must stay inside the scene folder.
    fn asset(&self, rel: &str, exts: &[&str]) -> Result<String, String> {
        let (Some(dir), Some(root)) = (self.paths.get(self.file).and_then(|p| p.as_ref()).and_then(|p| p.parent()), &self.root) else {
            return Err("assets need the scene to be loaded from a file".into());
        };
        resolve_asset(dir, root, rel, exts)
    }

    /// A component instance: a group object whose children are the
    /// component's objects. Its actions run from the current point in time.
    fn instance(&mut self, timing: Option<&Timing>, head: &Word, target: &Path, list: &[Prop]) -> R<()> {
        let comp = &self.comps[&head.name];
        let (file, params, body) = (comp.file, comp.params, comp.body);
        if self.stack.contains(&head.name) {
            return Err(Diag::new(head.span.clone(), format!("component `{}` uses itself, which is not allowed", head.name)));
        }
        let name = self.decl_name(target)?;
        let place = ["at", "rotation", "scale", "opacity", "z"];
        let allowed: Vec<&str> = params.iter().map(|p| p.name.as_str()).chain(place).chain(LAYOUT.iter().copied()).collect();
        let p = self.props(list, &allowed, &head.name)?;
        if let Some(m) = params.iter().find(|w| p.get(&w.name).is_none()) {
            return Err(Diag::new(head.span.clone(), format!("`{}` needs `{}`", head.name, m.name)));
        }
        let at = p.vec2("at")?;
        let state = State { pos: at.unwrap_or_default(), ..self.common_state(&p)? };
        let group = self.objects.len();
        self.names.insert(name.clone(), group);
        self.objects.push(Object { parent: self.parent, ..Object::new(Shape::Group, state) });
        let skip: Vec<&str> = place.iter().chain(LAYOUT).copied().collect();
        let env: HashMap<String, Val> = p.0.iter().filter(|(k, ..)| !skip.contains(&k.as_str())).map(|(k, v, _)| (k.clone(), v.clone())).collect();

        // The component's own actions start now, or at the instance's timing.
        let start = match timing {
            Some(t) => {
                let s = self.start_time(Some(t), head, 0.0)?;
                self.group = (s, s);
                s
            }
            None => self.group.1,
        };
        let saved = (
            std::mem::replace(&mut self.file, file),
            std::mem::replace(&mut self.env, env),
            std::mem::replace(&mut self.prefix, format!("{name}.")),
            std::mem::replace(&mut self.parent, Some(group)),
            std::mem::replace(&mut self.base, start),
        );
        self.stack.push(head.name.clone());
        self.block(body);
        self.stack.pop();
        (self.file, self.env, self.prefix, self.parent, self.base) = saved;
        self.place(group, &p, at.is_some())
    }

    /// What an action's target names: one object (or group), or every
    /// element of a declared list like `cell` for `cell[0]`, `cell[1]`, ...
    fn targets(&self, name: &str) -> Option<Vec<usize>> {
        if let Some(o) = self.lookup(name) {
            return Some(vec![o]);
        }
        let prefixed = format!("{}{name}[", self.prefix);
        let mut items: Vec<usize> = self
            .names
            .iter()
            .filter(|(k, _)| k.strip_prefix(&prefixed).is_some_and(|rest| rest.ends_with(']') && !rest.contains('.')))
            .map(|(_, &v)| v)
            .collect();
        if items.is_empty() && !self.prefix.is_empty() {
            let plain = format!("{name}[");
            items = self.names.iter().filter(|(k, _)| k.strip_prefix(&plain).is_some_and(|r| r.ends_with(']') && !r.contains('.'))).map(|(_, &v)| v).collect();
        }
        items.sort();
        (!items.is_empty()).then_some(items)
    }

    /// Direct children of a group, in declaration order.
    fn children(&self, i: usize) -> Vec<usize> {
        (i + 1..self.objects.len()).filter(|&j| self.objects[j].parent == Some(i)).collect()
    }

    /// The drawn objects under `i` (itself if it is not a group).
    fn leaves(&self, i: usize) -> Vec<usize> {
        match self.objects[i].shape {
            Shape::Group | Shape::Model { .. } => self.children(i).into_iter().flat_map(|c| self.leaves(c)).collect(),
            _ => vec![i],
        }
    }

    fn action(&mut self, timing: Option<&Timing>, head: &Word, target: &Path, list: &[Prop]) -> R<()> {
        let name = self.path_name(target)?;
        let found = self.targets(&name).ok_or_else(|| Diag::new(target.span.clone(), format!("no object named `{name}`")))?;
        let h = head.name.as_str();
        let p = self.props(list, &props_of(h), h)?;
        let duration = p.num("duration", Dim::Time)?.unwrap_or(if matches!(h, "highlight" | "shake") { 0.6 } else { 1.0 });
        if duration < 0.0 {
            return Err(Diag::new(p.get("duration").unwrap().2.clone(), "duration cannot be negative"));
        }
        let stagger = p.num("stagger", Dim::Time)?;
        if stagger.is_some_and(|s| s < 0.0) {
            return Err(Diag::new(p.get("stagger").unwrap().2.clone(), "stagger cannot be negative"));
        }
        // With `stagger`, a single group staggers over its parts.
        let members = match (&found[..], stagger) {
            ([g], Some(_)) if matches!(self.objects[*g].shape, Shape::Group) => self.children(*g),
            _ => found,
        };
        let ease = p.ease()?;
        let to_or_by = |ok: bool| if ok { Ok(()) } else { Err(Diag::new(head.span.clone(), format!("`{h}` needs exactly one of `to` or `by`"))) };
        let need_to = || Diag::new(head.span.clone(), format!("`{h}` needs `to`"));
        let num_to_by = |d: Dim| -> R<(Option<f32>, Option<f32>)> {
            let (to, by) = (p.num("to", d)?, p.num("by", d)?);
            to_or_by(to.is_some() != by.is_some())?;
            Ok((to.map(|v| v as f32), by.map(|v| v as f32)))
        };
        // A 3D move also animates depth.
        let mut depth = None;
        let mut point = |k: &str| -> R<Option<Vec2>> {
            match p.get(k) {
                Some((_, Val::Vec3(x, y, z), _)) => {
                    depth = Some((k == "to", *z as f32));
                    Ok(Some(Vec2::new(*x as f32, *y as f32)))
                }
                _ => p.vec2(k),
            }
        };
        // What happens to each member: changes on the member itself, and on
        // its drawn objects (colors live on shapes, not on groups).
        let mut own: Vec<Change> = Vec::new();
        let mut on_leaves: Vec<Change> = Vec::new();
        match h {
            "move" => {
                let (to, by) = (point("to")?, point("by")?);
                to_or_by(to.is_some() != by.is_some())?;
                if let Some((is_to, z)) = depth {
                    own.push(Change::Z { to: is_to.then_some(z), by: (!is_to).then_some(z) });
                }
                own.push(Change::Move { to, by });
            }
            "orbit" | "tilt" => {
                let (to, by) = num_to_by(Dim::Angle)?;
                own.push(Change::Turn { axis: (h == "orbit") as usize, to, by });
            }
            "rotate" => {
                let (to, by) = num_to_by(Dim::Angle)?;
                own.push(Change::Rotate { to, by });
            }
            "scale" => {
                let (to, by) = num_to_by(Dim::None)?;
                own.push(Change::Scale { to, by });
            }
            "fade" => own.push(Change::Fade(p.num("to", Dim::None)?.ok_or_else(need_to)? as f32)),
            "fade_in" => own.push(Change::FadeIn),
            "fade_out" => own.push(Change::Fade(0.0)),
            "color" => on_leaves.push(Change::Color(p.color("to")?.ok_or_else(need_to)?)),
            "highlight" => {
                own.push(Change::Pulse(p.num("scale", Dim::None)?.unwrap_or(1.12) as f32));
                if let Some(c) = p.color("color")? {
                    on_leaves.push(Change::Flash(c));
                }
            }
            "shake" => {
                let times = p.num("times", Dim::None)?.unwrap_or(3.0);
                if !(1.0..=50.0).contains(&times) || times.fract() != 0.0 {
                    return Err(Diag::new(p.get("times").unwrap().2.clone(), "`times` must be a whole number from 1 to 50"));
                }
                own.push(Change::Shake { amp: p.num("by", Dim::None)?.unwrap_or(12.0) as f32, times: times as u32 });
            }
            "section" => {
                let (to, by) = num_to_by(Dim::None)?;
                let axis = match p.get("axis") {
                    None => 1,
                    Some((_, Val::Word(w), _)) if w == "x" => 0,
                    Some((_, Val::Word(w), _)) if w == "y" => 1,
                    Some((_, Val::Word(w), _)) if w == "z" => 2,
                    Some((_, v, s)) => return Err(type_err(s, "`x`, `y` or `z`", v)),
                };
                for &m in &members {
                    let Shape::Model { section_axis, .. } = &mut self.objects[m].shape else {
                        return Err(Diag::new(target.span.clone(), format!("`{name}` is not a model")));
                    };
                    *section_axis = axis;
                }
                own.push(Change::Section { to, by });
            }
            "explode" => {
                let k = p.num("by", Dim::None)?.ok_or_else(|| Diag::new(head.span.clone(), "`explode` needs `by`, like `by 0.5`"))? as f32;
                let mut moves = Vec::new();
                for &m in &members {
                    let parts = self.models.get(&m).ok_or_else(|| Diag::new(target.span.clone(), format!("`{name}` is not a model")))?;
                    // Each part moves away from the model center by `k` times its offset.
                    moves.extend(parts.iter().map(|(o, c)| (*o, *c * k)));
                }
                let start = self.start_time(timing, head, duration)?;
                for (o, c) in moves {
                    for change in [Change::Move { to: None, by: Some(c.truncate()) }, Change::Z { to: None, by: Some(c.z) }] {
                        self.pending.push(Pending { object: o, start, end: start + duration, ease, change });
                    }
                }
                return Ok(());
            }
            _ => unreachable!("ACTIONS and this match agree"),
        }
        let step = stagger.unwrap_or(0.0);
        let start = self.start_time(timing, head, duration + step * members.len().saturating_sub(1) as f64)?;
        for (k, &m) in members.iter().enumerate() {
            let t = start + step * k as f64;
            for change in &own {
                self.pending.push(Pending { object: m, start: t, end: t + duration, ease, change: change.clone() });
            }
            for leaf in self.leaves(m) {
                for change in &on_leaves {
                    self.pending.push(Pending { object: leaf, start: t, end: t + duration, ease, change: change.clone() });
                }
            }
        }
        Ok(())
    }

    /// Start of an action from its timing; updates the current group.
    fn start_time(&mut self, timing: Option<&Timing>, head: &Word, duration: f64) -> R<f64> {
        let start = match timing {
            // Inside a component, `at` counts from when the instance's actions begin.
            Some(Timing::At(e)) => self.base + self.num(e, Dim::Time)?,
            Some(Timing::After(Some(e))) => self.group.1 + self.num(e, Dim::Time)?,
            Some(Timing::With) => self.group.0,
            None | Some(Timing::After(None)) => self.group.1,
        };
        let end = start + duration;
        if start < 0.0 {
            return Err(Diag::new(head.span.clone(), format!("action would start at {start}s, before the scene begins")));
        }
        self.group = match timing {
            Some(Timing::With) => (self.group.0, self.group.1.max(end)),
            _ => (start, end),
        };
        Ok(start)
    }

    fn finish(mut self) -> Timeline {
        self.pending.sort_by(|a, b| a.start.total_cmp(&b.start));
        // An object whose first action is fade_in starts hidden.
        for (i, obj) in self.objects.iter_mut().enumerate() {
            if let Some(first) = self.pending.iter().find(|p| p.object == i)
                && matches!(first.change, Change::FadeIn)
            {
                obj.state.opacity = 0.0;
            }
        }
        if let Some(cam) = self.camera_centered {
            self.objects[cam].state.pos = self.size / 2.0;
        }
        let end = self.pending.iter().map(|p| p.end).fold(0.0, f64::max);
        let mut tl = Timeline {
            width: self.size.x as u32,
            height: self.size.y as u32,
            fps: self.fps,
            duration: self.duration.unwrap_or(if self.pending.is_empty() { 1.0 } else { end }),
            background: self.background,
            objects: self.objects,
            tracks: Vec::new(),
        };
        // Each action starts from its object's state at its start time: the
        // object's tracks that began by then, applied in start order.
        let mut by_object: Vec<Vec<Track>> = vec![Vec::new(); tl.objects.len()];
        for p in self.pending {
            let o = p.object;
            let mut earlier: Vec<&Track> = by_object[o].iter().filter(|t| t.start <= p.start).collect();
            earlier.sort_by(|a, b| a.start.total_cmp(&b.start));
            let mut s = tl.objects[o].state;
            for tr in earlier {
                tr.apply(&mut s, p.start);
            }
            let track = |start: f64, end: f64, anim: Anim| Track { object: o, start, end, ease: p.ease, anim };
            let (start, end, mid) = (p.start, p.end, (p.start + p.end) / 2.0);
            let tracks = match p.change {
                Change::Move { to, by } => vec![track(start, end, Anim::Move { from: s.pos, to: to.unwrap_or_else(|| s.pos + by.unwrap()) })],
                Change::Rotate { to, by } => vec![track(start, end, Anim::Rotate { from: s.rotation, to: to.unwrap_or_else(|| s.rotation + by.unwrap()) })],
                Change::Scale { to, by } => vec![track(start, end, Anim::Scale { from: s.scale, to: to.unwrap_or_else(|| s.scale * by.unwrap()) })],
                Change::Fade(to) => vec![track(start, end, Anim::Fade { from: s.opacity, to })],
                Change::FadeIn => vec![track(start, end, Anim::Fade { from: s.opacity, to: 1.0 })],
                Change::Color(to) => vec![track(start, end, Anim::Color { from: s.color, to })],
                Change::Z { to, by } => vec![track(start, end, Anim::Z { from: s.z, to: to.unwrap_or_else(|| s.z + by.unwrap()) })],
                Change::Turn { axis: 0, to, by } => vec![track(start, end, Anim::Tilt { from: s.turn.x, to: to.unwrap_or_else(|| s.turn.x + by.unwrap()) })],
                Change::Turn { to, by, .. } => vec![track(start, end, Anim::Orbit { from: s.turn.y, to: to.unwrap_or_else(|| s.turn.y + by.unwrap()) })],
                Change::Section { to, by } => {
                    vec![track(start, end, Anim::Section { from: s.section, to: to.unwrap_or_else(|| s.section + by.unwrap()).clamp(0.0, 1.0) })]
                }
                Change::Pulse(k) => vec![
                    track(start, mid, Anim::Scale { from: s.scale, to: s.scale * k }),
                    track(mid, end, Anim::Scale { from: s.scale * k, to: s.scale }),
                ],
                Change::Flash(c) => vec![track(start, mid, Anim::Color { from: s.color, to: c }), track(mid, end, Anim::Color { from: c, to: s.color })],
                Change::Shake { amp, times } => {
                    // Out to one side, across to the other, `times` times, then back home.
                    let n = 2 * times + 1;
                    let dt = (end - start) / n as f64;
                    let offset = |j: u32| if j == n { 0.0 } else if j % 2 == 1 { amp } else { -amp };
                    (0..n)
                        .map(|j| {
                            let from = if j == 0 { 0.0 } else { offset(j) };
                            let (a, b) = (start + dt * j as f64, start + dt * (j + 1) as f64);
                            Track { object: o, start: a, end: b, ease: Ease::InOut, anim: Anim::Move { from: s.pos + Vec2::new(from, 0.0), to: s.pos + Vec2::new(offset(j + 1), 0.0) } }
                        })
                        .collect()
                }
            };
            for t in tracks {
                by_object[o].push(t.clone());
                tl.tracks.push(t);
            }
        }
        tl
    }

    fn num(&self, e: &Expr, d: Dim) -> R<f64> {
        match self.eval(e)? {
            Val::Num(n, dd) if dd == d => Ok(n),
            v => Err(type_err(&e.span(), want(d), &v)),
        }
    }

    fn eval(&self, e: &Expr) -> R<Val> {
        Ok(match e {
            Expr::Num(n, None, _) => Val::Num(*n, Dim::None),
            Expr::Num(n, Some(u), s) => match u.as_str() {
                "s" => Val::Num(*n, Dim::Time),
                "ms" => Val::Num(n / 1000.0, Dim::Time),
                "deg" => Val::Num(n.to_radians(), Dim::Angle),
                "rad" => Val::Num(*n, Dim::Angle),
                _ => return Err(Diag::new(s.clone(), format!("unknown unit `{u}`, expected s, ms, deg or rad"))),
            },
            Expr::Ident(w) => self.env.get(&w.name).cloned().unwrap_or_else(|| Val::Word(w.name.clone())),
            Expr::Color(c, _) => Val::Color(*c),
            Expr::Str(s, _) => Val::Str(s.clone()),
            Expr::Tuple(items, s) => match &items[..] {
                [x, y] => Val::Vec2(self.num(x, Dim::None)?, self.num(y, Dim::None)?),
                [x, y, z] => Val::Vec3(self.num(x, Dim::None)?, self.num(y, Dim::None)?, self.num(z, Dim::None)?),
                _ => return Err(Diag::new(s.clone(), "only `(x, y)` and `(x, y, z)` are supported")),
            },
            Expr::List(items, _) => Val::List(items.iter().map(|e| self.eval(e)).collect::<R<_>>()?),
            Expr::Index(list, i, s) => {
                let items = match self.eval(list)? {
                    Val::List(items) => items,
                    // `cell[3]` naming an object, as in `below cell[3]`.
                    Val::Word(w) => {
                        let n = self.num(i, Dim::None)?;
                        return Ok(Val::Word(format!("{w}[{n}]")));
                    }
                    v => return Err(type_err(&list.span(), "a list", &v)),
                };
                let n = self.num(i, Dim::None)?;
                if n < 0.0 || n.fract() != 0.0 || n as usize >= items.len() {
                    return Err(Diag::new(s.clone(), format!("index {n} is out of range for a list of {}", items.len())));
                }
                items[n as usize].clone()
            }
            Expr::Call(f, args, s) => self.call(f, args, s)?,
            Expr::Neg(inner, s) => match self.eval(inner)? {
                Val::Num(n, d) => Val::Num(-n, d),
                Val::Vec2(x, y) => Val::Vec2(-x, -y),
                v => return Err(type_err(s, "a number or point", &v)),
            },
            Expr::Bin(op, l, r, s) => {
                let (a, b) = (self.eval(l)?, self.eval(r)?);
                arith(*op, &a, &b).ok_or_else(|| {
                    let sym = match op {
                        Op::Add => "+",
                        Op::Sub => "-",
                        Op::Mul => "*",
                        Op::Div => "/",
                    };
                    for (v, e) in [(&a, l), (&b, r)] {
                        if let Val::Word(w) = v {
                            return Diag::new(e.span(), format!("unknown name `{w}`"));
                        }
                    }
                    Diag::new(s.clone(), format!("cannot compute {} {sym} {}", describe(&a), describe(&b)))
                })?
            }
        })
    }
}

impl Compiler<'_> {
    /// Built in functions. Data files are read relative to the current file,
    /// must be inside the scene folder and at most MAX_DATA_FILE bytes.
    fn call(&self, f: &Word, args: &[Expr], span: &Span) -> R<Val> {
        let name = f.name.as_str();
        let arity = |range: std::ops::RangeInclusive<usize>, usage: &str| {
            if range.contains(&args.len()) { Ok(()) } else { Err(Diag::new(span.clone(), format!("usage: `{usage}`"))) }
        };
        let whole = |i: usize| -> R<usize> {
            let n = self.num(&args[i], Dim::None)?;
            if n < 0.0 || n.fract() != 0.0 {
                return Err(Diag::new(args[i].span(), format!("expected a whole number, got {n}")));
            }
            Ok(n as usize)
        };
        let list = |i: usize| match self.eval(&args[i])? {
            Val::List(l) => Ok(l),
            v => Err(type_err(&args[i].span(), "a list", &v)),
        };
        let string = |i: usize| match self.eval(&args[i])? {
            Val::Str(s) => Ok(s),
            v => Err(type_err(&args[i].span(), "a string", &v)),
        };
        let text = |v: &Val, s: &Span| match v {
            Val::Str(t) => Ok(t.clone()),
            Val::Num(n, Dim::None) => Ok(fmt_num(*n)),
            v => Err(type_err(s, "a string or number", v)),
        };
        Ok(match name {
            "len" => {
                arity(1..=1, "len(list or string)")?;
                match self.eval(&args[0])? {
                    Val::List(l) => Val::Num(l.len() as f64, Dim::None),
                    Val::Str(s) => Val::Num(s.chars().count() as f64, Dim::None),
                    v => return Err(type_err(&args[0].span(), "a list or string", &v)),
                }
            }
            "floor" | "ceil" | "round" | "abs" => {
                arity(1..=1, &format!("{name}(number)"))?;
                let Val::Num(n, d) = self.eval(&args[0])? else { return Err(type_err(&args[0].span(), "a number", &self.eval(&args[0])?)) };
                let r = match name {
                    "floor" => n.floor(),
                    "ceil" => n.ceil(),
                    "round" => n.round(),
                    _ => n.abs(),
                };
                Val::Num(r, d)
            }
            "min" | "max" | "mod" => {
                arity(2..=2, &format!("{name}(a, b)"))?;
                match (self.eval(&args[0])?, self.eval(&args[1])?) {
                    (Val::Num(a, d), Val::Num(b, e)) if name == "mod" && e == Dim::None && b != 0.0 => Val::Num(a.rem_euclid(b), d),
                    (Val::Num(a, d), Val::Num(b, e)) if name != "mod" && d == e => Val::Num(if name == "min" { a.min(b) } else { a.max(b) }, d),
                    (a, b) => return Err(Diag::new(span.clone(), format!("cannot compute {name} of {} and {}", describe(&a), describe(&b)))),
                }
            }
            "str" => {
                arity(1..=1, "str(number)")?;
                Val::Str(text(&self.eval(&args[0])?, &args[0].span())?)
            }
            "hex" => {
                arity(1..=2, "hex(number, digits)")?;
                let n = whole(0)?;
                let w = if args.len() == 2 { whole(1)?.min(16) } else { 0 };
                Val::Str(format!("{n:0w$x}"))
            }
            "join" => {
                arity(2..=2, "join(list, separator)")?;
                let sep = string(1)?;
                let parts = list(0)?.iter().map(|v| text(v, &args[0].span())).collect::<R<Vec<_>>>()?;
                Val::Str(parts.join(&sep))
            }
            "slice" => {
                arity(2..=3, "slice(list, start, count)")?;
                let l = list(0)?;
                let start = whole(1)?.min(l.len());
                let end = if args.len() == 3 { start.saturating_add(whole(2)?).min(l.len()) } else { l.len() };
                Val::List(l[start..end].to_vec())
            }
            "ascii" => {
                arity(1..=1, "ascii(bytes)")?;
                let mut out = String::new();
                for v in list(0)? {
                    let b = match &v {
                        Val::Str(h) => u8::from_str_radix(h, 16).ok(),
                        Val::Num(n, Dim::None) if (0.0..256.0).contains(n) => Some(*n as u8),
                        _ => None,
                    }
                    .ok_or_else(|| Diag::new(args[0].span(), format!("expected bytes like \"41\", found {}", describe(&v))))?;
                    out.push(if (0x20..0x7f).contains(&b) { b as char } else { '.' });
                }
                Val::Str(out)
            }
            "bytes" => {
                arity(1..=3, "bytes(\"file\", offset, count)")?;
                let data = self.read_data(&string(0)?, &[], &args[0].span())?;
                let start = if args.len() >= 2 { whole(1)?.min(data.len()) } else { 0 };
                let end = if args.len() == 3 { start.saturating_add(whole(2)?).min(data.len()) } else { data.len() };
                if end - start > MAX_LIST {
                    return Err(Diag::new(span.clone(), format!("more than {MAX_LIST} bytes; pick a range with offset and count")));
                }
                Val::List(data[start..end].iter().map(|b| Val::Str(format!("{b:02x}"))).collect())
            }
            "file" => {
                arity(1..=1, "file(\"name\")")?;
                let data = self.read_data(&string(0)?, &[], &args[0].span())?;
                let t = String::from_utf8(data).map_err(|_| Diag::new(args[0].span(), "the file is not UTF-8 text"))?;
                if t.len() > MAX_TEXT_LEN {
                    return Err(Diag::new(args[0].span(), format!("the file is longer than {MAX_TEXT_LEN} bytes")));
                }
                Val::Str(t)
            }
            "table" => {
                arity(1..=1, "table(\"data.csv\")")?;
                let rel = string(0)?;
                let data = self.read_data(&rel, &["csv", "tsv"], &args[0].span())?;
                let t = String::from_utf8(data).map_err(|_| Diag::new(args[0].span(), "the file is not UTF-8 text"))?;
                let rows = parse_csv(&t, if rel.ends_with(".tsv") { '\t' } else { ',' });
                if rows.iter().map(Vec::len).sum::<usize>() > MAX_LIST {
                    return Err(Diag::new(span.clone(), format!("more than {MAX_LIST} cells")));
                }
                Val::List(rows.into_iter().map(|r| Val::List(r.into_iter().map(Val::Str).collect())).collect())
            }
            _ => {
                let msg = match FUNCTIONS.iter().min_by_key(|w| edit_distance(name, w)).filter(|w| edit_distance(name, w) <= 2) {
                    Some(w) => format!("unknown function `{name}`, did you mean `{w}`?"),
                    None => format!("unknown function `{name}`, expected one of: {}", FUNCTIONS.join(", ")),
                };
                return Err(Diag::new(f.span.clone(), msg));
            }
        })
    }

    fn read_data(&self, rel: &str, exts: &[&str], span: &Span) -> R<Vec<u8>> {
        let path = self.asset(rel, exts).map_err(|m| Diag::new(span.clone(), m))?;
        let len = std::fs::metadata(&path).map_err(|e| Diag::new(span.clone(), format!("`{rel}`: {e}")))?.len();
        if len > MAX_DATA_FILE {
            return Err(Diag::new(span.clone(), format!("`{rel}` is larger than {} KiB", MAX_DATA_FILE / 1024)));
        }
        let data = std::fs::read(&path).map_err(|e| Diag::new(span.clone(), format!("`{rel}`: {e}")))?;
        self.deps.borrow_mut().push(path.into());
        Ok(data)
    }
}

/// `3` not `3.0`, `0.25` as is.
fn fmt_num(n: f64) -> String {
    if n.fract() == 0.0 && n.abs() < 1e15 { format!("{}", n as i64) } else { format!("{n}") }
}

/// Rows of fields. Quoted fields may contain the separator and `""` for a
/// quote; a field cannot span lines.
fn parse_csv(text: &str, sep: char) -> Vec<Vec<String>> {
    let mut rows = Vec::new();
    for line in text.lines().filter(|l| !l.trim().is_empty()) {
        let (mut fields, mut cur, mut quoted) = (Vec::new(), String::new(), false);
        let mut chars = line.chars().peekable();
        while let Some(c) = chars.next() {
            match c {
                '"' if quoted && chars.peek() == Some(&'"') => {
                    cur.push('"');
                    chars.next();
                }
                '"' => quoted = !quoted,
                c if c == sep && !quoted => fields.push(std::mem::take(&mut cur)),
                c => cur.push(c),
            }
        }
        fields.push(cur);
        rows.push(fields);
    }
    rows
}

fn arith(op: Op, a: &Val, b: &Val) -> Option<Val> {
    use Val::*;
    Some(match (op, a, b) {
        (Op::Add, Num(x, d), Num(y, e)) if d == e => Num(x + y, *d),
        (Op::Sub, Num(x, d), Num(y, e)) if d == e => Num(x - y, *d),
        (Op::Mul, Num(x, d), Num(y, Dim::None)) => Num(x * y, *d),
        (Op::Mul, Num(x, Dim::None), Num(y, d)) => Num(x * y, *d),
        (Op::Div, Num(x, d), Num(y, Dim::None)) => Num(x / y, *d),
        (Op::Div, Num(x, d), Num(y, e)) if d == e => Num(x / y, Dim::None),
        (Op::Add, Vec2(a, b), Vec2(c, d)) => Vec2(a + c, b + d),
        (Op::Add, Vec3(a, b, c), Vec3(d, e, f)) => Vec3(a + d, b + e, c + f),
        (Op::Add, Str(a), Str(b)) => Str(format!("{a}{b}")),
        (Op::Sub, Vec2(a, b), Vec2(c, d)) => Vec2(a - c, b - d),
        (Op::Mul, Vec2(a, b), Num(k, Dim::None)) | (Op::Mul, Num(k, Dim::None), Vec2(a, b)) => Vec2(a * k, b * k),
        (Op::Div, Vec2(a, b), Num(k, Dim::None)) => Vec2(a / k, b / k),
        _ => return None,
    })
}

/// Properties an object or action accepts. The compiler, the reference docs
/// and the editor all use this one list.
pub fn props_of(command: &str) -> Vec<&'static str> {
    let own: &[&str] = match command {
        "rect" => &["size", "corner", "outline", "outline_width"],
        "circle" => &["radius", "outline", "outline_width"],
        "arrow" | "line" => &["from", "to", "width", "head"],
        "image" => &["content", "size", "width", "outline", "outline_width"],
        "code" => &["content", "size", "lang", "align"],
        "text" | "math" => &["content", "size", "align"],
        "model" => return vec!["content", "size", "z", "outline", "outline_width", "at", "rotation", "scale", "opacity"],
        "camera" => return vec!["fov", "at", "z", "rotation", "scale"],
        "move" | "rotate" | "scale" | "orbit" | "tilt" => return ACTION_PROPS.to_vec(),
        "explode" => return vec!["by", "duration", "ease"],
        "fade" | "color" => return vec!["to", "duration", "ease", "stagger"],
        "fade_in" | "fade_out" => return vec!["duration", "ease", "stagger"],
        "highlight" => return vec!["color", "scale", "duration", "ease", "stagger"],
        "shake" => return vec!["by", "times", "duration", "stagger"],
        "section" => return vec!["to", "by", "axis", "duration", "ease"],
        _ => return Vec::new(),
    };
    // Lines are placed by `from`, not `at`.
    let common: &[&str] = if own.contains(&"from") { &["color", "opacity", "rotation", "scale"] } else { COMMON_PROPS };
    common.iter().chain(own).chain(PLACE_2D).copied().collect()
}

/// Levenshtein distance, for "did you mean" suggestions.
fn edit_distance(a: &str, b: &str) -> usize {
    let b: Vec<char> = b.chars().collect();
    let mut row: Vec<usize> = (0..=b.len()).collect();
    for (i, ca) in a.chars().enumerate() {
        let mut prev = row[0];
        row[0] = i + 1;
        for (j, cb) in b.iter().enumerate() {
            let cur = row[j + 1];
            row[j + 1] = (prev + (ca != *cb) as usize).min(row[j] + 1).min(cur + 1);
            prev = cur;
        }
    }
    row[b.len()]
}

/// Canonical path of `rel` (relative to `dir`). It must be inside `root`
/// (canonical) and have one of `exts`, so scenes cannot read other files.
fn resolve_asset(dir: &std::path::Path, root: &std::path::Path, rel: &str, exts: &[&str]) -> Result<String, String> {
    let path = dir.join(rel).canonicalize().map_err(|e| format!("cannot read `{rel}`: {e}"))?;
    let ext = path.extension().and_then(|e| e.to_str()).map(str::to_ascii_lowercase);
    if !path.starts_with(root) {
        return Err(format!("`{rel}` must be inside {}", root.display()));
    }
    if !exts.is_empty() && !ext.is_some_and(|e| exts.contains(&e.as_str())) {
        return Err(format!("`{rel}` must be a .{} file inside {}", exts.join(" or ."), root.display()));
    }
    Ok(path.to_string_lossy().into_owned())
}

/// Build the Typst source for a text, code or math object.
/// Text and math may not use `#`, Typst's scripting escape, so they always terminate.
fn typst_source(kind: &str, text: &str, size: f64, lang: Option<&str>) -> Result<String, String> {
    let mut src = format!("#set text(size: {size}pt)\n");
    match kind {
        "code" => {
            // Only letters, digits and `+` reach Typst; `#` never does.
            let lang = lang.map(|l| if l.eq_ignore_ascii_case("c#") { "csharp" } else { l });
            if let Some(l) = lang.filter(|l| !l.chars().all(|c| c.is_ascii_alphanumeric() || c == '+')) {
                return Err(format!("bad language name `{l}`"));
            }
            let code = dedent(text);
            // A fence longer than any backtick run inside keeps the code literal.
            let longest = code.split(|c| c != '`').map(str::len).max().unwrap_or(0);
            let fence = "`".repeat(longest.max(2) + 1);
            src += &format!("#show raw: set text(size: {size}pt)\n{fence}{}\n{code}\n{fence}", lang.unwrap_or(""));
        }
        // `#` is escaped only by an odd number of backslashes (`\\#` is a backslash, then scripting).
        _ if text.match_indices('#').any(|(i, _)| (text[..i].len() - text[..i].trim_end_matches('\\').len()) % 2 == 0) => {
            return Err("`#` (Typst scripting) is not allowed in scenes; use `\\#` for a literal #".into());
        }
        "math" => src += &format!("$ {text} $"),
        _ => src += &format!("#set text(font: \"Inter\")\n{text}"),
    }
    Ok(src)
}

/// Strip a leading blank line and the common indentation, so code can be
/// written indented inside the scene.
fn dedent(s: &str) -> String {
    let s = s.strip_prefix('\n').unwrap_or(s).trim_end();
    let indent = s.lines().filter(|l| !l.trim().is_empty()).map(|l| l.len() - l.trim_start().len()).min().unwrap_or(0);
    s.lines().map(|l| l.get(indent..).unwrap_or("").trim_end()).collect::<Vec<_>>().join("\n")
}
