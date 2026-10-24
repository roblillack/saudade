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
use saudade::{Bevel, Button, Container, Font, Label, Painter, Rect, Theme, Widget, WindowChrome};

const W: i32 = 260;
const H: i32 = 120;

fn main() {
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
        let snap = backend.render_framed(&mut about_box(), &chrome);
        std::fs::write(file, snap.to_png()).unwrap_or_else(|e| panic!("write {file}: {e}"));
        println!("wrote {file} ({}×{})", snap.width(), snap.height());
    }
}

/// A small "about box"-style client area to sit inside the chrome, `W × H`
/// logical pixels. Public so `examples/screenshots.rs` can frame it too.
pub fn about_box() -> impl Widget {
    Container::new(W, H)
        .add(Face(Rect::new(0, 0, W, H)))
        .add(Bevel::sunken(Rect::new(16, 16, W - 32, 48)))
        .add(Label::new(
            Rect::new(28, 26, W - 56, 16),
            "Saudade Utilities",
        ))
        .add(Label::new(
            Rect::new(28, 42, W - 56, 16),
            "Version 1.0 — © 1992",
        ))
        .add(Button::new(Rect::new((W - 80) / 2, 80, 80, 26), "OK").default(true))
}

/// The about box's gray ground: the theme's button face, rather than a fixed
/// light gray, so the box goes dark along with the rest of a dark theme.
struct Face(Rect);

impl Widget for Face {
    fn bounds(&self) -> Rect {
        self.0
    }

    fn paint(&mut self, painter: &mut Painter, theme: &Theme) {
        painter.fill_rect(self.0, theme.face);
    }
}
