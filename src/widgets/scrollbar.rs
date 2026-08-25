use std::time::{Duration, Instant};

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

/// How long an arrow button must stay held before its line-step scroll begins
/// to auto-repeat, and the steady interval between repeats once it does. These
/// mirror a typical keyboard auto-repeat: a longer initial pause, then a
/// regular stream of clicks for as long as the button is held.
const REPEAT_DELAY: Duration = Duration::from_millis(300);
const REPEAT_INTERVAL: Duration = Duration::from_millis(50);

/// State of an arrow button held down with the primary mouse button. While a
/// button is held the bar paints it depressed and auto-repeats its scroll, the
/// same way Win 3.1's arrow buttons "machine-gun" when you hold them.
#[derive(Clone, Copy)]
struct HeldButton {
    /// Which arrow is held — the same direction its single click scrolls.
    dir: ArrowDir,
    /// Whether the pointer is currently over the held button. The button only
    /// paints depressed and only auto-repeats while armed; sliding the pointer
    /// off it pauses both (and re-arms on return), mirroring how a held button
    /// pops back out when you drag away from it.
    armed: bool,
    /// When the button was first pressed — drives the initial repeat delay.
    pressed_at: Instant,
    /// When the last auto-repeat fired — drives the steady repeat interval.
    last_repeat: Instant,
}

impl HeldButton {
    fn new(dir: ArrowDir) -> Self {
        let now = Instant::now();
        Self {
            dir,
            armed: true,
            pressed_at: now,
            last_repeat: now,
        }
    }
}

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
    /// `Some` while an arrow button is held down — the source of the depressed
    /// look and the click auto-repeat. `None` when no button is held.
    held: Option<HeldButton>,
    /// Sub-step scroll-wheel remainder, in *document units*. Wheel / trackpad
    /// deltas arrive in fractional lines; each is scaled by `line_step` into
    /// this bar's own units and accumulated here, and `value` only moves once a
    /// whole unit has built up. That is what lets a high-resolution trackpad
    /// scroll smoothly instead of snapping a whole step at a time — including
    /// on a bar whose unit is a pixel, where one line is `line_step` of them.
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
            held: None,
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

    /// How far one line travels, in this bar's document units: what an arrow
    /// button click scrolls, and what a wheel detent scrolls three of. Defaults
    /// to 1, which is right for a bar whose `value` counts rows; a bar that
    /// counts pixels of content sets it to a row's height, so the wheel covers
    /// the same ground either way.
    pub fn set_line_step(&mut self, step: i32) {
        self.line_step = step.max(1);
    }

    /// Abandon any in-progress thumb drag or held arrow button. A host that can
    /// be torn down mid-gesture (e.g. a dropdown popup that closes on focus
    /// loss) calls this so a stale `drag_offset` can't grab the thumb on the
    /// next pointer move, and a stale held button can't keep the bar capturing
    /// the pointer or requesting ticks.
    pub fn end_drag(&mut self) {
        self.drag_offset = None;
        self.held = None;
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

    /// Apply a wheel / trackpad scroll measured in (possibly fractional) lines
    /// along this bar's axis. A line is [`set_line_step`](Self::set_line_step)
    /// document units, the same distance an arrow button covers, so a gesture
    /// travels the same way on screen whether `value` counts rows or pixels.
    /// Sub-unit movement is banked in `wheel_accum` until it adds up to a whole
    /// unit. Returns `true` if `value` actually moved, so callers can decide
    /// whether to repaint.
    fn scroll_lines(&mut self, lines: f32) -> bool {
        self.wheel_accum += lines * self.line_step as f32;
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

    /// Rect of the arrow button currently held, if any.
    fn held_button_rect(&self) -> Option<Rect> {
        self.held.map(|h| match h.dir {
            ArrowDir::Negative => self.neg_arrow_rect(),
            ArrowDir::Positive => self.pos_arrow_rect(),
        })
    }

    /// Drive the held-button auto-repeat from an [`Event::Tick`]. After the
    /// initial [`REPEAT_DELAY`] from the press, scrolls one line-step every
    /// [`REPEAT_INTERVAL`] for as long as the button stays armed. Returns `true`
    /// if `value` actually moved, so the caller can decide whether to repaint.
    fn tick_repeat(&mut self) -> bool {
        let Some(held) = self.held else {
            return false;
        };
        if !held.armed {
            return false;
        }
        let now = Instant::now();
        // Wait out the initial delay (timed from the press) before the first
        // repeat, then pace the rest by the steady interval (timed from the
        // previous repeat).
        if now.duration_since(held.pressed_at) < REPEAT_DELAY
            || now.duration_since(held.last_repeat) < REPEAT_INTERVAL
        {
            return false;
        }
        if let Some(h) = self.held.as_mut() {
            h.last_repeat = now;
        }
        let step = match held.dir {
            ArrowDir::Negative => -self.line_step,
            ArrowDir::Positive => self.line_step,
        };
        let before = self.value;
        self.scroll_by(step);
        self.value != before
    }

    fn handle_press(&mut self, pos: Point) {
        if self.neg_arrow_rect().contains(pos) {
            self.scroll_by(-self.line_step);
            self.held = Some(HeldButton::new(ArrowDir::Negative));
        } else if self.pos_arrow_rect().contains(pos) {
            self.scroll_by(self.line_step);
            self.held = Some(HeldButton::new(ArrowDir::Positive));
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

        // A held arrow paints depressed — but only while the pointer is still
        // over it (armed), so dragging off a held button pops it back out.
        let (neg_pressed, pos_pressed) = match self.held {
            Some(HeldButton {
                dir: ArrowDir::Negative,
                armed: true,
                ..
            }) => (true, false),
            Some(HeldButton {
                dir: ArrowDir::Positive,
                armed: true,
                ..
            }) => (false, true),
            _ => (false, false),
        };

        // Track: the Win 3.1 "newsprint" checkerboard (black on gray) that shows
        // in the gap between the buttons wherever the thumb isn't.
        painter.fill_checker(self.track_rect(), theme.face, theme.border);

        // Buttons and thumb in the lighter frame (square outline, single
        // top/left highlight, 2px bottom/right shadow); a held button sinks in.
        painter.light_button(up, theme, neg_pressed);
        painter.light_button(down, theme, pos_pressed);
        if let Some(thumb) = thumb_opt {
            // Where the thumb sits flush against an arrow button, overlap that
            // button's frame by 1px so the thumb's black outline lands on the
            // *same* row/column as the button's, collapsing the divider to a
            // single 1px line instead of stacking the two 1px frames into a 2px
            // band.
            let track = self.track_rect();
            let mut t = thumb;
            match self.orientation {
                Orientation::Vertical => {
                    if thumb.y <= track.y {
                        t.y -= 1;
                        t.h += 1;
                    }
                    if thumb.bottom() >= track.bottom() {
                        t.h += 1;
                    }
                }
                Orientation::Horizontal => {
                    if thumb.x <= track.x {
                        t.x -= 1;
                        t.w += 1;
                    }
                    if thumb.right() >= track.right() {
                        t.w += 1;
                    }
                }
            }
            painter.light_button(t, theme, false);
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
        // A depressed button carries its glyph 1px down/right for the same
        // tactile "pushed in" feel the dialog buttons give their labels.
        let nudge = |r: Rect, on: bool| {
            if on {
                Rect::new(r.x + 1, r.y + 1, r.w, r.h)
            } else {
                r
            }
        };
        draw_arrow(
            painter,
            nudge(up, neg_pressed),
            self.orientation,
            ArrowDir::Negative,
            theme.text,
        );
        draw_arrow(
            painter,
            nudge(down, pos_pressed),
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
                ..
            } => {
                self.handle_press(*pos);
                ctx.request_paint();
            }
            Event::PointerMove { pos } if self.drag_offset.is_some() => {
                self.handle_drag(*pos);
                ctx.request_paint();
            }
            // While an arrow button is held, track whether the pointer is still
            // over it: sliding off pauses the depressed look and the repeat,
            // sliding back resumes them — the same way a real push button pops
            // out when you drag away from it.
            Event::PointerMove { pos } if self.held.is_some() => {
                let over = self.held_button_rect().is_some_and(|r| r.contains(*pos));
                if let Some(h) = self.held.as_mut()
                    && h.armed != over
                {
                    h.armed = over;
                    ctx.request_paint();
                }
            }
            Event::PointerUp {
                button: MouseButton::Left,
                ..
            } if self.drag_offset.is_some() || self.held.is_some() => {
                self.drag_offset = None;
                self.held = None;
                ctx.request_paint();
            }
            // The pointer left the window mid-gesture — most often because an
            // outbound drag-and-drop (or a popup teardown) revoked our pointer
            // focus, so the matching release never arrives. With no OS grab to
            // keep tracking, end the drag / release the held button here;
            // otherwise a stale `drag_offset` would make the thumb chase the
            // cursor when it returns, and a stale `held` would keep repeating.
            Event::PointerLeave if self.drag_offset.is_some() || self.held.is_some() => {
                self.drag_offset = None;
                self.held = None;
                ctx.request_paint();
            }
            // Auto-repeat the held arrow's line-step scroll at the key-repeat
            // cadence for as long as the button stays armed.
            Event::Tick if self.held.is_some() => {
                if self.tick_repeat() {
                    ctx.request_paint();
                }
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
        // While a button is held, keep the animation clock alive by pushing a
        // one-shot tick request on every event we handle (the press that grabs
        // the button, each repeat tick, the re-arming moves). This rides the
        // shared `EventCtx` straight to the runtime, so the bar auto-repeats
        // even when it's buried under custom wrappers that don't forward
        // `wants_ticks()`. Releasing the button stops the re-requests and the
        // ticks wind down on their own.
        if self.held.is_some() {
            ctx.request_tick();
        }
    }

    fn captures_pointer(&self) -> bool {
        // Hold the pointer while dragging the thumb or while an arrow button is
        // held, so the move / release (and the ticks driving the repeat) keep
        // routing here even when the host hit-tests by position.
        self.drag_offset.is_some() || self.held.is_some()
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
/// direction for the bar's orientation. The SVG is drawn across the whole
/// button rect: each glyph's 16-unit viewBox already centers the small triangle
/// with the classic Win 3.1 margin around it, so it maps 1:1 onto the 16px
/// button at 1.0x without any extra inset.
fn draw_arrow(painter: &mut Painter, btn: Rect, orient: Orientation, dir: ArrowDir, color: Color) {
    let arrow = match (orient, dir) {
        (Orientation::Vertical, ArrowDir::Negative) => &ARROW_UP,
        (Orientation::Vertical, ArrowDir::Positive) => &ARROW_DOWN,
        (Orientation::Horizontal, ArrowDir::Negative) => &ARROW_LEFT,
        (Orientation::Horizontal, ArrowDir::Positive) => &ARROW_RIGHT,
    };
    arrow.draw_tinted(painter, btn, color);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::event::{Modifiers, WHEEL_LINES_PER_DETENT};

    /// A 16×200 vertical bar with 100 lines of content, 10 of them visible —
    /// so `value` can travel 0..=90 a single line-step at a time.
    fn vbar() -> ScrollBar {
        let mut sb = ScrollBar::vertical(Rect::new(0, 0, 16, 200));
        sb.set_range(10, 90);
        sb
    }

    fn center(r: Rect) -> Point {
        Point::new(r.x + r.w / 2, r.y + r.h / 2)
    }

    /// Dispatch `event` and hand back the `EventCtx`, so tests can read both
    /// `paint_requested` and the push-based `tick_requested` it set.
    fn send(sb: &mut ScrollBar, event: Event) -> EventCtx {
        let mut ctx = EventCtx::new();
        sb.event(&event, &mut ctx);
        ctx
    }

    fn press(sb: &mut ScrollBar, pos: Point) -> EventCtx {
        send(
            sb,
            Event::PointerDown {
                pos,
                button: MouseButton::Left,
                modifiers: Modifiers::default(),
            },
        )
    }

    fn release(sb: &mut ScrollBar, pos: Point) -> EventCtx {
        send(
            sb,
            Event::PointerUp {
                pos,
                button: MouseButton::Left,
                modifiers: Modifiers::default(),
            },
        )
    }

    fn tick(sb: &mut ScrollBar) -> EventCtx {
        send(sb, Event::Tick)
    }

    /// Backdate the held button so the *next* tick is past both the initial
    /// delay and the repeat interval — lets the repeat be tested without
    /// actually sleeping.
    fn ripen_repeat(sb: &mut ScrollBar) {
        let past = Instant::now()
            .checked_sub(REPEAT_DELAY + REPEAT_INTERVAL)
            .expect("clock is far enough past the epoch");
        let h = sb.held.as_mut().expect("a button is held");
        h.pressed_at = past;
        h.last_repeat = past;
    }

    #[test]
    fn pressing_an_arrow_scrolls_one_step_and_grabs_the_pointer() {
        let mut sb = vbar();
        let p = center(sb.pos_arrow_rect());
        let ctx = press(&mut sb, p);
        assert_eq!(sb.value(), 1, "the press itself scrolls one line-step");
        assert!(sb.captures_pointer(), "a held button captures the pointer");
        assert!(
            ctx.tick_requested,
            "a held button pushes a tick request to drive the repeat"
        );
    }

    #[test]
    fn releasing_the_button_ends_the_repeat() {
        let mut sb = vbar();
        let p = center(sb.pos_arrow_rect());
        press(&mut sb, p);
        let ctx = release(&mut sb, p);
        assert!(!sb.captures_pointer(), "the pointer is freed on release");
        assert!(
            !ctx.tick_requested,
            "and the bar stops re-requesting ticks, so they wind down"
        );
    }

    #[test]
    fn no_repeat_before_the_initial_delay_elapses() {
        let mut sb = vbar();
        let p = center(sb.pos_arrow_rect());
        press(&mut sb, p);
        // A tick right after the press is inside the initial delay.
        let ctx = tick(&mut sb);
        assert_eq!(sb.value(), 1, "the button doesn't repeat during the delay");
        assert!(!ctx.paint_requested, "nothing moved, so no repaint");
        assert!(
            ctx.tick_requested,
            "but the held button keeps the clock alive through the delay"
        );
    }

    #[test]
    fn the_repeat_fires_once_the_delay_has_elapsed() {
        let mut sb = vbar();
        let p = center(sb.pos_arrow_rect());
        press(&mut sb, p);
        ripen_repeat(&mut sb);
        let ctx = tick(&mut sb);
        assert_eq!(sb.value(), 2, "a ripe held button repeats its line-step");
        assert!(
            ctx.paint_requested,
            "a repeat that moves asks for a repaint"
        );
        assert!(ctx.tick_requested, "and keeps requesting the next tick");
    }

    #[test]
    fn sliding_off_the_held_button_pauses_the_repeat() {
        let mut sb = vbar();
        let p = center(sb.pos_arrow_rect());
        press(&mut sb, p);
        // Move onto the track, away from the held arrow: still held, disarmed.
        let off = center(sb.track_rect());
        send(&mut sb, Event::PointerMove { pos: off });
        ripen_repeat(&mut sb);
        let ctx = tick(&mut sb);
        assert_eq!(
            sb.value(),
            1,
            "an off-button (disarmed) hold doesn't repeat"
        );
        assert!(sb.captures_pointer(), "but the hold persists until release");
        assert!(
            ctx.tick_requested,
            "and the clock keeps running so a slide back can resume it"
        );
    }

    #[test]
    fn the_up_arrow_at_the_top_holds_without_underflowing() {
        let mut sb = vbar();
        let p = center(sb.neg_arrow_rect());
        press(&mut sb, p);
        assert_eq!(sb.value(), 0, "already at the top, value can't go negative");
        assert!(
            sb.captures_pointer(),
            "the button still grabs so the release lands here"
        );
    }

    #[test]
    fn pointer_leave_releases_a_held_button() {
        let mut sb = vbar();
        let p = center(sb.pos_arrow_rect());
        press(&mut sb, p);
        let ctx = send(&mut sb, Event::PointerLeave);
        assert!(!sb.captures_pointer(), "leaving the window ends the hold");
        assert!(!ctx.tick_requested, "and stops the tick re-requests");
    }

    /// A bar whose `value` counts *pixels* of content, the shape a pane with
    /// unequal row heights needs: 24px rows, 560px of viewport, 1000px of
    /// overflow.
    fn pixel_bar() -> ScrollBar {
        let mut sb = ScrollBar::vertical(Rect::new(0, 0, 16, 560));
        sb.set_line_step(24);
        sb.set_range(560, 1000);
        sb
    }

    fn wheel(sb: &mut ScrollBar, lines: f32) -> EventCtx {
        send(
            sb,
            Event::Scroll {
                pos: center(sb.track_rect()),
                delta_x: 0.0,
                delta_y: lines,
            },
        )
    }

    #[test]
    fn a_wheel_detent_travels_one_line_step_per_line() {
        // A row-valued bar: a line *is* a unit, so a detent moves three.
        let mut rows = vbar();
        wheel(&mut rows, WHEEL_LINES_PER_DETENT);
        assert_eq!(rows.value(), 3, "three rows on a bar that counts rows");

        // A pixel-valued bar covers the same ground on screen — three rows of
        // it — rather than three pixels.
        let mut pixels = pixel_bar();
        wheel(&mut pixels, WHEEL_LINES_PER_DETENT);
        assert_eq!(
            pixels.value(),
            72,
            "three 24px rows on a bar that counts pixels"
        );
    }

    #[test]
    fn a_trackpad_moves_a_pixel_valued_bar_below_a_whole_line() {
        let mut sb = pixel_bar();
        // A fine trackpad delta: a sixteenth of a line, i.e. one logical pixel
        // of finger travel. Sub-line, but 1.5 units of *this* bar.
        let ctx = wheel(&mut sb, 1.0 / 16.0);
        assert_eq!(
            sb.value(),
            1,
            "a sub-line delta still moves a pixel-valued bar"
        );
        assert!(
            ctx.paint_requested,
            "and asks for the repaint that shows it"
        );

        // The half-pixel remainder is banked, not dropped: the next identical
        // delta lands a whole 2px further on.
        wheel(&mut sb, 1.0 / 16.0);
        assert_eq!(
            sb.value(),
            3,
            "the sub-unit remainder carries into the next delta"
        );
    }

    #[test]
    fn a_wheel_delta_too_small_to_move_the_bar_is_banked() {
        let mut sb = vbar();
        let ctx = wheel(&mut sb, 0.25);
        assert_eq!(
            sb.value(),
            0,
            "a quarter of a row doesn't move a row-valued bar"
        );
        assert!(
            !ctx.paint_requested,
            "and doesn't ask for a pointless repaint"
        );
        wheel(&mut sb, 0.75);
        assert_eq!(
            sb.value(),
            1,
            "but the bank pays out once a whole row adds up"
        );
    }

    #[test]
    fn the_wheel_bank_is_dropped_at_the_end_of_the_range() {
        let mut sb = pixel_bar();
        // Wheel far past the bottom, then reverse: the first delta back up has
        // to move immediately rather than unwinding what piled up at the stop.
        for _ in 0..40 {
            wheel(&mut sb, WHEEL_LINES_PER_DETENT);
        }
        assert_eq!(sb.value(), sb.max(), "the bar is parked at the bottom");
        wheel(&mut sb, -1.0 / 16.0);
        assert_eq!(
            sb.value(),
            sb.max() - 1,
            "reversing responds on the very next delta"
        );
    }

    /// A minimal host that forwards events to an inner scrollbar but
    /// deliberately does *not* override `wants_ticks` — so the pull model would
    /// never schedule a tick for it. Stands in for a custom wrapper widget like
    /// the `filer` example's `FileBrowser`.
    struct BareWrapper(ScrollBar);

    impl Widget for BareWrapper {
        fn bounds(&self) -> Rect {
            self.0.bounds()
        }
        fn paint(&mut self, _painter: &mut Painter, _theme: &Theme) {}
        fn event(&mut self, event: &Event, ctx: &mut EventCtx) {
            self.0.event(event, ctx);
        }
        fn layout(&mut self, bounds: Rect) {
            self.0.layout(bounds);
        }
    }

    #[test]
    fn a_held_button_requests_ticks_through_a_non_forwarding_wrapper() {
        let mut host = BareWrapper(vbar());
        // The wrapper never forwards tick appetite, so `wants_ticks()` is dead.
        assert!(
            !host.wants_ticks(),
            "the wrapper doesn't forward wants_ticks"
        );

        // Pressing the arrow *through* the wrapper still pushes a tick request
        // all the way to the top via the shared `EventCtx`.
        let p = center(host.0.pos_arrow_rect());
        let mut ctx = EventCtx::new();
        host.event(
            &Event::PointerDown {
                pos: p,
                button: MouseButton::Left,
                modifiers: Modifiers::default(),
            },
            &mut ctx,
        );
        assert!(
            ctx.tick_requested,
            "the held scrollbar's tick request reaches the top with no forwarding"
        );
        assert!(
            !host.wants_ticks(),
            "and it never had to rely on wants_ticks to do so"
        );
    }
}
