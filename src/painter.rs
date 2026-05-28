use crate::font::Font;
use crate::geometry::{Color, Rect, Size};
use crate::theme::Theme;

/// Pixel-perfect 2D painter over an ARGB32 framebuffer.
///
/// The painter exposes a *logical* coordinate space to widgets — the same
/// design units you'd type in by hand. Internally it multiplies every
/// coordinate by an integer `scale` and writes directly into the physical
/// surface buffer, so output is always crisp: no nearest-neighbor upscale,
/// no anti-aliased smudge at non-integer DPI scales.
pub struct Painter<'a> {
    pixels: &'a mut [u32],
    /// Physical buffer width in pixels.
    width: i32,
    /// Physical buffer height in pixels.
    height: i32,
    /// 1 logical pixel = `scale` × `scale` physical pixels. Always ≥ 1.
    scale: i32,
    /// Physical-pixel offset of the logical origin within the buffer. The
    /// runtime sets this to center the content when the window is larger than
    /// `logical_size × scale`, so the surroundings become clean letterbox area.
    origin_x: i32,
    origin_y: i32,
    font: Option<&'a Font>,
}

impl<'a> Painter<'a> {
    pub fn new(
        pixels: &'a mut [u32],
        width: i32,
        height: i32,
        scale: i32,
        origin_x: i32,
        origin_y: i32,
        font: Option<&'a Font>,
    ) -> Self {
        Self {
            pixels,
            width,
            height,
            scale: scale.max(1),
            origin_x,
            origin_y,
            font,
        }
    }

    pub fn size(&self) -> Size {
        Size::new(self.width, self.height)
    }

    pub fn scale(&self) -> i32 {
        self.scale
    }

    pub fn font(&self) -> Option<&Font> {
        self.font
    }

    /// Fill the whole physical buffer with a solid color. Used by the runtime
    /// to clear the surface (including any letterbox area outside the content).
    pub fn fill(&mut self, color: Color) {
        self.pixels.fill(color.0);
    }

    /// Alpha-blend a physical-pixel pixel. Used by glyph rasterization.
    /// Coordinates are relative to the logical origin (already pre-multiplied
    /// by `scale` by [`Painter::text`]); this function applies the origin
    /// offset and clips against the physical buffer.
    pub(crate) fn blend_pixel_phys(&mut self, x: i32, y: i32, color: Color, alpha: u8) {
        let x = x + self.origin_x;
        let y = y + self.origin_y;
        if x < 0 || y < 0 || x >= self.width || y >= self.height {
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

    /// Solid-fill a physical-pixel rectangle. Coordinates are relative to the
    /// logical origin; this helper applies the origin offset and clips.
    fn fill_phys(&mut self, x: i32, y: i32, w: i32, h: i32, color: Color) {
        if w <= 0 || h <= 0 {
            return;
        }
        let x = x + self.origin_x;
        let y = y + self.origin_y;
        let x0 = x.max(0);
        let y0 = y.max(0);
        let x1 = (x + w).min(self.width);
        let y1 = (y + h).min(self.height);
        for yy in y0..y1 {
            let row = (yy * self.width) as usize;
            for xx in x0..x1 {
                self.pixels[row + xx as usize] = color.0;
            }
        }
    }

    /// Logical-coordinate single-pixel write. One logical pixel becomes a
    /// `scale`-pixel square in the physical buffer.
    pub fn pixel(&mut self, x: i32, y: i32, color: Color) {
        self.fill_phys(x * self.scale, y * self.scale, self.scale, self.scale, color);
    }

    pub fn fill_rect(&mut self, rect: Rect, color: Color) {
        self.fill_phys(
            rect.x * self.scale,
            rect.y * self.scale,
            rect.w * self.scale,
            rect.h * self.scale,
            color,
        );
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

    /// Full Win 3.1 button chrome: optional 1px black outer border (for the
    /// default button), light-gray face, raised bevel, sunken when pressed.
    pub fn button(&mut self, rect: Rect, theme: &Theme, pressed: bool, default: bool) {
        if rect.w <= 0 || rect.h <= 0 {
            return;
        }
        let mut inner = rect;
        if default {
            self.stroke_rect(rect, theme.border);
            inner = rect.inset(1);
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

    /// Draw a line of text. `x` / `y` are in logical units; the painter
    /// rasterizes glyphs at `size × scale` physical pixels.
    pub fn text(&mut self, x: i32, y: i32, text: &str, size: f32, color: Color) -> i32 {
        let Some(font) = self.font else {
            return x;
        };
        let scale = self.scale as f32;
        let advance =
            font.draw_phys(self, text, (x * self.scale) as f32, (y * self.scale) as f32, size * scale, color);
        (advance / self.scale as f32).round() as i32
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
}
