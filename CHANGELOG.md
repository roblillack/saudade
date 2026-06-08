# Changelog

All notable changes to this project are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).
While pre-1.0, the minor version is bumped for breaking changes.

<!-- next-header -->

## [Unreleased] - ReleaseDate

### Added

- `ScrollBar` arrow buttons now behave like real push buttons. Clicking one
  sinks it — a single dark top/left shadow line, no highlight, the arrow glyph
  nudged 1px down-right — and *holding* it auto-repeats the line-step scroll at
  a keyboard-style cadence (a ~300ms initial delay, then every 50ms) for as long
  as the button stays pressed with the pointer over it; sliding off pauses the
  repeat and pops the button back out, sliding back resumes it. (#41)
- `EventCtx::request_tick` asks the runtime to deliver another `Event::Tick`
  without any ancestor having to forward the request — the *push* counterpart to
  `Widget::wants_ticks`. Like `request_paint`, it rides the shared `EventCtx`
  straight back to the runtime, so a widget buried under custom wrapper widgets
  can drive a transient animation on its own. It is one-shot: a widget that
  needs a stream re-requests on each tick. The scrollbar's hold-to-repeat uses
  it, which is why it works even inside a wrapper that doesn't forward
  `wants_ticks` (such as the `filer` example's `FileBrowser`). (#41)
- `FocusLabel` is a caption that carries a keyboard mnemonic and moves focus to
  the field beside it. Mark the accelerator with `&` exactly like a menu label
  (`"Last &name:"` underlines the **n** and binds **Alt+N**); pressing it
  transfers focus to the next focusable widget added to the same parent — the
  classic "buddy label" convention. The accelerator reaches the label even while
  a sibling holds focus, via a new `EventCtx::request_focus_next` request that
  `Container`, `Column`, and `Row` resolve. See the new `focus_form` example.
  (#39)
- `MockBackend::render_framed` now paints the window background pattern behind
  the content for regular (resizable / fixed) windows, matching the live
  backend's main surface; dialogs stay plain, as they do on screen. The pattern
  defaults to the live default (a `superlight` forward-diagonal hatch) and is
  overridable with the new `MockBackend::with_background_pattern`. (#38)
- `List` gained optional multi-selection, off by default so existing
  single-selection lists are unchanged. Enable it with `List::with_multi_select`
  / `set_multi_select`: Ctrl/Cmd+click toggles a row, Shift+click and Shift+Arrow
  select a contiguous range, and `selected_indices` / `set_selected_indices` read
  and set the whole set. A plain press on an already-selected row defers
  collapsing the selection until release, so a wrapper can drag the whole group
  out — the `picker` and `filer` examples now do. To carry the click modifiers,
  `Event::PointerDown` and `Event::PointerUp` now include a `modifiers` field.
  (#37)
- Widgets can request the mouse-pointer shape while handling a pointer event
  via `EventCtx::set_cursor`, choosing from the new `Cursor` enum (arrow, hand,
  I-beam, resize handles, …). The runtime applies it after each move on both
  backends (`wp_cursor_shape` on Wayland, `CursorIcon` on X11/Windows/macOS) and
  falls back to the arrow when no widget asks. `TextInput` / `TextEditor` show
  the I-beam over their text; every other widget keeps the default arrow. (#42)
- `WindowConfig::min_size` sets the smallest inner size a resizable window may
  be dragged to (in logical pixels). The window manager enforces the bound, so
  layouts never see sizes below it. (#36)

### Fixed

- On X11, dragging a `ScrollBar` / `Slider` thumb (or any captured press) no
  longer stops the moment the pointer leaves the window. winit reports the
  cursor crossing the window edge as a `CursorLeft` even while X11's implicit
  pointer grab keeps motion flowing during a held button, so the runtime took it
  for a real leave and ended the drag. It now ignores that leave while a button
  is held and a widget is capturing the pointer, so the drag keeps tracking
  up/down motion until release — matching the Wayland backend, whose compositor
  sends no leave during its implicit grab. (#40)

## [0.3.0] - 2026-06-07

### Fixed

- The `filer` example no longer confuses scrolling with dragging a file out.
  The drag-out gesture armed on a press anywhere inside the list bounds — which
  include the scrollbar pinned to the right edge — so grabbing the thumb both
  scrolled and armed a drag, and the drag won on the next move (yanking a file
  out instead of scrolling). It now yields the scrollbar strip via the new
  `List::scrollbar_hit`. Relatedly, `ScrollBar` and `Slider` now end an
  in-progress thumb drag on `PointerLeave`: with no OS pointer grab, a drag
  interrupted by the pointer leaving the window (as an outbound drag-and-drop
  does by revoking pointer focus) left a stale drag flag set, so the thumb
  chased the cursor when it returned. (#35)
- `TextEditor` is no longer pathologically slow to repaint with large or
  long-lined documents. Each frame rebuilt every visible row's caret-offset
  table by re-measuring every prefix of the line (O(n²)) and re-rasterized every
  glyph from scratch — including ones scrolled off the right edge — and the
  runtime repaints the whole tree on every scroll notch and resize step. The
  font now caches rasterized glyphs (in a bounded LRU) and per-glyph advances,
  the caret table is built in one O(n) pass over those advances, and
  `Font::draw_phys` stops at the clip's right edge. Output is snapshot-identical;
  the worst case is ~100× faster. (#34)
- `include_svg!` now maps every contour through its `abs_transform` (the full
  ancestor chain, viewBox→viewport origin offset and scale included), while still
  framing the baked image by the SVG's declared viewport (the box resvg renders
  into). This fixes SVGs that previously baked mis-scaled or off-frame — a viewBox
  with a non-zero origin or an `<svg>` whose width/height differ from the viewBox
  — without disturbing artwork that is deliberately padded inside its viewBox
  (the scrollbar, dropdown, dialog, and checkbox marks). (#27, #31)
- Firing a menu item by its keyboard mnemonic no longer leaks the letter into
  whatever the item opens. Picking File → Open with Alt+F, O previously typed an
  "o" into the dialog's freshly focused File name field; the menu now swallows
  the keystroke through its release. (#30)

### Added

- `FileDialog`: a modern, single-pane Open / Save file picker (built on `Modal`)
  with the current path along the top, one combined list of folders and files,
  a "File name" field, and a "File types" filter dropdown — the flat layout
  modern KDE / Windows pickers use. Section labels carry Alt+L / Alt+N / Alt+T
  accelerators. Glob-based `FileFilter`s drive the filter. The `notepad` example
  now uses it for File → Open and File → Save As. (#26)
- Window-chrome screenshots: `MockBackend::render_framed` wraps a rendered
  client area in Canoe's default desktop style — a teal background, a soft drop
  shadow, a navy active title bar, and a window frame. Choose the frame via
  `WindowChrome::resizable` / `fixed` / `dialog` (`WindowFrame`), which mirror
  Canoe's three window paints and differ in their window controls and border;
  `with_desktop_background` / `with_margin` tweak the backdrop. Windows are
  always drawn active. See the new `chrome` example. (#33)
- `include_svg!` now honors `clip-path`: clip regions are intersected with the
  drawn geometry at build time (via `i_overlay`), so clipped artwork bakes
  correctly instead of being dropped. `i_overlay` is a compile-time-only
  dependency of `saudade-macros` and never reaches a shipped binary. (#27)
- `include_svg!` takes an optional `crop` argument —
  `include_svg!("logo.svg", crop)` — that frames the baked image by the tight
  bounding box of the drawn geometry instead of the SVG's declared viewport,
  dropping any padding so the mark fills its target rect. The default is still
  viewport framing (matching resvg). (#31)
- `include_svg!` now approximates linear and radial gradient paint instead of
  dropping it: each gradient bakes into a stack of flat-color bands (strips for
  linear, nested disks for radial) clipped to the painted shape. Gradient fills
  and strokes are no longer reported as unsupported. (#27)
- File drag-and-drop: drop files from the OS onto a window. New `Event::DragEnter`
  / `DragMove` / `DragLeave` / `Drop` events carry a `DragData` of file paths,
  and a drop target opts in by calling `EventCtx::accept_drop()` while handling
  `DragEnter` / `DragMove`. Works on macOS, Windows, X11, and Wayland. See the
  `dnd` example. (#23)
- Dragging files *out* of a window (drag source), Wayland only:
  `EventCtx::start_drag()` begins an OS `text/uri-list` drag from a widget's
  press-and-drag gesture, with an icon that follows the cursor and shows a green
  checkmark over a target that accepts the drop or a red cross elsewhere. The
  winit backends (macOS, Windows, X11) expose no API to initiate a drag, so it
  is a no-op there. See the `filer` example. (#23)
- `Dropdown` popups now scroll: a list longer than 12 rows caps the popup height
  and grows a vertical scrollbar — mouse wheel, draggable thumb, Page Up/Down,
  and scroll-the-selection-into-view all work — so a long list (e.g. the full
  set of keyboard layouts) stays usable instead of opening a popup taller than
  the screen. (#28)
- `ScrollBar::end_drag()` to abandon an in-progress thumb drag, for hosts that
  can be torn down mid-drag (such as a dropdown popup that closes on focus
  loss). (#28)
- `ListItem::with_svg_icon`: list rows can now show a compile-time-baked
  `SvgImage` (from `include_svg!`), drawn crisply at any DPI, alongside the
  existing raster `ListIcon`. (#32)
- `EventCtx::swallow_key_until_release()`: a handler that fully acts on a key
  press can ask the runtime to discard the rest of it — the trailing `Char`,
  autorepeat, and the release. (#30)
- `Painter::light_button()` (a lighter chrome frame: square outline, single
  top/left highlight, 2px bottom/right shadow) and `Painter::fill_checker()` (a
  two-tone DPI-aware checkerboard fill). (#29)

### Changed

- The folder / file / up-arrow icons in the file dialog and the `filer` example
  are now real SVG assets (`assets/icons/*.svg`) baked via `include_svg!` and
  shared between the two, instead of hand-coded pixel buffers. (#32)
- `ScrollBar` chrome now matches Win 3.1 more closely: the arrow buttons and
  thumb use the lighter `light_button` frame (square outline, one highlight line
  instead of two), the track gains a thin black outline that collapses into the
  button/thumb frames where they meet, the arrow glyphs sit centered on the
  button face with the classic margin instead of filling it edge to edge, and
  the empty track shows the classic black-on-gray "newsprint"
  checkerboard instead of a flat gray fill. (#29)
- Adjacent `ScrollBar` outlines no longer double up into a 2px band where they
  meet — each shared edge collapses to a single 1px line: a thumb slid flush
  against an arrow button shares that button's edge, and a scrollbar embedded in
  a `List` or `TextEditor` shares its outer edge with the field's border. (#29)

## [0.2.0] - 2026-06-06

### Added

- Compile-time SVG support: the `include_svg!` macro bakes an SVG into
  flattened polygons at build time and expands to a `const SvgImage`, so no SVG
  parser is linked into the binary. The parsing weight (usvg + kurbo) is
  confined to the new `saudade-macros` proc-macro crate and runs only at compile
  time. (#22)
- `Modal::on_cancel()` to run a handler when a modal is dismissed; `App` now
  also invokes `on_cancel()` for top-level windows. (#18, #19)
- `Container::layout()` for custom child layout. (#17)
- Nested popups — a popup opened from within another popup. (#11)
- Painting at physical size, with a dedicated code path for crisper rendering at
  1.25× scale. (#13)
- The system scale factor is now readable on Wayland. (#16)

### Changed

- Reworked the dialog architecture. (#21)
- Refreshed checkbox and dropdown styling. (#24, #25)

### Fixed

- Buttons now activate on key release rather than on key press. (#14)
- Corrected button autorepeat behavior. (#20)
- FreeBSD: load fonts from `/usr/local/share/fonts`. (#12)

## [0.1.0] - 2026-06-01

Initial release.

<!-- next-url -->
[Unreleased]: https://github.com/roblillack/saudade/compare/v0.3.0...HEAD
[0.3.0]: https://github.com/roblillack/saudade/compare/v0.2.0...v0.3.0
[0.2.0]: https://github.com/roblillack/saudade/compare/v0.1.0...v0.2.0
[0.1.0]: https://github.com/roblillack/saudade/releases/tag/v0.1.0
