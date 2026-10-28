# Changelog

All notable changes to this project are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).
While pre-1.0, the minor version is bumped for breaking changes.

<!-- next-header -->

## [Unreleased] - ReleaseDate

### Changed

- Windows: a dialog is now owned by the window it belongs to, like a native
  one. It opens centered over that window rather than wherever Windows puts a
  new window, stays above it, has no taskbar button of its own, and shows up
  when that window is shared in a video call. (#72)
- macOS: a dialog is now a child window of the window it opens over. It opens
  centered over that window rather than in the middle of the screen, stays
  above it when that window is clicked, moves and is minimized along with it,
  and shows up when that window is shared in a video call. It also keeps the
  keyboard for as long as it is open, as a modal one does: clicking the window
  beneath no longer makes that window active while its keystrokes went to the
  dialog. (#78)
- X11: a dialog is now modal and transient for the window it opens over, and
  opens centered over it. Its frame only moves and closes it (WSLg still lets
  its edges resize it). (#84)

### Fixed

- `TextInput` and `TextEditor` no longer select the character just typed, for
  the next keystroke to replace, when the selection is empty: after
  `select_all()` on empty text, or after Shift+arrowing away from the caret and
  back. Backspace in that state no longer panics either. (#86)
- Wayland: a dialog opened over another one is now transient for that one, not
  for the main window, so it opens over it. (#85)
- Wayland: keys typed into the window beneath an open dialog no longer reach
  the dialog, where the compositor lets that window take the keyboard. (#85)
- `Painter::display_dpi` answers on X11, off RandR, so `examples/credit_card`
  draws a card at its real 85.60 mm there. (#84)
- X11: keys typed into the window beneath an open dialog no longer reach the
  dialog, where the window manager lets that window take the keyboard. (#84)
- The Wayland backend draws the UI at the size the other backends do. It laid
  its widgets out in raw surface coordinates, so the 96→72 dpi alignment (see
  "How big a logical pixel is") was missing there and a window came out a third
  smaller than the same program under X11. The scale now follows the factor the
  compositor presents the surface at, and a window keeps its logical size when
  that moves. (#76)
- `App::with_ui_scale`, `SAUDADE_UI_SCALE` and `SAUDADE_UI_DPI` work on
  Wayland, having been silently ignored there. (#76)
- Wayland windows are rasterized at the scale they are presented at, through
  `wp_viewporter`, rather than at the integer buffer scale above it: a display
  presented at 125% is drawn at 1.667x and goes to the glass 1:1, instead of
  being drawn at 2.667x for the compositor to rescale by 0.625 — which undid
  the painter's snapping, the grid every edge landed on no longer being the
  panel's. Compositors without the protocol keep the integer path. (#76)
- `Painter::display_dpi` answers on Wayland, off the physical size and current
  mode `wl_output` reports, so `examples/credit_card` draws a card at its real
  85.60 mm there rather than falling back to a nominal 72 logical pixels to the
  inch. (#76)
- Windows: opening a menu or dropdown no longer deactivates the window it drops
  from, and sharing that window in a video call now shows its open menus and
  dropdowns. A popup is now an owned `WS_POPUP` window that never activates,
  like a native menu, instead of an application window of its own. (#71)
- macOS: opening a menu or dropdown no longer takes the keyboard from the
  window it drops from, whose title bar stays active; an open one moves along
  with that window; and sharing that window in a video call now shows it. A
  popup is now a child window of that window that never becomes key, like a
  native menu, instead of a window of its own lying on top: the window server
  carries it along, and ScreenCaptureKit's single-window capture, which Chrome
  shares a window with, takes it in. A click on that window's title bar closes
  it. (#77)
- Windows: a menu or dropdown opened from a window on a display of another
  scale than the primary one is no longer cut off at its right and bottom
  edges. (#73)
- Windows: a click is no longer ignored when the pointer has just come back
  into a window at the very spot it left it — after a menu that opened under
  the pointer closes again, for instance. (#74)
- `examples/scaling` opens its slider at the scale the window is really drawn
  at. It read that once, on the first paint — which on Wayland comes before the
  compositor has reported a scale at all. (#76)
- The `circle_drawer` example's right-click menu now takes the keyboard:
  Escape, the arrow keys and Enter did nothing. `ContextMenu`'s docs now say
  that a widget owning one forwards `accepts_accelerators` to it too. (#75)
- Menus, dropdown lists and context menus no longer show a strip of window
  background along their right and bottom edges, or a square of it out over the
  desktop when they hang off their window. Popup windows are now transparent
  where the menu doesn't cover them on macOS, Windows and Wayland. X11 still
  repaints the app underneath, now up to the popup's edge. (#52)
- Windows: the window beneath an open dialog can no longer be activated while
  the keys typed into it went to the dialog. It is disabled for as long as the
  dialog is open, as the owner of a native modal dialog is, so the keyboard and
  the active title bar stay with the dialog, and a click on that window closes a
  dropdown open in the dialog. (#82)
- With a dialog open over another one, Escape or the close button of the one
  on top closes just that one, rather than both. A `Modal` now consumes the
  Escape it closes on, and a key still held as a window takes the keyboard is
  no longer taken for a fresh press there (Windows, X11). (#83)

## [0.7.0] - 2026-10-24

### Added

- Fullscreen windows! `EventCtx::set_fullscreen` makes the main window fill
  the display it is on — borderless, over the title bar and the taskbar, menu
  bar or panels — or puts it back where it was, at the size it had;
  `toggle_fullscreen` flips it, and `is_fullscreen` reports the current state,
  including a switch the user made through the platform. The new size arrives
  through `layout` like any resize. `WindowConfig::fullscreen` opens a window
  fullscreen. Windows and X11 switch in place, macOS moves the window into its
  own fullscreen space, and on Wayland saudade restores the windowed size the
  compositor needn't suggest. (#62)
- `WindowConfig::icon(SvgImage)` gives a program the icon the desktop shows for
  it. One `include_svg!` mark goes to every place that wants one: the Windows
  title bar (`ICON_SMALL`) and taskbar / Alt-Tab switcher (`ICON_BIG`), both
  rasterized at the window's own DPI, so a 150% display gets a 24-pixel icon
  drawn as 24 pixels rather than a 16-pixel one stretched; `_NET_WM_ICON` on
  X11; `xdg_toplevel_icon_v1` on Wayland, at the sizes the compositor asks for
  (with a 16→128 ladder for compositors that name none, and nothing at all for
  those without the protocol, where the app_id's desktop-entry file is the only
  icon there is); and the dock tile on macOS, whose windows have no icon of
  their own. Dialog windows get it too, so one doesn't sit beside the main
  window wearing the system default. Vectors rather than a bitmap precisely
  because that list spans 16 px to 512: every size is rasterized from the
  geometry, so none of them is a resample. `SvgImage::rasterize_rgba(size)`
  is that rasterization on its own, for anything else that wants an image
  rather than a draw call. (#56)
- Function keys. `NamedKey` gains `F1` to `F24`, which every backend now
  reports as `KeyDown` / `KeyUp` instead of dropping them, and accelerators
  take them like any other key: `with_accel("F5")`, `"Shift+F5"`,
  `"Ctrl+F1"`, labelled the same way in the menu. `Accel::new` builds a
  chord without the command modifier, such as F5 alone. A `match` over
  `NamedKey` without a catch-all arm needs the new variants. (#63)
- `Button::with_icon` leads a push button's label with a compile-time-baked
  `SvgImage`, or shows the icon alone when the label is empty. The icon is
  drawn tinted in the label's color, so it engraves along with the label on a
  disabled button. (#61)
- Two overrides of the scale a window draws at: `SAUDADE_UI_SCALE`, and
  `App::with_ui_scale`, set it outright, and `SAUDADE_UI_DPI` moves the
  reference density every scale is derived against (72 dpi, see _Changed_).
  Both variables accept `auto` and win over the program's own choice, so a UI
  can be tried at another size without a rebuild. The Wayland backend
  implements neither: it renders at the integer buffer scale the compositor
  asks for. (#53)
- `Painter::crisp(rect, |p, frame| …)` runs a frame recipe in device pixels,
  against a `Frame` (`depth`, `ring`, `inside`) that scales and rounds each
  boundary on the way to the buffer — the mechanism behind the crisp-chrome
  fix under _Fixed_, for custom chrome. `Painter::chrome_unit` is the scale
  rounded to a whole pixel, for a lone thin line. `Painter::draw_resampled`
  renders content at a scale of its own and area-averages it into a fixed
  on-screen box, which is what a DPI preview pane wants from a slider. (#53)
- Merged widget frames. Two widgets that share a border, such as a list or
  editor field and its scrollbar, now draw it as one line on HiDPI screens
  instead of two lines a device pixel apart. A frame marks the shared edge as
  merged (`Merged`, `Painter::crisp_merged`, merged variants of the frame
  primitives, and `Painter::push_clip_frame` to clip to the frame's interior).
  `List`, `TextEditor` and the scrollbar thumb use it. (#54)
- `Painter::display_dpi`, the display's physical density where the display
  reports its size (macOS and Windows). (#54)
- `Checkbox::set_rect` moves a checkbox after construction, as `Slider`,
  `Dropdown`, `ProgressBar` and `ScrollBar` already allow — for a layout that
  reflows, e.g. a control pinned to the bottom edge of a resizable window.
  (#53)
- Two examples: `fonts`, a specimen sheet of every font face at a range of
  sizes, and `credit_card`, which draws a bank card at its real size using
  `Painter::display_dpi`. (#54)
- Screenshots of several windows at once: `mock::Desktop` arranges framed
  windows (`MockBackend::place_window`) and open dialogs
  (`MockBackend::place_dialog`) on one desktop, and `WindowChrome::inactive`
  draws a window without the focus. (#65)
- The README shows a screenshot of every example, rendered by the new
  `screenshots` example. (#65)

### Changed

- A logical pixel now targets 72 dpi rather than 96, whatever resolution or
  density the display runs at. That is closer to the screens the Windows 3.1
  metrics were designed for. Windows, X11 and macOS now draw at 4/3 of the
  scale factor they report: 1.333x at 100%, 2x at 150%, 2.667x on a Retina
  Mac. `SAUDADE_UI_DPI=96` restores the old size. Wayland, which renders at
  the compositor's integer buffer scale, is unaffected. (#54)
- Fonts and widgets are adapted to the new size. The theme's `font_size`
  drops from 13 to 11. Push buttons set their labels in bold
  (`Theme::button_style`, drawn with the new `Painter::text_centered_styled`).
  Chrome lines and bevels are a whole number of device pixels thick at every
  scale (`Frame::bevel_ring`). A `List`'s selection band spans the full field
  width. (#54)
- Rounded corners and bevel corners look better at high scales. From 1.75x,
  push buttons and slider thumbs round their corners along a real curve.
  Before, the 1.0x design's missing corner pixel was enlarged into a square
  notch. Below 1.75x the corners are unchanged. Where a bevel's light and dark
  sides meet, at the top-right and bottom-left, they now split the corner
  along a diagonal. Before, the dark side took the whole corner, which looked
  like a staircase from 1.5x up. A `Dropdown`'s arrow button now uses the
  scrollbar arrows' square-cornered chrome instead of a push button's. Custom
  chrome can draw the same shapes with `Frame::rounded_band`,
  `Painter::fill_rounded_band`, `Painter::fill_ring_near` and
  `Painter::fill_ring_far`. (#58)
- The folder icon in `FileDialog` and the `filer` example is slightly
  improved. (#55)

### Removed

- `Painter::wants_1x_crispness`, whose `[0.9, 1.5)` range described a narrower
  fix than the one that replaced it: `Painter::crisp` is unconditional, so a
  widget no longer branches on the scale at all. Custom chrome that gated a
  `Painter::physical` pass on it becomes one `crisp` call. (#53)

### Fixed

- Dragging a window between two Windows displays of different densities no
  longer resizes it in a loop until the app crashes. winit 0.30 sends a window
  back to the display it is leaving whenever the density changes mid-move
  (rust-windowing/winit#4600, fixed on winit `master` but in no 0.30 release),
  and resizing the window from inside the scale change — as the runtime did —
  turned that into a runaway in which every round trip grew it by the ratio
  between the displays, until softbuffer could no longer allocate its surface.
  The runtime now hands its size to winit with the event instead, and places
  the window at the rect Windows suggests before winit gets to place it. A
  crossing is now one scale change each way, whether the window is dragged or
  moved with Win+Shift+Arrow, and a maximized window stays maximized. (#57)
- A dialog that opens on a display of another density than the main window's
  is now drawn at that display's scale. `Painter::system_scale` and
  `Painter::display_dpi` report the dialog's own display while it paints. (#60)
- Button frames, bevels, focus rings and etched dividers are crisp at a
  fractional scale. Each chrome line used to snap on its own, from its own
  position in the window: at 2.33x a frame came out heavier along one edge than
  the opposite one and different again on the button beside it, and a dotted
  focus ring degenerated into a smear of two- and three-pixel dots. Every frame
  primitive — `stroke_rect`, `raised_bevel`, `sunken_bevel`, `button`,
  `light_button`, `focus_rect`, `etched_h_line` — now draws in device pixels off
  a `Frame` that rounds each boundary's _depth_ rather than each line's
  thickness, so a ring is the same thickness on all four sides and the same on
  every widget at that scale, and nothing accumulates. Chrome at 1.0x, below
  1.5x and at integer scales is byte-for-byte unchanged; 1.5x moves, and so does
  `etched_h_line` below it, it having had no crisp pass before. (#53)
- The mouse wheel and trackpad scroll inside dialogs again, such as the file
  list of a `FileDialog`. On Windows, macOS and X11, wheel events over a
  dialog's window were dropped. Wayland was unaffected. (#64)

## [0.6.1] - 2026-08-25

### Fixed

- Walking a `MenuBar` from one menu to the next — the Left / Right cursor keys,
  or the pointer dragged across the labels with the button held — no longer
  closes the menu on the first step. A menu moving along the bar asks for a
  popup window at a new anchor, and that used to tear the open window down and
  build another one: the keyboard went back to the main window in between, and
  0.6.0's new "a menu folds away when the focus leaves it" rule could not tell
  that apart from the user clicking outside. The window is now moved and
  resized in place instead, so the walk never gives the focus up at all.
- Raised the `winit` floor to 0.30.12 so a macOS build cannot resolve a version
  that aborts on startup. Earlier 0.30.x releases leave
  `objc2/relax-sign-encoding` off, and on macOS 26 `NSScreen.screens` hands back
  a Swift-native array whose `countByEnumeratingWithState:` returns `Int` where
  objc2-foundation declares `NSUInteger`; objc2 0.5's debug-only encoding check
  turns that mismatch into a panic inside winit's `extern "C"` app delegate,
  which cannot unwind, so a debug binary died with `SIGABRT` before showing a
  window. The Core Text font path added in 0.6.0 is what pulls the Swift-backed
  AppKit machinery into the process, so 0.6.0 is where a stale lockfile started
  to bite.

## [0.6.0] - 2026-08-25

### Added

- `ContextMenu` — a right-click menu, anchored at a point rather than hanging
  off a bar label. It is the same panel a `MenuBar` drops open, built from the
  same `MenuItem`s and drawn by the same code, so mnemonics, accelerator hints,
  checkmarks, separators and `with_enabled` predicates all behave identically,
  and it gets its own borderless window so it is never clipped by the widget it
  belongs to — a menu right-clicked near a window border hangs off the edge
  rather than being folded back inside it. What bounds it is the _screen_: the
  panel flips rather than crossing an edge of the display, and a menu taller
  than the display is capped to it and scrolls — with the wheel, the arrow keys,
  or a click on either end's arrow. Items are usually built per opening
  (`set_items` then `open_at`); `open_within` bounds the panel by a rect of the
  caller's choosing instead. Apps that had hand-rolled a context menu can drop
  it; the `circle_drawer` example did. (#47)
- On macOS the _system_ fonts now come from Core Text rather than from a font
  file picked out of a database. `Font::load_sans` and `load_monospace` ask for
  the interface font and the user's fixed-pitch font by role, `load_serif` by
  family name, and glyphs are rasterized by the same text stack every other app
  on the desktop draws with. Three things follow: San Francisco can finally be
  emphasized (Core Text instantiates the weight axis a font database cannot see,
  so a heading is a real bold rather than a fallback to regular); text is
  antialiased the way the rest of the desktop is; and a character the interface
  font has no glyph for — an emoji, a CJK ideograph — is drawn by whichever
  installed face does have it instead of silently dropped.

  Fonts an app supplies as bytes (`Font::from_sans_bytes` and the
  `with_*_bytes` builders) stay on the portable fontdue path on every platform,
  so a bundled face renders identically everywhere and snapshot tests are
  unaffected.

  The DPI arrives as a scale matrix rather than as a bigger point size. macOS
  re-picks San Francisco's optical variant on every size change, so asking for a
  26-pixel font on a Retina screen would quietly switch a 13-point label to
  display metrics — around 10% narrower than the same app on a 1x screen. Asking
  instead for 13 points magnified 2x keeps the variant a native app would use,
  keeps advances exactly proportional to the DPI, and keeps a layout the same on
  every display. (#48)

- `Widget::paints_own_background` lets a root widget tell the runtime it fills
  every pixel of its own bounds. The window background pattern is then laid
  down only in the letterbox around those bounds instead of across the whole
  surface, where the widget tree would immediately cover it again. `Container`
  answers yes when it was given a background color; anything else has to opt in
  (and must mean it — the skipped pixels keep whatever the surface buffer held,
  which on most platforms is neither the last frame nor blank). (#49)
- `Painter::screen_area` tells a widget where the display is, in the root
  widget's own coordinates — what a widget placing its own top-level window
  needs, since a popup is not bounded by the app's window and the display is the
  only thing that is. It excludes the space the desktop reserves for itself
  (on macOS the menu bar and the Dock, via `NSScreen.visibleFrame`; X11 and
  Windows report no reservation, which winit does not surface). The runtime works
  it out once per window move / resize / DPI change rather than per frame.
  `MockBackend::with_screen_area` fakes one so the placement is testable
  offscreen; an ordinary offscreen render reports `None`, meaning unbounded.
  (#47)

### Changed

- The candidate font families are now chosen per platform, each chain leading
  with the host's own UI font so an app looks like it belongs on the desktop it
  was opened on: Segoe UI / Tahoma / MS Sans Serif on Windows, the San Francisco
  lineage down through Helvetica Neue and Lucida Grande on macOS, Cantarell /
  Ubuntu / Noto Sans on everything else, each ending in the near-universal
  DejaVu and Liberation faces. Previously one Windows-flavored list served every
  platform, which on macOS meant the Microsoft faces bundled in
  `/System/Library/Fonts/Supplemental` rather than anything Apple ships.
  A variable-weight system font (macOS's San Francisco) offers no bold face to
  load, so it is listed first but passed over until it can carry emphasis — see
  the bold-preference rule above. (#48)
- The arrow glyphs on the scrollbar's end buttons grew from a 5×3 triangle to
  the classic 7×4 one, so they read as arrows rather than as specks — at 1.0x
  the mark is now four rows deep and spans most of the 16px button. We're now
  way closer to the Windows 95 style regarding the button's content—but that's
  fine as the scrollbar thiumb itself is also not 100% resembling the 3.1 style.
  Nothing around them moved: the buttons, the track and the thumb keep their
  metrics, and the wider triangle is still centered in the button at every
  scale. (#51)
- `Painter::fill_pattern` no longer walks the window pixel by pixel. Every
  background pattern repeats after a handful of rows, so only the first period
  is computed and the rest of the surface is filled by copying whole rows —
  around 9x faster on a 2x HiDPI window (2.75 ms → 0.32 ms for 1800×1240),
  which had made the backdrop cost more per frame than the entire widget tree.
  The output is unchanged, pixel for pixel, at every scale and origin. (#49)

### Fixed

- A menu or dropdown now folds away when the keyboard moves out of it: clicking
  the window's title bar (on the way to dragging it), switching to another app,
  or activating another window of this one. A menu is a modal gesture belonging
  to the click that opened it, and every desktop's own menus end it there. Only
  a `PopupKind::Popup` is treated as transient — a dialog window stays put — and
  the dismissal goes through Escape, which the widget owning the popup consumes,
  so a dropdown open inside a dialog closes without taking the dialog with it.
  X11 popups are override-redirect and never hold focus, so nothing there
  changes. (#47)
- On macOS a popup window — every `MenuBar` drop-down, `Dropdown` list and
  `ContextMenu` panel — no longer arrives with the system's window chrome around
  it. `NSWindow`'s drop shadow is off, so the soft grey halo that read as a
  second, blurry frame outside the popup's black border is gone; and its
  `animationBehavior` is `None`, so a menu is on screen the instant it is asked
  for instead of fading in. Real dialog windows (`PopupKind::Dialog`) keep both.
  (#47)
- Bold and italic text now actually renders in the host's bold and italic
  faces. Two things stood in the way. A font _collection_ (`.ttc`) packs
  several faces into one file and fontdb reports which one it matched, but that
  index was dropped on the way to fontdue, so every emphasis face of a
  collection — which on macOS is most of them, Helvetica included — rasterized
  as its regular face. And the family search took the first candidate with a
  usable regular face, even when that family shipped no bold at all: on macOS
  that is Microsoft Sans Serif, one line above Tahoma, which does have a real
  bold. The family chain now prefers a family that can do bold, falling back to
  a regular-only one only when no candidate offers better. (#48)
- `ScrollBar` now honors its `line_step` when scrolled by wheel or trackpad,
  not just by the arrow buttons. A line has always been `line_step` document
  units everywhere else in the bar; the wheel path added raw line counts to
  `value`, so a bar whose unit is a _pixel_ of content — the shape a pane with
  unequal row heights needs — crept a single pixel per line instead of a row.
  The sub-step remainder is now banked in document units too, so a
  high-resolution trackpad still moves such a bar smoothly rather than a whole
  row at a time. Bars that count rows (`line_step` left at its default 1) are
  unaffected. (#50)

## [0.5.0] - 2026-06-10

### Added

- Menu items can be disabled. `MenuItem::with_enabled` attaches a predicate
  that is evaluated live on every paint and every attempt to fire, so a menu
  built once tracks changing application state (typically read through an
  `Rc<RefCell<…>>`). A disabled item renders greyed, shows no hover band, is
  skipped by arrow-key navigation and mnemonics, and never fires — by mouse,
  keyboard, or accelerator. (#46)
- Menu items can show a checkmark. `MenuItem::with_checked` attaches a
  predicate, also read live each paint, that draws a tick in the item's left
  gutter while true — for toggles and radio-style groups. The glyph is the
  same one `Checkbox` uses, tinted to the item's current text color, and it
  rides inside the existing label inset, so the layout never shifts between
  checked and unchecked. The mark is display-only: a checked item still fires
  its callback normally when picked. (#46)
- Menu accelerators are now live, not just decorative. `MenuItem::with_accel`
  takes the new `Accel` type (or its conventional string form: `"Ctrl+R"`,
  `"Ctrl+Shift+Enter"`), and pressing a matching chord while the bar is closed
  fires the item directly. `Accel` names its modifiers as platform-independent
  _roles_ (`AccelMods`): `Accel::primary('r')` means ⌘R on macOS and Ctrl+R
  everywhere else, resolved through a `ModifierScheme` both when matching
  input and when rendering the right-aligned hint. The scheme defaults to the
  build platform's; `MenuBar::with_scheme` pins it, e.g. for snapshot tests
  that must not drift between hosts. A chord whose only matches are disabled
  items falls through unconsumed, keeping its ordinary meaning in the focused
  widget (Ctrl+Left stays word-jump in an editor); while a menu is open it owns
  the keyboard, so chords wait. Breaking: `MenuItem::Action`'s `accel` field is
  now `Option<Accel>` instead of `Option<String>` — `with_accel("Ctrl+R")`
  call sites keep compiling via `From<&str>`, which panics on a malformed
  chord so a typo fails loudly at first use. (#46)
- Text can now be drawn in three font families — **sans-serif**, **serif**, and
  **monospace** — each in **bold**, _italic_, and **_bold-italic_**, all using
  the host's real installed faces rather than a synthesized smear or shear of the
  regular one. Two new enums name the axes: `FontFamily` (`Sans` / `Serif` /
  `Mono`) and `FontStyle` (`Regular` / `Bold` / `Italic` / `BoldItalic`). Draw
  with `Painter::text_styled(.., family, style)` and measure with
  `Painter::measure_text_styled(.., family, style)`; the existing `text` /
  `measure_text` stay sans-serif regular, so nothing already on screen moves. (#44)
- `Font::load_sans` (renamed from `load_system`), `Font::load_serif`, and
  `Font::load_monospace` load the three families, walking — for the serif — the
  classic Win 3.1 / Office serif families (Times New Roman, Georgia, …) down to
  modern Linux replacements. Each loader now also loads the bold, italic, and
  bold-italic faces of the chosen family alongside the regular one. Because real
  bold / italic / serif faces carry their own advance widths, measuring in the
  same family and style the text is drawn keeps word-wrap pixel-accurate. A
  family the host ships without a given face falls back to the nearest _real_
  face it does have (ultimately the regular face), and the loader verifies
  fontdb's match actually carries the requested weight/slant so a regular face is
  never passed off as bold or italic. (#44)
- For deterministic, font-bundling snapshot tests, `Font::from_sans_bytes`
  (renamed from `from_bytes`) gains `with_bold_bytes`, `with_italic_bytes`, and
  `with_bold_italic_bytes` builders to attach the emphasis faces from in-memory
  buffers, and `MockBackend` gains `with_serif_font` alongside `with_sans_font`
  (renamed from `with_font`) / `with_mono_font`.
- `Painter::blit_argb` blits a block of pre-composited, opaque ARGB pixels in
  one call — the bulk path for drawing a decoded or composed image, where a
  grid of per-pixel `pixel()` calls is the bottleneck. Each source pixel still
  snaps to the same physical block `pixel()` would produce and the physical
  clip rect is honored, but the logical→physical snap runs once per row and
  column instead of twice per pixel and the clip is resolved once for the whole
  blit. Alpha is ignored: the source is assumed already flattened to opaque, so
  no per-pixel blending happens. (#45)

### Changed

- **Breaking:** `Painter::new` and `Painter::with_popup_anchor` now take a single
  `FontSet { sans, serif, mono }` in place of the two separate `Option<&Font>`
  arguments. A backend builds one `FontSet` from the fonts it owns; offscreen
  painters with no text pass `FontSet::default()`. This replaces three
  same-typed positional font arguments (easy to transpose) with one named bundle
  and makes room for the serif family. (#44)
- **Breaking renames:** `Font::load_system` → `Font::load_sans`,
  `Font::from_bytes` → `Font::from_sans_bytes`, and `MockBackend::with_font` →
  `MockBackend::with_sans_font`, so each name states the family it loads. (#44)
- **Breaking:** the monospace-specific painter methods `mono_text`,
  `measure_mono_text`, and `mono_cumulative_widths` are removed in favor of the
  general family+style API: draw with `text_styled(.., FontFamily::Mono,
FontStyle::Regular)`, measure with `measure_text_styled(.., FontFamily::Mono,
FontStyle::Regular)`, and get caret offsets with the new
  `Painter::cumulative_widths(text, size, family, style)`. `Font::cumulative_widths`
  likewise gains a `FontStyle` argument. (#44)

### Fixed

- `TextEditor` and `List` no longer jump their scroll position back to the
  caret / selected row when the window merely gains or loses focus. Their
  `layout` re-synced the scrollbar range _and_ scrolled the caret/selection into
  view, but a layout pass also fires for reasons unrelated to editing — on
  Wayland the compositor sends a `configure` (hence a relayout) on every
  activation change — so a wheel-scrolled view snapped back the instant focus
  changed (visible in the `notepad` example as the caret jumping to the first
  line). `layout` now only re-clamps the scroll range to the new viewport;
  edits, keyboard navigation, and selection changes still scroll the
  caret/selection into view as before. (#43)

## [0.4.0] - 2026-06-08

### Added

- `ScrollBar` arrow buttons now behave like real push buttons. Clicking one
  sinks it — a single dark top/left shadow line, no highlight, the arrow glyph
  nudged 1px down-right — and _holding_ it auto-repeats the line-step scroll at
  a keyboard-style cadence (a ~300ms initial delay, then every 50ms) for as long
  as the button stays pressed with the pointer over it; sliding off pauses the
  repeat and pops the button back out, sliding back resumes it. (#41)
- `EventCtx::request_tick` asks the runtime to deliver another `Event::Tick`
  without any ancestor having to forward the request — the _push_ counterpart to
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

- On Wayland, the mouse pointer could keep a stale shape when entering a window
  or surface. Wayland leaves the pointer image undefined on `wl_pointer.enter`
  and makes the client set it, but the runtime deduplicated against the last
  shape it had shown and so skipped re-establishing the arrow on entry. It now
  forces the cursor shape on every enter (plain motion still dedups). (#42)
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
- Dragging files _out_ of a window (drag source), Wayland only:
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
[Unreleased]: https://github.com/roblillack/saudade/compare/v0.7.0...HEAD

[0.7.0]: https://github.com/roblillack/saudade/compare/v0.6.1...v0.7.0
[0.6.1]: https://github.com/roblillack/saudade/compare/v0.6.0...v0.6.1
[0.6.0]: https://github.com/roblillack/saudade/compare/v0.5.0...v0.6.0
[0.5.0]: https://github.com/roblillack/saudade/compare/v0.4.0...v0.5.0
[0.4.0]: https://github.com/roblillack/saudade/compare/v0.3.0...v0.4.0
[0.3.0]: https://github.com/roblillack/saudade/compare/v0.2.0...v0.3.0
[0.2.0]: https://github.com/roblillack/saudade/compare/v0.1.0...v0.2.0
[0.1.0]: https://github.com/roblillack/saudade/releases/tag/v0.1.0
