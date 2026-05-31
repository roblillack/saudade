//! patterns — a playground for the window background patterns.
//!
//! saudade lets a *regular* top-level window paint a 1-bit desktop texture
//! behind the widget tree, while dialogs and modals stay plain white. The
//! pattern and its color are runtime/debug state toggled from the keyboard,
//! the same in every saudade app:
//!
//! * `p` rotates the pattern: none → solid → dots → dots2 → lines →
//!   diagonal `///` → diagonal `\\\` → cross-stitch → none …
//! * `c` rotates the color: superlight → light → dark → black …
//!
//! Each press prints the new pattern + color to the console.
//!
//! Most saudade apps fill their whole window with an opaque `Container`, so
//! the pattern sits hidden behind it — the console line is then the only
//! feedback. This demo's root deliberately leaves the window background
//! exposed and floats a small "About"-style card on top, so you can actually
//! watch the texture change.

use saudade::{App, Painter, Rect, Theme, Widget, WindowConfig};

const W: i32 = 480;
const H: i32 = 320;

fn main() {
    App::new(
        WindowConfig::new("Background Patterns", W, H).resizable(true),
        Workspace {
            bounds: Rect::new(0, 0, W, H),
        },
    )
    .with_theme(Theme::windows_31())
    .run();
}

/// A root that paints only a small centered card and leaves the rest of the
/// window untouched, so the runtime's background pattern shows around it.
struct Workspace {
    bounds: Rect,
}

impl Widget for Workspace {
    fn bounds(&self) -> Rect {
        self.bounds
    }

    fn layout(&mut self, bounds: Rect) {
        // Track the live window size so the card stays centered on resize.
        self.bounds = bounds;
    }

    fn paint(&mut self, painter: &mut Painter, theme: &Theme) {
        // NOTE: intentionally no full-window fill here. Everything we *don't*
        // paint keeps the background pattern the runtime drew underneath.
        let cw = 300;
        let ch = 132;
        let cx = self.bounds.x + (self.bounds.w - cw) / 2;
        let cy = self.bounds.y + (self.bounds.h - ch) / 2;
        let card = Rect::new(cx, cy, cw, ch);

        // A raised, bordered gray card — the floating panel.
        painter.fill_rect(card, theme.face);
        painter.raised_bevel(card, theme.highlight, theme.shadow);
        painter.stroke_rect(card, theme.border);

        painter.text_centered(
            Rect::new(cx, cy + 20, cw, 18),
            "Window Background Patterns",
            13.0,
            theme.text,
        );
        painter.text_centered(
            Rect::new(cx, cy + 58, cw, 16),
            "Press  P  to change the pattern",
            11.0,
            theme.text,
        );
        painter.text_centered(
            Rect::new(cx, cy + 80, cw, 16),
            "Press  C  to change the color",
            11.0,
            theme.text,
        );
        painter.text_centered(
            Rect::new(cx, cy + 104, cw, 16),
            "(the name + color print to the console)",
            10.0,
            theme.disabled_text,
        );
    }
}
