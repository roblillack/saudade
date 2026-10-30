use std::time::Instant;

use crate::event::{Event, EventCtx, MouseButton, WHEEL_LINES_PER_DETENT};
use crate::geometry::{Point, Rect};
use crate::include_svg;
use crate::painter::{Merged, Painter};
use crate::svg::SvgImage;
use crate::theme::Theme;
use crate::widget::Widget;
use crate::widgets::scrollbar::{Orientation, REPEAT_DELAY, REPEAT_INTERVAL};

type StepHandler = Box<dyn FnMut(&mut EventCtx, i32)>;

// The scrollbar's arrow glyphs. Each sits centered in a 16-unit viewBox, the
// scrollbar's arrow button, and is drawn into a box of that size (see
// `GLYPH_BOX`) so the triangle comes out exactly as it does on a scrollbar.
const ARROW_UP: SvgImage = include_svg!("assets/scrollbar/up.svg");
const ARROW_DOWN: SvgImage = include_svg!("assets/scrollbar/down.svg");
const ARROW_LEFT: SvgImage = include_svg!("assets/scrollbar/left.svg");
const ARROW_RIGHT: SvgImage = include_svg!("assets/scrollbar/right.svg");

/// Side of the square box an arrow glyph is drawn into, centered on its half
/// of the control. Fixed at the glyphs' own 16-unit viewBox rather than fit
/// to the half, which is usually shorter than that — 12 px in a 24 px tall
/// [`NumberInput`](crate::widgets::NumberInput) — and would shrink the
/// triangle off the device pixels it lands on crisply at 1.0x.
const GLYPH_BOX: i32 = 16;

/// One of the two arrow buttons: the one that counts up — the top of a
/// vertical pair, the right of a horizontal one — or the one that counts down.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Half {
    Increase,
    Decrease,
}

impl Half {
    /// The step one press of this half takes.
    fn delta(self) -> i32 {
        match self {
            Half::Increase => 1,
            Half::Decrease => -1,
        }
    }
}

/// An arrow held down with the primary mouse button. The same model as the
/// scrollbar's: it paints pushed in and auto-repeats while the pointer stays
/// over it (armed), and pops back out, pausing the repeat, while the pointer
/// is off it.
#[derive(Clone, Copy)]
struct Held {
    half: Half,
    armed: bool,
    /// When the arrow was pressed — drives the initial repeat delay.
    pressed_at: Instant,
    /// When the last repeat fired — drives the steady repeat interval.
    last_repeat: Instant,
}

impl Held {
    fn new(half: Half) -> Self {
        let now = Instant::now();
        Self {
            half,
            armed: true,
            pressed_at: now,
            last_repeat: now,
        }
    }
}

/// A pair of arrow buttons that step something up or down — the Win 3.1 spin
/// button. Vertical, an up arrow sits over a down arrow; horizontal, a left
/// arrow sits beside a right one.
///
/// It holds no value of its own: it only reports steps, through
/// [`on_step`](Self::on_step), and leaves what a step means to its owner — a
/// number, a date, the next entry of a list. Pair it with whatever shows that
/// value. For a number typed in a field, [`NumberInput`](crate::widgets::NumberInput)
/// builds one into the field already.
///
/// Clicking an arrow steps once; holding it down repeats the step at the
/// scrollbar arrows' pace, for as long as the pointer stays on it. The mouse
/// wheel steps it too, once per notch. Like a scrollbar, it takes no keyboard
/// focus.
pub struct SpinButton {
    rect: Rect,
    orientation: Orientation,
    enabled: bool,
    held: Option<Held>,
    /// Wheel movement not yet worth a whole step, in steps. Trackpads deliver
    /// a notch's worth of lines in many small deltas, and those add up here.
    wheel_accum: f32,
    on_step: Option<StepHandler>,
}

impl SpinButton {
    pub fn new(rect: Rect, orientation: Orientation) -> Self {
        Self {
            rect,
            orientation,
            enabled: true,
            held: None,
            wheel_accum: 0.0,
            on_step: None,
        }
    }

    /// An up arrow over a down arrow.
    pub fn vertical(rect: Rect) -> Self {
        Self::new(rect, Orientation::Vertical)
    }

    /// A left arrow beside a right arrow.
    pub fn horizontal(rect: Rect) -> Self {
        Self::new(rect, Orientation::Horizontal)
    }

    /// Call `handler` for every step the buttons take, with the number of
    /// steps: positive for the up / right arrow, negative for the down / left
    /// one. A click and each auto-repeat are one step (`1` or `-1`); a quick
    /// turn of the wheel may deliver several at once.
    pub fn on_step<F>(mut self, handler: F) -> Self
    where
        F: FnMut(&mut EventCtx, i32) + 'static,
    {
        self.on_step = Some(Box::new(handler));
        self
    }

    /// Install (or replace) the step handler after construction, for callers
    /// that wire it up once the widgets it talks to exist.
    pub fn set_on_step<F>(&mut self, handler: F)
    where
        F: FnMut(&mut EventCtx, i32) + 'static,
    {
        self.on_step = Some(Box::new(handler));
    }

    pub fn orientation(&self) -> Orientation {
        self.orientation
    }

    pub fn with_enabled(mut self, enabled: bool) -> Self {
        self.set_enabled(enabled);
        self
    }

    pub fn is_enabled(&self) -> bool {
        self.enabled
    }

    /// Enable or disable the buttons. Disabled, they draw their arrows greyed
    /// and ignore the mouse.
    pub fn set_enabled(&mut self, enabled: bool) {
        self.enabled = enabled;
        if !enabled {
            self.held = None;
            self.wheel_accum = 0.0;
        }
    }

    /// The rect of one half. The two overlap by one logical pixel, so the line
    /// between them is one border the two share rather than two of them side
    /// by side. An even length leaves the down / right half a pixel longer.
    fn half_rect(&self, half: Half) -> Rect {
        let r = self.rect;
        match self.orientation {
            Orientation::Vertical => {
                let top = (r.h + 1) / 2;
                match half {
                    Half::Increase => Rect::new(r.x, r.y, r.w, top),
                    Half::Decrease => Rect::new(r.x, r.y + top - 1, r.w, r.h - top + 1),
                }
            }
            Orientation::Horizontal => {
                let left = (r.w + 1) / 2;
                match half {
                    Half::Decrease => Rect::new(r.x, r.y, left, r.h),
                    Half::Increase => Rect::new(r.x + left - 1, r.y, r.w - left + 1, r.h),
                }
            }
        }
    }

    /// The half under `pos`, if it is over the buttons at all.
    fn hit(&self, pos: Point) -> Option<Half> {
        if !self.rect.contains(pos) {
            return None;
        }
        Some(match self.orientation {
            Orientation::Vertical if pos.y < self.rect.y + self.rect.h / 2 => Half::Increase,
            Orientation::Vertical => Half::Decrease,
            Orientation::Horizontal if pos.x < self.rect.x + self.rect.w / 2 => Half::Decrease,
            Orientation::Horizontal => Half::Increase,
        })
    }

    fn is_pressed(&self, half: Half) -> bool {
        self.held.is_some_and(|h| h.half == half && h.armed)
    }

    /// Steps the wheel moved, banking anything short of a whole notch for the
    /// next delta. `delta_x` / `delta_y` are an [`Event::Scroll`]'s: a turn up
    /// or a swipe right counts up, whichever way the buttons are laid out.
    pub(crate) fn wheel_steps(&mut self, delta_x: f32, delta_y: f32) -> i32 {
        let notches = (delta_x - delta_y) / WHEEL_LINES_PER_DETENT;
        // Turning back drops what was banked the other way, so the first notch
        // back steps at once instead of first unwinding the bank.
        if notches * self.wheel_accum < 0.0 {
            self.wheel_accum = 0.0;
        }
        self.wheel_accum += notches;
        let whole = self.wheel_accum.trunc();
        self.wheel_accum -= whole;
        whole as i32
    }

    /// The step a held arrow repeats on this tick: none until the arrow has
    /// been held for [`REPEAT_DELAY`], then one every [`REPEAT_INTERVAL`]
    /// while it stays armed.
    fn tick_repeat(&mut self) -> i32 {
        let Some(held) = self.held.as_mut() else {
            return 0;
        };
        let now = Instant::now();
        if !held.armed
            || now.duration_since(held.pressed_at) < REPEAT_DELAY
            || now.duration_since(held.last_repeat) < REPEAT_INTERVAL
        {
            return 0;
        }
        held.last_repeat = now;
        held.half.delta()
    }

    /// Handle `event` and return the steps it took, without calling the step
    /// handler: [`Widget::event`] calls it with what this returns, and a
    /// `NumberInput` steps its value by it.
    pub(crate) fn handle(&mut self, event: &Event, ctx: &mut EventCtx) -> i32 {
        if !self.enabled {
            return 0;
        }
        let mut steps = 0;
        match event {
            Event::PointerDown {
                pos,
                button: MouseButton::Left,
                ..
            } => {
                if let Some(half) = self.hit(*pos) {
                    self.held = Some(Held::new(half));
                    steps = half.delta();
                    ctx.request_paint();
                }
            }
            // Sliding off the held arrow pops it back out and pauses the
            // repeat; sliding back on resumes both.
            Event::PointerMove { pos } => {
                let over = self.hit(*pos);
                if let Some(held) = self.held.as_mut() {
                    let armed = over == Some(held.half);
                    if held.armed != armed {
                        held.armed = armed;
                        ctx.request_paint();
                    }
                }
            }
            // A release ends the hold, and so does the pointer leaving the
            // window, after which the release may never arrive.
            Event::PointerUp {
                button: MouseButton::Left,
                ..
            }
            | Event::PointerLeave
                if self.held.is_some() =>
            {
                self.held = None;
                ctx.request_paint();
            }
            Event::Tick => steps = self.tick_repeat(),
            Event::Scroll {
                delta_x, delta_y, ..
            } => steps = self.wheel_steps(*delta_x, *delta_y),
            _ => {}
        }
        // Keep the ticks that drive the repeat coming while an arrow is held,
        // through any wrapper that doesn't forward `wants_ticks` (see
        // `EventCtx::request_tick`).
        if self.held.is_some() {
            ctx.request_tick();
        }
        steps
    }
}

impl Widget for SpinButton {
    fn bounds(&self) -> Rect {
        self.rect
    }

    fn paint(&mut self, painter: &mut Painter, theme: &Theme) {
        // The scrollbar's light button chrome for each half. The second one
        // drawn overlaps the first by a pixel, and marks that edge merged so
        // their borders land on the same device pixels at every scale.
        let (first, second, merged) = match self.orientation {
            Orientation::Vertical => (Half::Increase, Half::Decrease, Merged::TOP),
            Orientation::Horizontal => (Half::Decrease, Half::Increase, Merged::LEFT),
        };
        painter.light_button(self.half_rect(first), theme, self.is_pressed(first));
        painter.light_button_merged(
            self.half_rect(second),
            merged,
            theme,
            self.is_pressed(second),
        );

        let color = if self.enabled {
            theme.text
        } else {
            theme.disabled_text
        };
        for half in [Half::Increase, Half::Decrease] {
            let glyph = match (self.orientation, half) {
                (Orientation::Vertical, Half::Increase) => &ARROW_UP,
                (Orientation::Vertical, Half::Decrease) => &ARROW_DOWN,
                (Orientation::Horizontal, Half::Increase) => &ARROW_RIGHT,
                (Orientation::Horizontal, Half::Decrease) => &ARROW_LEFT,
            };
            let r = self.half_rect(half);
            // Only a half that is too small for the box shrinks the glyph.
            let side = GLYPH_BOX.min(2 * r.w.min(r.h)).max(0);
            // A pressed arrow carries its glyph a pixel down and right, the
            // way the scrollbar's does.
            let nudge = i32::from(self.is_pressed(half));
            let at = Rect::new(
                r.x + (r.w - side) / 2 + nudge,
                r.y + (r.h - side) / 2 + nudge,
                side,
                side,
            );
            glyph.draw_tinted(painter, at, color);
        }
    }

    fn event(&mut self, event: &Event, ctx: &mut EventCtx) {
        let steps = self.handle(event, ctx);
        if steps != 0
            && let Some(handler) = self.on_step.as_mut()
        {
            handler(ctx, steps);
        }
    }

    fn captures_pointer(&self) -> bool {
        self.held.is_some()
    }

    fn wants_ticks(&self) -> bool {
        self.held.is_some()
    }

    fn layout(&mut self, bounds: Rect) {
        self.rect = bounds;
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::rc::Rc;

    use super::*;
    use crate::event::Modifiers;

    fn center(r: Rect) -> Point {
        Point::new(r.x + r.w / 2, r.y + r.h / 2)
    }

    /// A spin button whose handler adds every step it reports to `total`.
    fn counting(spin: SpinButton) -> (SpinButton, Rc<RefCell<i32>>) {
        let total = Rc::new(RefCell::new(0));
        let sink = total.clone();
        let spin = spin.on_step(move |_cx, steps| *sink.borrow_mut() += steps);
        (spin, total)
    }

    fn send(spin: &mut SpinButton, event: Event) -> EventCtx {
        let mut ctx = EventCtx::new();
        spin.event(&event, &mut ctx);
        ctx
    }

    fn press(spin: &mut SpinButton, pos: Point) -> EventCtx {
        send(
            spin,
            Event::PointerDown {
                pos,
                button: MouseButton::Left,
                modifiers: Modifiers::default(),
            },
        )
    }

    fn release(spin: &mut SpinButton, pos: Point) -> EventCtx {
        send(
            spin,
            Event::PointerUp {
                pos,
                button: MouseButton::Left,
                modifiers: Modifiers::default(),
            },
        )
    }

    fn click(spin: &mut SpinButton, half: Half) {
        let at = center(spin.half_rect(half));
        press(spin, at);
        release(spin, at);
    }

    fn wheel(spin: &mut SpinButton, delta_x: f32, delta_y: f32) {
        let pos = center(spin.rect);
        send(
            spin,
            Event::Scroll {
                pos,
                delta_x,
                delta_y,
            },
        );
    }

    /// Backdate the held arrow so the next tick is past both the initial delay
    /// and the repeat interval.
    fn ripen_repeat(spin: &mut SpinButton) {
        let past = Instant::now()
            .checked_sub(REPEAT_DELAY + REPEAT_INTERVAL)
            .expect("clock is far enough past the epoch");
        let held = spin.held.as_mut().expect("an arrow is held");
        held.pressed_at = past;
        held.last_repeat = past;
    }

    #[test]
    fn the_halves_share_the_border_between_them() {
        let spin = SpinButton::vertical(Rect::new(0, 0, 16, 24));
        let (up, down) = (
            spin.half_rect(Half::Increase),
            spin.half_rect(Half::Decrease),
        );
        assert_eq!(up.bottom() - 1, down.y, "one row is both halves' border");
        assert_eq!(
            (up.y, down.bottom()),
            (0, 24),
            "together they fill the rect"
        );

        let spin = SpinButton::horizontal(Rect::new(0, 0, 33, 16));
        let (left, right) = (
            spin.half_rect(Half::Decrease),
            spin.half_rect(Half::Increase),
        );
        assert_eq!(left.right() - 1, right.x);
        assert_eq!(left.w, right.w, "an odd length splits evenly");
    }

    #[test]
    fn up_and_right_count_up_down_and_left_count_down() {
        let (mut spin, total) = counting(SpinButton::vertical(Rect::new(0, 0, 16, 24)));
        click(&mut spin, Half::Increase);
        click(&mut spin, Half::Increase);
        click(&mut spin, Half::Decrease);
        assert_eq!(*total.borrow(), 1);
        // The pixel row at the very top is the up arrow, the one at the very
        // bottom the down arrow.
        press(&mut spin, Point::new(8, 0));
        assert_eq!(*total.borrow(), 2);
        release(&mut spin, Point::new(8, 0));
        press(&mut spin, Point::new(8, 23));
        assert_eq!(*total.borrow(), 1);

        let (mut spin, total) = counting(SpinButton::horizontal(Rect::new(0, 0, 32, 16)));
        click(&mut spin, Half::Increase);
        assert_eq!(*total.borrow(), 1, "the right arrow counts up");
        press(&mut spin, Point::new(1, 8));
        assert_eq!(*total.borrow(), 0, "the left arrow counts down");
    }

    #[test]
    fn a_held_arrow_repeats_after_the_delay() {
        let (mut spin, total) = counting(SpinButton::vertical(Rect::new(0, 0, 16, 24)));
        let at = center(spin.half_rect(Half::Increase));
        let ctx = press(&mut spin, at);
        assert!(spin.captures_pointer(), "a held arrow grabs the pointer");
        assert!(ctx.tick_requested, "and asks for the ticks that repeat it");

        send(&mut spin, Event::Tick);
        assert_eq!(*total.borrow(), 1, "no repeat inside the initial delay");

        ripen_repeat(&mut spin);
        send(&mut spin, Event::Tick);
        assert_eq!(*total.borrow(), 2, "a ripe held arrow repeats its step");

        let ctx = release(&mut spin, at);
        assert!(!spin.captures_pointer(), "the release frees the pointer");
        assert!(!ctx.tick_requested, "and lets the ticks wind down");
    }

    #[test]
    fn sliding_off_the_held_arrow_pauses_the_repeat() {
        let (mut spin, total) = counting(SpinButton::vertical(Rect::new(0, 0, 16, 24)));
        let (up, down) = (
            spin.half_rect(Half::Increase),
            spin.half_rect(Half::Decrease),
        );
        press(&mut spin, center(up));
        // Onto the other arrow: still held, but disarmed.
        send(&mut spin, Event::PointerMove { pos: center(down) });
        ripen_repeat(&mut spin);
        send(&mut spin, Event::Tick);
        assert_eq!(*total.borrow(), 1, "a disarmed hold doesn't repeat");
        assert!(spin.captures_pointer(), "but lasts until the release");
    }

    #[test]
    fn the_pointer_leaving_the_window_ends_the_hold() {
        let mut spin = SpinButton::vertical(Rect::new(0, 0, 16, 24));
        let down = spin.half_rect(Half::Decrease);
        press(&mut spin, center(down));
        send(&mut spin, Event::PointerLeave);
        assert!(!spin.captures_pointer());
    }

    #[test]
    fn the_wheel_steps_once_a_notch() {
        let (mut spin, total) = counting(SpinButton::vertical(Rect::new(0, 0, 16, 24)));
        wheel(&mut spin, 0.0, -WHEEL_LINES_PER_DETENT);
        assert_eq!(*total.borrow(), 1, "a notch up counts up");
        wheel(&mut spin, 0.0, 2.0 * WHEEL_LINES_PER_DETENT);
        assert_eq!(*total.borrow(), -1, "two notches down count down twice");
        wheel(&mut spin, WHEEL_LINES_PER_DETENT, 0.0);
        assert_eq!(*total.borrow(), 0, "a swipe right counts up");
    }

    #[test]
    fn trackpad_deltas_add_up_to_a_step() {
        let (mut spin, total) = counting(SpinButton::horizontal(Rect::new(0, 0, 32, 16)));
        let quarter = -WHEEL_LINES_PER_DETENT / 4.0;
        for _ in 0..3 {
            wheel(&mut spin, 0.0, quarter);
        }
        assert_eq!(*total.borrow(), 0, "three quarters of a notch is no step");
        wheel(&mut spin, 0.0, quarter);
        assert_eq!(*total.borrow(), 1, "the fourth quarter completes one");

        // Turning back drops what was banked the other way.
        for _ in 0..3 {
            wheel(&mut spin, 0.0, quarter);
        }
        wheel(&mut spin, 0.0, WHEEL_LINES_PER_DETENT);
        assert_eq!(*total.borrow(), 0, "the first notch back steps at once");
    }

    #[test]
    fn a_disabled_spin_button_ignores_the_mouse() {
        let (spin, total) = counting(SpinButton::vertical(Rect::new(0, 0, 16, 24)));
        let mut spin = spin.with_enabled(false);
        click(&mut spin, Half::Increase);
        wheel(&mut spin, 0.0, -WHEEL_LINES_PER_DETENT);
        assert_eq!(*total.borrow(), 0);
        assert!(!spin.captures_pointer());
    }
}
