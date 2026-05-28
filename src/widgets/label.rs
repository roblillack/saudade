use crate::geometry::{Color, Rect};
use crate::painter::Painter;
use crate::theme::Theme;
use crate::widget::Widget;

/// A single line of text positioned at an absolute point.
pub struct Label {
    pub x: i32,
    pub y: i32,
    pub text: String,
    pub size: Option<f32>,
    pub color: Option<Color>,
}

impl Label {
    pub fn new(x: i32, y: i32, text: impl Into<String>) -> Self {
        Self {
            x,
            y,
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

impl Widget for Label {
    fn bounds(&self) -> Rect {
        Rect::new(self.x, self.y, 0, 0)
    }

    fn paint(&mut self, painter: &mut Painter, theme: &Theme) {
        let size = self.size.unwrap_or(theme.font_size);
        let color = self.color.unwrap_or(theme.text);
        painter.text(self.x, self.y, &self.text, size, color);
    }
}
