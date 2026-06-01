//! patterns — a playground for the window background patterns.
//!
//! Every saudade app paints a 1-bit desktop texture behind its widgets (dialogs
//! and modals stay white). The pattern + color are fixed per process: the
//! default is a `superlight` forward-diagonal hatch, overridable with the
//! `SAUDADE_WINDOW_PATTERN` and `SAUDADE_WINDOW_PATTERN_COLOR` environment
//! variables, e.g.
//!
//! ```console
//! $ SAUDADE_WINDOW_PATTERN=dots SAUDADE_WINDOW_PATTERN_COLOR=light cargo run --example notepad
//! ```
//!
//! This demo is the exception: it draws the background itself so you can preview
//! the options interactively —
//!
//! * `p` rotates the pattern: none → solid → dots → lines → diagonal → cross-stitch …
//! * `c` rotates the color: superlight → light → dark → black …
//!
//! Each press prints the new pattern + color to the console.

use saudade::{
    App, BackgroundPattern, Color, Event, EventCtx, Key, PATTERN_COLORS, Painter, Rect, Theme,
    Widget, WindowConfig,
};

const W: i32 = 480;
const H: i32 = 320;

fn main() {
    App::new(
        WindowConfig::new("Background Patterns", W, H).resizable(true),
        Workspace::new(),
    )
    .with_theme(Theme::windows_31())
    .run();
}

/// Fills the whole window with the currently-selected pattern (so this demo is
/// self-contained rather than relying on the process-wide background), and
/// floats a small instruction card on top. `p` / `c` cycle the selection.
struct Workspace {
    bounds: Rect,
    pattern: BackgroundPattern,
    color_idx: usize,
}

impl Workspace {
    fn new() -> Self {
        // Start on the same defaults the runtime uses.
        Self {
            bounds: Rect::new(0, 0, W, H),
            pattern: BackgroundPattern::DiagonalForward,
            color_idx: 0,
        }
    }

    fn color(&self) -> Color {
        PATTERN_COLORS[self.color_idx].1
    }

    fn log(&self) {
        let (name, c) = PATTERN_COLORS[self.color_idx];
        println!(
            "[patterns] pattern: {} | color: {} (#{:02X}{:02X}{:02X})",
            self.pattern.name(),
            name,
            c.red(),
            c.green(),
            c.blue(),
        );
    }
}

impl Widget for Workspace {
    fn bounds(&self) -> Rect {
        self.bounds
    }

    fn layout(&mut self, bounds: Rect) {
        // Track the live window size so the card stays centered on resize.
        self.bounds = bounds;
    }

    fn event(&mut self, event: &Event, ctx: &mut EventCtx) {
        if let Event::KeyDown {
            key: Key::Char(ch), ..
        } = event
        {
            match ch.to_ascii_lowercase() {
                'p' => self.pattern = self.pattern.next(),
                'c' => self.color_idx = (self.color_idx + 1) % PATTERN_COLORS.len(),
                _ => return,
            }
            self.log();
            ctx.request_paint();
        }
    }

    fn paint(&mut self, painter: &mut Painter, theme: &Theme) {
        painter.fill_pattern(Color::WHITE, self.pattern, self.color());

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
