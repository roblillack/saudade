# retrogui

A minimal, retained-mode GUI library for small Windows 3.1–styled utilities
written in Rust. Built on `winit` + `softbuffer` with `fontdue` + `fontdb`
for text — no GPU, no browser engine, no procedural-macro DSL.

retrogui exists to make tiny dialogs and tools (about boxes, system
viewers, mini control panels) that look like they fell out of 1992 while
staying portable, density-independent, and crisp on modern displays.


## Status

Pre-1.0, intentionally small. The current widget set is enough to assemble
a Win 3.1 about box. Scope is roughly that of NeXTSTEP's *WINGs*: a
toolkit for utilities, not for full applications.


## At a glance

```rust
use retrogui::*;

fn main() {
    let root = Container::new(220, 100)
        .with_background(Color::WHITE)
        .with_border(Color::BLACK)
        .add(Label::new(20, 20, "Hello, retrogui!"))
        .add(
            Button::new(Rect::new(70, 60, 80, 24), "OK")
                .default(true)
                .on_click(|cx| cx.close()),
        );

    App::new(WindowConfig::new("Hello", 220, 100), root).run();
}
```


## Adding retrogui to your project

retrogui currently lives inside the retrofetch repository as a workspace
member. Add it via a path dependency:

```toml
# Cargo.toml
[dependencies]
retrogui = { path = "../retrogui" }
```

It will be published as a regular crate once the API has settled.


## Design philosophy

retrogui follows the architecture sketched in `retrogui.md`:

* widgets are ordinary Rust values implementing the `Widget` trait
* events are typed Rust enums — no integer message IDs
* widgets request repaint / window-close via a small `EventCtx`
* the runtime drives `winit` and writes pixels through `softbuffer`
* widgets paint in **logical pixels**; the library handles DPI

The mental model is closer to "a typed, ownership-safe GUI runtime" than
to an object-oriented UI framework.


## Module map

| Module    | Contents                                                        |
|-----------|-----------------------------------------------------------------|
| geometry  | `Point`, `Size`, `Rect`, `Color`                                |
| event     | `Event`, `MouseButton`, `EventCtx`                              |
| theme     | `Theme`, default `Theme::windows_31()` palette                  |
| painter   | `Painter` — drawing primitives + Win 3.1 chrome helpers         |
| font      | `Font` — system font lookup + glyph rasterization               |
| widget    | `Widget` trait                                                  |
| widgets   | `Container`, `Label`, `Button`, `Bevel`, `Image`                |
| app       | `App`, `WindowConfig` — runtime entry point                     |

Everything user-facing is re-exported from the crate root; you generally
just `use retrogui::*;`.


## Core types

### `Color`

Packed 32-bit ARGB. Helpers cover the Win 3.1 default palette:

```rust
Color::rgb(0x40, 0x40, 0x40);
Color::argb(0x80, 0x00, 0x00, 0xFF); // half-transparent blue
Color::BLACK;      Color::WHITE;
Color::LIGHT_GRAY; Color::MID_GRAY; Color::DARK_GRAY;
Color::NAVY;       Color::RED;
Color::GREEN;      Color::YELLOW;
Color::TRANSPARENT;
```

`Color::TRANSPARENT` is used by `Image` to mark "skip this pixel".

### `Point`, `Size`, `Rect`

```rust
let p = Point::new(10, 20);
let s = Size::new(60, 24);
let r = Rect::new(10, 20, 60, 24);

assert!(r.contains(Point::new(15, 25)));
assert_eq!(r.right(), 70);
assert_eq!(r.bottom(), 44);
let inset = r.inset(2); // shrinks by 2 px on every side
```

All coordinates are *logical* pixels (i32). The library multiplies by the
OS-reported scale factor when drawing.


## Events

```rust
pub enum Event {
    PointerMove  { pos: Point },
    PointerDown  { pos: Point, button: MouseButton },
    PointerUp    { pos: Point, button: MouseButton },
    PointerLeave,
}

pub enum MouseButton { Left, Right, Middle }
```

`Event::position()` returns the cursor `Point` for positional events, or
`None` for `PointerLeave`.

Inside an event handler, widgets receive a mutable `&mut EventCtx` and
can ask the runtime to do things:

```rust
pub struct EventCtx { /* opaque */ }

impl EventCtx {
    pub fn request_paint(&mut self);   // mark window dirty
    pub fn close(&mut self);           // close the window after dispatch
}
```

Widgets never poke at the runtime directly. The runtime collects the
requests after a dispatch completes and applies them all at once, which
keeps event handling deterministic and re-entrancy-free.


## Theme

```rust
pub struct Theme {
    pub background: Color,
    pub face: Color,
    pub highlight: Color,
    pub shadow: Color,
    pub border: Color,
    pub text: Color,
    pub disabled_text: Color,
    pub font_size: f32,
}
```

The default is `Theme::windows_31()`: white workspace, light-gray button
face, white top/left highlight, mid-gray bottom/right shadow, black outer
border, 11pt text. Pass an alternative via `App::with_theme(...)` if you
want to skin the same widgets differently.


## Built-in widgets

All widgets implement `Widget` and own their own state. Coordinates are
always in logical pixels.

### `Container`

A flat collection of widgets at absolute positions. This is the only
container retrogui ships with right now — enough for WINGs-style dialog
layouts. It handles **capture-on-press dispatch** so a widget that goes
"pressed" on `PointerDown` keeps receiving events (including
`PointerMove` outside its bounds) until `PointerUp`.

```rust
let root = Container::new(395, 305)        // size in logical pixels
    .with_background(Color::WHITE)         // optional fill
    .with_border(Color::BLACK)             // optional 1-px outer border
    .add(Label::new(20, 20, "Hello"))
    .add(Button::new(Rect::new(150, 50, 80, 24), "OK"));

// or, imperatively:
let mut root = Container::new(395, 305);
root.push(Label::new(20, 20, "Hello"));
```

Add order matters: later widgets paint on top and are hit-tested first.

### `Label`

A single line of text positioned by its top-left corner. Inherits color
and size from the active `Theme` unless overridden.

```rust
Label::new(10, 10, "Plain label");
Label::new(10, 30, "Smaller").with_size(8.0);
Label::new(10, 50, "Red").with_color(Color::RED);
```

There is no built-in multi-line / word-wrap label yet.

### `Button`

A classic Win 3.1 push button: raised face by default, sunken while
pressed, optional 1-pixel outer black border for the dialog's default
action.

```rust
Button::new(Rect::new(317, 16, 60, 22), "OK")
    .default(true)                       // adds default-button border
    .on_click(|cx| cx.close());          // closure runs on release-inside
```

Press behavior matches Windows: pressing inside arms the button, dragging
out un-arms (sunken pops back up), dragging back in re-arms, releasing
inside fires `on_click`, releasing outside cancels.

### `Bevel`

Decorative chrome — no events, no state.

```rust
Bevel::etched_line(20, 200, 350);                       // two-tone divider
Bevel::raised(Rect::new(10, 10, 100, 30));              // raised frame
Bevel::sunken(Rect::new(10, 50, 100, 30));              // sunken frame
```

### `Image`

A static ARGB32 pixel buffer at an absolute position. Pixels with
`alpha == 0` are skipped (transparent). Useful for small procedural
glyphs and logos:

```rust
let mut logo = Image::new(0, 0, 40, 28);
logo.fill_rect(Rect::new(2, 2, 16, 10), Color::RED);
logo.fill_rect(Rect::new(20, 4, 16, 10), Color::GREEN);
// ...
logo.set_pixel(1, 1, Color::BLACK);
```

To load PNG/BMP, decode externally and hand the ARGB32 buffer to
`Image::from_pixels(x, y, w, h, pixels)`.


## The `Widget` trait

If a built-in doesn't fit, implement `Widget` yourself:

```rust
pub trait Widget {
    fn bounds(&self) -> Rect;
    fn paint(&mut self, painter: &mut Painter, theme: &Theme);
    fn event(&mut self, _event: &Event, _ctx: &mut EventCtx) {}
    fn captures_pointer(&self) -> bool { false }
}
```

* `bounds` is the widget's logical-pixel hit rectangle.
* `paint` draws the widget using `Painter` and the active `Theme`.
* `event` reacts to typed input; default is no-op.
* `captures_pointer` enables capture-on-press: while it returns `true`,
  the parent `Container` keeps routing every pointer event to this
  widget until it returns `false` again. `Button` uses this so a press
  that drags off and back on still fires.

Minimal custom widget:

```rust
struct ColorBox { rect: Rect, color: Color }

impl Widget for ColorBox {
    fn bounds(&self) -> Rect { self.rect }

    fn paint(&mut self, p: &mut Painter, _theme: &Theme) {
        p.fill_rect(self.rect, self.color);
        p.stroke_rect(self.rect, Color::BLACK);
    }
}
```


## Painter API

`Painter` is the only thing widgets use to draw. It exposes a
logical-pixel API; internally it snaps to physical pixels at the current
DPI.

### Low-level primitives

```rust
p.fill(color);                              // clear the whole surface
p.fill_rect(rect, color);
p.stroke_rect(rect, color);                 // 1-logical-px outline
p.h_line(x, y, w, color);
p.v_line(x, y, h, color);
p.pixel(x, y, color);                       // 1×1 logical pixel
```

### Win 3.1 chrome helpers

```rust
p.raised_bevel(rect, theme.highlight, theme.shadow);
p.sunken_bevel(rect, theme.highlight, theme.shadow);
p.etched_h_line(x, y, w, theme);            // dark + light two-tone line
p.button(rect, theme, pressed, default);    // full button face + bevels
```

### Text

```rust
p.text(x, y, "Hello", 11.0, Color::BLACK);
p.text_centered(rect, "OK", 11.0, Color::BLACK);

let size = p.measure_text("Hello", 11.0);   // returns Size in logical px
```

`Painter::font()` returns the loaded font, if any. If no system font
could be loaded, text calls become no-ops; layout code that depends on
text measurement should be defensive.

### Querying state

```rust
let s = p.size();    // physical buffer size in pixels
let z = p.scale();   // f32 logical-to-physical scale (e.g. 1.0, 1.25, 2.0)
```


## Font handling

`Font::load_system()` walks `fontdb` for a reasonable proportional sans
serif, preferring MS Sans Serif → Microsoft Sans Serif → Tahoma → Segoe
UI → Arial → Helvetica → Geneva → DejaVu Sans → Liberation Sans, then
falling back to any face it can load. Returns `Option<Font>` — `None`
means no font was found, and the painter silently skips text.

The runtime calls `Font::load_system()` once at startup and hands the
font reference to every `Painter` it constructs.

retrogui does **not** ship a bundled bitmap font, so its text rendering
inherits the local system font. The Win 3.1 chrome still looks right,
but the typography will be Liberation Sans on most Linux boxes rather
than MS Sans Serif — close enough for retro nostalgia, not faithful to
the pixel.


## Runtime

### `WindowConfig`

```rust
pub struct WindowConfig {
    pub title: String,
    pub size: Size,        // logical pixels
    pub resizable: bool,
}

WindowConfig::new("About Retrofetch", 395, 305);
WindowConfig::new("Editor", 640, 480).resizable(true);
```

### `App`

```rust
App::new(window_cfg, root_widget)
   .with_theme(Theme::windows_31())   // optional
   .run();                            // blocks until window closes
```

`App::run` consumes the `App`, creates the winit event loop + softbuffer
surface, loads a system font, and dispatches events to the widget tree
until the user closes the window or a widget calls `EventCtx::close`.

You can have at most one `App` per process today; multi-window support
is on the roadmap.


## DPI and resizing

Widgets always work in **logical pixels**. The library handles the
transformation to physical pixels itself.

* The window is requested at `LogicalSize(size.w, size.h)`. winit + the
  compositor pick the physical buffer for the monitor's actual DPI.
* The `Painter` uses `winit.scale_factor()` (a possibly-fractional `f32`,
  e.g. 1.0, 1.25, 1.5, 2.0) directly.
* Rectangle edges are snapped independently to physical pixels —
  adjacent rects always share an exact pixel boundary, so chrome stays
  crisp regardless of DPI.
* Text is rasterized once at `font_size × scale` physical pixels via
  fontdue. No upscale, no resample, no blur.

When the window is resized larger than the design size, **content does
not stretch**. It stays at its natural logical size; the runtime
centers it and fills the surroundings with `theme.background`. This
matches retrogui's philosophy: resizing a dialog reflows content, it
doesn't zoom it.

Reflow itself is per-widget; built-in widgets are absolutely positioned
today, so they stay put. A constraints-based layout pass (Column / Row /
Grid) is on the roadmap and will fit on top of this scale model
without breaking existing widget code.

Trade-off to be aware of: at non-integer scale factors (1.25, 1.5,…) a
1-logical-pixel chrome line can land on a y-coordinate where the
physical width rounds to 1 vs 2 pixels. The variation is invisible in
practice on the dialogs we've built; if you hit a case where it
matters, draw chrome at a fixed `round(scale)` thickness using
`Painter::scale()`.


## Non-goals

The library does **not**:

* emulate HTML/CSS
* embed a browser engine
* provide immediate-mode-only APIs
* rely on heavy procedural-macro DSLs
* hide ownership semantics
* support GPU rendering, animation, or accessibility yet

It is meant to stay small enough that you can hold the whole codebase in
your head.


## Roadmap

Things that would fit retrogui's spirit but aren't there yet:

* `Column` / `Row` / `Grid` containers with constraints-based layout
* `TextBox` and `Checkbox` widgets
* Keyboard focus and keyboard events
* Multi-line / wrapping `Label`
* Optional bitmap fonts for fully retro-faithful text
* Multi-window support
* Native menu bars where the platform offers them

Things explicitly **not** on the roadmap: a 3D scene graph, async
runtimes, plugin systems, themable web components.


## License

Same as the parent retrofetch project.
