use crate::background::BackgroundPattern;
use crate::font::Font;
use crate::geometry::{Color, Rect, Size};
use crate::theme::Theme;

/// Pixel-perfect 2D painter over an ARGB32 framebuffer.
///
/// Widgets paint in **logical pixels**: density-independent design units. The
/// painter applies the OS-reported scale factor (which may be fractional —
/// 1.0, 1.25, 1.5, 2.0, …) and writes straight into the physical surface
/// buffer. Rectangle edges are snapped independently so adjacent rectangles
/// always share an exact physical-pixel boundary, which keeps Win 3.1 chrome
/// crisp at every DPI. Text is rasterized once at its final physical size via
/// fontdue — no resampling, no smudge.
/// Opaque clip-stack token returned by [`Painter::push_clip`]. Pass it back
/// to [`Painter::restore_clip`] to pop the clip the caller installed.
#[derive(Clone, Copy)]
pub struct SavedClip(Option<(i32, i32, i32, i32)>);

/// Opaque scale-stack token returned by [`Painter::push_physical_pixels`].
/// Pass it back to [`Painter::restore_scale`] to leave physical-pixel mode.
#[derive(Clone, Copy)]
pub struct SavedScale(f32);

pub struct Painter<'a> {
    pixels: &'a mut [u32],
    /// Physical buffer width in pixels.
    width: i32,
    /// Physical buffer height in pixels.
    height: i32,
    /// Logical→physical scale. Equals winit's `scale_factor` for the current
    /// monitor (always ≥ 1 in practice).
    scale: f32,
    /// Physical-pixel offset of the logical origin within the buffer. The
    /// runtime sets this to center the content when the window has been
    /// resized larger than the design — surroundings become clean letterbox.
    origin_x: i32,
    origin_y: i32,
    /// Default proportional font, used by `text`/`text_centered`/etc. Most
    /// widgets pick this up via the `Theme`.
    font: Option<&'a Font>,
    /// Fixed-width font, used by text-editor widgets that need predictable
    /// per-character advances. May be the same physical face as `font` on
    /// systems where no dedicated monospace face was discovered.
    mono_font: Option<&'a Font>,
    /// `Some(anchor)` when this painter is drawing into a popup top-level
    /// window, where `anchor` is the popup's [`PopupRequest::rect`] (the
    /// same value the runtime opened the popup window with). `None` in the
    /// main pass. Widgets that maintain floating overlays (menu popups,
    /// tooltips) inspect this in `paint_overlay` so they only draw on the
    /// surface that actually hosts them — and, when several popups are
    /// stacked (e.g. a dropdown opened inside a dialog), only into the one
    /// whose anchor matches their own [`Widget::popup_request`].
    popup_anchor: Option<Rect>,
    /// Physical-pixel clip rectangle. When set, all draws are restricted to
    /// pixels inside this rect. The runtime uses this to keep the popup
    /// pass from leaking widget content past the popup's footprint.
    clip: Option<(i32, i32, i32, i32)>,
}

impl<'a> Painter<'a> {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        pixels: &'a mut [u32],
        width: i32,
        height: i32,
        scale: f32,
        origin_x: i32,
        origin_y: i32,
        font: Option<&'a Font>,
        mono_font: Option<&'a Font>,
    ) -> Self {
        Self::with_popup_anchor(
            pixels, width, height, scale, origin_x, origin_y, font, mono_font, None,
        )
    }

    /// Like [`Painter::new`] but tags the painter as running inside a
    /// popup top-level window whose [`PopupRequest::rect`] is `anchor`.
    /// `None` means the main window pass (equivalent to [`Painter::new`]).
    /// Widgets compare `anchor` against their own popup request in
    /// `paint_overlay` so that, when several popups are stacked, the
    /// dropdown / menu / dialog body is drawn only into the surface that
    /// actually hosts it.
    #[allow(clippy::too_many_arguments)]
    pub fn with_popup_anchor(
        pixels: &'a mut [u32],
        width: i32,
        height: i32,
        scale: f32,
        origin_x: i32,
        origin_y: i32,
        font: Option<&'a Font>,
        mono_font: Option<&'a Font>,
        popup_anchor: Option<Rect>,
    ) -> Self {
        Self {
            pixels,
            width,
            height,
            scale: scale.max(0.01),
            origin_x,
            origin_y,
            font,
            mono_font,
            popup_anchor,
            clip: None,
        }
    }

    pub fn is_popup_pass(&self) -> bool {
        self.popup_anchor.is_some()
    }

    /// [`PopupRequest::rect`] of the popup this painter is drawing into,
    /// or `None` in the main pass. Widgets that report a
    /// [`Widget::popup_request`](crate::Widget::popup_request) use this in
    /// `paint_overlay` to decide whether the current popup pass is *theirs*
    /// — only then should they draw their popup body.
    pub fn popup_anchor(&self) -> Option<Rect> {
        self.popup_anchor
    }

    /// Restrict all subsequent drawing to a physical-pixel rectangle. Used
    /// by the popup runtime to confine paint operations to the popup's
    /// footprint inside its (often oversized) host window.
    pub fn set_clip_phys(&mut self, x: i32, y: i32, w: i32, h: i32) {
        self.clip = Some((x, y, x + w, y + h));
    }

    pub fn clear_clip(&mut self) {
        self.clip = None;
    }

    /// Restrict subsequent draws to the intersection of `rect` (in logical
    /// pixels) and any clip already in effect. Returns the previous clip
    /// state — pass it to [`Painter::restore_clip`] when done. Widget code
    /// uses this to keep overflow text from leaking past its field edges
    /// without having to know its own physical-pixel placement.
    pub fn push_clip(&mut self, rect: Rect) -> SavedClip {
        let prev = SavedClip(self.clip);
        let x0 = self.origin_x + self.snap(rect.x);
        let y0 = self.origin_y + self.snap(rect.y);
        let x1 = self.origin_x + self.snap(rect.x + rect.w);
        let y1 = self.origin_y + self.snap(rect.y + rect.h);
        let combined = match self.clip {
            Some((px0, py0, px1, py1)) => (x0.max(px0), y0.max(py0), x1.min(px1), y1.min(py1)),
            None => (x0, y0, x1, y1),
        };
        self.clip = Some(combined);
        prev
    }

    pub fn restore_clip(&mut self, saved: SavedClip) {
        self.clip = saved.0;
    }

    fn clip_bounds(&self) -> (i32, i32, i32, i32) {
        match self.clip {
            Some((x0, y0, x1, y1)) => (
                x0.max(0),
                y0.max(0),
                x1.min(self.width),
                y1.min(self.height),
            ),
            None => (0, 0, self.width, self.height),
        }
    }

    pub fn size(&self) -> Size {
        Size::new(self.width, self.height)
    }

    pub fn scale(&self) -> f32 {
        self.scale
    }

    /// Translate a logical-pixel `rect` to the physical-pixel rectangle it
    /// would occupy on the buffer, snapping each edge independently the
    /// same way the draw primitives do. Pair with
    /// [`Self::push_physical_pixels`] to draw chrome at exact physical
    /// pixel positions — useful when a logical-pixel-thin line would
    /// otherwise round inconsistently at fractional scales.
    pub fn rect_to_physical(&self, rect: Rect) -> Rect {
        let x0 = self.snap(rect.x);
        let y0 = self.snap(rect.y);
        let x1 = self.snap(rect.x + rect.w);
        let y1 = self.snap(rect.y + rect.h);
        Rect::new(x0, y0, x1 - x0, y1 - y0)
    }

    /// `true` if the painter's current scale lies in the range where
    /// 1-logical-pixel chrome edges look cleaner drawn at exact physical
    /// pixels (currently `[0.9, 1.5)`). At those scales, a 1-logical-pixel
    /// line rounds to either 1 or 2 physical pixels depending on where
    /// it falls, which makes adjacent bevel / outline edges look
    /// inconsistent. Outside the range — at, say, 1.0x or 2.0x — the
    /// regular scale-aware draw is exact, and at very small or very
    /// large scales the "1 logical = 1 physical" trick would make the
    /// chrome too thin (or too thick) relative to the rest of the
    /// widget.
    ///
    /// Widgets that want to opt into the crisp behaviour pair this with
    /// [`Self::rect_to_physical`] + [`Self::push_physical_pixels`] for
    /// the chrome pass, then restore the scale before drawing labels.
    pub fn wants_crisp_chrome(&self) -> bool {
        (0.9..1.5).contains(&self.scale)
    }

    /// Switch the painter into physical-pixel mode: subsequent `fill_rect`
    /// / `h_line` / `v_line` / `stroke_rect` / `raised_bevel` / `button` /
    /// `pixel` calls treat their `i32` coordinates as physical pixels
    /// (relative to the painter's logical origin) rather than logical
    /// pixels — the internal snap pass becomes the identity. The logical
    /// origin offset is preserved, so a widget that converts its bounds
    /// via [`Self::rect_to_physical`] keeps drawing in the same place on
    /// the buffer; the only thing that changes is that 1-unit chrome
    /// lines become exactly 1 physical pixel thick rather than
    /// `round(scale)` physical pixels.
    ///
    /// Restore the previous scale with [`Self::restore_scale`].
    pub fn push_physical_pixels(&mut self) -> SavedScale {
        let saved = SavedScale(self.scale);
        self.scale = 1.0;
        saved
    }

    /// Restore the scale that was active before
    /// [`Self::push_physical_pixels`].
    pub fn restore_scale(&mut self, saved: SavedScale) {
        self.scale = saved.0.max(0.01);
    }

    pub fn font(&self) -> Option<&Font> {
        self.font
    }

    pub fn mono_font(&self) -> Option<&Font> {
        self.mono_font
    }

    /// Snap a logical-pixel coordinate (edge or position) to a physical pixel.
    /// Edges of adjacent rectangles are snapped *independently*, so they
    /// always meet on the same physical pixel without gaps or overlap.
    fn snap(&self, logical: i32) -> i32 {
        (logical as f32 * self.scale).round() as i32
    }

    /// Fill the whole physical buffer with a solid color.
    pub fn fill(&mut self, color: Color) {
        self.pixels.fill(color.0);
    }

    /// Paint a regular top-level window's background: flood the whole buffer
    /// with `base`, then stamp `pattern` on top in `fg`. The pattern grid is
    /// anchored to the logical origin (so it doesn't crawl when the window is
    /// resized or letterboxed) and its spacing scales with the DPI so the
    /// texture keeps its proportions. [`BackgroundPattern::None`] is a plain
    /// `base` fill; [`BackgroundPattern::Solid`] is a plain `fg` fill.
    pub fn fill_pattern(&mut self, base: Color, pattern: BackgroundPattern, fg: Color) {
        self.fill(base);
        match pattern {
            BackgroundPattern::None => return,
            BackgroundPattern::Solid => {
                self.fill(fg);
                return;
            }
            _ => {}
        }
        // Grid spacing + feature thickness in physical pixels. `near` is the
        // tight 4px grid (lines, diagonal hatching); `wide` is the looser 6px
        // grid used by the staggered dots and the cross-stitch weave.
        let near = (4.0 * self.scale).round().max(1.0) as i32;
        let wide = (6.0 * self.scale).round().max(1.0) as i32;
        let thick = self.scale.round().max(1.0) as i32;
        // A staggered dot field: dots sit on a `step` grid, but every other
        // row is shifted half a step so each dot falls in the gap of the row
        // above — the quincunx of the classic Mac desktop, not a square grid.
        let dotted = |ax: i32, ay: i32, step: i32| -> bool {
            if ay.rem_euclid(step) >= thick {
                return false;
            }
            let shift = if ay.div_euclid(step).rem_euclid(2) == 1 {
                step / 2
            } else {
                0
            };
            (ax - shift).rem_euclid(step) < thick
        };
        let fg = fg.0;
        for y in 0..self.height {
            // Offset by the logical origin so the grid stays put regardless of
            // letterboxing; `rem_euclid` keeps it stable for negative offsets.
            let ay = y - self.origin_y;
            let row = (y * self.width) as usize;
            for x in 0..self.width {
                let ax = x - self.origin_x;
                let on = match pattern {
                    BackgroundPattern::Dots => dotted(ax, ay, wide),
                    BackgroundPattern::Lines => ay.rem_euclid(near) < thick,
                    BackgroundPattern::DiagonalForward => (ax + ay).rem_euclid(near) < thick,
                    BackgroundPattern::CrossStitch => {
                        (ax + ay).rem_euclid(wide) < thick || (ax - ay).rem_euclid(wide) < thick
                    }
                    // Handled above with an early return.
                    BackgroundPattern::None | BackgroundPattern::Solid => false,
                };
                if on {
                    self.pixels[row + x as usize] = fg;
                }
            }
        }
    }

    /// Solid-fill a physical-pixel rectangle. Used internally after logical
    /// coordinates have been snapped + offset.
    fn fill_phys(&mut self, x: i32, y: i32, w: i32, h: i32, color: Color) {
        if w <= 0 || h <= 0 {
            return;
        }
        let (cx0, cy0, cx1, cy1) = self.clip_bounds();
        let x0 = x.max(cx0);
        let y0 = y.max(cy0);
        let x1 = (x + w).min(cx1);
        let y1 = (y + h).min(cy1);
        for yy in y0..y1 {
            let row = (yy * self.width) as usize;
            for xx in x0..x1 {
                self.pixels[row + xx as usize] = color.0;
            }
        }
    }

    /// Alpha-blend a single physical-pixel pixel. Coordinates are relative to
    /// the logical origin — the origin offset and clipping happen here. Used
    /// by glyph rasterization in [`Font::draw_phys`].
    pub(crate) fn blend_pixel_phys(&mut self, x: i32, y: i32, color: Color, alpha: u8) {
        let x = x + self.origin_x;
        let y = y + self.origin_y;
        let (cx0, cy0, cx1, cy1) = self.clip_bounds();
        if x < cx0 || y < cy0 || x >= cx1 || y >= cy1 {
            return;
        }
        if alpha == 0 {
            return;
        }
        if alpha == 255 {
            self.pixels[(y * self.width + x) as usize] = color.0;
            return;
        }
        let idx = (y * self.width + x) as usize;
        let dst = self.pixels[idx];
        let a = alpha as u32;
        let inv = 255 - a;
        let sr = color.red() as u32;
        let sg = color.green() as u32;
        let sb = color.blue() as u32;
        let dr = (dst >> 16) & 0xFF;
        let dg = (dst >> 8) & 0xFF;
        let db = dst & 0xFF;
        let r = (sr * a + dr * inv) / 255;
        let g = (sg * a + dg * inv) / 255;
        let b = (sb * a + db * inv) / 255;
        self.pixels[idx] = 0xFF000000 | (r << 16) | (g << 8) | b;
    }

    /// Logical-coordinate single-pixel write — a 1×1 logical pixel becomes the
    /// physical area between (x, y) and (x+1, y+1) after edge snapping.
    pub fn pixel(&mut self, x: i32, y: i32, color: Color) {
        self.fill_rect(Rect::new(x, y, 1, 1), color);
    }

    pub fn fill_rect(&mut self, rect: Rect, color: Color) {
        self.fill_rect_with_phys_offset(rect, 0, 0, color);
    }

    /// Fill a rectangle with an additional physical-pixel offset applied
    /// *after* the logical→physical snap. Pair with `text_with_phys_offset`
    /// when you want chrome (e.g., a mnemonic underline) to track text that
    /// has been nudged a fraction of a logical pixel.
    pub fn fill_rect_with_phys_offset(
        &mut self,
        rect: Rect,
        dx_phys: i32,
        dy_phys: i32,
        color: Color,
    ) {
        let x0 = self.origin_x + self.snap(rect.x) + dx_phys;
        let y0 = self.origin_y + self.snap(rect.y) + dy_phys;
        let x1 = self.origin_x + self.snap(rect.x + rect.w) + dx_phys;
        let y1 = self.origin_y + self.snap(rect.y + rect.h) + dy_phys;
        self.fill_phys(x0, y0, x1 - x0, y1 - y0, color);
    }

    pub fn h_line(&mut self, x: i32, y: i32, w: i32, color: Color) {
        self.fill_rect(Rect::new(x, y, w, 1), color);
    }

    pub fn v_line(&mut self, x: i32, y: i32, h: i32, color: Color) {
        self.fill_rect(Rect::new(x, y, 1, h), color);
    }

    pub fn stroke_rect(&mut self, rect: Rect, color: Color) {
        if rect.w <= 0 || rect.h <= 0 {
            return;
        }
        self.h_line(rect.x, rect.y, rect.w, color);
        self.h_line(rect.x, rect.bottom() - 1, rect.w, color);
        self.v_line(rect.x, rect.y, rect.h, color);
        self.v_line(rect.right() - 1, rect.y, rect.h, color);
    }

    /// Raised 3D bevel: light highlight on top/left, dark shadow on bottom/right.
    pub fn raised_bevel(&mut self, rect: Rect, highlight: Color, shadow: Color) {
        if rect.w <= 0 || rect.h <= 0 {
            return;
        }
        self.h_line(rect.x, rect.y, rect.w, highlight);
        self.v_line(rect.x, rect.y, rect.h, highlight);
        self.h_line(rect.x, rect.bottom() - 1, rect.w, shadow);
        self.v_line(rect.right() - 1, rect.y, rect.h, shadow);
    }

    pub fn sunken_bevel(&mut self, rect: Rect, highlight: Color, shadow: Color) {
        self.raised_bevel(rect, shadow, highlight);
    }

    /// Two-tone horizontal etched line (dark + light) — the divider above the
    /// system stats block in the Win 3.1 about box.
    pub fn etched_h_line(&mut self, x: i32, y: i32, w: i32, theme: &Theme) {
        self.h_line(x, y, w, theme.shadow);
        self.h_line(x, y + 1, w, theme.highlight);
    }

    /// Full Win 3.1 button chrome: every button has a 1px black outer border
    /// with rounded (unpainted) corners; the default button gets an
    /// additional sharp-cornered outer border. Light-gray face, raised
    /// bevel, sunken when pressed.
    pub fn button(&mut self, rect: Rect, theme: &Theme, pressed: bool, default: bool) {
        if rect.w <= 0 || rect.h <= 0 {
            return;
        }
        // Rounded black outline: skip the four corner pixels.
        if rect.w > 2 {
            self.h_line(rect.x + 1, rect.y, rect.w - 2, theme.border);
            self.h_line(rect.x + 1, rect.bottom() - 1, rect.w - 2, theme.border);
        }
        if rect.h > 2 {
            self.v_line(rect.x, rect.y + 1, rect.h - 2, theme.border);
            self.v_line(rect.right() - 1, rect.y + 1, rect.h - 2, theme.border);
        }
        let mut inner = rect.inset(1);
        if default {
            self.stroke_rect(inner, theme.border);
            inner = inner.inset(1);
        }
        self.fill_rect(inner, theme.face);
        if pressed {
            self.sunken_bevel(inner, theme.highlight, theme.shadow);
            let inner2 = inner.inset(1);
            self.h_line(inner2.x, inner2.y, inner2.w, theme.shadow);
            self.v_line(inner2.x, inner2.y, inner2.h, theme.shadow);
        } else {
            self.raised_bevel(inner, theme.highlight, theme.shadow);
            let inner2 = inner.inset(1);
            self.h_line(inner2.x, inner2.y, inner2.w, theme.highlight);
            self.v_line(inner2.x, inner2.y, inner2.h, theme.highlight);
            self.h_line(inner2.x, inner2.bottom() - 1, inner2.w, theme.shadow);
            self.v_line(inner2.right() - 1, inner2.y, inner2.h, theme.shadow);
        }
    }

    /// Draw a line of text. `x` / `y` and `size` are in logical units; the
    /// painter rasterizes glyphs once at `size × scale` physical pixels for
    /// crisp output regardless of fractional DPI.
    pub fn text(&mut self, x: i32, y: i32, text: &str, size: f32, color: Color) {
        self.text_with_phys_offset(x, y, 0, 0, text, size, color);
    }

    /// Draw text with an additional physical-pixel offset applied *after*
    /// the logical→physical snap. Useful for fine alignment tweaks (e.g.
    /// nudging menu-bar labels down a single physical pixel) that don't
    /// correspond cleanly to any whole logical-pixel value.
    #[allow(clippy::too_many_arguments)]
    pub fn text_with_phys_offset(
        &mut self,
        x: i32,
        y: i32,
        dx_phys: i32,
        dy_phys: i32,
        text: &str,
        size: f32,
        color: Color,
    ) {
        let Some(font) = self.font else {
            return;
        };
        let x_phys = (self.snap(x) + dx_phys) as f32;
        let y_phys = (self.snap(y) + dy_phys) as f32;
        let size_phys = size * self.scale;
        font.draw_phys(self, text, x_phys, y_phys, size_phys, color);
    }

    pub fn text_centered(&mut self, rect: Rect, text: &str, size: f32, color: Color) {
        let Some(font) = self.font else {
            return;
        };
        let (w, h) = font.measure(text, size);
        let tx = rect.x + ((rect.w as f32 - w) / 2.0).round() as i32;
        let ty = rect.y + ((rect.h as f32 - h) / 2.0).round() as i32;
        self.text(tx, ty, text, size, color);
    }

    pub fn measure_text(&self, text: &str, size: f32) -> Size {
        let Some(font) = self.font else {
            return Size::new(0, 0);
        };
        let (w, h) = font.measure(text, size);
        Size::new(w.ceil() as i32, h.ceil() as i32)
    }

    /// Draw text using the monospace font. Returns immediately if no
    /// monospace font is available — the caller should be prepared for
    /// `measure_mono_text` to return zero in that case.
    pub fn mono_text(&mut self, x: i32, y: i32, text: &str, size: f32, color: Color) {
        let Some(font) = self.mono_font else { return };
        let x_phys = self.snap(x) as f32;
        let y_phys = self.snap(y) as f32;
        let size_phys = size * self.scale;
        font.draw_phys(self, text, x_phys, y_phys, size_phys, color);
    }

    pub fn measure_mono_text(&self, text: &str, size: f32) -> Size {
        let Some(font) = self.mono_font else {
            return Size::new(0, 0);
        };
        let (w, h) = font.measure(text, size);
        Size::new(w.ceil() as i32, h.ceil() as i32)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::background::BackgroundPattern;

    /// Paint `pattern` into a fresh `w × h` buffer at scale 1 and hand the
    /// pixels back for inspection.
    fn render(w: i32, h: i32, pattern: BackgroundPattern, fg: Color) -> Vec<u32> {
        let mut pixels = vec![0u32; (w * h) as usize];
        {
            let mut p = Painter::new(&mut pixels, w, h, 1.0, 0, 0, None, None);
            p.fill_pattern(Color::WHITE, pattern, fg);
        }
        pixels
    }

    #[test]
    fn none_pattern_is_a_plain_base_fill() {
        let px = render(8, 8, BackgroundPattern::None, Color::BLACK);
        assert!(px.iter().all(|&c| c == Color::WHITE.0));
    }

    #[test]
    fn solid_pattern_floods_with_foreground() {
        let px = render(8, 8, BackgroundPattern::Solid, Color::BLACK);
        assert!(px.iter().all(|&c| c == Color::BLACK.0));
    }

    #[test]
    fn dots_are_staggered_between_rows() {
        let px = render(12, 12, BackgroundPattern::Dots, Color::BLACK);
        let at = |x: i32, y: i32| px[(y * 12 + x) as usize];
        // Even dot-row (y == 0): dots on the 6px grid.
        assert_eq!(at(0, 0), Color::BLACK.0);
        assert_eq!(at(6, 0), Color::BLACK.0);
        assert_eq!(at(3, 0), Color::WHITE.0);
        // Odd dot-row (y == 6): shifted half a step, so dots land in the gaps
        // of the row above rather than directly beneath it.
        assert_eq!(at(3, 6), Color::BLACK.0);
        assert_eq!(at(9, 6), Color::BLACK.0);
        assert_eq!(at(0, 6), Color::WHITE.0);
        assert_eq!(at(6, 6), Color::WHITE.0);
        // Rows between dot-rows stay blank.
        assert_eq!(at(0, 1), Color::WHITE.0);
        assert_eq!(at(3, 3), Color::WHITE.0);
    }

    #[test]
    fn lines_fill_whole_rows() {
        let px = render(8, 8, BackgroundPattern::Lines, Color::BLACK);
        let at = |x: i32, y: i32| px[(y * 8 + x) as usize];
        for x in 0..8 {
            assert_eq!(at(x, 0), Color::BLACK.0, "row 0 should be a line");
            assert_eq!(at(x, 4), Color::BLACK.0, "row 4 should be a line");
            assert_eq!(at(x, 1), Color::WHITE.0, "row 1 should be blank");
        }
    }

    #[test]
    fn cross_stitch_weave_is_wider_than_the_diagonals() {
        let px = render(12, 12, BackgroundPattern::CrossStitch, Color::BLACK);
        let at = |x: i32, y: i32| px[(y * 12 + x) as usize];
        // Lit where (x+y) or (x-y) is a multiple of the 6px cross spacing.
        assert_eq!(at(0, 0), Color::BLACK.0);
        assert_eq!(at(6, 0), Color::BLACK.0);
        assert_eq!(at(3, 3), Color::BLACK.0); // forward diagonal: x+y == 6
        assert_eq!(at(1, 1), Color::BLACK.0); // back diagonal: x-y == 0
        // The "slightly wider" check: a 4px step is now blank — the plain
        // diagonals (still on the 4px grid) would have drawn here.
        assert_eq!(at(4, 0), Color::WHITE.0);
        assert_eq!(at(2, 0), Color::WHITE.0);
    }

    #[test]
    fn diagonal_uses_the_tight_grid() {
        // DiagonalForward is unchanged: lit where x+y is a multiple of 4px.
        let fwd = render(8, 8, BackgroundPattern::DiagonalForward, Color::BLACK);
        let at = |x: i32, y: i32| fwd[(y * 8 + x) as usize];
        assert_eq!(at(4, 0), Color::BLACK.0); // x+y == 4
        assert_eq!(at(2, 2), Color::BLACK.0); // x+y == 4
        assert_eq!(at(1, 0), Color::WHITE.0); // x+y == 1
    }
}
