//! temperature — 7GUIs task 2.
//!
//! "Two textfields TC and TF for Celsius and Fahrenheit. When the user enters
//! a numerical value into TC the corresponding value in TF is automatically
//! updated and vice versa. A non-numerical string leaves the other field
//! untouched."
//!
//! The two `TextInput`s live behind shared cells so each one's `on_change`
//! handler can write the converted value into the other. `TextInput::set_text`
//! deliberately does *not* fire `on_change`, so updating one field from the
//! other can't trigger an infinite ping-pong.

use std::cell::RefCell;
use std::rc::Rc;

use saudade::{
    App, Container, Event, EventCtx, Label, Painter, Rect, TextInput, Theme, Widget, WindowConfig,
};

const W: i32 = 320;
const H: i32 = 72;

fn main() {
    let celsius = Rc::new(RefCell::new(TextInput::new(Rect::new(16, 24, 70, 24))));
    let fahrenheit = Rc::new(RefCell::new(TextInput::new(Rect::new(170, 24, 70, 24))));

    celsius.borrow_mut().set_on_change({
        let fahrenheit = fahrenheit.clone();
        move |_cx, text| {
            if let Ok(c) = text.trim().parse::<f64>() {
                let f = c * 9.0 / 5.0 + 32.0;
                fahrenheit.borrow_mut().set_text(&fmt_temp(f));
            }
        }
    });

    fahrenheit.borrow_mut().set_on_change({
        let celsius = celsius.clone();
        move |_cx, text| {
            if let Ok(f) = text.trim().parse::<f64>() {
                let c = (f - 32.0) * 5.0 / 9.0;
                celsius.borrow_mut().set_text(&fmt_temp(c));
            }
        }
    });

    let root = Container::new(W, H)
        .add(SharedTextInput(celsius.clone()))
        .add(Label::new(Rect::new(94, 29, 72, 16), "Celsius ="))
        .add(SharedTextInput(fahrenheit.clone()))
        .add(Label::new(Rect::new(248, 29, 64, 16), "Fahrenheit"));

    App::new(WindowConfig::new("TempConv", W, H), root)
        .with_theme(Theme::windows_31())
        .run();
}

/// Format a converted temperature: round to one decimal, then drop a trailing
/// ".0" so whole degrees show as integers ("32" rather than "32.0").
fn fmt_temp(v: f64) -> String {
    let rounded = (v * 10.0).round() / 10.0;
    if rounded == rounded.trunc() {
        format!("{}", rounded as i64)
    } else {
        format!("{rounded}")
    }
}

/// Adapter so a `TextInput` can be shared between the widget tree and the
/// sibling field's `on_change` closure. Mirrors the one in the `picker`
/// example; `wants_ticks` is forwarded so the caret keeps blinking.
struct SharedTextInput(Rc<RefCell<TextInput>>);

impl Widget for SharedTextInput {
    fn bounds(&self) -> Rect {
        self.0.borrow().bounds()
    }
    fn paint(&mut self, painter: &mut Painter, theme: &Theme) {
        self.0.borrow_mut().paint(painter, theme);
    }
    fn event(&mut self, event: &Event, ctx: &mut EventCtx) {
        self.0.borrow_mut().event(event, ctx);
    }
    fn captures_pointer(&self) -> bool {
        self.0.borrow().captures_pointer()
    }
    fn focusable(&self) -> bool {
        self.0.borrow().focusable()
    }
    fn set_focused(&mut self, focused: bool) {
        self.0.borrow_mut().set_focused(focused);
    }
    fn layout(&mut self, bounds: Rect) {
        self.0.borrow_mut().layout(bounds);
    }
    fn wants_ticks(&self) -> bool {
        self.0.borrow().wants_ticks()
    }
}
