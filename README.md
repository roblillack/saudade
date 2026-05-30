# retrogui

A minimal, retained-mode GUI library for small Windows 3.1–styled utilities
written in Rust. Built on `winit` + `softbuffer` with `fontdue` + `fontdb`
for text — no GPU, no browser engine, no procedural-macro DSL.

retrogui exists to make tiny dialogs and tools (about boxes, system
viewers, simple text editors, mini control panels) that look like they
fell out of 1992 while staying portable, density-independent, and crisp
on modern displays.


## Status

Pre-1.0, intentionally small. The current widget set is enough to
assemble a Win 3.1 about box, a Notepad-style text editor, and similar
single-window utilities. Scope is roughly that of NeXTSTEP's *WINGs*: a
toolkit for utilities, not for full applications.

Four reference apps live in the same workspace:

| Crate       | What it shows                                            |
|-------------|----------------------------------------------------------|
| `retrofetch`| About-box dialog (`Container`, `Label`, `Button`, `Image`, `Bevel`). |
| `notepad`   | Editor window with menu bar (`MenuBar`, `TextEditor`).   |
| `filer`     | Filesystem browser using `List` with folder/file icons.  |
| `picker`    | Pick-an-item dialog: `List` + buttons + `Dialog`, with Tab/Shift+Tab focus cycling. |


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
* widgets request repaint / window-close / focus via a small `EventCtx`
* the runtime drives `winit` and writes pixels through `softbuffer`
* widgets paint in **logical pixels**; the library handles DPI

The mental model is closer to "a typed, ownership-safe GUI runtime" than
to an object-oriented UI framework.


## Module map

| Module    | Contents                                                        |
|-----------|-----------------------------------------------------------------|
| geometry  | `Point`, `Size`, `Rect`, `Color`                                |
| event     | `Event`, `MouseButton`, `Key`, `NamedKey`, `Modifiers`, `EventCtx` |
| theme     | `Theme`, default `Theme::windows_31()` palette                  |
| painter   | `Painter` — drawing primitives + Win 3.1 chrome helpers         |
| font      | `Font` — system font lookup + glyph rasterization               |
| widget    | `Widget` trait (paint / event / focus / overlay hooks)          |
| widgets   | `Container`, `Column`, `Label`, `Button`, `Bevel`, `Image`, `MenuBar`, `Menu`, `MenuItem`, `ScrollBar`, `TextEditor` |
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
    KeyDown      { key: Key, modifiers: Modifiers },
    KeyUp        { key: Key, modifiers: Modifiers },
    Char         { ch: char, modifiers: Modifiers },
}

pub enum MouseButton { Left, Right, Middle }

pub enum Key {
    Named(NamedKey),  // editing / navigation keys
    Char(char),       // physical key as a logical character
}

pub enum NamedKey {
    Enter, Backspace, Delete, Tab, Escape, Space,
    Left, Right, Up, Down, Home, End, PageUp, PageDown,
}

pub struct Modifiers { pub shift: bool, pub control: bool, pub alt: bool, pub logo: bool }
```

`Event::position()` returns the cursor `Point` for positional events, or
`None` for `PointerLeave` and keyboard events. `Event::is_keyboard()`
distinguishes the three keyboard variants.

`KeyDown` / `KeyUp` are for *keys* — useful for Backspace, arrows, and
modifier-bearing shortcuts. `Char` is for *text input* — what the
keyboard layout decided the user typed. The runtime suppresses `Char`
when a command modifier (Ctrl / Alt / Logo) is held so editors don't
ingest "\x01" for Ctrl+A; the matching `KeyDown` still fires.

`Modifiers::has_command()` is true if any of Ctrl / Alt / Logo is held.

### `EventCtx`

Inside an event handler, widgets receive a mutable `&mut EventCtx` and
can ask the runtime to do things:

```rust
pub struct EventCtx { /* opaque */ }

impl EventCtx {
    pub fn request_paint(&mut self);   // mark window dirty
    pub fn close(&mut self);           // close the window after dispatch
    pub fn request_focus(&mut self);   // become the keyboard target
    pub fn release_focus(&mut self);   // drop keyboard focus
}
```

Widgets never poke at the runtime directly. The runtime collects the
requests after a dispatch completes and applies them all at once, which
keeps event handling deterministic and re-entrancy-free.


## Theme

```rust
pub struct Theme {
    pub background: Color,      // window / workspace fill
    pub face: Color,            // button / menu-bar face
    pub highlight: Color,       // light bevel edge
    pub shadow: Color,          // dark bevel edge
    pub border: Color,          // 1-px outer black border
    pub text: Color,
    pub disabled_text: Color,
    pub highlight_bg: Color,    // selected-item bg (Win 3.1: navy)
    pub highlight_text: Color,  // selected-item fg (Win 3.1: white)
    pub font_size: f32,
}
```

The default is `Theme::windows_31()`: white workspace, light-gray button
face, white top/left highlight, mid-gray bottom/right shadow, black outer
border, navy/white selection, 11pt text. Pass an alternative via
`App::with_theme(...)` if you want to skin the same widgets differently.


## Built-in widgets

All widgets implement `Widget` and own their own state. Coordinates are
always in logical pixels.

### Layout vs. absolute positioning

retrogui ships with two top-level container styles:

* **`Container`** — children are placed at absolute logical-pixel positions.
  This is what you want for *dialogs* (about boxes, simple alerts) that
  have a fixed design size and shouldn't reflow. If the OS gives the
  window a larger buffer than the design, the runtime centers the
  Container and fills the surroundings with `theme.background`.
* **`Column`** — children are stacked top-to-bottom and *flex* with the
  window. Each child is either `add_fixed(widget, height)` (takes a
  declared height) or `add_fill(widget)` (shares whatever space is left
  over). On every window resize, the runtime calls `layout` on the root
  widget; `Column` propagates that to its children, so a menu bar stays
  pinned to the top and a text editor below it grows with the window.

Widgets opt into layout by overriding `Widget::layout(&mut self, bounds:
Rect)`. Most interactive widgets do — `MenuBar`, `TextEditor`, `List`,
`ScrollBar`, `Row`, `Button` and `Checkbox` all store the new rect (and
rebuild any cached geometry), so they reflow inside a `Column`/`Row` or any
container that propagates `layout`. Widgets that don't override `layout`
(e.g. `Label`) keep the position they were given at construction — which is
exactly what `Container`'s children want.

```rust
// Notepad layout: menu bar pinned to the top, editor fills the rest.
// The runtime auto-focuses the first focusable widget (the editor) at
// startup, so the user can type immediately.
let root = Column::new()
    .with_background(Color::WHITE)
    .add_fixed(menu_bar, MENU_BAR_H)
    .add_fill(text_editor);
```

`Column` also handles capture, focus, accelerator routing, and the
overlay pass — same contract as `Container`.

* **`Row`** — the horizontal sibling of `Column`. Same `add_fixed(widget,
  width)` / `add_fill(widget)` API, laying children left-to-right across the
  full height, with the same capture / focus / accelerator / Tab handling.
  Unlike `Column` it carries no overlay layer — keep modal dialogs on the
  top-level container so there's a single overlay owner.

Both `Column` and `Row` expose `focus_child(index)` to choose a non-default
initial focus target (e.g. focus a content list instead of a leading toolbar
field). Custom container widgets outside the crate can reuse retrogui's focus
protocol via `EventCtx::is_focus_requested` / `is_focus_released` /
`clear_focus_flags`.

### `Container`

A flat collection of widgets at absolute positions. The container handles:

* **hit testing** — pointer events go to the top-most child whose bounds
  contain the cursor;
* **pointer capture** — a child whose `captures_pointer()` returns true
  keeps receiving pointer events until it un-captures (used by `Button`
  and `MenuBar`);
* **keyboard focus** — clicking a focusable child makes it the keyboard
  target; keyboard events route there only;
* **focus cycling** — Tab and Shift+Tab walk forward / backward through
  focusable children, wrapping at either end. The container looks at
  each child's `focusable()` and calls `focus_first` on the new target,
  so wrapper widgets that delegate focus to a nested leaf are handled
  transparently;
* **accelerator routing** — keyboard events also go to any child whose
  `accepts_accelerators()` returns true (used by `MenuBar` to catch
  Alt+letter combos while a sibling holds focus);
* **overlay pass** — every widget's `paint_overlay` runs after every
  widget's regular `paint`, so popups (menus, tooltips) draw on top of
  siblings.

```rust
let root = Container::new(395, 305)        // size in logical pixels
    .with_background(Color::WHITE)         // optional fill
    .with_border(Color::BLACK)             // optional 1-px outer border
    .add(Label::new(20, 20, "Hello"))
    .add(Button::new(Rect::new(150, 50, 80, 24), "OK"));

// imperatively:
let mut root = Container::new(395, 305);
root.push(Label::new(20, 20, "Hello"));
```

The runtime calls `Widget::focus_first` on the root once the window is
ready, so a container that holds a `TextEditor` or `List` will hand it
keyboard focus automatically. Override the trait method to choose a
different initial target.

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
    .default(true)
    .on_click(|cx| cx.close());
```

Press behavior matches Windows: pressing inside arms the button,
dragging out un-arms (sunken pops back up), dragging back in re-arms,
releasing inside fires `on_click`, releasing outside cancels.

Buttons are focusable: Tab/Shift+Tab cycle through them and the focused
button draws a dotted focus rectangle inside its bevel. Enter or Space
fires the button while it holds focus.

A button created with `.default(true)` is also the **container's Enter
accelerator**: pressing Enter anywhere inside the same `Container` or
`Column` fires the default button, regardless of which sibling holds
focus. The widget that consumed the event sets `EventCtx::consume_event`
so the focused widget (e.g., a list whose Enter handler would otherwise
activate the selected row) doesn't also react to the same keystroke.

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
logo.set_pixel(1, 1, Color::BLACK);
```

Use `Image::from_pixels(x, y, w, h, pixels)` to attach an externally
decoded raster (PNG/BMP/etc.) as ARGB32.

### `MenuBar`, `Menu`, `MenuItem`

A classic Win 3.1 menu bar. Top labels live in a white bar (matching
Win 3.1's program-manager chrome); clicking one drops a white popup with
a sharp L-shape drop shadow. The currently-open top-level label and any
hovered popup item are drawn with a navy background and white text. The
popup is rendered in the overlay paint pass so it floats over every
sibling widget.

```rust
let menu_bar = MenuBar::new(Rect::new(0, 0, 520, 20))
    .add_menu(Menu::new(
        "&File",
        vec![
            MenuItem::action("&New",   |cx| { /* … */ cx.request_paint(); }),
            MenuItem::action("&Open",  |_| { /* … */ }),
            MenuItem::action("&Save",  |_| { /* … */ }),
            MenuItem::separator(),
            MenuItem::action("E&xit",  |cx| cx.close()),
        ],
    ))
    .add_menu(Menu::new("&Help", vec![
        MenuItem::action("&About", |_| {}),
    ]));
```

**Mnemonics.** Labels may include `&` immediately before a character to
declare the mnemonic. `"&File"` displays as `File` with **F** underlined;
press `Alt+F` (closed bar) or just `F` (open menu) to fire it. Use `&&`
to render a literal `&`. Mnemonics route through the
`accepts_accelerators` hook on the menu bar, so they keep working even
while a `TextEditor` holds keyboard focus.

**Mouse behavior.** A single click on a top-level label opens the menu;
moving the cursor over items highlights them, and a second click on an
item fires it. **A click that opens the menu without dragging
pre-highlights the first action**, so the user can immediately fire it
with Enter or keep arrow-navigating. The press-drag-release gesture
also works: press on a top-level label, drag down through the popup,
release on an item to fire it without an intermediate click.
Releasing anywhere else just disarms the gesture and leaves the menu
open. Sliding the cursor along the bar with a menu open swaps between
top-level menus. Click outside (or press Esc) to dismiss.

**Keyboard navigation** (active while a menu is open):

| Key             | Effect                                              |
|-----------------|-----------------------------------------------------|
| ↑ / ↓           | move highlight to the previous / next action (skipping separators; wraps) |
| Home / End      | jump to first / last action                         |
| ← / →           | switch to the previous / next top-level menu        |
| Enter           | fire the currently highlighted action               |
| letter          | fire the action whose mnemonic matches              |
| Esc             | dismiss the menu                                    |

Menus opened with Alt+letter (or arrow-switched left/right) always
pre-highlight the **first** action of the newly opened menu — the
previous highlight position never carries over. Click-to-open menus
also pre-highlight the first item if the cursor never reached the
popup before release; only drag-style opens leave nothing hovered.

While a menu is active no keyboard event is forwarded to the focused
widget below — typing in an open menu doesn't leak into the editor.

**Popups live in their own window.** When a menu opens, the runtime
spawns a borderless window for the popup, sized exactly to its
contents and behaving like Chrome / Firefox menus on each backend:

* **X11** (through winit): an *override-redirect* window with the
  `_NET_WM_WINDOW_TYPE_DROPDOWN_MENU` hint. The WM is bypassed
  entirely, so the popup appears instantly at the requested position
  and size and can extend beyond the main window's edges. The runtime
  also re-anchors it via `Window::set_outer_position` whenever the
  main window emits a `Moved` event, so the popup follows window
  drags.
* **Wayland** (through smithay-client-toolkit): a real `xdg_popup`
  surface created with an `xdg_positioner` anchored to the parent
  surface. The compositor handles placement, follow-on-drag, and
  auto-dismiss (sending `popup_done`, which we translate into a
  synthesized Escape).

The popup is dismissed by clicking outside it (the main window
receives the click and the menu folds up), pressing Escape, or firing
an item.

`MenuBar::open(idx)` programmatically opens a menu — handy for custom
application-level keybindings.

### `ScrollBar`

A Win 3.1 scrollbar: two arrow buttons bracketing a track with a
proportionally-sized thumb. Built standalone — embed it next to any
scrollable view, or let `TextEditor` carry one for you.

```rust
let mut bar = ScrollBar::vertical(Rect::new(380, 20, 16, 280));
bar.set_range(/* viewport */ 20, /* max */ 60);  // 80-row file, 20 visible
bar.set_value(0);
bar.set_line_step(1);
```

Interaction:

| Input              | Effect                                            |
|--------------------|---------------------------------------------------|
| click arrow        | scroll by `line_step` toward the arrow            |
| click track        | scroll by `viewport` (one page) toward the click  |
| drag thumb         | scroll proportionally to the drag distance        |

The thumb is sized as `track_extent × viewport / (viewport + max)` with a
sane minimum so it stays grabbable even on huge documents. Use
`SCROLLBAR_THICKNESS` (16 logical pixels) to lay siblings out around it.

### `TextEditor`

A minimal multi-line text editor: sunken white field, monospace text,
vertical cursor, selection, cut/copy/paste against the OS clipboard,
and a built-in vertical scrollbar pinned to the right edge. Only the
visible rows are measured and drawn each paint, so large files stay
cheap. Designed for system-utility editors (Notepad-style); undo and
word wrap come later.

```rust
let mut editor = TextEditor::new(Rect::new(4, 24, 512, 312))
    .with_font_size(11.0)
    .with_text("Hello\nWorld");

let text: String = editor.text();
```

The editor renders with the monospace font loaded by the runtime
(Consolas / Courier / Liberation Mono / DejaVu Sans Mono, in that
preference order). The rest of the UI (menu labels, dialog text) keeps
the proportional default — pick whichever font you want per call via
`Painter::text` vs `Painter::mono_text`.

Editing operations:

| Input               | Effect                                         |
|---------------------|------------------------------------------------|
| typing              | inserts the character (replaces selection)     |
| Backspace           | deletes the previous char or the selection     |
| Delete              | deletes the next char or the selection         |
| Enter               | splits the line (replacing the selection)      |
| ← / →               | move cursor one character                      |
| ↑ / ↓               | move cursor one line, clamping column          |
| Home / End          | jump to line start / end                       |
| PageUp / PageDown   | jump by one viewport                           |
| Shift + any move    | extends the selection                          |
| Ctrl + A            | select all                                     |
| Ctrl + C            | copy selection to the OS clipboard             |
| Ctrl + X            | cut selection to the OS clipboard              |
| Ctrl + V            | paste at the cursor (replaces selection)       |
| left click          | place the cursor                               |
| drag with left      | extend the selection                           |

Selected text renders with `theme.highlight_bg` (navy) behind it and
`theme.highlight_text` (white) on top. Multi-line selections show a
small visual continuation past end-of-line so the band looks unbroken.

Programmatic methods mirror the keyboard shortcuts so menu items can
invoke the same operations:

```rust
editor.cut();
editor.copy();
editor.paste();
editor.select_all();
```

The clipboard handle is lazily initialized via `arboard`; in headless
environments where the OS clipboard isn't reachable, `copy`/`cut`/
`paste` simply become no-ops — editing still works. On Wayland sessions
arboard is built with the `wayland-data-control` feature so it speaks
the native `wlr-data-control` protocol; clipboard exchange with other
Wayland-native apps works without needing XWayland.

`TextEditor` keeps content as `Vec<String>` (one entry per line) and
tracks `(row, col)` in *characters*, not bytes — multi-byte UTF-8 is
handled correctly. Per-character widths are cached during paint so a
click can be mapped to a column position without a `Painter` at event
time — and the cache is keyed by row, so only rows currently on screen
contribute work. The scrollbar's canonical position is its own
`value()`; the editor reads it (no duplicate state). Clicking focuses
the widget; the cursor only renders while focused; vertical scroll
follows the cursor automatically.


## The `Widget` trait

If a built-in doesn't fit, implement `Widget` yourself:

```rust
pub trait Widget {
    fn bounds(&self) -> Rect;
    fn paint(&mut self, painter: &mut Painter, theme: &Theme);
    fn paint_overlay(&mut self, _painter: &mut Painter, _theme: &Theme) {}
    fn event(&mut self, _event: &Event, _ctx: &mut EventCtx) {}
    fn captures_pointer(&self) -> bool { false }
    fn focusable(&self) -> bool { false }
    fn set_focused(&mut self, _focused: bool) {}
    fn accepts_accelerators(&self) -> bool { false }
    fn layout(&mut self, _bounds: Rect) {}
    fn focus_first(&mut self) -> bool { /* focus self if focusable */ }
    fn popup_request(&self) -> Option<PopupRequest> { None }
}
```

* `bounds` is the widget's logical-pixel hit rectangle.
* `paint` draws the widget using `Painter` and the active `Theme`.
* `paint_overlay` runs after every sibling's `paint` — for popups,
  tooltips, drag previews. Default: no-op.
* `event` reacts to typed input; default is no-op.
* `captures_pointer` keeps pointer events flowing to this widget while
  it's true, even if the cursor leaves its bounds (used by buttons
  during press, by menus while open).
* `focusable` flags the widget as a keyboard target. The container only
  routes keyboard events to focused children.
* `set_focused` is called when the widget gains or loses focus — use
  this to show/hide a cursor, commit pending input, etc.
* `accepts_accelerators` makes the widget receive keyboard events even
  without focus — used by menu bars for Alt+letter combos.
* `layout` is called by a layout-aware parent (e.g., `Column`) whenever
  the available rect changes. Widgets used in absolutely-positioned
  layouts ignore it; flexible widgets store the new rect and propagate
  it to their own children.
* `focus_first` is called by the runtime on the root widget once the
  window is configured. The default focuses `self` if `focusable()` is
  true; `Container` and `Column` override it to walk their children and
  delegate, so the first focusable widget in the tree becomes the
  initial keyboard target without any manual wiring.
* `popup_request` returns `Some` while the widget wants the runtime to
  host a popup (e.g., menubar dropdowns) in its own top-level window.
  Containers propagate it from their children; the runtime polls it
  after each event burst and opens / repositions / closes the popup
  window to match.

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

A monospace counterpart is loaded the same way via
`Font::load_monospace`, preferring Lucida Console → Consolas → Courier
New → Courier → Liberation Mono → DejaVu Sans Mono → Menlo → Monaco. If
none of those match, fontdb's monospace flag is used as a fallback.
`Painter::mono_text` / `Painter::measure_mono_text` use that font;
`Painter::text` / `Painter::measure_text` keep using the proportional
default.

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
WindowConfig::new("Notepad", 520, 340).resizable(true);
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


## Backends

retrogui picks the windowing backend at startup based on the session:

* If `WAYLAND_DISPLAY` is set and non-empty, the runtime talks **pure
  smithay-client-toolkit** — no winit on the Wayland code path.
  This is what gets us real `xdg_popup` popups and lets us drop
  winit's `wayland-csd-adwaita` and `wayland-dlopen` features from
  the dependency tree.
* Otherwise (X11, including XWayland when `WAYLAND_DISPLAY` is unset)
  the runtime drives winit 0.30 with only the `x11` feature enabled.
  Popups are X11 override-redirect windows.

The widget tree, painter, fonts, clipboard, theme, and every public
API are identical across both paths — only `app.rs` + `wayland.rs`
differ.


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
not stretch** — it stays at its natural logical size. What happens
around it depends on the root widget:

* a `Container` (absolute positioning) keeps its design size; the
  runtime centers it and fills the surroundings with `theme.background`,
  so dialogs always look the same regardless of window size;
* a `Column` (layout container) receives the new bounds via
  `Widget::layout` and reflows its children so the window's chrome and
  content fill the available space — pixels stay the same physical size
  but, e.g., the editor grows wider and taller.

Resize **never** scales pixels — it only changes how much space is
available for layout decisions.

Trade-off to be aware of: at non-integer scale factors (1.25, 1.5,…) a
1-logical-pixel chrome line can land on a y-coordinate where the
physical width rounds to 1 vs 2 pixels. The variation is invisible in
practice on the dialogs we've built; if you hit a case where it
matters, draw chrome at a fixed `round(scale)` thickness using
`Painter::scale()`.


## End-to-end example: a Notepad-style editor

```rust
use std::cell::RefCell;
use std::rc::Rc;

use retrogui::{
    App, Container, Event, EventCtx, Menu, MenuBar, MenuItem, Painter, Rect,
    TextEditor, Theme, Widget, WindowConfig,
};

const W: i32 = 520;
const H: i32 = 340;
const BAR_H: i32 = 20;

fn main() {
    let editor = Rc::new(RefCell::new(
        TextEditor::new(Rect::new(4, BAR_H + 4, W - 8, H - BAR_H - 8))
            .with_text("Hello, retrogui!"),
    ));

    let menu_bar = MenuBar::new(Rect::new(0, 0, W, BAR_H))
        .add_menu(Menu::new(
            "&File",
            vec![
                MenuItem::action("&New", {
                    let editor = editor.clone();
                    move |cx| {
                        editor.borrow_mut().set_text("");
                        cx.request_paint();
                    }
                }),
                MenuItem::separator(),
                MenuItem::action("E&xit", |cx| cx.close()),
            ],
        ));

    let root = Container::new(W, H)
        .with_background(retrogui::Color::WHITE)
        .add(menu_bar)
        .add(SharedEditor(editor.clone()));

    App::new(WindowConfig::new("Notepad", W, H).resizable(true), root).run();
}

// Tiny adapter so the menu callbacks can mutate the shared editor.
struct SharedEditor(Rc<RefCell<TextEditor>>);

impl Widget for SharedEditor {
    fn bounds(&self) -> Rect { self.0.borrow().bounds() }
    fn paint(&mut self, p: &mut Painter, t: &Theme) { self.0.borrow_mut().paint(p, t) }
    fn event(&mut self, e: &Event, c: &mut EventCtx) { self.0.borrow_mut().event(e, c) }
    fn focusable(&self) -> bool { self.0.borrow().focusable() }
    fn set_focused(&mut self, f: bool) { self.0.borrow_mut().set_focused(f) }
}
```

A more complete version, including Open/Save against a path passed as
`argv[1]`, lives in `notepad/src/main.rs` in this repository.


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

* `Grid` container (the horizontal `Row` sibling of `Column` now exists)
* `RadioButton` (single-line `TextInput`, `Checkbox` and `List` now exist)
* Horizontal scrolling in `TextEditor` (a horizontal `ScrollBar` is
  already implemented; the editor just doesn't ride it yet)
* Mouse-wheel scroll events
* Multi-line / wrapping `Label`
* Undo / redo in `TextEditor`
* Save-As / Open file dialogs
* Optional bitmap fonts for fully retro-faithful text
* Multi-window support
* Native menu bars where the platform offers them

Things explicitly **not** on the roadmap: a 3D scene graph, async
runtimes, plugin systems, themable web components.


## License

Same as the parent retrofetch project.
