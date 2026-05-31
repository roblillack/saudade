use crate::geometry::{Color, Rect};
use crate::painter::Painter;
use crate::theme::Theme;
use crate::widget::Widget;

/// A box of text.
///
/// A `Label` occupies a fixed rectangle. Text is laid out from the box's
/// top-left corner and word-wrapped to the box width: explicit `\n`
/// characters start a new line, and any line too wide for the box breaks at
/// whitespace (a single word wider than the box overflows rather than being
/// split mid-word). Anything that extends past the box — horizontally or
/// vertically — is clipped to its bounds, so a label never paints outside
/// the rectangle it was given. Color and size are inherited from the active
/// [`Theme`] unless overridden.
pub struct Label {
    pub rect: Rect,
    pub text: String,
    pub size: Option<f32>,
    pub color: Option<Color>,
}

impl Label {
    pub fn new(rect: Rect, text: impl Into<String>) -> Self {
        Self {
            rect,
            text: text.into(),
            size: None,
            color: None,
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
}

/// Break `text` into the lines that will actually be drawn: split on
/// explicit newlines, then greedily word-wrap each paragraph to `max_width`
/// logical pixels. `painter` is only used to measure candidate lines, so this
/// stays correct at any DPI. A non-positive `max_width` (a degenerate box)
/// skips wrapping — the clip rectangle hides the overflow anyway.
fn layout_lines(painter: &Painter, text: &str, size: f32, max_width: i32) -> Vec<String> {
    let mut lines = Vec::new();
    for paragraph in text.split('\n') {
        if max_width > 0 {
            wrap_paragraph(painter, paragraph, size, max_width, &mut lines);
        } else {
            lines.push(paragraph.to_string());
        }
    }
    lines
}

/// Greedily pack the words of `paragraph` into lines no wider than
/// `max_width`, appending each finished line to `out`. An empty paragraph
/// (e.g. from a blank line between two `\n`) still emits one empty line so
/// vertical spacing is preserved.
fn wrap_paragraph(
    painter: &Painter,
    paragraph: &str,
    size: f32,
    max_width: i32,
    out: &mut Vec<String>,
) {
    let mut current = String::new();
    for word in paragraph.split_whitespace() {
        if current.is_empty() {
            current.push_str(word);
            continue;
        }
        let candidate = format!("{current} {word}");
        if painter.measure_text(&candidate, size).w <= max_width {
            current = candidate;
        } else {
            out.push(std::mem::take(&mut current));
            current.push_str(word);
        }
    }
    out.push(current);
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
        let size = self.size.unwrap_or(theme.font_size);
        let color = self.color.unwrap_or(theme.text);
        // Em height with a little leading; identical for every line, so any
        // sample string gives the same value.
        let line_height = painter.measure_text("", size).h.max(1);

        let lines = layout_lines(painter, &self.text, size, self.rect.w);

        let saved = painter.push_clip(self.rect);
        let mut y = self.rect.y;
        for line in &lines {
            painter.text(self.rect.x, y, line, size, color);
            y += line_height;
        }
        painter.restore_clip(saved);
    }
}
