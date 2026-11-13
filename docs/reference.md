# Language reference

Generated from the compiler (`lang/src/docs.rs`); do not edit by hand.
Regenerate with `UPDATE_DOCS=1 cargo test -p crowmation-lang reference`.

For an introduction, see the [guide](guide.md).

## Structure and timing

### `scene`

`scene "name" { ... }`: the scene. One per file.

### `component`

`component name(a, b) { ... }`: a reusable group of objects. Parts are named `instance.part`.

### `import`

`import "lib/file.cwm"` or `import "std:cybersec"`: use components from another file.

### `let`

`let name = value`: a constant. Values can be numbers, times, angles, points, colors, strings or lists.

### `repeat`

`repeat n as i { ... }`: run the block n times (at most 10000) with i = 0, 1, ...

### `wait`

`wait 1s`: pause before the next action.

### `at`

Timing: `at 2s move ...` starts at an absolute time. Also the position property of objects: `at (x, y)`.

### `after`

Timing: `after move ...` starts when the previous action ends. `after 0.5s` adds a gap, `after -0.2s` overlaps.

### `with`

Timing: `with move ...` starts together with the previous action.

### `as`

`repeat n as i`: names the loop counter.


## Scene settings

### `size`

Scene: `size (1920, 1080)` frame size in pixels (up to 4096). Objects: `size (w, h)` for rect and image, font size for text, pixels across for model.

### `fps`

`fps 60`: frames per second (up to 240). Default 60.

### `background`

`background #0d1117`: frame color.

### `duration`

Scene: `duration 10s` fixes the length (otherwise it ends with the last action). Actions: how long the action takes, default `1s`.

Limits: `size` up to 4096×4096, `fps` up to 240, scenes up to 3600s long. Text `size` is 1 to 1000, `repeat` counts are at most 10000.

## Objects

### `rect`

`rect name size (w, h) corner 8 color #388bfd at (x, y)`: a rectangle centered on `at`.

Properties: `at`, `color`, `opacity`, `rotation`, `scale`, `size`, `corner`, `outline`, `outline_width`, `z`, `layer`, `pin`, `below`, `above`, `left_of`, `right_of`, `gap`

### `circle`

`circle name radius 20 at (x, y)`: a circle.

Properties: `at`, `color`, `opacity`, `rotation`, `scale`, `radius`, `outline`, `outline_width`, `z`, `layer`, `pin`, `below`, `above`, `left_of`, `right_of`, `gap`

### `arrow`

`arrow name from (x, y) to (x, y) width 4 head 16`: a line with an arrow head.

Properties: `color`, `opacity`, `rotation`, `scale`, `from`, `to`, `width`, `head`, `z`, `layer`, `pin`, `below`, `above`, `left_of`, `right_of`, `gap`

### `line`

`line name from (x, y) to (x, y) width 4`: a straight line.

Properties: `color`, `opacity`, `rotation`, `scale`, `from`, `to`, `width`, `head`, `z`, `layer`, `pin`, `below`, `above`, `left_of`, `right_of`, `gap`

### `image`

`image name "assets/pic.png" width 400`: a PNG or JPEG from the scene's folder.

Properties: `at`, `color`, `opacity`, `rotation`, `scale`, `content`, `size`, `width`, `outline`, `outline_width`, `z`, `layer`, `pin`, `below`, `above`, `left_of`, `right_of`, `gap`

### `text`

`text name "Hello *bold*" size 32`: text laid out by Typst, in Inter. `#` must be written `\#`.

Properties: `at`, `color`, `opacity`, `rotation`, `scale`, `content`, `size`, `align`, `z`, `layer`, `pin`, `below`, `above`, `left_of`, `right_of`, `gap`

### `code`

`code name "..." lang "c" size 24`: syntax highlighted, monospace code. Indentation is stripped.

Properties: `at`, `color`, `opacity`, `rotation`, `scale`, `content`, `size`, `lang`, `align`, `z`, `layer`, `pin`, `below`, `above`, `left_of`, `right_of`, `gap`

### `math`

`math name "sum_(i=0)^n i" size 40`: Typst math.

Properties: `at`, `color`, `opacity`, `rotation`, `scale`, `content`, `size`, `align`, `z`, `layer`, `pin`, `below`, `above`, `left_of`, `right_of`, `gap`

### `model`

`model name "models/x.glb" size 600`: a 3D model; makes the scene 3D. Parts are `name.<glTF node name>`.

Properties: `content`, `size`, `z`, `outline`, `outline_width`, `at`, `rotation`, `scale`, `opacity`

### `camera`

`camera cam fov 30deg at (x, y)`: the 3D camera, looking at `at`; makes the scene 3D. Animate with orbit, tilt, scale, move.

Properties: `fov`, `at`, `z`, `rotation`, `scale`


## Actions

### `move`

`move obj to (x, y)` or `by (dx, dy)`. A third number moves in depth: `to (x, y, z)`.

Properties: `to`, `by`, `duration`, `ease`, `stagger`

### `rotate`

`rotate obj to 90deg` or `by 45deg`: rotate in the screen plane, clockwise.

Properties: `to`, `by`, `duration`, `ease`, `stagger`

### `scale`

`scale obj to 2` or `by 0.5`. On a camera this zooms.

Properties: `to`, `by`, `duration`, `ease`, `stagger`

### `fade`

`fade obj to 0.5`: change opacity.

Properties: `to`, `duration`, `ease`, `stagger`

### `fade_in`

`fade_in obj`: fade to fully visible. An object whose first action is fade_in starts hidden.

Properties: `duration`, `ease`, `stagger`

### `fade_out`

`fade_out obj`: fade to invisible.

Properties: `duration`, `ease`, `stagger`

### `color`

`color obj to #e5484d`: change color. Also the color property: `color #ffffff`.

Properties: `to`, `duration`, `ease`, `stagger`

### `orbit`

`orbit cam by 45deg` or `to`: turn a camera (or model) around the vertical axis.

Properties: `to`, `by`, `duration`, `ease`, `stagger`

### `tilt`

`tilt cam to 30deg`: raise the camera to look down (or tilt a model).

Properties: `to`, `by`, `duration`, `ease`, `stagger`

### `explode`

`explode model by 0.5`: move every part away from the model's center.

Properties: `by`, `duration`, `ease`

### `highlight`

`highlight obj color #ffb347`: draw attention: scale up and back (`scale 1.12` by default), flashing `color` if given. Default duration `600ms`.

Properties: `color`, `scale`, `duration`, `ease`, `stagger`

### `shake`

`shake obj by 12 times 3`: move side to side `times` times by `by` pixels, then back. Default duration `600ms`.

Properties: `by`, `times`, `duration`, `stagger`

### `section`

`section model to 0.5 axis y`: cut the model away, keeping that fraction (1 keeps all). `axis x` cuts from the right, `y` from the top (default), `z` from the front.

Properties: `to`, `by`, `axis`, `duration`, `ease`


## Properties

### `to`

Target value of an action.

### `stagger`

`fade_in cells stagger 80ms`: start each element (of a list like `cell[0]`, `cell[1]`, ... or the parts of a group) this much after the previous one.

### `align`

Text, code and math: which edge `at` refers to: `left`, `center` (default) or `right`.

### `below`

`below title`: place the object under another one, centered, `gap` pixels apart (default 24). Replaces `at`.

### `above`

`above box`: place the object over another one, centered, `gap` pixels apart.

### `left_of`

`left_of box`: place the object to the left of another one, centered vertically, `gap` pixels apart.

### `right_of`

`right_of box`: place the object to the right of another one, centered vertically, `gap` pixels apart.

### `gap`

Space in pixels for `below`, `above`, `left_of` and `right_of` (default 24).

### `axis`

`section`: `x`, `y` (default) or `z`.

### `times`

`shake`: how many times to go side to side (default 3).

### `by`

Amount an action changes the value by.

### `ease`

Easing of an action: `linear`, `in`, `out` or `in_out` (default).

### `from`

Start point of an arrow or line.

### `width`

Line width of arrows and lines (default 4); display width of an image (keeps aspect).

### `head`

Arrow head size in pixels; 0 for none.

### `radius`

Circle radius in pixels.

### `corner`

Rectangle corner radius in pixels.

### `outline`

Outline color: `outline #30363d`.

### `outline_width`

Outline width in pixels (default 2).

### `opacity`

0 (invisible) to 1 (default).

### `rotation`

Starting rotation, like `rotation 15deg`.

### `lang`

Language for code highlighting: `lang "c"`, `"python"`, `"rust"`, `"c#"`, ...

### `content`

The text, file path or code of an object; usually written as a string right after the name.

### `fov`

Camera field of view, default `30deg`.

### `z`

3D scenes: depth in pixels, positive toward the viewer. 2D objects at different z are separate planes.

### `layer`

3D scenes: `layer overlay` keeps a 2D object fixed on screen and on top.

### `pin`

3D scenes: `pin model.part` makes a 2D object follow that part on screen; `at` becomes an offset.


## Values

### `overlay`

`layer overlay`: fixed on screen, drawn on top of everything.

### `world`

`layer world`: a plane in the 3D scene (the default).

### `linear`

Easing: constant speed.

### `in`

Easing: starts slow.

### `out`

Easing: ends slow.

### `in_out`

Easing: starts and ends slow (the default).

### `left`

`align left`: `at` is the left edge.

### `center`

`align center`: `at` is the center (the default).

### `right`

`align right`: `at` is the right edge.


## Functions

### `len`

`len(xs)`: number of items in a list, or characters in a string.

### `floor`

`floor(x)`: round down.

### `ceil`

`ceil(x)`: round up. `ceil(len(data) / 16)` rows of 16.

### `round`

`round(x)`: round to the nearest whole number.

### `abs`

`abs(x)`: absolute value.

### `min`

`min(a, b)`: the smaller value (same units).

### `max`

`max(a, b)`: the larger value (same units).

### `mod`

`mod(i, 8)`: remainder, for wrapping into columns.

### `str`

`str(42)`: a number as text, to join with `+`.

### `hex`

`hex(255)` is `"ff"`, `hex(16, 8)` is `"00000010"`.

### `join`

`join(xs, " ")`: list items as one string with a separator.

### `slice`

`slice(xs, start, count)`: part of a list (count may be left out).

### `ascii`

`ascii(bytes)`: bytes like `"41"` as printable text, `.` for the rest, like a hex dump's right column.

### `bytes`

`bytes("payload.bin", offset, count)`: a file's bytes as a list of hex strings like `"41"`. Up to 1 MiB, from the scene's folder.

### `file`

`file("exploit.c")`: a text file's contents, for `code` or `text`. From the scene's folder.

### `table`

`table("packets.csv")`: rows of a .csv or .tsv file as lists of strings: `rows[1][2]`.

Data files (`bytes`, `file`, `table`) must be inside the scene's folder and at most 1 MiB; the preview reloads when they change.

## Values and units

| Write | Meaning |
|---|---|
| `12`, `0.5` | number |
| `2s`, `500ms` | time |
| `45deg`, `1.2rad` | angle (deg are converted to rad) |
| `(x, y)`, `(x, y, z)` | point; pixels from the top left, `z` toward the viewer |
| `#ff8800`, `#ff880080` | color, optionally with alpha |
| `"text"` | string; `\"`, `\\` and `\n` are escapes, may span lines |
| `[a, b, c]`, `xs[i]` | list and indexing |

Math with units is checked: `1s + 500ms` is fine, `1s + 2deg` is an error. Strings join with `+`. `//` starts a comment.

## Built-in library: `std:cybersec`

Import with `import "std:cybersec"`. Every component is centered on its `at`.

### `byte(label, fill)`

One byte: a cell with a label, like "41".

### `memory_row(n, fill)`

A row of n empty cells, 48px apart, centered.

### `memory_grid(rows, cols, fill)`

rows x cols cells, named cell[row * cols + col], centered.

### `panel(title, body, width, height, accent)`

HTTP request or response: title bar plus a monospace body.

### `browser(url, width, height)`

Stylized browser window with an address bar.

### `hexdump(data, cols, size)`

Hex dump of a byte list, like `bytes("payload.bin")`: offset, hex and ASCII
columns, `cols` bytes per row, `size` font size, top left corner at `at`.
Rows are off[r], hex[r] and asc[r], so they can be colored or faded one by one.

