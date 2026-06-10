//! chrome — generate window screenshots with title bars and frames.
//!
//! Saudade never draws its own window chrome on a real desktop (that is the
//! window manager's job), but for documentation and store screenshots it is
//! handy to capture a window the way a user sees it: on the desktop, inside a
//! title bar and frame, with a drop shadow. [`MockBackend::render_framed`]
//! does exactly that, reproducing the default rendering style of Canoe.
//!
//! Unlike the other examples this one opens no window — it renders offscreen
//! and writes PNG files. Run it with:
//!
//! ```sh
//! cargo run --example chrome
//! ```
//!
//! and it drops `chrome-resizable.png`, `chrome-fixed.png`, and
//! `chrome-dialog.png` in the current directory, one per frame style.

use saudade::mock::MockBackend;
use saudade::{Bevel, Button, Color, Container, Font, Label, Rect, Widget, WindowChrome};

const W: i32 = 260;
const H: i32 = 120;

fn main() {
    // A small "about box"-style client area to sit inside the chrome.
    let build = || -> Box<dyn Widget> {
        Box::new(
            Container::new(W, H)
                .with_background(Color::LIGHT_GRAY)
                .add(Bevel::sunken(Rect::new(16, 16, W - 32, 48)))
                .add(Label::new(
                    Rect::new(28, 26, W - 56, 16),
                    "Saudade Utilities",
                ))
                .add(Label::new(
                    Rect::new(28, 42, W - 56, 16),
                    "Version 1.0 — © 1992",
                ))
                .add(Button::new(Rect::new((W - 80) / 2, 80, 80, 26), "OK").default(true)),
        )
    };

    // Render each frame style at 2× so the screenshots are crisp, and write a
    // PNG per style. The window is always captured in its active (focused)
    // state. A real font gives the title bar and labels text; without one the
    // chrome still draws, just untitled.
    let shots = [
        (
            "chrome-resizable.png",
            WindowChrome::resizable("About Saudade"),
        ),
        ("chrome-fixed.png", WindowChrome::fixed("About Saudade")),
        ("chrome-dialog.png", WindowChrome::dialog("About Saudade")),
    ];

    for (file, chrome) in shots {
        let mut backend = MockBackend::new(W, H).with_scale(2.0);
        if let Some(font) = Font::load_sans() {
            backend = backend.with_sans_font(font);
        }
        let snap = backend.render_framed(build().as_mut(), &chrome);
        std::fs::write(file, snap.to_png()).unwrap_or_else(|e| panic!("write {file}: {e}"));
        println!("wrote {file} ({}×{})", snap.width(), snap.height());
    }
}
