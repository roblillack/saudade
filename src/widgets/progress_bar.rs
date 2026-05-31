use crate::geometry::Rect;
use crate::painter::Painter;
use crate::theme::Theme;
use crate::widget::Widget;

/// A horizontal progress indicator: a sunken white field that fills from the
/// left with a solid grey bar in proportion to its [`fraction`](Self::fraction)
/// (0.0–1.0). The fill is a neutral grey rather than the selection navy on
/// purpose — navy means "focused / selected" on text fields and lists, which a
/// progress bar isn't. With [`with_percentage`](Self::with_percentage) it also
/// draws the rounded percentage centered over the bar in the normal text color.
///
/// `ProgressBar` is purely presentational: it has no events, no focus, and no
/// internal animation. Drive it by calling [`set_fraction`](Self::set_fraction)
/// from whatever owns the underlying progress (a download, a timer, …).
pub struct ProgressBar {
    rect: Rect,
    /// Fill amount, always clamped to `0.0..=1.0`.
    fraction: f32,
    show_percentage: bool,
}

impl ProgressBar {
    pub fn new(rect: Rect) -> Self {
        Self {
            rect,
            fraction: 0.0,
            show_percentage: false,
        }
    }

    pub fn with_fraction(mut self, fraction: f32) -> Self {
        self.set_fraction(fraction);
        self
    }

    /// Draw the rounded percentage centered over the bar.
    pub fn with_percentage(mut self, show: bool) -> Self {
        self.show_percentage = show;
        self
    }

    pub fn fraction(&self) -> f32 {
        self.fraction
    }

    pub fn set_fraction(&mut self, fraction: f32) {
        self.fraction = fraction.clamp(0.0, 1.0);
    }

    pub fn rect(&self) -> Rect {
        self.rect
    }

    pub fn set_rect(&mut self, rect: Rect) {
        self.rect = rect;
    }
}

impl Widget for ProgressBar {
    fn bounds(&self) -> Rect {
        self.rect
    }

    fn paint(&mut self, painter: &mut Painter, theme: &Theme) {
        // Sunken white field with a 1px black border — the same chrome a
        // single-line input wears, so the bar reads as a recessed gauge.
        painter.fill_rect(self.rect, theme.background);
        painter.sunken_bevel(self.rect, theme.highlight, theme.shadow);
        painter.stroke_rect(self.rect, theme.border);

        let inner = self.rect.inset(2);
        if inner.w <= 0 || inner.h <= 0 {
            return;
        }

        let fill_w = ((inner.w as f32) * self.fraction).round() as i32;
        if fill_w > 0 {
            // Neutral grey, not the selection navy: a progress bar is never
            // "focused" the way a highlighted list row or text field is.
            painter.fill_rect(Rect::new(inner.x, inner.y, fill_w, inner.h), theme.shadow);
        }

        if self.show_percentage {
            let pct = (self.fraction * 100.0).round() as i32;
            let text = format!("{pct}%");
            let size = theme.font_size;
            let m = painter.measure_text(&text, size);
            let tx = inner.x + ((inner.w - m.w) / 2).max(0);
            let ty = inner.y + ((inner.h - m.h) / 2).max(0);
            // One pass in the normal text color: black reads cleanly over both
            // the white (empty) and mid-grey (filled) halves, so there's no
            // need to invert the caption across the fill edge.
            painter.text(tx, ty, &text, size, theme.text);
        }
    }

    fn layout(&mut self, bounds: Rect) {
        self.rect = bounds;
    }
}
