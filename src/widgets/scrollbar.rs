use crate::event::{Event, EventCtx, MouseButton};
use crate::geometry::{Color, Point, Rect};
use crate::painter::Painter;
use crate::theme::Theme;
use crate::widget::Widget;

/// Logical-pixel size of the arrow buttons at each end of the bar and the
/// long-axis breadth of the bar itself. Matches Win 3.1's chrome.
pub const SCROLLBAR_THICKNESS: i32 = 16;
const ARROW_BTN: i32 = SCROLLBAR_THICKNESS;
const MIN_THUMB: i32 = 16;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Orientation {
    Vertical,
    Horizontal,
}

/// A classic Win 3.1 scrollbar: two arrow buttons bracketing a track with a
/// proportionally-sized thumb in the middle.
///
/// The scrollbar owns its own scroll position (`value`, in document units —
/// for a text editor that's typically "rows"). Composite widgets that embed
/// it just read `value()` to know what to render and call `set_range` /
/// `set_value` to keep the scrollbar in sync with their content.
pub struct ScrollBar {
    rect: Rect,
    orientation: Orientation,
    value: i32,
    /// Maximum scroll position. `value` is always clamped to `0..=max`.
    max: i32,
    /// Size of the visible portion in document units (used for thumb size
    /// and as the default page-step amount).
    viewport: i32,
    /// How much one arrow-button click scrolls.
    line_step: i32,
    /// While dragging the thumb, the pointer's offset from the thumb's
    /// leading edge (top for vertical, left for horizontal).
    drag_offset: Option<i32>,
}

impl ScrollBar {
    pub fn new(rect: Rect, orientation: Orientation) -> Self {
        Self {
            rect,
            orientation,
            value: 0,
            max: 0,
            viewport: 0,
            line_step: 1,
            drag_offset: None,
        }
    }

    pub fn vertical(rect: Rect) -> Self {
        Self::new(rect, Orientation::Vertical)
    }

    pub fn horizontal(rect: Rect) -> Self {
        Self::new(rect, Orientation::Horizontal)
    }

    pub fn rect(&self) -> Rect {
        self.rect
    }

    pub fn set_rect(&mut self, rect: Rect) {
        self.rect = rect;
    }

    pub fn value(&self) -> i32 {
        self.value
    }

    pub fn set_value(&mut self, value: i32) {
        self.value = value.clamp(0, self.max);
    }

    pub fn max(&self) -> i32 {
        self.max
    }

    pub fn viewport(&self) -> i32 {
        self.viewport
    }

    /// Tell the bar how large the visible window is and how far it can
    /// scroll. `value` is re-clamped to the new range.
    pub fn set_range(&mut self, viewport: i32, max: i32) {
        self.viewport = viewport.max(0);
        self.max = max.max(0);
        if self.value > self.max {
            self.value = self.max;
        }
    }

    pub fn set_line_step(&mut self, step: i32) {
        self.line_step = step.max(1);
    }

    fn track_rect(&self) -> Rect {
        match self.orientation {
            Orientation::Vertical => Rect::new(
                self.rect.x,
                self.rect.y + ARROW_BTN,
                self.rect.w,
                (self.rect.h - 2 * ARROW_BTN).max(0),
            ),
            Orientation::Horizontal => Rect::new(
                self.rect.x + ARROW_BTN,
                self.rect.y,
                (self.rect.w - 2 * ARROW_BTN).max(0),
                self.rect.h,
            ),
        }
    }

    fn track_extent(&self) -> i32 {
        let t = self.track_rect();
        match self.orientation {
            Orientation::Vertical => t.h,
            Orientation::Horizontal => t.w,
        }
    }

    fn thumb_size(&self) -> i32 {
        let track = self.track_extent();
        if self.max <= 0 || self.viewport <= 0 {
            return track;
        }
        let total = self.viewport + self.max;
        ((track * self.viewport) / total.max(1)).max(MIN_THUMB).min(track)
    }

    fn thumb_offset(&self) -> i32 {
        if self.max <= 0 {
            return 0;
        }
        let movable = (self.track_extent() - self.thumb_size()).max(0);
        (movable as i64 * self.value as i64 / self.max.max(1) as i64) as i32
    }

    fn thumb_rect(&self) -> Rect {
        let track = self.track_rect();
        let off = self.thumb_offset();
        let size = self.thumb_size();
        match self.orientation {
            Orientation::Vertical => Rect::new(track.x, track.y + off, track.w, size),
            Orientation::Horizontal => Rect::new(track.x + off, track.y, size, track.h),
        }
    }

    fn neg_arrow_rect(&self) -> Rect {
        match self.orientation {
            Orientation::Vertical => Rect::new(self.rect.x, self.rect.y, self.rect.w, ARROW_BTN),
            Orientation::Horizontal => Rect::new(self.rect.x, self.rect.y, ARROW_BTN, self.rect.h),
        }
    }

    fn pos_arrow_rect(&self) -> Rect {
        match self.orientation {
            Orientation::Vertical => Rect::new(
                self.rect.x,
                self.rect.bottom() - ARROW_BTN,
                self.rect.w,
                ARROW_BTN,
            ),
            Orientation::Horizontal => Rect::new(
                self.rect.right() - ARROW_BTN,
                self.rect.y,
                ARROW_BTN,
                self.rect.h,
            ),
        }
    }

    fn scroll_by(&mut self, delta: i32) {
        self.set_value(self.value.saturating_add(delta));
    }

    fn page_step(&self) -> i32 {
        self.viewport.max(1)
    }

    fn handle_press(&mut self, pos: Point) {
        if self.neg_arrow_rect().contains(pos) {
            self.scroll_by(-self.line_step);
        } else if self.pos_arrow_rect().contains(pos) {
            self.scroll_by(self.line_step);
        } else if self.thumb_rect().contains(pos) {
            let thumb = self.thumb_rect();
            let offset = match self.orientation {
                Orientation::Vertical => pos.y - thumb.y,
                Orientation::Horizontal => pos.x - thumb.x,
            };
            self.drag_offset = Some(offset);
        } else if self.track_rect().contains(pos) {
            // Page step toward the click.
            let thumb = self.thumb_rect();
            let page = self.page_step();
            match self.orientation {
                Orientation::Vertical => {
                    if pos.y < thumb.y {
                        self.scroll_by(-page);
                    } else if pos.y >= thumb.bottom() {
                        self.scroll_by(page);
                    }
                }
                Orientation::Horizontal => {
                    if pos.x < thumb.x {
                        self.scroll_by(-page);
                    } else if pos.x >= thumb.right() {
                        self.scroll_by(page);
                    }
                }
            }
        }
    }

    fn handle_drag(&mut self, pos: Point) {
        let Some(offset) = self.drag_offset else { return };
        let track = self.track_rect();
        let thumb_size = self.thumb_size();
        let movable = (self.track_extent() - thumb_size).max(1);
        let pos_in_track = match self.orientation {
            Orientation::Vertical => pos.y - offset - track.y,
            Orientation::Horizontal => pos.x - offset - track.x,
        };
        let clamped = pos_in_track.clamp(0, movable);
        self.value = ((self.max as i64 * clamped as i64) / movable as i64) as i32;
    }
}

impl Widget for ScrollBar {
    fn bounds(&self) -> Rect {
        self.rect
    }

    fn paint(&mut self, painter: &mut Painter, theme: &Theme) {
        // Track first — the light-gray strip that always shows behind the
        // thumb. Win 3.1 used a checkered "newsprint" pattern; the flat
        // light-gray fill we use here reads as the same thing at small
        // scale and keeps the chrome simpler.
        painter.fill_rect(self.rect, theme.face);

        let up = self.neg_arrow_rect();
        let down = self.pos_arrow_rect();
        painter.button(up, theme, false, false);
        painter.button(down, theme, false, false);
        draw_arrow(painter, up, self.orientation, ArrowDir::Negative, theme.text);
        draw_arrow(painter, down, self.orientation, ArrowDir::Positive, theme.text);

        if self.max > 0 {
            let thumb = self.thumb_rect();
            painter.button(thumb, theme, false, false);
        }
    }

    fn event(&mut self, event: &Event, ctx: &mut EventCtx) {
        match event {
            Event::PointerDown {
                pos,
                button: MouseButton::Left,
            } => {
                self.handle_press(*pos);
                ctx.request_paint();
            }
            Event::PointerMove { pos }
                if self.drag_offset.is_some() => {
                    self.handle_drag(*pos);
                    ctx.request_paint();
                }
            Event::PointerUp {
                button: MouseButton::Left,
                ..
            }
                if self.drag_offset.is_some() => {
                    self.drag_offset = None;
                    ctx.request_paint();
                }
            _ => {}
        }
    }

    fn captures_pointer(&self) -> bool {
        self.drag_offset.is_some()
    }

    fn layout(&mut self, bounds: Rect) {
        self.rect = bounds;
    }
}

#[derive(Clone, Copy)]
enum ArrowDir {
    Negative,
    Positive,
}

/// Solid-triangle arrow centered in `btn`, pointing in the requested
/// direction for the bar's orientation. The triangle is built from three or
/// five short horizontal/vertical lines — small enough that scanline-fill
/// would be overkill.
fn draw_arrow(painter: &mut Painter, btn: Rect, orient: Orientation, dir: ArrowDir, color: Color) {
    let cx = btn.x + btn.w / 2;
    let cy = btn.y + btn.h / 2;
    match (orient, dir) {
        (Orientation::Vertical, ArrowDir::Negative) => {
            // Up: tip on top, base on bottom.
            painter.h_line(cx, cy - 1, 1, color);
            painter.h_line(cx - 1, cy, 3, color);
            painter.h_line(cx - 2, cy + 1, 5, color);
        }
        (Orientation::Vertical, ArrowDir::Positive) => {
            painter.h_line(cx - 2, cy - 1, 5, color);
            painter.h_line(cx - 1, cy, 3, color);
            painter.h_line(cx, cy + 1, 1, color);
        }
        (Orientation::Horizontal, ArrowDir::Negative) => {
            painter.v_line(cx - 1, cy, 1, color);
            painter.v_line(cx, cy - 1, 3, color);
            painter.v_line(cx + 1, cy - 2, 5, color);
        }
        (Orientation::Horizontal, ArrowDir::Positive) => {
            painter.v_line(cx - 1, cy - 2, 5, color);
            painter.v_line(cx, cy - 1, 3, color);
            painter.v_line(cx + 1, cy, 1, color);
        }
    }
}
