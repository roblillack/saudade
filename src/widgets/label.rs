use crate::geometry::{Color, Rect};
use crate::painter::Painter;
use crate::theme::Theme;
use crate::widget::Widget;

/// A box of text.
///
/// A `Label` occupies a fixed rectangle. Text is laid out from the box's
/// top-left corner and word-wrapped to the box width: explicit `\n`
/// characters start a new line, and any line too wide for the box breaks
/// between words — a single word wider than the box is broken too, rather
/// than cut off (see [`Painter::wrap_text`]). Anything that still extends
/// past the box — horizontally or vertically — is clipped to its bounds, so a
/// label never paints outside the rectangle it was given. Color and size are
/// inherited from the active [`Theme`] unless overridden.
///
/// A label is transparent by default — handy now that windows can carry a
/// background pattern — but [`with_background`](Label::with_background) fills
/// its rectangle first, giving the text a clean opaque slab to sit on.
pub struct Label {
    pub rect: Rect,
    pub text: String,
    pub size: Option<f32>,
    pub color: Option<Color>,
    /// Optional fill painted across `rect` before the text. `None` leaves the
    /// label transparent, so whatever is behind it (a sibling, the window
    /// background pattern) shows through.
    pub background: Option<Color>,
}

impl Label {
    pub fn new(rect: Rect, text: impl Into<String>) -> Self {
        Self {
            rect,
            text: text.into(),
            size: None,
            color: None,
            background: None,
        }
    }

    pub fn with_color(mut self, color: Color) -> Self {
        self.color = Some(color);
        self
    }

    pub fn with_size(mut self, size: f32) -> Self {
        self.size = Some(size);
        self
    }

    /// Paint a solid fill across the label's rectangle before the text. Use
    /// this to keep a label legible over a busy background (e.g. a window
    /// background pattern) without wrapping it in a `Container`.
    pub fn with_background(mut self, color: Color) -> Self {
        self.background = Some(color);
        self
    }

    /// How tall the text comes out wrapped to `width`: the height the label
    /// needs to show all of it. A dialog sizes itself around its message with
    /// this.
    pub(crate) fn text_height(&self, painter: &Painter, theme: &Theme, width: i32) -> i32 {
        let size = self.size.unwrap_or(theme.font_size);
        painter.wrap_text(&self.text, size, width).len() as i32 * line_height(painter, size)
    }
}

/// Em height with a little leading; identical for every line, so any sample
/// string gives the same value.
fn line_height(painter: &Painter, size: f32) -> i32 {
    painter.measure_text("", size).h.max(1)
}

impl Widget for Label {
    fn bounds(&self) -> Rect {
        self.rect
    }

    /// Adopt the slot a layout container (`Column`, `Row`) hands us, so the
    /// text wraps to and is clipped against that slot. Labels placed at
    /// absolute positions in a `Container` are never laid out and keep the
    /// rectangle they were constructed with.
    fn layout(&mut self, bounds: Rect) {
        self.rect = bounds;
    }

    fn paint(&mut self, painter: &mut Painter, theme: &Theme) {
        if let Some(bg) = self.background {
            painter.fill_rect(self.rect, bg);
        }
        let size = self.size.unwrap_or(theme.font_size);
        let color = self.color.unwrap_or(theme.text);
        let line_height = line_height(painter, size);

        let saved = painter.push_clip(self.rect);
        let mut y = self.rect.y;
        for line in painter.wrap_text(&self.text, size, self.rect.w) {
            painter.text(self.rect.x, y, line, size, color);
            y += line_height;
        }
        painter.restore_clip(saved);
    }
}
