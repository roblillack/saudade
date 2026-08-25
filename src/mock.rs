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
use crate::geometry::{Color, Rect, Size};
use crate::painter::Painter;
use crate::theme::Theme;
use crate::widget::Widget;

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
        }
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
        let mut ctx = EventCtx::new();
        root.event(event, &mut ctx);
        DispatchOutcome {
            paint_requested: ctx.paint_requested,
            close_requested: ctx.close_requested,
        }
    }

    /// Lay out the widget at the backend's logical size, paint into a
    /// fresh buffer, and return a [`Snapshot`]. If the widget reports a
    /// [`PopupRequest`](crate::widget::PopupRequest), the popup pass is
    /// composited on top of the main pass so the snapshot looks the same
    /// as what the user sees on-screen.
    pub fn render(&self, root: &mut dyn Widget) -> Snapshot {
        // The bare client-area pass stays plain, like the live popup/dialog
        // pass; the window background pattern is reserved for the framed,
        // regular-window path (see [`Self::render_framed`]).
        self.render_backdrop(root, Backdrop::Plain)
    }

    /// Shared body of [`Self::render`] and the content pass of
    /// [`Self::render_framed`]. `backdrop` selects what is filled behind the
    /// widget tree before it paints — a plain theme fill or the window
    /// background pattern — exactly mirroring the live runtime's two passes.
    fn render_backdrop(&self, root: &mut dyn Widget, backdrop: Backdrop) -> Snapshot {
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
                FontSet {
                    sans: self.font.as_ref(),
                    serif: self.serif_font.as_ref(),
                    mono: self.mono_font.as_ref(),
                },
                None,
            );
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
                FontSet {
                    sans: self.font.as_ref(),
                    serif: self.serif_font.as_ref(),
                    mono: self.mono_font.as_ref(),
                },
                Some(req.rect),
            );
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
    /// The window is always drawn *active* (focused). [`WindowChrome`] picks
    /// the title and the frame style — [`WindowFrame::Resizable`],
    /// [`WindowFrame::Fixed`], or [`WindowFrame::Dialog`] — which differ in
    /// their window controls and border, matching Canoe's three window paints.
    ///
    /// [`WindowFrame::Resizable`]: crate::WindowFrame::Resizable
    /// [`WindowFrame::Fixed`]: crate::WindowFrame::Fixed
    /// [`WindowFrame::Dialog`]: crate::WindowFrame::Dialog
    pub fn render_framed(&self, root: &mut dyn Widget, chrome: &WindowChrome) -> Snapshot {
        let backdrop = if chrome.paints_background_pattern() {
            Backdrop::Pattern
        } else {
            Backdrop::Plain
        };
        let content = self.render_backdrop(root, backdrop);
        let content_size = Size::new(content.width, content.height);
        let m = chrome::metrics(content_size, self.scale, chrome);

        let mut pixels = vec![0u32; (m.buffer.w * m.buffer.h) as usize];
        {
            let mut painter = Painter::new(
                &mut pixels,
                m.buffer.w,
                m.buffer.h,
                // The chrome metrics are already in physical pixels, so the
                // painter draws 1:1 — no second scale pass over the frame.
                1.0,
                0,
                0,
                FontSet {
                    sans: self.font.as_ref(),
                    serif: self.serif_font.as_ref(),
                    mono: self.mono_font.as_ref(),
                },
            );
            chrome::paint(&mut painter, &m, chrome);
        }

        // Blit the rendered client area into the frame's content slot. Both
        // buffers are physical ARGB32 and the slot fits by construction, so a
        // straight per-row copy suffices.
        let cw = content.width as usize;
        for row in 0..content.height {
            let src = (row * content.width) as usize;
            let dst = ((m.content.y + row) * m.buffer.w + m.content.x) as usize;
            pixels[dst..dst + cw].copy_from_slice(&content.pixels[src..src + cw]);
        }

        Snapshot {
            width: m.buffer.w,
            height: m.buffer.h,
            pixels,
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
}
