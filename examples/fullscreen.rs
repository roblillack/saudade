//! fullscreen — switching the window in and out of fullscreen.
//!
//! A card in the middle of the window says whether it is fullscreen and how
//! big it is. The button, or F, toggles fullscreen (`EventCtx::toggle_fullscreen`);
//! Escape leaves it (`EventCtx::set_fullscreen(false)`). The state shown is
//! read back from `EventCtx::is_fullscreen`, so a switch made through the
//! platform (macOS's green button, a compositor keybinding) shows up too.
//!
//! ```console
//! $ cargo run --example fullscreen                 # starts windowed
//! $ cargo run --example fullscreen -- --fullscreen # starts fullscreen
//! ```

use saudade::{
    App, Button, Column, Event, EventCtx, Key, NamedKey, Painter, Rect, Theme, Widget, WindowConfig,
};

const CARD_W: i32 = 280;
const CARD_H: i32 = 120;

struct Demo {
    rect: Rect,
    button: Button,
    fullscreen: bool,
}

impl Demo {
    fn card(&self) -> Rect {
        Rect::new(
            self.rect.x + (self.rect.w - CARD_W) / 2,
            self.rect.y + (self.rect.h - CARD_H) / 2,
            CARD_W,
            CARD_H,
        )
    }
}

impl Widget for Demo {
    fn bounds(&self) -> Rect {
        self.rect
    }

    fn layout(&mut self, bounds: Rect) {
        self.rect = bounds;
        let card = self.card();
        self.button.layout(Rect::new(
            card.x + (CARD_W - 150) / 2,
            card.bottom() - 40,
            150,
            26,
        ));
    }

    fn paint(&mut self, painter: &mut Painter, theme: &Theme) {
        let card = self.card();
        painter.fill_rect(card, theme.background);
        painter.stroke_rect(card, theme.border);
        let state = if self.fullscreen {
            "Full screen"
        } else {
            "Windowed"
        };
        let size = format!("{} × {} logical pixels", self.rect.w, self.rect.h);
        painter.text_centered(
            Rect::new(card.x, card.y + 14, CARD_W, 18),
            state,
            theme.font_size * 1.5,
            theme.text,
        );
        painter.text_centered(
            Rect::new(card.x, card.y + 42, CARD_W, 16),
            &size,
            theme.font_size,
            theme.disabled_text,
        );
        self.button.label = if self.fullscreen {
            "Leave Full Screen".into()
        } else {
            "Full Screen".into()
        };
        self.button.paint(painter, theme);
    }

    fn event(&mut self, event: &Event, ctx: &mut EventCtx) {
        match event {
            Event::KeyDown {
                key: Key::Char('f'),
                modifiers,
            } if !modifiers.has_command() => ctx.toggle_fullscreen(),
            Event::KeyDown {
                key: Key::Named(NamedKey::Escape),
                ..
            } => ctx.set_fullscreen(false),
            _ if event.is_keyboard() => {}
            _ => {
                let over = event
                    .position()
                    .is_none_or(|p| self.button.bounds().contains(p));
                if over || self.button.captures_pointer() {
                    self.button.event(event, ctx);
                }
            }
        }
        if ctx.is_fullscreen() != self.fullscreen {
            self.fullscreen = ctx.is_fullscreen();
            ctx.request_paint();
        }
    }

    fn captures_pointer(&self) -> bool {
        self.button.captures_pointer()
    }

    fn focusable(&self) -> bool {
        true
    }
}

fn main() {
    let start_fullscreen = std::env::args().any(|a| a == "--fullscreen");
    let (config, root) = window(start_fullscreen);
    App::new(config, root).with_theme(Theme::windows_31()).run();
}

/// The window and the widget tree inside it, opening fullscreen if
/// `start_fullscreen`, kept apart from `main` so `examples/screenshots.rs` can
/// render the same window offscreen.
pub fn window(start_fullscreen: bool) -> (WindowConfig, impl Widget) {
    let demo = Demo {
        rect: Rect::new(0, 0, 0, 0),
        button: Button::new(Rect::new(0, 0, 0, 0), "Full Screen")
            .on_click(|cx| cx.toggle_fullscreen()),
        fullscreen: start_fullscreen,
    };
    (
        WindowConfig::new("Fullscreen", 420, 260)
            .resizable(true)
            .fullscreen(start_fullscreen),
        Column::new().add_fill(demo),
    )
}
