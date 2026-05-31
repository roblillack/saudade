//! counter — 7GUIs task 1.
//!
//! "Build a frame containing a label or read-only textfield T and a button B.
//! Initially the value in T is 0 and each click of B increases the value in T
//! by one."
//!
//! Here T is a `Label` sitting inside a sunken `Bevel` (so it reads as a
//! read-only field) and B is a default `Button`. The count lives in a shared
//! cell so the button's `on_click` can bump it and rewrite the label.

use std::cell::RefCell;
use std::rc::Rc;

use saudade::{App, Bevel, Button, Container, Label, Painter, Rect, Theme, Widget, WindowConfig};

const W: i32 = 220;
const H: i32 = 64;

fn main() {
    let count = Rc::new(RefCell::new(0i32));
    let display = Rc::new(RefCell::new(Label::new(Rect::new(28, 24, 64, 16), "0")));

    let button = Button::new(Rect::new(120, 20, 84, 24), "Count")
        .default(true)
        .on_click({
            let count = count.clone();
            let display = display.clone();
            move |cx| {
                *count.borrow_mut() += 1;
                display.borrow_mut().text = count.borrow().to_string();
                cx.request_paint();
            }
        });

    let root = Container::new(W, H)
        .add(Bevel::sunken(Rect::new(16, 18, 88, 28)))
        .add(SharedLabel(display.clone()))
        .add(button);

    App::new(WindowConfig::new("Counter", W, H), root)
        .with_theme(Theme::windows_31())
        .run();
}

/// Adapter so the button callback and the widget tree can share one `Label`.
/// A `Label` has no events or focus, so only the visual hooks need forwarding.
struct SharedLabel(Rc<RefCell<Label>>);

impl Widget for SharedLabel {
    fn bounds(&self) -> Rect {
        self.0.borrow().bounds()
    }
    fn paint(&mut self, painter: &mut Painter, theme: &Theme) {
        self.0.borrow_mut().paint(painter, theme);
    }
    fn layout(&mut self, bounds: Rect) {
        self.0.borrow_mut().layout(bounds);
    }
}
