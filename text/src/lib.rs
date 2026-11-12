//! Typst integration: Typst source in, positioned glyph runs and paths out,
//! ready for vello. Layouts are cached by source text.
//!
//! The Typst world has no file access: only the bundled fonts and the given
//! source exist.

use std::collections::HashMap;
use std::sync::{Arc, LazyLock, Mutex};
use typst::diag::{FileError, FileResult};
use typst::foundations::{Bytes, Datetime, Duration};
use typst::layout::{Frame, FrameItem, Transform};
use typst::syntax::{FileId, Source};
use typst::text::{Font, FontBook};
use typst::utils::LazyHash;
use typst::visualize::{Curve, CurveItem, Geometry, Paint};
use typst::{Library, LibraryExt, World};
use typst_layout::PagedDocument;
use vello::kurbo::{Affine, BezPath, Line, Rect, Shape};
use vello::peniko::{Blob, Color, FontData};

/// Laid out content. Coordinates in pixels (1pt = 1px), origin top left.
pub struct Layout {
    pub width: f64,
    pub height: f64,
    pub items: Vec<Item>,
}

/// `color: None` means "the object's color"; set colors (syntax highlighting) stay as is.
pub enum Item {
    Glyphs { font: FontData, size: f32, color: Option<Color>, xf: Affine, glyphs: Vec<vello::Glyph> },
    Fill { path: BezPath, color: Option<Color>, xf: Affine },
    Stroke { path: BezPath, width: f64, color: Option<Color>, xf: Affine },
}

struct Base {
    library: LazyHash<Library>,
    book: LazyHash<FontBook>,
    fonts: Vec<Font>,
}

/// Inter (SIL OFL, fonts/Inter-LICENSE.txt), the default for `text` objects.
const INTER: [&[u8]; 5] = [
    include_bytes!("../fonts/Inter-Regular.ttf"),
    include_bytes!("../fonts/Inter-Italic.ttf"),
    include_bytes!("../fonts/Inter-SemiBold.ttf"),
    include_bytes!("../fonts/Inter-Bold.ttf"),
    include_bytes!("../fonts/Inter-BoldItalic.ttf"),
];

// ponytail: bundled fonts only; add a project fonts/ folder when scenes need others
static BASE: LazyLock<Base> = LazyLock::new(|| {
    let fonts: Vec<Font> = typst_assets::fonts().chain(INTER).flat_map(|d| Font::iter(Bytes::new(d))).collect();
    Base { library: LazyHash::new(Library::default()), book: LazyHash::new(FontBook::from_fonts(&fonts)), fonts }
});

/// Layouts kept before the cache is cleared; a scene rarely has more distinct texts.
const CACHE_MAX: usize = 4096;
// ponytail: clears everything when full; LRU if scenes routinely exceed CACHE_MAX texts
static CACHE: LazyLock<Mutex<HashMap<String, Arc<Layout>>>> = LazyLock::new(Default::default);
static FONT_DATA: LazyLock<Mutex<HashMap<Font, FontData>>> = LazyLock::new(Default::default);

struct Doc {
    main: Source,
}

impl World for Doc {
    fn library(&self) -> &LazyHash<Library> {
        &BASE.library
    }
    fn book(&self) -> &LazyHash<FontBook> {
        &BASE.book
    }
    fn main(&self) -> FileId {
        self.main.id()
    }
    fn source(&self, id: FileId) -> FileResult<Source> {
        if id == self.main.id() { Ok(self.main.clone()) } else { Err(FileError::AccessDenied) }
    }
    fn file(&self, _: FileId) -> FileResult<Bytes> {
        Err(FileError::AccessDenied)
    }
    fn font(&self, index: usize) -> Option<Font> {
        BASE.fonts.get(index).cloned()
    }
    fn today(&self, _: Option<Duration>) -> Option<Datetime> {
        None
    }
}

/// Lay out Typst markup on an auto sized, transparent page.
pub fn layout(source: &str) -> Result<Arc<Layout>, String> {
    if let Some(l) = CACHE.lock().unwrap().get(source) {
        return Ok(l.clone());
    }
    let full = format!("#set page(width: auto, height: auto, margin: 0pt, fill: none)\n{source}");
    let doc = Doc { main: Source::detached(full) };
    let out = typst::compile::<PagedDocument>(&doc).output.map_err(|errs| {
        errs.iter().map(|e| e.message.to_string()).collect::<Vec<_>>().join("; ")
    })?;
    let page = out.pages().first().ok_or("typst produced no page")?;
    let size = page.frame.size();
    let mut items = Vec::new();
    frame(&page.frame, Affine::IDENTITY, &mut items);
    let l = Arc::new(Layout { width: size.x.to_pt(), height: size.y.to_pt(), items });
    let mut cache = CACHE.lock().unwrap();
    if cache.len() >= CACHE_MAX {
        cache.clear();
    }
    cache.insert(source.to_string(), l.clone());
    Ok(l)
}

fn affine(t: &Transform) -> Affine {
    Affine::new([t.sx.get(), t.ky.get(), t.kx.get(), t.sy.get(), t.tx.to_pt(), t.ty.to_pt()])
}

fn color(p: &Paint) -> Option<Color> {
    match p {
        Paint::Solid(c) => {
            let v = c.to_vec4();
            // Typst's default black means "use the object's color".
            (v != [0.0, 0.0, 0.0, 1.0]).then(|| Color::new(v))
        }
        // ponytail: gradients and tilings fall back to the object color
        _ => None,
    }
}

fn font_data(f: &Font) -> FontData {
    FONT_DATA
        .lock()
        .unwrap()
        .entry(f.clone())
        .or_insert_with(|| FontData::new(Blob::new(Arc::new(f.data().clone())), f.index()))
        .clone()
}

fn curve(c: &Curve) -> BezPath {
    let mut p = BezPath::new();
    for item in c.0.iter() {
        match item {
            CurveItem::Move(a) => p.move_to((a.x.to_pt(), a.y.to_pt())),
            CurveItem::Line(a) => p.line_to((a.x.to_pt(), a.y.to_pt())),
            CurveItem::Cubic(a, b, c) => {
                p.curve_to((a.x.to_pt(), a.y.to_pt()), (b.x.to_pt(), b.y.to_pt()), (c.x.to_pt(), c.y.to_pt()))
            }
            CurveItem::Close => p.close_path(),
        }
    }
    p
}

fn frame(f: &Frame, xf: Affine, out: &mut Vec<Item>) {
    for (pos, item) in f.items() {
        let here = xf * Affine::translate((pos.x.to_pt(), pos.y.to_pt()));
        match item {
            // ponytail: group clips ignored; matters only for clipped Typst content (boxes with clip: true)
            FrameItem::Group(g) => frame(&g.frame, here * affine(&g.transform), out),
            FrameItem::Text(t) => {
                let size = t.size.to_pt();
                let mut x = 0.0;
                let glyphs = t
                    .glyphs
                    .iter()
                    .map(|g| {
                        let gl = vello::Glyph {
                            id: g.id as u32,
                            x: (x + g.x_offset.at(t.size).to_pt()) as f32,
                            y: -g.y_offset.at(t.size).to_pt() as f32,
                        };
                        x += g.x_advance.at(t.size).to_pt();
                        gl
                    })
                    .collect();
                out.push(Item::Glyphs { font: font_data(t.font.font()), size: size as f32, color: color(&t.fill), xf: here, glyphs });
            }
            FrameItem::Shape(s, _) => {
                let path = match &s.geometry {
                    Geometry::Line(p) => Line::new((0.0, 0.0), (p.x.to_pt(), p.y.to_pt())).to_path(0.1),
                    Geometry::Rect(sz) => Rect::new(0.0, 0.0, sz.x.to_pt(), sz.y.to_pt()).to_path(0.1),
                    Geometry::Curve(c) => curve(c),
                };
                if let Some(fill) = &s.fill {
                    out.push(Item::Fill { path: path.clone(), color: color(fill), xf: here });
                }
                if let Some(st) = &s.stroke {
                    out.push(Item::Stroke { path, width: st.thickness.to_pt(), color: color(&st.paint), xf: here });
                }
            }
            // ponytail: images inside Typst content not drawn yet (phase 6 adds images)
            FrameItem::Image(..) | FrameItem::Link(..) | FrameItem::Tag(_) => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lays_out_text_code_and_math() {
        for src in ["Hello *world*", "```c\nint main() { return 0; }\n```", "$ x^2 + y^2 = r^2 $"] {
            let l = layout(src).unwrap();
            assert!(l.width > 0.0 && l.height > 0.0, "{src}");
            assert!(l.items.iter().any(|i| matches!(i, Item::Glyphs { .. })), "{src}");
        }
    }

    #[test]
    fn no_file_access() {
        let e = layout("#read(\"/etc/hostname\")").err().unwrap();
        assert!(e.contains("denied") || e.contains("access"), "{e}");
    }

    #[test]
    fn reports_errors() {
        assert!(layout("#let x = ").is_err());
    }
}
