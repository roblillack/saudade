use crate::geometry::{Color, Rect};
use crate::painter::Painter;
use crate::theme::Theme;
use crate::widget::Widget;

/// A run of text positioned at an absolute point.
///
/// A `Label` renders one or more lines. Explicit `\n` characters always
/// start a new line. When a wrap width is set via [`Label::wrap`], each line
/// is additionally word-wrapped to fit within that many logical pixels —
/// long lines break at whitespace, and a word wider than the limit overflows
/// on its own line rather than being split mid-word.
pub struct Label {
    pub x: i32,
    pub y: i32,
    pub text: String,
    pub size: Option<f32>,
    pub color: Option<Color>,
    /// Maximum line width in logical pixels for word wrapping. `None`
    /// disables wrapping; lines are then only broken on explicit `\n`.
    wrap_width: Option<i32>,
    /// Total rendered height in logical pixels, cached on each paint so
    /// [`Widget::bounds`] can report an accurate box. Zero until first paint.
    measured_height: i32,
}

impl Label {
    pub fn new(x: i32, y: i32, text: impl Into<String>) -> Self {
        Self {
            x,
            y,
            text: text.into(),
            size: None,
            color: None,
            wrap_width: None,
            measured_height: 0,
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

    /// Word-wrap the text to at most `max_width` logical pixels per line.
    /// Lines are still split on explicit `\n` first, then each is wrapped at
    /// whitespace to fit the width.
    pub fn wrap(mut self, max_width: i32) -> Self {
        self.wrap_width = Some(max_width);
        self
    }

    /// Break `text` into the lines that will actually be drawn: split on
    /// explicit newlines, then greedily word-wrap each paragraph to the wrap
    /// width (if one is set). `painter` is only used to measure candidate
    /// lines, so this stays correct at any DPI.
    fn layout_lines(&self, painter: &Painter, size: f32) -> Vec<String> {
        let mut lines = Vec::new();
        for paragraph in self.text.split('\n') {
            match self.wrap_width {
                Some(max_width) if max_width > 0 => {
                    wrap_paragraph(painter, paragraph, size, max_width, &mut lines)
                }
                _ => lines.push(paragraph.to_string()),
            }
        }
        lines
    }
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
        Rect::new(
            self.x,
            self.y,
            self.wrap_width.unwrap_or(0),
            self.measured_height,
        )
    }

    fn paint(&mut self, painter: &mut Painter, theme: &Theme) {
        let size = self.size.unwrap_or(theme.font_size);
        let color = self.color.unwrap_or(theme.text);
        // Em height with a little leading; identical for every line, so any
        // sample string gives the same value.
        let line_height = painter.measure_text("", size).h.max(1);

        let lines = self.layout_lines(painter, size);
        let mut y = self.y;
        for line in &lines {
            painter.text(self.x, y, line, size, color);
            y += line_height;
        }
        self.measured_height = lines.len() as i32 * line_height;
    }
}
