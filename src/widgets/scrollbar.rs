use crate::event::{Event, EventCtx, MouseButton};
use crate::geometry::{Color, Point, Rect};
use crate::include_svg;
use crate::painter::Painter;
use crate::svg::SvgImage;
use crate::theme::Theme;
use crate::widget::Widget;

/// Logical-pixel size of the arrow buttons at each end of the bar and the
/// long-axis breadth of the bar itself. Matches Win 3.1's chrome.
pub const SCROLLBAR_THICKNESS: i32 = 16;
const ARROW_BTN: i32 = SCROLLBAR_THICKNESS;
const MIN_THUMB: i32 = 16;
/// Logical-pixel margin left around the arrow glyph inside its button, so the
/// small triangle sits centered on the face rather than filling the button edge
/// to edge — the classic Win 3.1 proportion. The lighter [`Painter::light_button`]
/// frame no longer visually recesses the glyph the way the old heavy bevel did,
/// so the inset restores that breathing room explicitly.
const ARROW_INSET: i32 = 3;

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
    /// Sub-line scroll-wheel remainder. Wheel / trackpad deltas arrive in
    /// fractional lines; we accumulate them here and only move `value` once a
    /// whole line has built up, so a high-resolution trackpad scrolls smoothly
    /// instead of snapping a line at a time.
    wheel_accum: f32,
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
            wheel_accum: 0.0,
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

    /// Abandon any in-progress thumb drag. A host that can be torn down mid-drag
    /// (e.g. a dropdown popup that closes on focus loss) calls this so a stale
    /// `drag_offset` can't grab the thumb on the next pointer move.
    pub fn end_drag(&mut self) {
        self.drag_offset = None;
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
        ((track * self.viewport) / total.max(1))
            .max(MIN_THUMB)
            .min(track)
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

    /// Apply a wheel / trackpad scroll measured in (possibly fractional)
    /// lines along this bar's axis. Sub-line movement is banked in
    /// `wheel_accum` until it adds up to a whole line. Returns `true` if
    /// `value` actually moved, so callers can decide whether to repaint.
    fn scroll_lines(&mut self, lines: f32) -> bool {
        self.wheel_accum += lines;
        let whole = self.wheel_accum.trunc();
        self.wheel_accum -= whole;
        let step = whole as i32;
        if step == 0 {
            return false;
        }
        let before = self.value;
        self.scroll_by(step);
        if self.value == before {
            // Saturated at an end — drop the leftover so reversing direction
            // responds on the very next notch instead of unwinding the bank.
            self.wheel_accum = 0.0;
            false
        } else {
            true
        }
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
        let Some(offset) = self.drag_offset else {
            return;
        };
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
        let up = self.neg_arrow_rect();
        let down = self.pos_arrow_rect();
        let thumb_opt = (self.max > 0).then(|| self.thumb_rect());

        // Track: the Win 3.1 "newsprint" checkerboard (black on gray) that shows
        // in the gap between the buttons wherever the thumb isn't.
        painter.fill_checker(self.track_rect(), theme.face, theme.border);

        // Buttons and thumb in the lighter frame (square outline, single
        // top/left highlight, 2px bottom/right shadow).
        painter.light_button(up, theme);
        painter.light_button(down, theme);
        if let Some(thumb) = thumb_opt {
            painter.light_button(thumb, theme);
        }

        // A single black outline around the whole bar. Its long sides are the
        // track's own outline; they collapse into the button/thumb frames where
        // they meet, and the button frames supply the dividers between the
        // buttons and the track.
        painter.stroke_rect(self.rect, theme.border);
        // The arrow glyphs are baked SVGs; `SvgImage::draw_tinted` already drops
        // to a crisp physical-pixel pass at every scale, so no manual `physical`
        // branch is needed. Tinted with `theme.text` so they track the theme
        // (the SVGs' own black is just a placeholder). Sized to the classic
        // footprint, they are pixel-clean at 1.0x and anti-aliased (rather than
        // blocky) at fractional / HiDPI scales.
        draw_arrow(
            painter,
            up,
            self.orientation,
            ArrowDir::Negative,
            theme.text,
        );
        draw_arrow(
            painter,
            down,
            self.orientation,
            ArrowDir::Positive,
            theme.text,
        );
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
            Event::PointerMove { pos } if self.drag_offset.is_some() => {
                self.handle_drag(*pos);
                ctx.request_paint();
            }
            Event::PointerUp {
                button: MouseButton::Left,
                ..
            } if self.drag_offset.is_some() => {
                self.drag_offset = None;
                ctx.request_paint();
            }
            Event::Scroll {
                delta_x, delta_y, ..
            } => {
                let lines = match self.orientation {
                    Orientation::Vertical => *delta_y,
                    Orientation::Horizontal => *delta_x,
                };
                if self.scroll_lines(lines) {
                    ctx.request_paint();
                }
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

// The four arrow glyphs, baked from SVG at compile time. Each viewBox is 16
// units — the arrow-button size — so at 1.0x the triangle lands on exactly the
// device pixels the hand-drawn glyph used to, and `SvgImage::draw_tinted`
// re-snaps it crisply at other scales. Their baked black is only a placeholder:
// they are drawn tinted with `theme.text` so they follow the theme.
const ARROW_UP: SvgImage = include_svg!("assets/scrollbar/up.svg");
const ARROW_DOWN: SvgImage = include_svg!("assets/scrollbar/down.svg");
const ARROW_LEFT: SvgImage = include_svg!("assets/scrollbar/left.svg");
const ARROW_RIGHT: SvgImage = include_svg!("assets/scrollbar/right.svg");

/// Fill the arrow glyph into `btn` in `color`, pointing in the requested
/// direction for the bar's orientation.
fn draw_arrow(painter: &mut Painter, btn: Rect, orient: Orientation, dir: ArrowDir, color: Color) {
    let arrow = match (orient, dir) {
        (Orientation::Vertical, ArrowDir::Negative) => &ARROW_UP,
        (Orientation::Vertical, ArrowDir::Positive) => &ARROW_DOWN,
        (Orientation::Horizontal, ArrowDir::Negative) => &ARROW_LEFT,
        (Orientation::Horizontal, ArrowDir::Positive) => &ARROW_RIGHT,
    };
    arrow.draw_tinted(painter, btn.inset(ARROW_INSET), color);
}
