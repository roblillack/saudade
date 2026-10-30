use crate::event::{Event, EventCtx, Key, MouseButton, NamedKey};
use crate::geometry::Rect;
use crate::painter::Painter;
use crate::theme::Theme;
use crate::widget::Widget;
use crate::widgets::scrollbar::SCROLLBAR_THICKNESS;
use crate::widgets::spin_button::SpinButton;
use crate::widgets::text_input::TextInput;

type ChangeHandler = Box<dyn FnMut(&mut EventCtx, f64)>;

/// Width of the spin buttons along the field's right edge: a scrollbar's
/// breadth, which is what the arrows on them are drawn for.
const SPIN_W: i32 = SCROLLBAR_THICKNESS;
/// The most decimals a [`NumberInput`] shows: as many as an `f64` keeps.
const MAX_PRECISION: usize = 15;
/// Steps Page Up / Page Down take at once.
const PAGE_STEPS: i32 = 10;
/// Logical pixels from the field's border to where its text starts, which
/// the unit keeps from the arrows and the text from the unit.
const UNIT_PAD: i32 = 4;

/// A text field for a number, with spin buttons to step it: the sunken field
/// of a [`TextInput`] with an up / down [`SpinButton`] pair along its right
/// edge.
///
/// The number lies in an inclusive [range](Self::set_range), is shown with a
/// fixed number of decimals, its [precision](Self::set_precision), and the
/// arrows move it from one multiple of its [step](Self::set_step) to the next.
/// An optional [unit](Self::set_unit) — `"mm"`, `"%"` — shows in grey at the
/// right end of the field.
///
/// The field only takes text that reads as such a number, or as the start of
/// one: digits, one `-` in front if the range reaches below zero, and a
/// decimal point followed by no more digits than the precision allows. A
/// keystroke or a paste that would leave anything else is dropped. A comma
/// typed stands in for the decimal point.
///
/// Taking the focus selects the whole number, however the focus comes — Tab, a
/// mnemonic, or a click in the text — so typing replaces it straight away; a
/// click in a field that already has the focus places the caret. Without the
/// focus the field shows neither a caret nor a selection.
///
/// The value follows the text as it is typed: whenever the text reads as a
/// number, the value is that number, clamped to the range, and
/// [`on_change`](Self::on_change) fires. Enter, or leaving the field, rewrites
/// the text to show the value at its precision — `"250"` becomes `"100"` in a
/// field that goes up to 100, and a field left empty shows its value again.
pub struct NumberInput {
    rect: Rect,
    field: TextInput,
    spin: SpinButton,
    value: f64,
    min: f64,
    max: f64,
    step: f64,
    precision: usize,
    /// Shown after the number, inside the field; empty for none.
    unit: String,
    on_change: Option<ChangeHandler>,
}

impl NumberInput {
    /// A field for any number, stepped by 1 and shown without decimals.
    /// Starts at 0.
    pub fn new(rect: Rect) -> Self {
        Self {
            rect,
            field: TextInput::new(field_rect(rect))
                .with_text("0")
                .without_unfocused_marks(),
            spin: SpinButton::vertical(spin_rect(rect)),
            value: 0.0,
            min: f64::MIN,
            max: f64::MAX,
            step: 1.0,
            precision: 0,
            unit: String::new(),
            on_change: None,
        }
    }

    pub fn with_unit(mut self, unit: impl Into<String>) -> Self {
        self.set_unit(unit);
        self
    }

    pub fn unit(&self) -> &str {
        &self.unit
    }

    /// Show `unit` — `"mm"`, `"%"`, `"pt"` — in grey at the right end of the
    /// field, next to the arrows; the number keeps clear of it. Empty (the
    /// default) shows none. It is only shown, not part of the text: it can't
    /// be typed over or selected, and the value doesn't include it.
    pub fn set_unit(&mut self, unit: impl Into<String>) {
        self.unit = unit.into();
    }

    pub fn with_value(mut self, value: f64) -> Self {
        self.set_value(value);
        self
    }

    pub fn with_range(mut self, min: f64, max: f64) -> Self {
        self.set_range(min, max);
        self
    }

    pub fn with_step(mut self, step: f64) -> Self {
        self.set_step(step);
        self
    }

    pub fn with_precision(mut self, decimals: usize) -> Self {
        self.set_precision(decimals);
        self
    }

    pub fn with_enabled(mut self, enabled: bool) -> Self {
        self.set_enabled(enabled);
        self
    }

    /// Call `handler` with the new value whenever the user changes it — by
    /// typing, with the arrows or keys, or with the wheel. Never for
    /// [`set_value`](Self::set_value) or the other setters, so two inputs
    /// that update each other don't loop.
    pub fn on_change<F>(mut self, handler: F) -> Self
    where
        F: FnMut(&mut EventCtx, f64) + 'static,
    {
        self.on_change = Some(Box::new(handler));
        self
    }

    /// Install (or replace) the change handler after construction, for callers
    /// that wire it up once the widgets it talks to exist.
    pub fn set_on_change<F>(&mut self, handler: F)
    where
        F: FnMut(&mut EventCtx, f64) + 'static,
    {
        self.on_change = Some(Box::new(handler));
    }

    pub fn value(&self) -> f64 {
        self.value
    }

    /// Set the value, rounded to the precision and clamped to the range, and
    /// show it. A NaN is ignored.
    pub fn set_value(&mut self, value: f64) {
        if value.is_nan() {
            return;
        }
        self.value = self.fit(value);
        self.show_value();
    }

    pub fn min(&self) -> f64 {
        self.min
    }

    pub fn max(&self) -> f64 {
        self.max
    }

    /// Limit the value to `min..=max` (either order), clamping the current
    /// one into it. Unlimited until set. A range with a NaN end is ignored.
    ///
    /// The ends are best given at the precision: the value is clamped after
    /// it is rounded, so it never leaves the range, but an end with more
    /// decimals than the field shows would show rounded.
    pub fn set_range(&mut self, min: f64, max: f64) {
        if min.is_nan() || max.is_nan() {
            return;
        }
        (self.min, self.max) = if min <= max { (min, max) } else { (max, min) };
        self.set_value(self.value);
    }

    pub fn step(&self) -> f64 {
        self.step
    }

    /// What the arrows, Up / Down and the wheel step the value through: they
    /// move it to the next multiple of the step, up or down — with a step of
    /// 0.5, from 1.3 up to 1.5 or down to 1.0, and from there on by 0.5. Page
    /// Up / Page Down go ten multiples. 1 until set; anything but a positive,
    /// finite step is ignored. Keep it a multiple of the smallest decimal the
    /// precision shows, or steps will be rounded to it.
    pub fn set_step(&mut self, step: f64) {
        if step > 0.0 && step.is_finite() {
            self.step = step;
        }
    }

    pub fn precision(&self) -> usize {
        self.precision
    }

    /// Show the value with `decimals` digits after the decimal point, and
    /// round it to as many; 0 (the default) takes whole numbers only. At most
    /// 15, the decimal digits an `f64` holds.
    ///
    /// Set it before the value: a value is rounded to the precision it is set
    /// at, so `.with_value(2.5).with_precision(1)` shows `3.0`.
    pub fn set_precision(&mut self, decimals: usize) {
        self.precision = decimals.min(MAX_PRECISION);
        self.set_value(self.value);
    }

    pub fn is_focused(&self) -> bool {
        self.field.is_focused()
    }

    pub fn is_enabled(&self) -> bool {
        self.field.is_enabled()
    }

    /// Enable or disable the field and its arrows. Disabled, the field greys
    /// out like a disabled [`TextInput`], the arrows grey too, and neither
    /// takes focus or input.
    pub fn set_enabled(&mut self, enabled: bool) {
        self.field.set_enabled(enabled);
        self.spin.set_enabled(enabled);
    }

    /// Select the whole text, so whatever is typed next replaces it.
    pub fn select_all(&mut self) {
        self.field.select_all();
    }

    /// `value` rounded to the precision and clamped to the range.
    fn fit(&self, value: f64) -> f64 {
        let scale = 10f64.powi(self.precision as i32);
        let scaled = (value * scale).round();
        // A value too large to scale has no decimals left to round anyway.
        let rounded = if scaled.is_finite() {
            scaled / scale
        } else {
            value
        };
        let fitted = rounded.clamp(self.min, self.max);
        // Rounding a small negative number leaves a negative zero, which would
        // show as "-0".
        if fitted == 0.0 { 0.0 } else { fitted }
    }

    fn format(&self, value: f64) -> String {
        format!("{value:.*}", self.precision)
    }

    /// Rewrite the text to show the value — unless it already does, which
    /// leaves the caret and selection where they are.
    fn show_value(&mut self) {
        let text = self.format(self.value);
        if self.field.text() != text {
            self.field.set_text(&text);
        }
    }

    /// `true` if `text` reads as a number this field takes, or as the start of
    /// one: an optional `-` when the range reaches below zero, digits, and a
    /// decimal point followed by at most `precision` more.
    fn takes(&self, text: &str) -> bool {
        let unsigned = match text.strip_prefix('-') {
            Some(rest) if self.min < 0.0 => rest,
            Some(_) => return false,
            None => text,
        };
        let (whole, decimals) = match unsigned.split_once('.') {
            Some(_) if self.precision == 0 => return false,
            Some((whole, decimals)) => (whole, decimals),
            None => (unsigned, ""),
        };
        whole.bytes().all(|b| b.is_ascii_digit())
            && decimals.bytes().all(|b| b.is_ascii_digit())
            && decimals.len() <= self.precision
    }

    /// Take `value` (already fit) as the new value, telling the change
    /// handler if it is one.
    fn change_to(&mut self, value: f64, ctx: &mut EventCtx) {
        if value == self.value {
            return;
        }
        self.value = value;
        if let Some(handler) = self.on_change.as_mut() {
            handler(ctx, value);
        }
    }

    /// Move the value `steps` multiples of the step on and show it, replacing
    /// whatever was being typed. A value between two multiples counts the one
    /// it moves towards as the first.
    fn step_by(&mut self, steps: i32, ctx: &mut EventCtx) {
        if steps == 0 {
            return;
        }
        // The value in steps from zero. One a hair off a multiple is on it:
        // 0.3 / 0.1 is 2.9999999999999996, and stepping up from there has to
        // land on 0.4, not on 0.3 again.
        let mut at = self.value / self.step;
        let nearest = at.round();
        if (at - nearest).abs() <= 1e-9 * nearest.abs().max(1.0) {
            at = nearest;
        }
        let from = if steps > 0 { at.floor() } else { at.ceil() };
        let target = |n: f64| self.fit((from + n) * self.step);
        let mut value = target(f64::from(steps));
        // A step finer than the precision can round straight back onto the
        // value it left; go one further then, so the arrows never stall.
        if value == self.value {
            value = target(f64::from(steps + steps.signum()));
        }
        self.change_to(value, ctx);
        self.show_value();
        ctx.request_paint();
    }

    /// Hand an editing event on to the field, and keep what it did to the
    /// text only if the text is still one the field [takes](Self::takes).
    /// Otherwise put the field back the way it was.
    fn edit(&mut self, event: &Event, ctx: &mut EventCtx) {
        let before = self.field.edit_state();
        let old_text = self.field.text();
        self.field.event(event, ctx);
        let text = self.field.text();
        if text == old_text {
            return;
        }
        if !self.takes(&text) {
            self.field.restore_edit_state(before);
            return;
        }
        // "", "-" and "." don't read as a number yet, and leave the value be.
        if let Ok(value) = text.parse::<f64>() {
            let value = self.fit(value);
            self.change_to(value, ctx);
        }
    }

    fn place_parts(&mut self) {
        self.field.layout(field_rect(self.rect));
        self.spin.layout(spin_rect(self.rect));
    }
}

/// The text field's rect: all of `rect` but the spin buttons on its right,
/// plus the two pixels of the field's own right border and bevel, which the
/// buttons cover — so the text area inside them reaches up to the buttons'
/// border.
fn field_rect(rect: Rect) -> Rect {
    Rect::new(rect.x, rect.y, (rect.w - SPIN_W + 2).max(0), rect.h)
}

/// The spin buttons' rect, flush with the field's right edge: their top,
/// right and bottom borders are the field's own.
fn spin_rect(rect: Rect) -> Rect {
    Rect::new(rect.right() - SPIN_W, rect.y, SPIN_W, rect.h)
}

impl Widget for NumberInput {
    fn bounds(&self) -> Rect {
        self.rect
    }

    fn paint(&mut self, painter: &mut Painter, theme: &Theme) {
        let size = theme.font_size;
        let unit_w = if self.unit.is_empty() {
            0
        } else {
            painter.measure_text(&self.unit, size).w
        };
        // The text keeps clear of the unit by the room it keeps from the
        // field's left border.
        let reserved = if unit_w > 0 { unit_w + UNIT_PAD } else { 0 };
        self.field.set_reserved_right(reserved);
        self.field.paint(painter, theme);

        let spin = spin_rect(self.rect);
        if unit_w > 0 {
            // As far in from the arrows as the text is from the left border,
            // on the text's own line.
            let inner = Rect::new(
                self.rect.x + 2,
                self.rect.y + 2,
                (spin.x - self.rect.x - 2).max(0),
                (self.rect.h - 4).max(0),
            );
            let h = painter.measure_text("Ag", size).h;
            let y = self.rect.y + ((self.rect.h - h) / 2).max(0);
            let saved = painter.push_clip(inner);
            painter.text(
                spin.x - (UNIT_PAD - 1) - unit_w,
                y,
                &self.unit,
                size,
                theme.disabled_text,
            );
            painter.restore_clip(saved);
        }

        // The buttons go over the right end of the field, the way a
        // dropdown's arrow button sits in its field.
        self.spin.paint(painter, theme);
    }

    fn event(&mut self, event: &Event, ctx: &mut EventCtx) {
        if !self.field.is_enabled() {
            return;
        }
        let spin = spin_rect(self.rect);
        match event {
            // Many layouts' numeric keypads type a comma for the decimal key.
            Event::Char { ch: ',', modifiers } => {
                let point = Event::Char {
                    ch: '.',
                    modifiers: *modifiers,
                };
                self.edit(&point, ctx);
            }
            Event::Char { .. } => self.edit(event, ctx),
            Event::KeyDown { key, modifiers } => {
                let steps = match key {
                    Key::Named(NamedKey::Up) => 1,
                    Key::Named(NamedKey::Down) => -1,
                    Key::Named(NamedKey::PageUp) => PAGE_STEPS,
                    Key::Named(NamedKey::PageDown) => -PAGE_STEPS,
                    _ => 0,
                };
                if steps != 0 && self.field.is_focused() && !modifiers.has_command() {
                    self.step_by(steps, ctx);
                    ctx.consume_event();
                    return;
                }
                // Enter shows what was typed as the value it became. It isn't
                // consumed, the field never takes it, and a default button in
                // the same container gets it first anyway.
                if *key == Key::Named(NamedKey::Enter) && self.field.is_focused() {
                    self.show_value();
                    ctx.request_paint();
                }
                self.edit(event, ctx);
            }
            Event::PointerDown {
                pos,
                button: MouseButton::Left,
                ..
            } if spin.contains(*pos) && !self.field.captures_pointer() => {
                ctx.request_focus();
                let steps = self.spin.handle(event, ctx);
                self.step_by(steps, ctx);
            }
            // The wheel steps the value over the arrows, and anywhere over the
            // field while it has focus — not as it merely passes over.
            Event::Scroll {
                pos,
                delta_x,
                delta_y,
            } => {
                if spin.contains(*pos) || self.field.is_focused() {
                    let steps = self.spin.wheel_steps(*delta_x, *delta_y);
                    self.step_by(steps, ctx);
                }
            }
            // Ticks repeat a held arrow and blink the caret.
            Event::Tick => {
                let steps = self.spin.handle(event, ctx);
                self.step_by(steps, ctx);
                self.field.event(event, ctx);
            }
            _ if self.spin.captures_pointer() => {
                let steps = self.spin.handle(event, ctx);
                self.step_by(steps, ctx);
            }
            Event::PointerMove { pos } if spin.contains(*pos) && !self.field.captures_pointer() => {
                // Over the arrows the pointer keeps its arrow shape rather
                // than the field's I-beam — unless it is selecting text.
            }
            _ => self.field.event(event, ctx),
        }
    }

    fn captures_pointer(&self) -> bool {
        self.field.captures_pointer() || self.spin.captures_pointer()
    }

    fn focusable(&self) -> bool {
        self.field.focusable()
    }

    fn set_focused(&mut self, focused: bool) {
        let gained = focused && !self.field.is_focused();
        self.field.set_focused(focused);
        if gained {
            // Select it all, so typing replaces it. A click in the text that
            // brought the focus has placed the caret and started a selection
            // drag by now; ending the drag keeps the click's release, and any
            // jitter before it, from shrinking the selection again.
            self.field.select_all();
            self.field.end_drag();
        } else if !focused {
            self.show_value();
        }
    }

    fn wants_ticks(&self) -> bool {
        self.field.wants_ticks() || self.spin.wants_ticks()
    }

    fn layout(&mut self, bounds: Rect) {
        self.rect = bounds;
        self.place_parts();
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::rc::Rc;

    use super::*;
    use crate::event::{Modifiers, WHEEL_LINES_PER_DETENT};
    use crate::geometry::Point;

    const RECT: Rect = Rect::new(0, 0, 80, 24);

    /// `input`, focused, with its text selected so typing replaces it, and
    /// the values its change handler is called with.
    fn editing(input: NumberInput) -> (NumberInput, Rc<RefCell<Vec<f64>>>) {
        let changes = Rc::new(RefCell::new(Vec::new()));
        let sink = changes.clone();
        let mut input = input.on_change(move |_cx, value| sink.borrow_mut().push(value));
        input.set_focused(true);
        input.select_all();
        (input, changes)
    }

    fn send(input: &mut NumberInput, event: Event) -> EventCtx {
        let mut ctx = EventCtx::new();
        input.event(&event, &mut ctx);
        ctx
    }

    fn type_text(input: &mut NumberInput, text: &str) {
        for ch in text.chars() {
            send(
                input,
                Event::Char {
                    ch,
                    modifiers: Modifiers::default(),
                },
            );
        }
    }

    fn key(input: &mut NumberInput, key: NamedKey) -> EventCtx {
        send(
            input,
            Event::KeyDown {
                key: Key::Named(key),
                modifiers: Modifiers::default(),
            },
        )
    }

    fn text(input: &NumberInput) -> String {
        input.field.text()
    }

    fn up_arrow() -> Point {
        let spin = spin_rect(RECT);
        Point::new(spin.x + spin.w / 2, spin.y + 4)
    }

    fn wheel_up(input: &mut NumberInput, pos: Point) {
        send(
            input,
            Event::Scroll {
                pos,
                delta_x: 0.0,
                delta_y: -WHEEL_LINES_PER_DETENT,
            },
        );
    }

    fn pointer(input: &mut NumberInput, event: fn(Point) -> Event, x: i32) {
        send(input, event(Point::new(x, 12)));
    }

    fn down(pos: Point) -> Event {
        Event::PointerDown {
            pos,
            button: MouseButton::Left,
            modifiers: Modifiers::default(),
        }
    }

    fn up(pos: Point) -> Event {
        Event::PointerUp {
            pos,
            button: MouseButton::Left,
            modifiers: Modifiers::default(),
        }
    }

    fn moved(pos: Point) -> Event {
        Event::PointerMove { pos }
    }

    #[test]
    fn taking_the_focus_selects_the_number() {
        let mut input = NumberInput::new(RECT).with_value(42.0);
        input.set_focused(true);
        type_text(&mut input, "7");
        assert_eq!(text(&input), "7");
    }

    #[test]
    fn a_click_that_brings_the_focus_selects_the_number() {
        let mut input = NumberInput::new(RECT).with_value(42.0);
        // The press lands first; the container hands over the focus after it.
        pointer(&mut input, down, 10);
        input.set_focused(true);
        pointer(&mut input, moved, 30);
        pointer(&mut input, up, 30);
        type_text(&mut input, "5");
        assert_eq!(text(&input), "5");
    }

    #[test]
    fn a_click_in_a_focused_field_places_the_caret() {
        let mut input = NumberInput::new(RECT).with_value(42.0);
        input.set_focused(true);
        // Nothing has been painted, so every x maps to the start of the text.
        pointer(&mut input, down, 10);
        pointer(&mut input, up, 10);
        type_text(&mut input, "1");
        assert_eq!(text(&input), "142");
    }

    #[test]
    fn typing_keeps_only_what_reads_as_a_number() {
        let (mut input, _) = editing(NumberInput::new(RECT).with_range(0.0, 1000.0));
        type_text(&mut input, "4x2 e+");
        assert_eq!(text(&input), "42");
        assert_eq!(input.value(), 42.0);
    }

    #[test]
    fn the_value_follows_the_text_as_it_is_typed() {
        let (mut input, changes) = editing(NumberInput::new(RECT).with_range(0.0, 1000.0));
        type_text(&mut input, "125");
        key(&mut input, NamedKey::Backspace);
        assert_eq!(*changes.borrow(), [1.0, 12.0, 125.0, 12.0]);
    }

    #[test]
    fn decimals_stop_at_the_precision() {
        let (mut input, _) = editing(NumberInput::new(RECT).with_precision(2));
        type_text(&mut input, "1.23456");
        assert_eq!(text(&input), "1.23");
        assert_eq!(input.value(), 1.23);

        let (mut input, _) = editing(NumberInput::new(RECT));
        type_text(&mut input, "3.5");
        assert_eq!(text(&input), "35", "no decimal point without decimals");
    }

    #[test]
    fn a_minus_sign_needs_a_range_below_zero() {
        let (mut input, _) = editing(NumberInput::new(RECT).with_range(0.0, 10.0));
        type_text(&mut input, "-5");
        assert_eq!(text(&input), "5");

        let (mut input, _) = editing(NumberInput::new(RECT).with_range(-10.0, 10.0));
        type_text(&mut input, "5-");
        assert_eq!(text(&input), "5", "only in front");
        input.select_all();
        type_text(&mut input, "-5");
        assert_eq!(input.value(), -5.0);
    }

    #[test]
    fn a_comma_types_the_decimal_point() {
        let (mut input, _) = editing(NumberInput::new(RECT).with_precision(1));
        type_text(&mut input, "2,5");
        assert_eq!(text(&input), "2.5");
        assert_eq!(input.value(), 2.5);
    }

    #[test]
    fn a_value_typed_outside_the_range_is_clamped() {
        let (mut input, changes) = editing(NumberInput::new(RECT).with_range(10.0, 100.0));
        type_text(&mut input, "250");
        // "2" clamps to the 10 the field already holds, which is no change.
        assert_eq!(*changes.borrow(), [25.0, 100.0]);
        assert_eq!(text(&input), "250", "the text stays as typed while editing");
        input.set_focused(false);
        assert_eq!(text(&input), "100", "and shows the value once left");
    }

    #[test]
    fn enter_shows_the_value() {
        let (mut input, _) = editing(NumberInput::new(RECT).with_range(0.0, 100.0));
        type_text(&mut input, "250");
        key(&mut input, NamedKey::Enter);
        assert_eq!(text(&input), "100");
    }

    #[test]
    fn leaving_unfinished_text_shows_the_value_again() {
        let (mut input, changes) = editing(
            NumberInput::new(RECT)
                .with_range(-10.0, 10.0)
                .with_value(7.0),
        );
        type_text(&mut input, "-");
        assert_eq!(input.value(), 7.0, "a lone minus is no number yet");
        input.set_focused(false);
        assert_eq!(text(&input), "7");
        assert!(changes.borrow().is_empty());
    }

    #[test]
    fn a_negative_zero_shows_as_zero() {
        let (mut input, _) = editing(
            NumberInput::new(RECT)
                .with_range(-1.0, 1.0)
                .with_precision(1)
                .with_value(0.5),
        );
        type_text(&mut input, "-0");
        input.set_focused(false);
        assert_eq!(text(&input), "0.0");
    }

    #[test]
    fn keys_step_the_value_within_the_range() {
        let (mut input, changes) = editing(
            NumberInput::new(RECT)
                .with_range(0.0, 10.0)
                .with_step(2.0)
                .with_value(9.0),
        );
        let ctx = key(&mut input, NamedKey::Up);
        assert!(ctx.is_consumed());
        assert_eq!(text(&input), "10", "clamped at the top");
        key(&mut input, NamedKey::Up);
        key(&mut input, NamedKey::Down);
        assert_eq!(input.value(), 8.0);
        key(&mut input, NamedKey::PageDown);
        assert_eq!(input.value(), 0.0, "ten steps, clamped at the bottom");
        key(&mut input, NamedKey::PageUp);
        assert_eq!(input.value(), 10.0);
        assert_eq!(
            *changes.borrow(),
            [10.0, 8.0, 0.0, 10.0],
            "no change, no call"
        );
    }

    #[test]
    fn a_step_replaces_what_was_being_typed() {
        let (mut input, _) = editing(NumberInput::new(RECT).with_range(0.0, 100.0));
        type_text(&mut input, "250");
        key(&mut input, NamedKey::Down);
        assert_eq!(text(&input), "99");
    }

    #[test]
    fn steps_are_rounded_to_the_precision() {
        let (mut input, _) = editing(NumberInput::new(RECT).with_step(0.1).with_precision(1));
        for _ in 0..3 {
            key(&mut input, NamedKey::Up);
        }
        assert_eq!(text(&input), "0.3");
        assert_eq!(input.value(), 0.3, "not 0.30000000000000004");
        key(&mut input, NamedKey::Up);
        assert_eq!(input.value(), 0.4, "0.3 counts as on a multiple of 0.1");
    }

    #[test]
    fn steps_snap_to_the_next_multiple_of_the_step() {
        let (mut input, changes) = editing(
            NumberInput::new(RECT)
                .with_step(0.5)
                .with_precision(1)
                .with_value(1.3),
        );
        key(&mut input, NamedKey::Up);
        key(&mut input, NamedKey::Up);
        input.set_value(1.3);
        key(&mut input, NamedKey::Down);
        input.set_value(1.3);
        key(&mut input, NamedKey::PageUp);
        assert_eq!(*changes.borrow(), [1.5, 2.0, 1.0, 6.0]);
    }

    #[test]
    fn a_step_finer_than_the_precision_still_moves() {
        // 0.25 shows as 0.3, and the step down from there to 0.25 rounds back
        // up to 0.3: it has to go on to 0.0.
        let (mut input, _) = editing(
            NumberInput::new(RECT)
                .with_step(0.25)
                .with_precision(1)
                .with_value(0.5),
        );
        key(&mut input, NamedKey::Down);
        assert_eq!(text(&input), "0.3");
        key(&mut input, NamedKey::Down);
        assert_eq!(text(&input), "0.0");
    }

    #[test]
    fn an_arrow_click_steps_and_takes_the_focus() {
        let mut input = NumberInput::new(RECT).with_value(5.0);
        let ctx = send(
            &mut input,
            Event::PointerDown {
                pos: up_arrow(),
                button: MouseButton::Left,
                modifiers: Modifiers::default(),
            },
        );
        assert_eq!(input.value(), 6.0);
        assert!(ctx.is_focus_requested());
        assert!(input.captures_pointer(), "the held arrow keeps the pointer");
        send(
            &mut input,
            Event::PointerUp {
                pos: up_arrow(),
                button: MouseButton::Left,
                modifiers: Modifiers::default(),
            },
        );
        assert!(!input.captures_pointer());
    }

    #[test]
    fn the_wheel_steps_over_the_arrows_or_a_focused_field() {
        let mut input = NumberInput::new(RECT);
        let over_text = Point::new(10, 12);
        wheel_up(&mut input, over_text);
        assert_eq!(input.value(), 0.0, "not while it only passes over");
        wheel_up(&mut input, up_arrow());
        assert_eq!(input.value(), 1.0);
        input.set_focused(true);
        wheel_up(&mut input, over_text);
        assert_eq!(input.value(), 2.0);
    }

    #[test]
    fn setters_fit_and_show_the_value_without_calling_the_handler() {
        let (mut input, changes) = editing(NumberInput::new(RECT).with_precision(2));
        input.set_value(1.23456);
        assert_eq!(text(&input), "1.23");
        input.set_range(0.0, 1.0);
        assert_eq!(text(&input), "1.00");
        input.set_precision(0);
        assert_eq!(text(&input), "1");
        assert!(changes.borrow().is_empty());
    }

    #[test]
    fn a_disabled_input_takes_nothing() {
        let (input, changes) = editing(NumberInput::new(RECT));
        let mut input = input.with_enabled(false);
        assert!(!input.focusable());
        type_text(&mut input, "9");
        key(&mut input, NamedKey::Up);
        wheel_up(&mut input, up_arrow());
        assert_eq!(input.value(), 0.0);
        assert!(changes.borrow().is_empty());
    }
}
