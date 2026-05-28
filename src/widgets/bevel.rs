use crate::geometry::Rect;
use crate::painter::Painter;
use crate::theme::Theme;
use crate::widget::Widget;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BevelStyle {
    /// Two-line etched horizontal divider — shadow over highlight.
    EtchedLine,
    /// Raised 3D frame (light top/left, dark bottom/right).
    Raised,
    /// Sunken 3D frame (dark top/left, light bottom/right).
    Sunken,
}

/// Decorative chrome: a thin etched line or a 3D frame.
pub struct Bevel {
    pub rect: Rect,
    pub style: BevelStyle,
}

impl Bevel {
    pub fn etched_line(x: i32, y: i32, w: i32) -> Self {
        Self {
            rect: Rect::new(x, y, w, 2),
            style: BevelStyle::EtchedLine,
        }
    }

    pub fn raised(rect: Rect) -> Self {
        Self {
            rect,
            style: BevelStyle::Raised,
        }
    }

    pub fn sunken(rect: Rect) -> Self {
        Self {
            rect,
            style: BevelStyle::Sunken,
        }
    }
}

impl Widget for Bevel {
    fn bounds(&self) -> Rect {
        self.rect
    }

    fn paint(&mut self, painter: &mut Painter, theme: &Theme) {
        match self.style {
            BevelStyle::EtchedLine => {
                painter.etched_h_line(self.rect.x, self.rect.y, self.rect.w, theme);
            }
            BevelStyle::Raised => {
                painter.raised_bevel(self.rect, theme.highlight, theme.shadow);
            }
            BevelStyle::Sunken => {
                painter.sunken_bevel(self.rect, theme.highlight, theme.shadow);
            }
        }
    }
}
