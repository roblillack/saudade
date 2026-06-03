//! scaling — read and override the runtime's logical→physical scale factor.
//!
//! The runtime adopts the OS-reported scale factor at startup (and refreshes it
//! whenever the compositor sends a `ScaleFactorChanged`), but widgets can also
//! ask the runtime to use a different value via [`EventCtx::set_scale_factor`].
//! The current value is always reflected by [`Painter::scale`] inside `paint`,
//! so a widget can both *read* and *write* the active scale.
//!
//! This demo pairs a small read-out widget (rendering `Painter::scale()` as
//! text) with a `Slider` and a `Reset` button that call `set_scale_factor`.
//! Drag the slider, click a preset, or hit the arrow keys with the slider
//! focused — the chrome resizes live.
//!
//! The window starts wide enough that the demo content still fits at the
//! biggest preset (3.0x). Smaller scales letterbox; on backends where popups
//! were open, the runtime tears them down so they're rebuilt at the new
//! scale on the next paint.

use std::cell::Cell;
use std::rc::Rc;

use saudade::{
    App, Button, Color, Container, Label, Painter, Rect, Slider, Theme, Widget, WindowConfig,
};

const W: i32 = 480;
const H: i32 = 260;

fn main() {
    // Captured on the first paint, so the "Reset" button knows what scale to
    // restore. Stays at 0.0 until the first frame; the button no-ops in that
    // window, which is fine — the very first paint always lands before any
    // user input.
    let initial_scale = Rc::new(Cell::new(0.0_f32));

    let slider = Slider::new(Rect::new(100, 92, 280, 24), 50, 300)
        .with_value(100)
        .with_step(25)
        .on_change(|cx, percent| {
            cx.set_scale_factor(percent as f32 / 100.0);
        });

    let reset = Button::new(Rect::new(190, 196, 100, 24), "Reset to OS").on_click({
        let initial = initial_scale.clone();
        move |cx| {
            let s = initial.get();
            if s > 0.0 {
                cx.set_scale_factor(s);
            }
        }
    });

    let root = Container::new(W, H)
        .with_background(Color::WHITE)
        .add(Label::new(Rect::new(20, 16, W - 40, 18), "Scale factor").with_size(13.0))
        .add(
            Label::new(
                Rect::new(20, 40, W - 40, 32),
                "Drag the slider to ask the runtime to redraw at a different\n\
                 logical-to-physical scale.",
            )
            .with_size(10.0),
        )
        .add(ScaleDisplay::new(
            Rect::new(20, 84, 80, 36),
            initial_scale.clone(),
        ))
        .add(slider)
        .add(Label::new(Rect::new(100, 120, 40, 14), "0.5x").with_size(9.0))
        .add(
            Label::new(Rect::new(W - 60, 120, 40, 14), "3.0x")
                .with_size(9.0)
                .with_color(Color::DARK_GRAY),
        )
        .add(preset(70, 152, "0.5x", 0.5))
        .add(preset(140, 152, "1.0x", 1.0))
        .add(preset(210, 152, "1.5x", 1.5))
        .add(preset(280, 152, "2.0x", 2.0))
        .add(preset(350, 152, "3.0x", 3.0))
        .add(reset);

    App::new(
        WindowConfig::new("Scale Factor", W, H).resizable(true),
        root,
    )
    .with_theme(Theme::windows_31())
    .run();
}

fn preset(x: i32, y: i32, label: &str, factor: f32) -> Button {
    Button::new(Rect::new(x, y, 60, 22), label).on_click(move |cx| cx.set_scale_factor(factor))
}

/// Live read-out of the runtime's current scale factor. There is no
/// dedicated "scale changed" event in saudade — the canonical source is
/// [`Painter::scale`] during `paint`, which is exactly what this widget
/// reads. The first paint also stashes the OS-reported scale into a
/// shared cell so the "Reset" button can later restore it.
struct ScaleDisplay {
    rect: Rect,
    initial: Rc<Cell<f32>>,
}

impl ScaleDisplay {
    fn new(rect: Rect, initial: Rc<Cell<f32>>) -> Self {
        Self { rect, initial }
    }
}

impl Widget for ScaleDisplay {
    fn bounds(&self) -> Rect {
        self.rect
    }

    fn paint(&mut self, painter: &mut Painter, theme: &Theme) {
        let scale = painter.scale();
        if self.initial.get() == 0.0 {
            self.initial.set(scale);
        }
        painter.text(
            self.rect.x,
            self.rect.y,
            &format!("{scale:.2}x"),
            22.0,
            theme.text,
        );
        let initial = self.initial.get();
        if initial > 0.0 {
            painter.text(
                self.rect.x,
                self.rect.y + 22,
                &format!("OS: {initial:.2}x"),
                10.0,
                theme.disabled_text,
            );
        }
    }
}
