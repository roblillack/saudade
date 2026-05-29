//! Offscreen backend for snapshot / image-based tests.
//!
//! The [`MockBackend`] mirrors the rendering pipeline of the live runtime
//! (`App::paint_main` and friends) but draws into an owned ARGB32 pixel
//! buffer instead of a winit/softbuffer surface. After a render the
//! resulting [`Snapshot`] exposes the raw pixels or a PNG encoding suitable
//! for `insta::assert_binary_snapshot!`.
//!
//! ```no_run
//! use retrogui::*;
//! use retrogui::mock::MockBackend;
//!
//! let mut root = Container::new(120, 40)
//!     .with_background(Color::WHITE)
//!     .add(Label::new(10, 12, "Hi"));
//!
//! let snap = MockBackend::new(120, 40).with_scale(2.0).render(&mut root);
//! let png_bytes = snap.to_png();
//! ```
//!
//! Input simulation is not implemented yet — once added, callers will be
//! able to feed [`Event`](crate::event::Event)s into the same backend to
//! drive widgets between renders.

use crate::event::{Event, EventCtx};
use crate::font::Font;
use crate::geometry::{Rect, Size};
use crate::painter::Painter;
use crate::theme::Theme;
use crate::widget::Widget;

/// Offscreen renderer used by snapshot tests.
pub struct MockBackend {
    logical_size: Size,
    scale: f32,
    theme: Theme,
    font: Option<Font>,
    mono_font: Option<Font>,
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
            mono_font: None,
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

    /// Supply the proportional font used for [`Painter::text`]. Tests
    /// should bundle a known font (e.g. via `include_bytes!`) so glyph
    /// output stays byte-identical across machines.
    pub fn with_font(mut self, font: Font) -> Self {
        self.font = Some(font);
        self
    }

    /// Supply the monospace font used for [`Painter::mono_text`].
    pub fn with_mono_font(mut self, font: Font) -> Self {
        self.mono_font = Some(font);
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
            let mut painter = Painter::with_popup_pass(
                &mut pixels,
                physical.w,
                physical.h,
                self.scale,
                origin_x,
                origin_y,
                self.font.as_ref(),
                self.mono_font.as_ref(),
                false,
            );
            painter.fill(self.theme.background);
            root.paint(&mut painter, &self.theme);
        }

        if let Some(req) = root.popup_request() {
            let popup_phys_x = origin_x + (req.rect.x as f32 * self.scale).round() as i32;
            let popup_phys_y = origin_y + (req.rect.y as f32 * self.scale).round() as i32;
            let popup_phys_w = (req.rect.w as f32 * self.scale).round() as i32;
            let popup_phys_h = (req.rect.h as f32 * self.scale).round() as i32;
            let mut painter = Painter::with_popup_pass(
                &mut pixels,
                physical.w,
                physical.h,
                self.scale,
                origin_x,
                origin_y,
                self.font.as_ref(),
                self.mono_font.as_ref(),
                true,
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
            let mut encoder =
                png::Encoder::new(&mut buf, self.width as u32, self.height as u32);
            encoder.set_color(png::ColorType::Rgba);
            encoder.set_depth(png::BitDepth::Eight);
            let mut writer = encoder.write_header().expect("retrogui::mock: png header");
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
                .expect("retrogui::mock: png data");
        }
        buf
    }
}
