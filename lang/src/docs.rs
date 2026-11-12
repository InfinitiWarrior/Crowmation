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
    // Properties
    ("to", Kind::Property, "Target value of an action."),
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
];

/// Documentation for a word, if the language knows it.
pub fn lookup(word: &str) -> Option<(Kind, &'static str)> {
    DOCS.iter().find(|d| d.0 == word).map(|d| (d.1, d.2))
}
