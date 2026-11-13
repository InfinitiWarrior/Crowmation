//! The `.cwm` scene language: source text in, compiled timeline out.

mod compile;
pub mod analyze;
pub mod docs;
mod lex;
mod parse;

use crowmation_core::Timeline;
use parse::Item;
use std::path::{Path, PathBuf};

pub type Span = std::ops::Range<usize>;

#[derive(Clone, Debug, PartialEq)]
pub struct Diag {
    /// Index into the source list the error came from (0 is the scene file).
    pub file: usize,
    pub span: Span,
    pub msg: String,
}

impl Diag {
    pub fn new(span: Span, msg: impl Into<String>) -> Self {
        Self { file: 0, span, msg: msg.into() }
    }
}

/// Component libraries built into the binary, imported as `import "std:<name>"`.
const STD: &[(&str, &str)] = &[("std:cybersec", include_str!("../std/cybersec.cwm"))];

/// A loaded source file: display name and text.
pub struct Source {
    pub name: String,
    pub path: Option<PathBuf>,
    pub text: String,
}

fn parse_source(text: &str, file: usize, diags: &mut Vec<Diag>) -> Vec<Item> {
    let mut d = Vec::new();
    let toks = lex::lex(text, &mut d);
    let items = parse::parse(&toks, &mut d);
    diags.extend(d.into_iter().map(|d| Diag { file, ..d }));
    items
}

/// Compile `.cwm` source that has no imports. On failure returns every error found.
pub fn compile(src: &str) -> Result<Timeline, Vec<Diag>> {
    let mut diags = Vec::new();
    let mut files = vec![parse_source(src, 0, &mut diags)];
    let mut imports: Vec<(String, Span)> = Vec::new();
    for i in &files[0] {
        if let Item::Import { path, span } = i {
            imports.push((path.clone(), span.clone()));
        }
    }
    for (path, span) in imports {
        match STD.iter().find(|(n, _)| *n == path) {
            Some((_, text)) => {
                let id = files.len();
                files.push(parse_source(text, id, &mut diags));
            }
            None => diags.push(Diag::new(span, "file imports need the scene to be loaded from a file")),
        }
    }
    let paths = vec![None; files.len()];
    finish(&files, &paths, diags, &mut Vec::new())
}

/// Compile parsed files; data files the scene reads are added to `deps`.
pub(crate) fn finish(files: &[Vec<Item>], paths: &[Option<PathBuf>], mut diags: Vec<Diag>, deps: &mut Vec<PathBuf>) -> Result<Timeline, Vec<Diag>> {
    // Type errors in a file that failed to parse are mostly noise, so stop here.
    if !diags.is_empty() {
        return Err(diags);
    }
    compile::compile(files, paths, &mut diags, deps).ok_or(diags)
}

/// Render diagnostics as annotated source, colored if `color`.
pub fn report(sources: &[Source], diags: &[Diag], color: bool) -> String {
    use ariadne::{Config, Label, Report, ReportKind, Source as Src};
    let mut out = Vec::new();
    for d in diags {
        let s = &sources[d.file];
        let name = s.name.as_str();
        Report::build(ReportKind::Error, (name, d.span.clone()))
            .with_config(Config::default().with_color(color))
            .with_message(&d.msg)
            .with_label(Label::new((name, d.span.clone())))
            .finish()
            .write((name, Src::from(&s.text)), &mut out)
            .unwrap();
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Load a scene file and everything it imports.
/// Imports must be `.cwm` files inside the scene file's folder.
/// Load a scene and its imports. `text` replaces the scene file's contents
/// (an editor's unsaved buffer); imports are always read from disk.
pub fn load_sources(path: &Path, text: Option<&str>) -> Result<(Vec<Source>, Vec<Vec<Item>>, Vec<Diag>), String> {
    let main = path.canonicalize().map_err(|e| format!("{}: {e}", path.display()))?;
    let root = main.parent().unwrap().to_path_buf();
    let mut sources: Vec<Source> = Vec::new();
    let mut files = Vec::new();
    let mut diags = Vec::new();
    // Depth first, so the stack holds the current import chain for cycle detection.
    let mut stack: Vec<(usize, usize)> = Vec::new(); // (file, next item to look at)
    let text = match text {
        Some(t) => t.to_string(),
        None => std::fs::read_to_string(&main).map_err(|e| format!("{}: {e}", path.display()))?,
    };
    files.push(parse_source(&text, 0, &mut diags));
    sources.push(Source { name: path.display().to_string(), path: Some(main), text });
    stack.push((0, 0));
    while let Some((file, item)) = stack.pop() {
        let Some(it) = files[file].get(item) else { continue };
        stack.push((file, item + 1));
        let Item::Import { path: rel, span } = it else { continue };
        let err = |msg: String| Diag { file, span: span.clone(), msg };
        if rel.starts_with("std:") {
            let Some((name, text)) = STD.iter().find(|(n, _)| n == rel) else {
                let known: Vec<&str> = STD.iter().map(|s| s.0).collect();
                diags.push(err(format!("unknown library `{rel}`, expected one of: {}", known.join(", "))));
                continue;
            };
            if !sources.iter().any(|s| s.name == *name) {
                let id = sources.len();
                files.push(parse_source(text, id, &mut diags));
                sources.push(Source { name: name.to_string(), path: None, text: text.to_string() });
                stack.push((id, 0));
            }
            continue;
        }
        let Some(dir) = sources[file].path.as_ref().and_then(|p| p.parent()) else {
            diags.push(err("built in libraries can only import other `std:` libraries".into()));
            continue;
        };
        let target = match dir.join(rel).canonicalize() {
            Ok(p) => p,
            Err(e) => {
                diags.push(err(format!("cannot read `{rel}`: {e}")));
                continue;
            }
        };
        if !target.starts_with(&root) || target.extension().is_none_or(|e| e != "cwm") {
            diags.push(err(format!("`{rel}` must be a .cwm file inside {}", root.display())));
            continue;
        }
        if stack.iter().any(|(f, _)| sources[*f].path.as_ref() == Some(&target)) {
            diags.push(err(format!("`{rel}` imports itself through a cycle")));
            continue;
        }
        if sources.iter().any(|s| s.path.as_ref() == Some(&target)) {
            continue; // already loaded through another import
        }
        let text = match std::fs::read_to_string(&target) {
            Ok(t) => t,
            Err(e) => {
                diags.push(err(format!("cannot read `{rel}`: {e}")));
                continue;
            }
        };
        let id = sources.len();
        files.push(parse_source(&text, id, &mut diags));
        let name = target.strip_prefix(&root).unwrap_or(&target).display().to_string();
        sources.push(Source { name, path: Some(target), text });
        stack.push((id, 0));
    }
    Ok((sources, files, diags))
}

/// Load and compile a `.cwm` scene.
/// Also returns every file read, so callers can watch them for changes.
pub fn load(path: &Path, color: bool) -> (Result<Timeline, String>, Vec<PathBuf>) {
    if path.extension().is_none_or(|e| e != "cwm") {
        return (Err(format!("{}: expected a .cwm scene file", path.display())), vec![path.to_path_buf()]);
    }
    let (sources, files, diags) = match load_sources(path, None) {
        Ok(x) => x,
        Err(e) => return (Err(e), vec![path.to_path_buf()]),
    };
    let paths: Vec<Option<PathBuf>> = sources.iter().map(|s| s.path.clone()).collect();
    let mut watched: Vec<PathBuf> = paths.iter().flatten().cloned().collect();
    let res = finish(&files, &paths, diags, &mut watched).map_err(|d| report(&sources, &d, color));
    (res, watched)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crowmation_core::{Anim, Ease};
    use glam::Vec2;

    fn errs(src: &str) -> Vec<String> {
        compile(src).unwrap_err().into_iter().map(|d| d.msg).collect()
    }

    #[test]
    fn compiles_scene() {
        let tl = compile(
            r#"
            scene "demo" {
              size (800, 600)
              fps 30
              background #101218
              let gap = 100
              rect box size (200, 100) at (gap, 300) color #3399ff
              circle dot radius 40 at (400, 300)

              at 0s   move box to (600, 300) duration 2s ease linear
              with    rotate box by 90deg duration 1s
              after   fade_in dot duration 500ms
              after 1s scale dot by 2
              wait 1s
              move box by (-gap * 2, 0)
            }
            "#,
        )
        .unwrap();
        assert_eq!((tl.width, tl.height, tl.fps), (800, 600, 30));
        assert_eq!(tl.objects[0].state.pos, Vec2::new(100.0, 300.0));
        assert_eq!(tl.objects[1].state.opacity, 0.0, "first action is fade_in, so starts hidden");
        let t: Vec<(f64, f64)> = tl.tracks.iter().map(|t| (t.start, t.end)).collect();
        // move 0-2, rotate 0-1 (with), fade 2-2.5 (after group end), scale 3.5-4.5 (after 1s gap), move 5.5-6.5 (after wait)
        assert_eq!(t, vec![(0.0, 2.0), (0.0, 1.0), (2.0, 2.5), (3.5, 4.5), (5.5, 6.5)]);
        assert_eq!(tl.tracks[0].ease, Ease::Linear);
        assert_eq!(tl.tracks[3].anim, Anim::Scale { from: 1.0, to: 2.0 });
        assert_eq!(tl.tracks[4].anim, Anim::Move { from: Vec2::new(600.0, 300.0), to: Vec2::new(400.0, 300.0) });
        assert_eq!(tl.duration, 6.5);
    }

    #[test]
    fn from_values_follow_time_not_source_order() {
        let tl = compile(
            "scene {\n circle c radius 1\n at 2s move c to (10, 0)\n at 0s move c to (5, 0)\n}",
        )
        .unwrap();
        assert_eq!(tl.tracks[1].anim, Anim::Move { from: Vec2::new(5.0, 0.0), to: Vec2::new(10.0, 0.0) });
    }

    #[test]
    fn reports_many_errors() {
        let e = errs("scene {\n rect r\n circle c radius 2s\n at 1 move c to (0, 0)\n spin c\n move nope by (1, 1)\n}");
        assert_eq!(e.len(), 5, "{e:#?}");
        assert!(e[0].contains("needs `size`"));
        assert!(e[1].contains("expected a number, found a time"));
        assert!(e[2].contains("expected a time"));
        assert!(e[3].contains("unknown command `spin`"));
        assert!(e[4].contains("no object named `nope`"));
    }

    #[test]
    fn parse_errors_recover_per_line() {
        let e = errs("scene {\n rect r size (1, \n circle c radius 3 @\n fps\n repeat 2 as {\n circle x radius 1\n }\n}");
        assert_eq!(e.len(), 4, "{e:#?}");
    }

    #[test]
    fn unit_math() {
        assert!(compile("scene {\n circle c radius 1\n at 1s + 500ms move c by (1, 0) duration 2s / 4\n}").is_ok());
        let e = errs("scene {\n circle c radius 1\n at 1s + 5deg move c by (1, 0)\n}");
        assert!(e[0].contains("cannot compute a time + an angle"), "{e:?}");
        let e = errs("scene {\n circle c radius 1\n move c by (1, 0) duration 2 ss\n}");
        assert!(!e.is_empty());
    }

    #[test]
    fn typo_reads_as_unknown_name() {
        let e = errs("scene {\n let gap = 10\n circle c radius gapp\n}");
        assert!(e[0].contains("unknown name `gapp`"), "{e:?}");
    }

    #[test]
    fn report_points_at_source() {
        let src = "scene {\n circle c radius 2s\n}";
        let sources = [Source { name: "t.cwm".into(), path: None, text: src.into() }];
        let out = report(&sources, &compile(src).unwrap_err(), false);
        assert!(out.contains("t.cwm:2:") && out.contains("radius 2s"), "{out}");
    }

    #[test]
    fn repeat_and_components() {
        let tl = compile(
            r#"
            component byte_row(n) {
              repeat n as i {
                rect c[i] size (40, 40) at (i * 42, 0)
              }
            }
            scene {
              byte_row row n 4 at (100, 50)
              repeat 2 as k {
                after move row.c[k * 2] by (0, 10)
              }
            }
            "#,
        )
        .unwrap();
        // The instance is a group at (100, 50); its parts are placed relative to it.
        assert_eq!(tl.objects.len(), 5);
        assert_eq!(tl.objects[0].shape, crowmation_core::Shape::Group);
        assert_eq!(tl.objects[0].state.pos, Vec2::new(100.0, 50.0));
        assert_eq!((tl.objects[4].state.pos, tl.objects[4].parent), (Vec2::new(3.0 * 42.0, 0.0), Some(0)));
        assert_eq!(tl.tracks[1].object, 3, "row.c[2]");
        assert_eq!(tl.tracks[1].start, 1.0);
    }

    #[test]
    fn component_errors() {
        let e = errs(
            "component a(x) {\n b inner\n}\ncomponent b() {\n a inner x 1\n move q by (1, 1)\n}\ncomponent rect() {\n}\ncomponent ok() {\n circle c radius 1\n circle d radius 1\n}\nscene {\n a thing\n a other x 1\n ok fine\n rotate fine by 10deg\n repeat 100000 as i {\n }\n}",
        );
        let has = |s: &str| e.iter().any(|m| m.contains(s));
        assert!(has("is a built in name"), "{e:#?}");
        assert!(has("`a` needs `x`"), "{e:#?}");
        assert!(has("uses itself"), "{e:#?}");
        assert!(has("no object named `q`"), "{e:#?}");
        assert!(has("repeat count must be"), "{e:#?}");
    }

    #[test]
    fn repeat_errors_report_once() {
        let e = errs("scene {\n repeat 50 as i {\n circle c[i] radius 1s\n }\n}");
        assert_eq!(e.len(), 1, "{e:#?}");
    }

    #[test]
    fn imports() {
        let dir = std::env::temp_dir().join(format!("cwm-import-test-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("lib")).unwrap();
        std::fs::write(dir.join("lib/shapes.cwm"), "import \"more.cwm\"\ncomponent dot(r) {\n circle c radius r\n}\n").unwrap();
        std::fs::write(dir.join("lib/more.cwm"), "import \"shapes.cwm\"\ncomponent pair() {\n dot a r 1\n dot b r 2 at (10, 0)\n}\n").unwrap();
        std::fs::write(dir.join("main.cwm"), "import \"lib/shapes.cwm\"\nscene {\n pair p\n}\n").unwrap();
        let (res, files) = load(&dir.join("main.cwm"), false);
        let err = res.unwrap_err();
        assert!(err.contains("cycle") && err.contains("more.cwm"), "{err}");
        assert_eq!(files.len(), 3);

        std::fs::write(dir.join("lib/more.cwm"), "component pair() {\n dot a r 1\n dot b r 2 at (10, 0)\n}\n").unwrap();
        let tl = load(&dir.join("main.cwm"), false).0.unwrap();
        assert_eq!(tl.objects.len(), 5, "groups p, p.a, p.b and two circles");

        std::fs::write(dir.join("main.cwm"), "import \"../outside.cwm\"\nscene {\n}\n").unwrap();
        std::fs::write(dir.join("../outside.cwm"), "").ok();
        let err = load(&dir.join("main.cwm"), false).0.unwrap_err();
        assert!(err.contains("must be a .cwm file inside"), "{err}");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn text_code_math() {
        let tl = compile(
            "scene {\n text t \"Hello *bold* \\#1\" size 40\n code c \"\n    int x = 1;\n    ```\n  \" lang \"c\"\n math m \"x^2\"\n let s = \"a\" + \"b\"\n text u content s\n}",
        )
        .unwrap();
        let src = |i: usize| match &tl.objects[i].shape {
            crowmation_core::Shape::Typst { source, .. } => source.clone(),
            _ => panic!(),
        };
        assert!(src(0).contains("Hello *bold* \\#1"), "{}", src(0));
        assert!(src(1).ends_with("````c\nint x = 1;\n```\n````"), "{}", src(1));
        assert!(src(2).contains("$ x^2 $"));
        assert!(src(3).ends_with("ab"));
    }

    #[test]
    fn text_blocks_typst_scripting() {
        for bad in [r##""#while true {}""##, r##""\\\\#read(1)""##] {
            let e = errs(&format!("scene {{\n text t {bad}\n}}"));
            assert!(e[0].contains("not allowed"), "{bad}: {e:?}");
        }
        let e = errs("scene {\n math m \"x^\"\n}");
        assert!(e[0].starts_with("typst:"), "{e:?}");
    }

    #[test]
    fn group_actions_and_shapes() {
        let tl = compile(
            "component card() {\n rect bg size (100, 50) corner 8 outline #ffffff\n text t \"hi\"\n}\nscene {\n card c at (10, 10)\n fade_in c\n move c by (5, 0)\n arrow a from (0, 0) to (30, 40)\n line l from (1, 1) to (2, 2) width 2\n}",
        )
        .unwrap();
        assert_eq!(tl.tracks.len(), 2, "fade and move on the group");
        assert!(tl.tracks.iter().all(|t| t.object == 0));
        assert_eq!(tl.objects[0].state.opacity, 0.0, "the group starts hidden");
        assert_eq!(tl.objects[1].outline.as_ref().unwrap().width, 2.0);
        match &tl.objects[3].shape {
            crowmation_core::Shape::Arrow { to, head, .. } => assert_eq!((*to, *head), (Vec2::new(30.0, 40.0), 16.0)),
            s => panic!("{s:?}"),
        }
        assert!(errs("scene {\n image i \"x.png\"\n}")[0].contains("need the scene to be loaded from a file"));
    }

    #[test]
    fn std_library_compiles() {
        let tl = compile(
            "import \"std:cybersec\"\nscene {\n byte b label \"41\" fill #333333\n memory_row r n 4 fill #333333\n memory_grid g rows 2 cols 3 fill #333333\n panel p title \"GET /\" body \"Host: a\" width 400 height 200 accent #58a6ff\n browser w url \"https://x\" width 800 height 500\n fade_in w\n}",
        )
        .unwrap();
        assert_eq!(tl.objects.len(), 5 + 2 + 4 + 6 + 4 + 7, "a group per instance plus its parts");
        assert!(errs("import \"std:nope\"\nscene {\n}")[0].contains("file imports"));
    }

    #[test]
    fn lists_and_overlap() {
        let tl = compile("scene {\n let xs = [\n  10, 20,\n  30,\n ]\n repeat 3 as i {\n  circle c[i] radius xs[i]\n }\n move c[0] by (1, 0) duration 2s\n after -1s move c[1] by (1, 0)\n}").unwrap();
        assert_eq!(tl.objects[2].shape, crowmation_core::Shape::Circle { radius: 30.0 });
        assert_eq!(tl.tracks[1].start, 1.0, "negative gap overlaps the previous action");
        assert!(errs("scene {\n let xs = [1]\n circle c radius xs[1]\n}")[0].contains("out of range"));
    }

    #[test]
    fn model_scene_3d() {
        let path = Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../examples/router.cwm"));
        let (tl, _) = load(path, false);
        let tl = tl.unwrap();
        assert!(tl.is_3d());
        let chip = tl.objects.iter().position(|o| matches!(&o.shape, crowmation_core::Shape::Mesh { part, .. } if *part == 4)).unwrap();
        assert!(tl.objects.iter().any(|o| o.pin == Some(chip)), "label pinned to the flash chip");
        // Tilt and orbit run together without overwriting each other.
        let cam = tl.objects.iter().position(|o| matches!(o.shape, crowmation_core::Shape::Camera { .. })).unwrap();
        let s = tl.eval(8.0)[cam];
        assert!((s.turn.x - 32f32.to_radians()).abs() < 1e-4 && (s.turn.y + 25f32.to_radians()).abs() < 1e-4, "{:?}", s.turn);
        assert!(errs("scene {\n text t \"a\" pin nowhere\n}")[0].contains("no object named `nowhere`"));
    }

    #[test]
    fn limits() {
        assert!(errs("scene {\n size (100000, 1080)\n}")[0].contains("1 to 4096"));
        assert!(errs("scene {\n fps 100000\n}")[0].contains("1 to 240"));
        assert!(errs("scene {\n duration 1000000s\n}")[0].contains("duration"));
        assert!(errs("scene {\n circle c radius 1\n move c by (1, 0) duration 9999s\n}")[0].contains("duration"), "total length is capped too");
        assert!(errs("scene {\n text t \"hi\" size 5000\n}")[0].contains("text size"));
        // `#` never reaches Typst through the language name.
        assert!(compile("scene {\n code c \"int x;\" lang \"c#\"\n}").is_ok());
        assert!(errs("scene {\n code c \"x\" lang \"c#x\"\n}")[0].contains("bad language"));
    }

    #[test]
    fn docs_cover_the_language() {
        use compile::{ACTIONS, FUNCTIONS, KEYWORDS, SHAPES};
        for w in SHAPES.iter().chain(ACTIONS).chain(KEYWORDS).chain(parse::SETTINGS).chain(FUNCTIONS) {
            assert!(docs::lookup(w).is_some(), "`{w}` has no docs");
        }
    }

    #[test]
    fn suggests_close_commands() {
        assert_eq!(errs("scene {\n circle c radius 1\n at 1s fade_inn c\n}"), ["unknown command `fade_inn`, did you mean `fade_in`?"]);
    }

    #[test]
    fn reference_is_up_to_date() {
        let path = Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../docs/reference.md"));
        let fresh = docs::reference();
        if std::env::var_os("UPDATE_DOCS").is_some() {
            std::fs::write(path, &fresh).unwrap();
        }
        let current = std::fs::read_to_string(path).unwrap_or_default();
        assert!(current == fresh, "docs/reference.md is out of date: run `UPDATE_DOCS=1 cargo test -p crowmation-lang reference`");
    }

    #[test]
    fn guide_examples_compile() {
        let guide = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/../docs/guide.md")).unwrap();
        let blocks: Vec<&str> = guide.split("```cwm\n").skip(1).map(|b| b.split("```").next().unwrap()).collect();
        assert!(blocks.len() >= 6);
        for b in blocks {
            if let Err(d) = compile(b) {
                panic!("guide example does not compile: {:?}\n{b}", d.iter().map(|d| &d.msg).collect::<Vec<_>>());
            }
        }
    }

    #[test]
    fn stagger_highlight_shake_layout() {
        let tl = compile(
            "component pop() {\n circle c radius 5\n fade_in c duration 200ms\n}\nscene {\n repeat 3 as i {\n  rect c[i] size (10, 10) at (i * 20, 0)\n }\n rect a size (100, 50) at (500, 300)\n rect b size (20, 20) below a gap 10\n text t \"hi\" right_of a gap 0 align left\n fade_in c stagger 100ms duration 200ms\n after highlight a\n after shake b times 2\n after pop p at (0, 0)\n rotate p by 90deg\n}",
        )
        .unwrap();
        let starts: Vec<f64> = tl.tracks.iter().filter(|t| t.object < 3).map(|t| t.start).collect();
        assert_eq!(starts, [0.0, 0.1, 0.2], "staggered");
        assert!(tl.objects[..3].iter().all(|o| o.state.opacity == 0.0), "fade_in first: hidden");
        assert_eq!(tl.objects[4].state.pos, Vec2::new(500.0, 345.0), "below a, 10px gap");
        let t = &tl.objects[5];
        assert_eq!(t.state.pos.y, 300.0, "right_of centers vertically");
        assert!((t.state.pos.x - 550.0).abs() < 0.01, "left aligned text starts at a's right edge: {}", t.state.pos.x);
        let pulse: Vec<_> = tl.tracks.iter().filter(|t| t.object == 3).collect();
        assert_eq!(pulse.len(), 2, "highlight scales up and back");
        let shake: Vec<_> = tl.tracks.iter().filter(|t| t.object == 4).collect();
        assert_eq!(shake.len(), 5);
        assert_eq!(shake.last().unwrap().anim, crowmation_core::Anim::Move { from: Vec2::new(488.0, 345.0), to: Vec2::new(500.0, 345.0) }, "ends where it started (default 12px)");
        // The component's own fade starts where the instance is declared; the group then rotates.
        let fade = tl.tracks.iter().find(|t| t.object == 7).unwrap();
        assert_eq!(fade.start, shake.last().unwrap().end);
        assert!(tl.tracks.iter().any(|t| t.object == 6 && matches!(t.anim, crowmation_core::Anim::Rotate { .. })));
        assert!(errs("scene {\n rect r size (1, 1)\n section r to 0.5\n}")[0].contains("is not a model"));
        assert!(errs("scene {\n rect a size (1, 1)\n rect b size (1, 1) at (0, 0) below a\n}")[0].contains("both place"));
    }

    #[test]
    fn functions_and_data_files() {
        let tl = compile(
            "scene {\n let xs = [\"41\", \"42\", \"0a\"]\n text a content join(xs, \" \") + \" \" + ascii(xs) + \" \" + hex(255, 4) + str(len(xs))\n circle c radius max(mod(7, 4), floor(2.7))\n}",
        )
        .unwrap();
        match &tl.objects[0].shape {
            crowmation_core::Shape::Typst { source, .. } => assert!(source.ends_with("41 42 0a AB. 00ff3"), "{source}"),
            s => panic!("{s:?}"),
        }
        assert_eq!(tl.objects[1].shape, crowmation_core::Shape::Circle { radius: 3.0 });
        assert!(errs("scene {\n circle c radius lenn([1])\n}")[0].contains("did you mean `len`"));

        let dir = std::env::temp_dir().join(format!("cwm-data-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("p.bin"), b"AB\x00\xff").unwrap();
        std::fs::write(dir.join("t.csv"), "name,port\n\"web, main\",443\n").unwrap();
        std::fs::write(
            dir.join("s.cwm"),
            "scene {\n let b = bytes(\"p.bin\", 1, 2)\n let t = table(\"t.csv\")\n text x content join(b, \"-\") + \"/\" + t[1][0] + \"/\" + t[1][1]\n}",
        )
        .unwrap();
        let (tl, watched) = load(&dir.join("s.cwm"), false);
        match &tl.unwrap().objects[0].shape {
            crowmation_core::Shape::Typst { source, .. } => assert!(source.ends_with("42-00/web, main/443"), "{source}"),
            s => panic!("{s:?}"),
        }
        assert!(watched.iter().any(|p| p.ends_with("p.bin")) && watched.iter().any(|p| p.ends_with("t.csv")), "data files are watched");
        std::fs::write(dir.join("s.cwm"), "scene {\n let b = bytes(\"../../etc/passwd\")\n}").unwrap();
        assert!(load(&dir.join("s.cwm"), false).0.unwrap_err().contains("must be inside"));
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
