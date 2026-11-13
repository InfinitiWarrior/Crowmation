# Crowmation guide

This guide walks through writing scenes, from a first shape to 3D. Every
word of the language is listed in the [reference](reference.md).

1. [First scene](#first-scene)
2. [Objects and positions](#objects-and-positions)
3. [Timing](#timing)
4. [Reuse: constants, lists, loops, components](#reuse)
5. [Layout](#layout)
6. [Effects: stagger, highlight, shake](#effects)
7. [Text, code and math](#text-code-and-math)
8. [Images](#images)
9. [Data from files](#data-from-files)
10. [3D](#3d)
11. [Exporting](#exporting)
12. [Editors](#editors)
13. [Limits and safety](#limits-and-safety)

## First scene

Save this as `hello.cwm`:

```cwm
scene "hello" {
  size (1920, 1080)
  background #0d1117

  circle dot radius 40 color #388bfd at (400, 540)
  text label "Hello" size 64 at (960, 300)

  at 0.5s fade_in label
  after move dot to (1520, 540) duration 2s
  with color dot to #e5484d duration 2s
}
```

Open a live preview, which reloads every time you save:

```sh
crowmation preview hello.cwm
```

Space plays and pauses, Left/Right step one frame, dragging scrubs. When it
looks right, export it:

```sh
crowmation render hello.cwm hello.mp4
```

## Objects and positions

A scene has two kinds of lines: **declarations** create objects, **actions**
animate them. A declaration is the object type, a name, then properties:

```
rect box size (200, 100) corner 12 color #21262d outline #30363d at (960, 540)
```

- Positions are pixels from the top left corner; `y` grows downward.
- Objects are centered on their `at`. Lines and arrows use `from` and `to`.
- Later declarations are drawn on top of earlier ones.
- `color`, `opacity`, `rotation` and `scale` set the starting state.

## Timing

Each action has a start and a `duration` (default `1s`). Where it starts
depends on what comes before it:

| Line | Starts |
|---|---|
| `at 2s move a ...` | at 2 seconds |
| `move a ...` or `after move a ...` | when the previous action ends |
| `after 0.5s move a ...` | half a second after the previous action ends |
| `after -0.2s move a ...` | 0.2 seconds *before* it ends (overlap) |
| `with move b ...` | together with the previous action |
| `wait 1s` | (not an action) pushes the next `after` back by 1 second |

`with` actions join a group: the next `after` waits for the longest action in
the group to finish.

```cwm
scene {
  rect a size (80, 80) at (300, 300)
  rect b size (80, 80) at (300, 500)

  at 0s move a by (600, 0) duration 2s
  with move b by (600, 0) duration 1s   // runs alongside, ends at 1s
  after rotate a by 90deg               // starts at 2s, after the group
  wait 1s
  after fade_out b                      // starts at 4s
}
```

An object whose first action is `fade_in` starts invisible. Easing is
`in_out` by default; set `ease linear`, `in` or `out` on any action.

## Reuse

**Constants and lists** keep numbers in one place:

```cwm
scene {
  let left = 200
  let gap = 72
  let labels = ["41", "42", "43"]
  repeat 3 as i {
    text t[i] content labels[i] size 32 at (left + i * gap, 300)
  }
  at 0s fade_in t[0]
  after -0.6s fade_in t[1]
  after -0.6s fade_in t[2]
}
```

Units are checked: `2s + 500ms` is a time, `left + 1s` is an error.

**Loops** (`repeat n as i`) can hold declarations and actions. A staggered
animation is an `after` with a negative gap inside a loop.

**Components** group objects. Each instance gets a name, and its parts are
addressed as `instance.part`:

```cwm
component cell(label, fill) {
  rect box size (56, 56) corner 6 color fill
  code t content label size 22
}

scene {
  cell a label "41" fill #21262d at (400, 300)
  cell b label "42" fill #21262d at (480, 300)

  at 0s fade_in a                       // the whole instance
  after color b.box to #e5484d          // one part
  after move a by (0, 100)
}
```

An instance is a group: moving, rotating, scaling or fading it applies to
all of its parts, around the instance's `at`. `color` recolors every part.

Components can animate themselves. Their actions run when the instance is
declared (or at the instance's own timing), and `at` inside a component
counts from there:

```cwm
component pop(label) {
  rect box size (56, 56) corner 6 color #21262d
  code t content label size 22
  fade_in box duration 200ms
  with fade_in t duration 200ms
  after scale box by 1.15 duration 120ms
  after scale box by 0.87 duration 120ms
}

scene {
  pop a label "41" at (400, 300)
  after 0.2s pop b label "42" at (480, 300)   // starts after a finishes
}
```

**Imports** load components from another file in the scene's folder
(`import "lib/memory.cwm"`) or from the built-in library:

```cwm
import "std:cybersec"

scene {
  browser web url "https://shop.example/login" width 900 height 500 at (960, 540)
  panel req title "Request" body "POST /login" width 420 height 160 accent #388bfd at (960, 900)
  at 0s fade_in web
  after fade_in req
}
```

## Layout

Instead of computing coordinates, place an object next to another one:

```cwm
scene {
  text title "Stack frame" size 48 at (960, 120)
  text sub "of greet()" size 24 color #6e7681 below title gap 8
  rect box size (400, 200) corner 12 color #21262d below sub gap 40
  text note "return address" size 22 right_of box gap 16 align left
}
```

`below`, `above`, `left_of` and `right_of` center the object on the other
one along the other axis, `gap` pixels apart (default 24). They work for
components too, using the bounding box of all their parts. `align left` or
`align right` makes `at` the left or right edge of a text instead of its
center.

## Effects

**`stagger`** starts each element of a list (`cell[0]`, `cell[1]`, ...) or
each part of a group a bit after the previous one:

```cwm
scene {
  repeat 8 as i {
    rect cell[i] size (56, 56) corner 6 color #21262d at (500 + i * 62, 400)
  }
  fade_in cell stagger 80ms duration 300ms
  after highlight cell color #e5484d
  after shake cell by 10 times 3
}
```

Naming a list without an index (`cell`) targets all of its elements.

**`highlight`** scales an object up and back (`scale 1.12` by default) and,
with `color`, flashes it. **`shake`** moves it side to side `times` times by
`by` pixels. Both take 600ms unless given a `duration`.

## Text, code and math

Text, code and math are laid out by [Typst](https://typst.app/) and drawn as
vector outlines, so they stay sharp at any scale.

```cwm
scene {
  text title "The *gets()* problem" size 56 at (960, 150)
  code src "
    char buf[8];
    gets(buf);   // no length check
  " lang "c" size 28 at (960, 400)
  math bound "sum_(i=0)^(n-1) 1 = n > 8" size 40 at (960, 650)
}
```

- `text` uses Inter and Typst markup: `*bold*`, `_italic_`.
- `code` is highlighted for `lang` and its indentation is stripped, so it can
  be written indented inside the scene.
- `#` is Typst's scripting character and is not allowed in `text` and `math`;
  write `\#` for a literal `#`. Code blocks can contain anything.
- In strings, `\"`, `\\` and `\n` are escapes; other backslashes pass through.

## Images

```
image shot "assets/screenshot.png" width 800 at (960, 540)
```

PNG and JPEG files inside the scene's folder. Give `width` to keep the
aspect ratio, or `size (w, h)`.

## Data from files

Show real bytes, code and tables instead of typing them in:

```
import "std:cybersec"

scene {
  let payload = bytes("payload.bin")          // ["41", "41", ...]
  hexdump dump data payload cols 16 size 22 at (300, 200)
  code src content file("exploit.c") lang "c" size 20 at (960, 700)
  let rows = table("ports.csv")               // rows[1][0] is a cell
  text first content rows[1][0] at (1500, 300)

  fade_in dump stagger 100ms
}
```

- `bytes("file", offset, count)` gives a list of hex strings, `file(...)` a
  text file's contents, `table(...)` the rows of a `.csv` or `.tsv` file.
- Files must be inside the scene's folder and at most 1 MiB. The preview
  reloads when they change.
- `hexdump` (in `std:cybersec`) draws offset, hex and ASCII columns; its rows
  are `off[r]`, `hex[r]` and `asc[r]`, so `highlight dump.hex[1]` works.
- Functions for working with data: `len`, `slice`, `join`, `hex`, `ascii`,
  `str`, `floor`, `ceil`, `round`, `abs`, `min`, `max`, `mod`. See the
  [reference](reference.md#functions).

## 3D

A scene becomes 3D when it declares a `camera` or a `model`. Until the camera
moves, everything 2D looks exactly as before: the default camera shows the
`z = 0` plane at one unit per pixel.

```
camera cam fov 30deg at (960, 600)
model router "models/router.glb" size 760 at (960, 640)

text title "Inside a home router" size 56 layer overlay at (960, 90)
text lbl "SPI flash" size 30 pin router.flash_chip at (0, -100)

at 0s fade_in router
after tilt cam to 32deg duration 1.6s
with orbit cam to -25deg duration 1.6s
after move router.lid by (0, -300)
after color router.flash_chip to #e5484d
with fade_in lbl
after explode router by 0.4
```

- **Models** are self contained `.glb` files. Every glTF node with a mesh
  becomes a part, named after the node: `router.lid`, `router.flash_chip`.
  `size` is the model's largest dimension in pixels. Parts are drawn in their
  material's base color with flat shading and outlines.
- **Camera:** `orbit` turns around the vertical axis, `tilt` looks down,
  `scale` zooms, `move` pans the point it looks at.
- **2D and 3D share space.** 2D objects sit on planes facing the camera at
  their `z` (positive is toward the viewer), so a model can pass through a
  card and a card can hide part of a model. `move card to (x, y, z)` moves
  in depth.
- `layer overlay` keeps an object fixed on screen, on top of everything.
- `pin part` makes a 2D object follow a 3D part; its `at` becomes an offset.
- **Section cuts:** `section router to 0.45 axis x` cuts the model away,
  keeping that fraction. `axis x` cuts from the right, `y` from the top
  (default), `z` from the front. Outlines follow the cut edges.

## Exporting

```sh
crowmation render scene.cwm out.mp4                  # NVENC if available, else openh264
crowmation render scene.cwm out.mp4 --quality high   # ~10% smaller, ~2.5x slower
crowmation frame scene.cwm 3.5 still.png             # one frame
crowmation check scene.cwm                           # errors only, no rendering
crowmation check scene.cwm --dump-ir                 # print the compiled timeline
```

Output is H.264 in MP4, BT.709, at the scene's size and fps (width must be a
multiple of 8). On NVIDIA GPUs the hardware encoder is used; elsewhere the
video is split into segments encoded in parallel with openh264. Frames where
nothing changes are not rendered again, so pauses cost almost nothing.

## Editors

`crowmation lsp` gives Neovim and VS Code errors as you type, highlighting of
your own components, objects and parts, go to definition (also into imported
files), completion and hover docs. Setup is in the
[README](../README.md#editor-support).

## Limits and safety

Scenes are safe to open from anyone:

- No unbounded loops or recursion: `repeat` counts are constants (at most
  10000) and components cannot use themselves.
- No file access beyond images, models and imports inside the scene's folder.
  Typst has no file or network access.
- Frame size is at most 4096×4096, fps at most 240, length at most an hour.
