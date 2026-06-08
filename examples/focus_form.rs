//! focus_form — keyboard mnemonics that move focus to a field.
//!
//! Each caption is a [`FocusLabel`](saudade::FocusLabel): the letter after `&`
//! is underlined and bound as an Alt accelerator. Pressing **Alt+N**,
//! **Alt+A**, or **Alt+E** jumps focus straight to the matching text field —
//! the classic "buddy label" convention — without the user ever reaching for
//! the mouse or tabbing through the form. A label hands focus to the *next
//! focusable widget* added after it, so each caption simply precedes its field.

use saudade::{App, Container, FocusLabel, Rect, TextInput, Theme, WindowConfig};

const W: i32 = 300;
const H: i32 = 148;

fn main() {
    let label = |y, text| FocusLabel::new(Rect::new(16, y, 72, 24), text);
    let field = |y| TextInput::new(Rect::new(96, y, 188, 24));

    // Each label focuses the field that follows it.
    let content = Container::new(W, H)
        .add(label(16, "Last &name:"))
        .add(field(16))
        .add(label(56, "&Address:"))
        .add(field(56))
        .add(label(96, "&Email:"))
        .add(field(96));

    App::new(WindowConfig::new("Focus form", W, H), content)
        .with_theme(Theme::windows_31())
        .run();
}
