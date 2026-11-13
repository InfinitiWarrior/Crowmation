//! One line of documentation for every word the language knows. Editors use
//! it for completion and hover.

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Kind {
    Keyword,
    Setting,
    Object,
    Action,
    Property,
    Value,
    Function,
}

pub const DOCS: &[(&str, Kind, &str)] = &[
    // Structure
    ("scene", Kind::Keyword, "`scene \"name\" { ... }`: the scene. One per file."),
    ("component", Kind::Keyword, "`component name(a, b) { ... }`: a reusable group of objects. Parts are named `instance.part`."),
    ("import", Kind::Keyword, "`import \"lib/file.cwm\"` or `import \"std:cybersec\"`: use components from another file."),
    ("let", Kind::Keyword, "`let name = value`: a constant. Values can be numbers, times, angles, points, colors, strings or lists."),
    ("repeat", Kind::Keyword, "`repeat n as i { ... }`: run the block n times (at most 10000) with i = 0, 1, ..."),
    ("wait", Kind::Keyword, "`wait 1s`: pause before the next action."),
    ("at", Kind::Keyword, "Timing: `at 2s move ...` starts at an absolute time. Also the position property of objects: `at (x, y)`."),
    ("after", Kind::Keyword, "Timing: `after move ...` starts when the previous action ends. `after 0.5s` adds a gap, `after -0.2s` overlaps."),
    ("with", Kind::Keyword, "Timing: `with move ...` starts together with the previous action."),
    ("as", Kind::Keyword, "`repeat n as i`: names the loop counter."),
    // Settings
    ("size", Kind::Setting, "Scene: `size (1920, 1080)` frame size in pixels (up to 4096). Objects: `size (w, h)` for rect and image, font size for text, pixels across for model."),
    ("fps", Kind::Setting, "`fps 60`: frames per second (up to 240). Default 60."),
    ("background", Kind::Setting, "`background #0d1117`: frame color."),
    ("duration", Kind::Setting, "Scene: `duration 10s` fixes the length (otherwise it ends with the last action). Actions: how long the action takes, default `1s`."),
    // Objects
    ("rect", Kind::Object, "`rect name size (w, h) corner 8 color #388bfd at (x, y)`: a rectangle centered on `at`."),
    ("circle", Kind::Object, "`circle name radius 20 at (x, y)`: a circle."),
    ("arrow", Kind::Object, "`arrow name from (x, y) to (x, y) width 4 head 16`: a line with an arrow head."),
    ("line", Kind::Object, "`line name from (x, y) to (x, y) width 4`: a straight line."),
    ("image", Kind::Object, "`image name \"assets/pic.png\" width 400`: a PNG or JPEG from the scene's folder."),
    ("text", Kind::Object, "`text name \"Hello *bold*\" size 32`: text laid out by Typst, in Inter. `#` must be written `\\#`."),
    ("code", Kind::Object, "`code name \"...\" lang \"c\" size 24`: syntax highlighted, monospace code. Indentation is stripped."),
    ("math", Kind::Object, "`math name \"sum_(i=0)^n i\" size 40`: Typst math."),
    ("model", Kind::Object, "`model name \"models/x.glb\" size 600`: a 3D model; makes the scene 3D. Parts are `name.<glTF node name>`."),
    ("camera", Kind::Object, "`camera cam fov 30deg at (x, y)`: the 3D camera, looking at `at`; makes the scene 3D. Animate with orbit, tilt, scale, move."),
    // Actions
    ("move", Kind::Action, "`move obj to (x, y)` or `by (dx, dy)`. A third number moves in depth: `to (x, y, z)`."),
    ("rotate", Kind::Action, "`rotate obj to 90deg` or `by 45deg`: rotate in the screen plane, clockwise."),
    ("scale", Kind::Action, "`scale obj to 2` or `by 0.5`. On a camera this zooms."),
    ("fade", Kind::Action, "`fade obj to 0.5`: change opacity."),
    ("fade_in", Kind::Action, "`fade_in obj`: fade to fully visible. An object whose first action is fade_in starts hidden."),
    ("fade_out", Kind::Action, "`fade_out obj`: fade to invisible."),
    ("color", Kind::Action, "`color obj to #e5484d`: change color. Also the color property: `color #ffffff`."),
    ("orbit", Kind::Action, "`orbit cam by 45deg` or `to`: turn a camera (or model) around the vertical axis."),
    ("tilt", Kind::Action, "`tilt cam to 30deg`: raise the camera to look down (or tilt a model)."),
    ("explode", Kind::Action, "`explode model by 0.5`: move every part away from the model's center."),
    ("highlight", Kind::Action, "`highlight obj color #ffb347`: draw attention: scale up and back (`scale 1.12` by default), flashing `color` if given. Default duration `600ms`."),
    ("shake", Kind::Action, "`shake obj by 12 times 3`: move side to side `times` times by `by` pixels, then back. Default duration `600ms`."),
    ("section", Kind::Action, "`section model to 0.5 axis y`: cut the model away, keeping that fraction (1 keeps all). `axis x` cuts from the right, `y` from the top (default), `z` from the front."),
    // Properties
    ("to", Kind::Property, "Target value of an action."),
    ("stagger", Kind::Property, "`fade_in cells stagger 80ms`: start each element (of a list like `cell[0]`, `cell[1]`, ... or the parts of a group) this much after the previous one."),
    ("align", Kind::Property, "Text, code and math: which edge `at` refers to: `left`, `center` (default) or `right`."),
    ("below", Kind::Property, "`below title`: place the object under another one, centered, `gap` pixels apart (default 24). Replaces `at`."),
    ("above", Kind::Property, "`above box`: place the object over another one, centered, `gap` pixels apart."),
    ("left_of", Kind::Property, "`left_of box`: place the object to the left of another one, centered vertically, `gap` pixels apart."),
    ("right_of", Kind::Property, "`right_of box`: place the object to the right of another one, centered vertically, `gap` pixels apart."),
    ("gap", Kind::Property, "Space in pixels for `below`, `above`, `left_of` and `right_of` (default 24)."),
    ("axis", Kind::Property, "`section`: `x`, `y` (default) or `z`."),
    ("times", Kind::Property, "`shake`: how many times to go side to side (default 3)."),
    ("by", Kind::Property, "Amount an action changes the value by."),
    ("ease", Kind::Property, "Easing of an action: `linear`, `in`, `out` or `in_out` (default)."),
    ("from", Kind::Property, "Start point of an arrow or line."),
    ("width", Kind::Property, "Line width of arrows and lines (default 4); display width of an image (keeps aspect)."),
    ("head", Kind::Property, "Arrow head size in pixels; 0 for none."),
    ("radius", Kind::Property, "Circle radius in pixels."),
    ("corner", Kind::Property, "Rectangle corner radius in pixels."),
    ("outline", Kind::Property, "Outline color: `outline #30363d`."),
    ("outline_width", Kind::Property, "Outline width in pixels (default 2)."),
    ("opacity", Kind::Property, "0 (invisible) to 1 (default)."),
    ("rotation", Kind::Property, "Starting rotation, like `rotation 15deg`."),
    ("lang", Kind::Property, "Language for code highlighting: `lang \"c\"`, `\"python\"`, `\"rust\"`, `\"c#\"`, ..."),
    ("content", Kind::Property, "The text, file path or code of an object; usually written as a string right after the name."),
    ("fov", Kind::Property, "Camera field of view, default `30deg`."),
    ("z", Kind::Property, "3D scenes: depth in pixels, positive toward the viewer. 2D objects at different z are separate planes."),
    ("layer", Kind::Property, "3D scenes: `layer overlay` keeps a 2D object fixed on screen and on top."),
    ("pin", Kind::Property, "3D scenes: `pin model.part` makes a 2D object follow that part on screen; `at` becomes an offset."),
    // Values
    ("overlay", Kind::Value, "`layer overlay`: fixed on screen, drawn on top of everything."),
    ("world", Kind::Value, "`layer world`: a plane in the 3D scene (the default)."),
    ("linear", Kind::Value, "Easing: constant speed."),
    ("in", Kind::Value, "Easing: starts slow."),
    ("out", Kind::Value, "Easing: ends slow."),
    ("in_out", Kind::Value, "Easing: starts and ends slow (the default)."),
    ("left", Kind::Value, "`align left`: `at` is the left edge."),
    ("center", Kind::Value, "`align center`: `at` is the center (the default)."),
    ("right", Kind::Value, "`align right`: `at` is the right edge."),
    // Functions
    ("len", Kind::Function, "`len(xs)`: number of items in a list, or characters in a string."),
    ("floor", Kind::Function, "`floor(x)`: round down."),
    ("ceil", Kind::Function, "`ceil(x)`: round up. `ceil(len(data) / 16)` rows of 16."),
    ("round", Kind::Function, "`round(x)`: round to the nearest whole number."),
    ("abs", Kind::Function, "`abs(x)`: absolute value."),
    ("min", Kind::Function, "`min(a, b)`: the smaller value (same units)."),
    ("max", Kind::Function, "`max(a, b)`: the larger value (same units)."),
    ("mod", Kind::Function, "`mod(i, 8)`: remainder, for wrapping into columns."),
    ("str", Kind::Function, "`str(42)`: a number as text, to join with `+`."),
    ("hex", Kind::Function, "`hex(255)` is `\"ff\"`, `hex(16, 8)` is `\"00000010\"`."),
    ("join", Kind::Function, "`join(xs, \" \")`: list items as one string with a separator."),
    ("slice", Kind::Function, "`slice(xs, start, count)`: part of a list (count may be left out)."),
    ("ascii", Kind::Function, "`ascii(bytes)`: bytes like `\"41\"` as printable text, `.` for the rest, like a hex dump's right column."),
    ("bytes", Kind::Function, "`bytes(\"payload.bin\", offset, count)`: a file's bytes as a list of hex strings like `\"41\"`. Up to 1 MiB, from the scene's folder."),
    ("file", Kind::Function, "`file(\"exploit.c\")`: a text file's contents, for `code` or `text`. From the scene's folder."),
    ("table", Kind::Function, "`table(\"packets.csv\")`: rows of a .csv or .tsv file as lists of strings: `rows[1][2]`."),
];

/// Documentation for a word, if the language knows it.
pub fn lookup(word: &str) -> Option<(Kind, &'static str)> {
    DOCS.iter().find(|d| d.0 == word).map(|d| (d.1, d.2))
}

/// The language reference (docs/reference.md), generated from the tables the
/// compiler uses, so it cannot drift from what the compiler accepts.
pub fn reference() -> String {
    use crowmation_core::{MAX_DURATION, MAX_FPS, MAX_SIZE};
    let mut s = String::from(
        "# Language reference\n\n\
         Generated from the compiler (`lang/src/docs.rs`); do not edit by hand.\n\
         Regenerate with `UPDATE_DOCS=1 cargo test -p crowmation-lang reference`.\n\n\
         For an introduction, see the [guide](guide.md).\n",
    );
    let section = |s: &mut String, title: &str, kind: Kind, with_props: bool| {
        *s += &format!("\n## {title}\n\n");
        for (word, k, doc) in DOCS {
            if *k != kind {
                continue;
            }
            *s += &format!("### `{word}`\n\n{doc}\n");
            let props = crate::compile::props_of(word);
            if with_props && !props.is_empty() {
                *s += &format!("\nProperties: {}\n", props.iter().map(|p| format!("`{p}`")).collect::<Vec<_>>().join(", "));
            }
            *s += "\n";
        }
    };
    section(&mut s, "Structure and timing", Kind::Keyword, false);
    section(&mut s, "Scene settings", Kind::Setting, false);
    s += &format!(
        "Limits: `size` up to {MAX_SIZE}×{MAX_SIZE}, `fps` up to {MAX_FPS}, scenes up to {}s long. \
         Text `size` is 1 to {}, `repeat` counts are at most {}.\n",
        MAX_DURATION,
        crate::compile::MAX_TEXT_SIZE,
        crate::compile::MAX_REPEAT
    );
    section(&mut s, "Objects", Kind::Object, true);
    section(&mut s, "Actions", Kind::Action, true);
    section(&mut s, "Properties", Kind::Property, false);
    section(&mut s, "Values", Kind::Value, false);
    section(&mut s, "Functions", Kind::Function, false);
    s += "Data files (`bytes`, `file`, `table`) must be inside the scene's folder and at most 1 MiB; \
          the preview reloads when they change.\n";
    s += "\n## Values and units\n\n\
          | Write | Meaning |\n|---|---|\n\
          | `12`, `0.5` | number |\n\
          | `2s`, `500ms` | time |\n\
          | `45deg`, `1.2rad` | angle (deg are converted to rad) |\n\
          | `(x, y)`, `(x, y, z)` | point; pixels from the top left, `z` toward the viewer |\n\
          | `#ff8800`, `#ff880080` | color, optionally with alpha |\n\
          | `\"text\"` | string; `\\\"`, `\\\\` and `\\n` are escapes, may span lines |\n\
          | `[a, b, c]`, `xs[i]` | list and indexing |\n\n\
          Math with units is checked: `1s + 500ms` is fine, `1s + 2deg` is an error. \
          Strings join with `+`. `//` starts a comment.\n";

    s += "\n## Built-in library: `std:cybersec`\n\nImport with `import \"std:cybersec\"`. Every component is centered on its `at`.\n\n";
    for (_, text) in crate::STD {
        let mut diags = Vec::new();
        for item in crate::parse_source(text, 0, &mut diags) {
            if let crate::parse::Item::Component { name: n, params, .. } = item {
                let params: Vec<&str> = params.iter().map(|p| p.name.as_str()).collect();
                let doc = crate::analyze::doc_comment(text, n.span.start);
                s += &format!("### `{}({})`\n\n{doc}\n\n", n.name, params.join(", "));
            }
        }
    }
    s
}
