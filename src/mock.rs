//! Offscreen backend for snapshot / image-based tests.
//!
//! The [`MockBackend`] mirrors the rendering pipeline of the live runtime
//! (`App::paint_main` and friends) but draws into an owned ARGB32 pixel
//! buffer instead of a winit/softbuffer surface. After a render the
//! resulting [`Snapshot`] exposes the raw pixels or a PNG encoding suitable
//! for `insta::assert_binary_snapshot!`.
//!
//! ```no_run
//! use saudade::*;
//! use saudade::mock::MockBackend;
//!
//! let mut root = Container::new(120, 40)
//!     .with_background(Color::WHITE)
//!     .add(Label::new(Rect::new(10, 12, 100, 16), "Hi"));
//!
//! let snap = MockBackend::new(120, 40).with_scale(2.0).render(&mut root);
//! let png_bytes = snap.to_png();
//! ```
//!
//! Input simulation is not implemented yet — once added, callers will be
//! able to feed [`Event`](crate::event::Event)s into the same backend to
//! drive widgets between renders.

use crate::background::{BackgroundPattern, BackgroundState, PATTERN_COLORS};
use crate::chrome::{self, WindowChrome};
use crate::event::{Event, EventCtx};
use crate::font::{Font, FontSet};
use crate::geometry::{Color, Point, Rect, Size};
use crate::painter::Painter;
use crate::theme::Theme;
use crate::widget::{PopupKind, PopupRequest, Widget};

/// Offscreen renderer used by snapshot tests.
pub struct MockBackend {
    logical_size: Size,
    scale: f32,
    theme: Theme,
    font: Option<Font>,
    serif_font: Option<Font>,
    mono_font: Option<Font>,
    /// The window background pattern painted behind the widget tree, exactly
    /// as the live backend's main surface does. Only [`Self::render_framed`]
    /// honors it (and only for non-dialog frames); the bare [`Self::render`]
    /// client-area pass stays plain.
    bg: BackgroundState,
    /// What [`Painter::screen_area`] reports, for widgets that place a popup
    /// against the display. `None` — the default — is the honest answer for an
    /// offscreen render; [`Self::with_screen_area`] fakes one.
    screen: Option<Rect>,
    /// What [`Painter::display_dpi`] reports. `None` by default, for the same
    /// reason; [`Self::with_display_dpi`] fakes one.
    display_dpi: Option<f32>,
    /// Whether the pretend window is fullscreen: what dispatches report through
    /// [`EventCtx::is_fullscreen`], updated as soon as a widget asks — the
    /// mock honors [`EventCtx::set_fullscreen`] on the spot. A `Cell` because
    /// [`Self::dispatch`] takes `&self`.
    fullscreen: std::cell::Cell<bool>,
}

/// Which backdrop the content pass fills behind the widget tree, mirroring the
/// two fills the live runtime uses: a plain theme fill for the popup/dialog
/// pass, and the window background pattern for a regular main window.
#[derive(Clone, Copy)]
enum Backdrop {
    Plain,
    Pattern,
}

impl MockBackend {
    /// Create a backend that paints into a buffer matching a logical
    /// window of `width × height` pixels. The physical buffer size is
    /// `logical × scale` (rounded), exactly as the live runtime would
    /// receive from winit at the same DPI.
    pub fn new(width: i32, height: i32) -> Self {
        Self {
            logical_size: Size::new(width.max(1), height.max(1)),
            scale: 1.0,
            theme: Theme::default(),
            font: None,
            serif_font: None,
            mono_font: None,
            // The live runtime's default backdrop (`BackgroundState::from_env`
            // with nothing set): a `superlight` forward-diagonal hatch.
            bg: BackgroundState {
                pattern: BackgroundPattern::DiagonalForward,
                color: PATTERN_COLORS[0].1,
            },
            screen: None,
            display_dpi: None,
            fullscreen: std::cell::Cell::new(false),
        }
    }

    /// Start with the pretend window fullscreen (or not, the default). Widgets
    /// see it through [`EventCtx::is_fullscreen`], and [`Self::render_framed`]
    /// leaves the window chrome off, as a fullscreen window has none.
    pub fn with_fullscreen(self, fullscreen: bool) -> Self {
        self.fullscreen.set(fullscreen);
        self
    }

    /// Whether the pretend window is fullscreen — as configured, or as a widget
    /// last asked through [`EventCtx::set_fullscreen`] while handling an event
    /// from [`Self::dispatch`].
    pub fn is_fullscreen(&self) -> bool {
        self.fullscreen.get()
    }

    /// Pretend the window sits on a display whose usable area is `area`, in the
    /// root widget's coordinates — what the live runtime works out from the
    /// monitor and the window's position. Widgets that keep a popup on screen
    /// (a [`ContextMenu`](crate::ContextMenu) panel) read it back through
    /// [`Painter::screen_area`], so this is how a test exercises that placement
    /// without a window server. Unset by default, which reads as "no screen to
    /// speak of" and leaves such a widget unbounded.
    pub fn with_screen_area(mut self, area: Rect) -> Self {
        self.screen = Some(area);
        self
    }

    /// Pretend the window sits on a display fitting `dpi` device pixels to the
    /// inch, which widgets drawing something at its real size read back through
    /// [`Painter::display_dpi`]. Unset by default: an offscreen render has no
    /// glass to measure.
    pub fn with_display_dpi(mut self, dpi: f32) -> Self {
        self.display_dpi = Some(dpi);
        self
    }

    /// Set the logical→physical scale factor. Defaults to 1.0. Use this
    /// to exercise fractional-DPI snapping at e.g. 1.25, 1.5, 2.0.
    pub fn with_scale(mut self, scale: f32) -> Self {
        self.scale = scale.max(0.01);
        self
    }

    pub fn with_theme(mut self, theme: Theme) -> Self {
        self.theme = theme;
        self
    }

    /// Supply the sans-serif font used for [`Painter::text`]. Tests should
    /// bundle a known font (e.g. via `include_bytes!`) so glyph output stays
    /// byte-identical across machines.
    pub fn with_sans_font(mut self, font: Font) -> Self {
        self.font = Some(font);
        self
    }

    /// Supply the serif font used for [`Painter::text_styled`] with
    /// [`FontFamily::Serif`](crate::FontFamily::Serif).
    pub fn with_serif_font(mut self, font: Font) -> Self {
        self.serif_font = Some(font);
        self
    }

    /// Supply the monospace font used by the text editors and by
    /// [`Painter::text_styled`] with [`FontFamily::Mono`](crate::FontFamily::Mono).
    pub fn with_mono_font(mut self, font: Font) -> Self {
        self.mono_font = Some(font);
        self
    }

    /// Override the window background pattern painted behind the widget tree by
    /// [`Self::render_framed`] for regular (resizable / fixed) windows. Defaults
    /// to the live runtime's default — a `superlight` forward-diagonal hatch.
    /// Dialog frames ignore this and stay plain, matching the live backend.
    pub fn with_background_pattern(mut self, pattern: BackgroundPattern, color: Color) -> Self {
        self.bg = BackgroundState { pattern, color };
        self
    }

    /// Physical pixel size of the buffer that [`Self::render`] will
    /// produce.
    pub fn physical_size(&self) -> Size {
        let w = (self.logical_size.w as f32 * self.scale).round().max(1.0) as i32;
        let h = (self.logical_size.h as f32 * self.scale).round().max(1.0) as i32;
        Size::new(w, h)
    }

    /// Send a synthetic event to a widget tree, returning the
    /// [`DispatchOutcome`] flags the widget set on its `EventCtx`. Used by
    /// tests to drive focus / keyboard behavior without spinning up the
    /// full winit / Wayland runtime.
    pub fn dispatch(&self, root: &mut dyn Widget, event: &Event) -> DispatchOutcome {
        let mut ctx = EventCtx::with_fullscreen(self.fullscreen.get());
        root.event(event, &mut ctx);
        if let Some(fullscreen) = ctx.fullscreen_request {
            self.fullscreen.set(fullscreen);
        }
        DispatchOutcome {
            paint_requested: ctx.paint_requested,
            close_requested: ctx.close_requested,
            fullscreen_request: ctx.fullscreen_request,
        }
    }

    /// Lay out the widget at the backend's logical size, paint into a
    /// fresh buffer, and return a [`Snapshot`]. If the widget reports a
    /// [`PopupRequest`], the popup pass is
    /// composited on top of the main pass so the snapshot looks the same
    /// as what the user sees on-screen.
    pub fn render(&self, root: &mut dyn Widget) -> Snapshot {
        // The bare client-area pass stays plain, like the live popup/dialog
        // pass; the window background pattern is reserved for the framed,
        // regular-window path (see [`Self::render_framed`]).
        self.render_backdrop(root, Backdrop::Plain, true)
    }

    /// Shared body of [`Self::render`] and the content pass of
    /// [`Self::render_framed`] and [`Self::place_window`]. `backdrop` selects
    /// what is filled behind the
    /// widget tree before it paints — a plain theme fill or the window
    /// background pattern — exactly mirroring the live runtime's two passes.
    ///
    /// Without `dialogs`, an open dialog — and anything opened in it — is left
    /// out, as it is a window of its own on a [`Desktop`].
    fn render_backdrop(
        &self,
        root: &mut dyn Widget,
        backdrop: Backdrop,
        dialogs: bool,
    ) -> Snapshot {
        let physical = self.physical_size();
        // The runtime derives the logical content rect from the actual
        // physical buffer size rather than the requested logical size —
        // mirror that here so fractional scales (1.25, 1.5) match.
        let logical_w = (physical.w as f32 / self.scale).round().max(1.0) as i32;
        let logical_h = (physical.h as f32 / self.scale).round().max(1.0) as i32;
        root.layout(Rect::new(0, 0, logical_w, logical_h));

        let mut pixels = vec![0u32; (physical.w * physical.h) as usize];
        let (origin_x, origin_y) = origin_centered(self.logical_size, self.scale, physical);

        {
            let mut painter = Painter::with_popup_anchor(
                &mut pixels,
                physical.w,
                physical.h,
                self.scale,
                origin_x,
                origin_y,
                self.fonts(),
                None,
            )
            .with_screen(self.screen)
            .with_display_dpi(self.display_dpi);
            match backdrop {
                Backdrop::Plain => painter.fill(self.theme.background),
                // Mirrors the live runtime: a root that paints its own
                // background gets only the letterbox around it.
                Backdrop::Pattern if root.paints_own_background() => painter.fill_pattern_around(
                    self.theme.background,
                    self.bg.pattern,
                    self.bg.color,
                    root.bounds(),
                ),
                Backdrop::Pattern => {
                    painter.fill_pattern(self.theme.background, self.bg.pattern, self.bg.color)
                }
            }
            root.paint(&mut painter, &self.theme);
        }

        // Composite the popup stack outermost-first, so a dropdown opened
        // inside a dialog lands on top of the dialog — exactly the layering the
        // live runtime produces with its stack of popup windows.
        let mut popups = Vec::new();
        root.collect_popups(&mut popups);
        for req in &popups {
            // Outermost-first, so whatever follows a dialog was opened in it.
            if !dialogs && req.kind == PopupKind::Dialog {
                break;
            }
            let popup_phys_x = origin_x + (req.rect.x as f32 * self.scale).round() as i32;
            let popup_phys_y = origin_y + (req.rect.y as f32 * self.scale).round() as i32;
            let popup_phys_w = (req.rect.w as f32 * self.scale).round() as i32;
            let popup_phys_h = (req.rect.h as f32 * self.scale).round() as i32;
            let mut painter = Painter::with_popup_anchor(
                &mut pixels,
                physical.w,
                physical.h,
                self.scale,
                origin_x,
                origin_y,
                self.fonts(),
                Some(req.rect),
            )
            .with_screen(self.screen)
            .with_display_dpi(self.display_dpi);
            painter.set_clip_phys(popup_phys_x, popup_phys_y, popup_phys_w, popup_phys_h);
            root.paint(&mut painter, &self.theme);
            painter.clear_clip();
        }

        Snapshot {
            width: physical.w,
            height: physical.h,
            pixels,
        }
    }

    /// Render `root` at the backend's logical size, then compose Canoe-style
    /// window chrome around it — a desktop backdrop, a soft drop shadow, a
    /// title bar with window controls, and a frame — returning a [`Snapshot`]
    /// of the whole framed window. This is the screenshot path for capturing a
    /// window the way a user sees it on the desktop, rather than just its
    /// client area (which is what [`Self::render`] produces).
    ///
    /// A regular ([`WindowFrame::Resizable`] / [`WindowFrame::Fixed`]) window
    /// paints the [background pattern](Self::with_background_pattern) behind its
    /// content, exactly as the live backend's main surface does; a
    /// [`WindowFrame::Dialog`] stays plain, matching the live runtime's
    /// popup/dialog pass.
    ///
    /// The window is drawn *active* (focused) unless the chrome is
    /// [inactive](WindowChrome::inactive). [`WindowChrome`] picks the title and
    /// the frame style — [`WindowFrame::Resizable`], [`WindowFrame::Fixed`], or
    /// [`WindowFrame::Dialog`] — which differ in their window controls and
    /// border, matching Canoe's three window paints.
    ///
    /// With no desktop to put it on, a dialog `root` has open is drawn inside
    /// the window, as [`Self::render`] draws it. To show it as the window of
    /// its own it is on a real desktop, arrange both on a [`Desktop`].
    ///
    /// A [fullscreen](Self::is_fullscreen) window has no chrome to draw: it is
    /// the whole display, so the snapshot is just its content, over the
    /// window background.
    ///
    /// [`WindowFrame::Resizable`]: crate::WindowFrame::Resizable
    /// [`WindowFrame::Fixed`]: crate::WindowFrame::Fixed
    /// [`WindowFrame::Dialog`]: crate::WindowFrame::Dialog
    pub fn render_framed(&self, root: &mut dyn Widget, chrome: &WindowChrome) -> Snapshot {
        let content = self.render_backdrop(root, Backdrop::for_chrome(chrome), true);
        if self.is_fullscreen() {
            return content;
        }
        let m = chrome::metrics(content.size(), self.scale, chrome);
        let mut framed = Snapshot::filled(m.buffer, chrome.desktop_background());
        self.paint_frame(&mut framed, (0, 0), &m, chrome, &content);
        framed
    }

    /// Render `root` and place it on `desktop`, framed in `chrome`, with its
    /// frame's top-left corner at `at` (logical desktop pixels) — the
    /// several-windows version of [`Self::render_framed`]. The window and its
    /// drop shadow go over whatever is already on the desktop, so place windows
    /// back to front. The desktop has its own color and size, so the chrome's
    /// desktop color and margin don't apply.
    ///
    /// A dialog `root` has open is left out: on a desktop it is a window of its
    /// own, which [`Self::place_dialog`] places. An open popup — a menu, a
    /// dropdown's list — still draws over the window, as it does in
    /// [`Self::render`].
    pub fn place_window(
        &self,
        desktop: &mut Desktop,
        at: Point,
        root: &mut dyn Widget,
        chrome: &WindowChrome,
    ) {
        let content = self.render_backdrop(root, Backdrop::for_chrome(chrome), false);
        self.place(desktop, at, &content, chrome);
    }

    /// Place the dialog `root` has open on `desktop` as the window of its own
    /// the live runtime opens for it: framed as a dialog and titled with the
    /// dialog's title, its frame's top-left corner at `at` (logical desktop
    /// pixels). It is drawn active, as a modal dialog has the focus. Of several
    /// open dialogs, the frontmost is placed.
    ///
    /// Returns `false`, placing nothing, if `root` has no dialog open.
    pub fn place_dialog(&self, desktop: &mut Desktop, at: Point, root: &mut dyn Widget) -> bool {
        // The main pass first, as the live runtime paints the main window
        // before its popups: it lays the tree out for the dialog to find its
        // size in.
        let _ = self.render_backdrop(root, Backdrop::Plain, false);
        let mut popups = Vec::new();
        root.collect_popups(&mut popups);
        let Some(dialog) = popups.into_iter().rfind(|p| p.kind == PopupKind::Dialog) else {
            return false;
        };
        let content = self.render_popup(root, &dialog);
        let chrome = WindowChrome::dialog(dialog.title.unwrap_or_default());
        self.place(desktop, at, &content, &chrome);
        true
    }

    /// Paint `req`'s window on its own, sized to it, as the live runtime paints
    /// each window it opens for a popup or dialog.
    fn render_popup(&self, root: &mut dyn Widget, req: &PopupRequest) -> Snapshot {
        let phys = |v: i32| (v as f32 * self.scale).round() as i32;
        let size = Size::new(phys(req.rect.w).max(1), phys(req.rect.h).max(1));
        let mut pixels = vec![0u32; (size.w * size.h) as usize];
        {
            let mut painter = Painter::with_popup_anchor(
                &mut pixels,
                size.w,
                size.h,
                self.scale,
                -phys(req.rect.x),
                -phys(req.rect.y),
                self.fonts(),
                Some(req.rect),
            )
            .with_screen(self.screen)
            .with_display_dpi(self.display_dpi);
            painter.fill(self.theme.background);
            painter.set_clip_phys(0, 0, size.w, size.h);
            root.paint(&mut painter, &self.theme);
        }
        Snapshot {
            width: size.w,
            height: size.h,
            pixels,
        }
    }

    /// Frame `content` in `chrome` on `desktop`, the frame's top-left corner at
    /// `at` (logical desktop pixels).
    fn place(&self, desktop: &mut Desktop, at: Point, content: &Snapshot, chrome: &WindowChrome) {
        // Lay the frame out with just enough margin for its whole shadow; the
        // metrics put the frame at that margin, and the offset moves it to `at`.
        let chrome = chrome.clone().with_margin(chrome::SHADOW_SIZE);
        let m = chrome::metrics(content.size(), self.scale, &chrome);
        let x = (at.x as f32 * desktop.scale).round() as i32 - m.frame.x;
        let y = (at.y as f32 * desktop.scale).round() as i32 - m.frame.y;
        self.paint_frame(&mut desktop.snapshot, (x, y), &m, &chrome, content);
    }

    /// Paint the chrome `m` lays out, then blit `content` into its content
    /// slot, onto `canvas` shifted by `(x, y)` physical pixels — over whatever
    /// the canvas already holds, which shows through the drop shadow.
    fn paint_frame(
        &self,
        canvas: &mut Snapshot,
        (x, y): (i32, i32),
        m: &chrome::Metrics,
        chrome: &WindowChrome,
        content: &Snapshot,
    ) {
        {
            let mut painter = Painter::new(
                &mut canvas.pixels,
                canvas.width,
                canvas.height,
                // The chrome metrics are already in physical pixels, so the
                // painter draws 1:1 — no second scale pass over the frame.
                1.0,
                x,
                y,
                self.fonts(),
            );
            chrome::paint_window(&mut painter, m, chrome);
        }
        canvas.blit(content, x + m.content.x, y + m.content.y);
    }

    fn fonts(&self) -> FontSet<'_> {
        FontSet {
            sans: self.font.as_ref(),
            serif: self.serif_font.as_ref(),
            mono: self.mono_font.as_ref(),
        }
    }
}

/// A stretch of desktop to arrange several framed windows on — an app next to
/// the dialog it has open, say — for a screenshot of more than one window.
/// [`MockBackend::place_window`] and [`MockBackend::place_dialog`] each draw a
/// window on it, over the windows placed before.
///
/// ```no_run
/// use saudade::*;
/// use saudade::mock::{Desktop, MockBackend};
///
/// let mut editor = Container::new(240, 120).with_background(Color::WHITE);
/// let mut about = Container::new(160, 80);
///
/// let mut desktop = Desktop::new(420, 220, 2.0);
/// MockBackend::new(240, 120).with_scale(2.0).place_window(
///     &mut desktop,
///     Point::new(20, 20),
///     &mut editor,
///     &WindowChrome::resizable("Editor").inactive(),
/// );
/// MockBackend::new(160, 80).with_scale(2.0).place_window(
///     &mut desktop,
///     Point::new(220, 100),
///     &mut about,
///     &WindowChrome::fixed("About"),
/// );
/// std::fs::write("desktop.png", desktop.snapshot().to_png()).unwrap();
/// ```
pub struct Desktop {
    scale: f32,
    snapshot: Snapshot,
}

impl Desktop {
    /// An empty desktop of `width × height` logical pixels, in Canoe's teal, at
    /// `scale` — the scale of the backends that place windows on it.
    pub fn new(width: i32, height: i32, scale: f32) -> Self {
        let scale = scale.max(0.01);
        let phys = |v: i32| ((v as f32 * scale).round() as i32).max(1);
        Self {
            scale,
            snapshot: Snapshot::filled(
                Size::new(phys(width), phys(height)),
                chrome::DESKTOP_BACKGROUND,
            ),
        }
    }

    /// Fill the desktop with `color` instead of the teal. It paints over every
    /// window placed so far, so pick the color first.
    pub fn with_background(mut self, color: Color) -> Self {
        self.snapshot.pixels.fill(color.0);
        self
    }

    /// The desktop, with every window placed on it so far.
    pub fn snapshot(&self) -> &Snapshot {
        &self.snapshot
    }
}

impl Backdrop {
    /// What a window framed in `chrome` fills behind its widgets.
    fn for_chrome(chrome: &WindowChrome) -> Self {
        if chrome.paints_background_pattern() {
            Backdrop::Pattern
        } else {
            Backdrop::Plain
        }
    }
}

fn origin_centered(logical: Size, scale: f32, physical: Size) -> (i32, i32) {
    let content_w = (logical.w as f32 * scale).round() as i32;
    let content_h = (logical.h as f32 * scale).round() as i32;
    let ox = ((physical.w - content_w) / 2).max(0);
    let oy = ((physical.h - content_h) / 2).max(0);
    (ox, oy)
}

/// Flags a widget can set on its `EventCtx` after handling an event,
/// surfaced for tests so they can confirm a button fired, focus moved,
/// etc.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DispatchOutcome {
    pub paint_requested: bool,
    pub close_requested: bool,
    /// What a widget asked for with [`EventCtx::set_fullscreen`], if anything.
    pub fullscreen_request: Option<bool>,
}

/// Result of [`MockBackend::render`]. Holds a physical-pixel ARGB32
/// framebuffer plus its dimensions.
pub struct Snapshot {
    width: i32,
    height: i32,
    pixels: Vec<u32>,
}

impl Snapshot {
    pub fn width(&self) -> i32 {
        self.width
    }

    pub fn height(&self) -> i32 {
        self.height
    }

    /// Raw ARGB32 pixel buffer, row-major, top-down.
    pub fn pixels(&self) -> &[u32] {
        &self.pixels
    }

    /// A `size` buffer filled with `color`.
    fn filled(size: Size, color: Color) -> Self {
        Self {
            width: size.w,
            height: size.h,
            pixels: vec![color.0; (size.w * size.h) as usize],
        }
    }

    fn size(&self) -> Size {
        Size::new(self.width, self.height)
    }

    /// Copy `src` in with its top-left corner at `(x, y)`, cropped to fit.
    fn blit(&mut self, src: &Snapshot, x: i32, y: i32) {
        let x0 = x.max(0);
        let x1 = (x + src.width).min(self.width);
        if x0 >= x1 {
            return;
        }
        let n = (x1 - x0) as usize;
        for row in y.max(0)..(y + src.height).min(self.height) {
            let from = ((row - y) * src.width + (x0 - x)) as usize;
            let to = (row * self.width + x0) as usize;
            self.pixels[to..to + n].copy_from_slice(&src.pixels[from..from + n]);
        }
    }

    /// Encode the framebuffer as a deterministic PNG byte stream — the
    /// canonical artifact for `insta::assert_binary_snapshot!`.
    pub fn to_png(&self) -> Vec<u8> {
        let mut buf = Vec::new();
        {
            let mut encoder = png::Encoder::new(&mut buf, self.width as u32, self.height as u32);
            encoder.set_color(png::ColorType::Rgba);
            encoder.set_depth(png::BitDepth::Eight);
            let mut writer = encoder.write_header().expect("saudade::mock: png header");
            let mut rgba = Vec::with_capacity(self.pixels.len() * 4);
            for &px in &self.pixels {
                let a = ((px >> 24) & 0xFF) as u8;
                let r = ((px >> 16) & 0xFF) as u8;
                let g = ((px >> 8) & 0xFF) as u8;
                let b = (px & 0xFF) as u8;
                rgba.extend_from_slice(&[r, g, b, a]);
            }
            writer
                .write_image_data(&rgba)
                .expect("saudade::mock: png data");
        }
        buf
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Container;

    /// A regular (resizable / fixed) framed window paints the window background
    /// pattern behind its content, exactly like the live main surface; a dialog
    /// stays plain. With a `Solid` pattern and a transparent root the whole
    /// content slot is flooded with the pattern color for a regular window and
    /// left at the (plain) theme background for a dialog, so a pixel census of
    /// the framed buffer distinguishes the two without any baseline image.
    #[test]
    fn framed_regular_window_paints_pattern_but_dialog_stays_plain() {
        // A color the Canoe chrome palette never uses, so every matching pixel
        // came from the content backdrop.
        let distinct = Color::rgb(0xAB, 0xCD, 0xEF);
        // Transparent container: paints nothing, so the backdrop shows through.
        let build = || Container::new(40, 30);
        let backend =
            MockBackend::new(40, 30).with_background_pattern(BackgroundPattern::Solid, distinct);

        let census = |snap: &Snapshot| snap.pixels().iter().filter(|&&px| px == distinct.0).count();
        let content_px = (backend.physical_size().w * backend.physical_size().h) as usize;

        let resizable = backend.render_framed(&mut build(), &WindowChrome::resizable("App"));
        let dialog = backend.render_framed(&mut build(), &WindowChrome::dialog("App"));

        // Regular window: the entire content slot is the pattern color.
        assert_eq!(census(&resizable), content_px);
        // Dialog: the pattern is suppressed, so it appears nowhere.
        assert_eq!(census(&dialog), 0);
    }

    /// The default backend paints the live runtime's default backdrop — a
    /// `superlight` forward-diagonal hatch — behind a regular framed window, and
    /// suppresses it for a dialog.
    #[test]
    fn framed_uses_live_default_pattern() {
        let hatch = PATTERN_COLORS[0].1.0; // superlight, the default fg
        let build = || Container::new(40, 30);
        let backend = MockBackend::new(40, 30);

        let has_hatch = |snap: &Snapshot| snap.pixels().contains(&hatch);

        let resizable = backend.render_framed(&mut build(), &WindowChrome::resizable("App"));
        let dialog = backend.render_framed(&mut build(), &WindowChrome::dialog("App"));

        assert!(
            has_hatch(&resizable),
            "regular window should show the default hatch"
        );
        assert!(!has_hatch(&dialog), "dialog must stay plain");
    }

    /// The bare client-area pass keeps its plain fill — the pattern is reserved
    /// for the framed, regular-window path.
    #[test]
    fn bare_render_stays_plain() {
        let distinct = Color::rgb(0xAB, 0xCD, 0xEF);
        let mut root = Container::new(40, 30);
        let snap = MockBackend::new(40, 30)
            .with_background_pattern(BackgroundPattern::Solid, distinct)
            .render(&mut root);
        assert!(snap.pixels().iter().all(|&px| px != distinct.0));
    }

    fn count(snap: &Snapshot, color: Color) -> usize {
        snap.pixels().iter().filter(|&&px| px == color.0).count()
    }

    /// An inactive window's title bar is white where an active one's is navy.
    #[test]
    fn inactive_window_has_white_title_bar() {
        let backend = MockBackend::new(120, 30);
        let active =
            backend.render_framed(&mut Container::new(120, 30), &WindowChrome::fixed("App"));
        let inactive = backend.render_framed(
            &mut Container::new(120, 30),
            &WindowChrome::fixed("App").inactive(),
        );
        assert!(count(&active, Color::NAVY) > 0);
        assert_eq!(count(&inactive, Color::NAVY), 0);
    }

    /// Windows on a desktop stack back to front: a later window covers an
    /// earlier one where they overlap, and its drop shadow falls over it.
    #[test]
    fn desktop_stacks_windows_back_to_front() {
        let distinct = Color::rgb(0xAB, 0xCD, 0xEF);
        let backend =
            MockBackend::new(40, 30).with_background_pattern(BackgroundPattern::Solid, distinct);
        let chrome = WindowChrome::fixed("App");
        let mut desktop = Desktop::new(120, 100, 1.0);

        backend.place_window(
            &mut desktop,
            Point::new(10, 10),
            &mut Container::new(40, 30),
            &chrome,
        );
        assert_eq!(count(desktop.snapshot(), distinct), 40 * 30);
        // The pixel the second window's shadow will fall on: inside the first
        // window's content, left of the second window's frame.
        let at = |desktop: &Desktop| desktop.snapshot().pixels()[50 * 120 + 25];
        assert_eq!(at(&desktop), distinct.0);

        backend.place_window(
            &mut desktop,
            Point::new(30, 30),
            &mut Container::new(40, 30),
            &chrome,
        );
        // The second window's content is whole, the first lost the overlap.
        assert!(count(desktop.snapshot(), distinct) < 2 * 40 * 30);
        assert!(count(desktop.snapshot(), distinct) >= 40 * 30);
        let shaded = at(&desktop);
        assert!(
            shaded & 0xFF < distinct.0 & 0xFF,
            "no shadow over the window behind"
        );
    }

    /// On a desktop, a dialog is a window of its own: `place_window` leaves it
    /// out of its parent, and `place_dialog` places it, framed and active.
    #[test]
    fn desktop_shows_dialog_as_window_of_its_own() {
        let backend = MockBackend::new(200, 150);
        let place = |dialog: crate::Dialog| {
            let mut root = crate::Column::new().add_overlay(dialog);
            let mut desktop = Desktop::new(300, 250, 1.0);
            let chrome = WindowChrome::fixed("App").inactive();
            backend.place_window(&mut desktop, Point::new(10, 10), &mut root, &chrome);
            let parent = desktop.snapshot().pixels().to_vec();
            let placed = backend.place_dialog(&mut desktop, Point::new(60, 60), &mut root);
            (parent, placed, count(desktop.snapshot(), Color::NAVY))
        };

        let (closed, placed, _) = place(crate::Dialog::new());
        assert!(!placed, "no dialog open, none to place");

        let mut dialog = crate::Dialog::new();
        dialog.show_info("Note", "Hello");
        let (parent, placed, navy) = place(dialog);
        assert_eq!(parent, closed, "the dialog drew inside its parent window");
        assert!(placed);
        assert!(navy > 0, "the dialog should have an active title bar");
    }
}
