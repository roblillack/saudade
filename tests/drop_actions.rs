//! What a drag offers its drop to do, and what a drop target says it would do
//! there — passed between the applications on Wayland, and so the cursor the
//! compositor shows.

use saudade::mock::MockBackend;
use saudade::*;

/// Takes a drag from elsewhere as a copy; one of the window's own it moves,
/// or copies while Ctrl is held.
struct Folder;

impl Widget for Folder {
    fn bounds(&self) -> Rect {
        Rect::new(0, 0, 100, 60)
    }

    fn paint(&mut self, painter: &mut Painter, _theme: &Theme) {
        painter.fill_rect(self.bounds(), Color::WHITE);
    }

    fn event(&mut self, event: &Event, ctx: &mut EventCtx) {
        match event {
            Event::DragEnter { .. } | Event::DragMove { .. } if ctx.is_dragging_out() => {
                ctx.accept_drop_as(if ctx.modifiers().control {
                    DropAction::Copy
                } else {
                    DropAction::Move
                });
            }
            Event::DragEnter { .. } | Event::DragMove { .. } => ctx.accept_drop(),
            _ => {}
        }
    }
}

fn hover() -> Event {
    Event::DragMove {
        pos: Point::new(10, 10),
    }
}

#[test]
fn a_drop_target_names_what_a_drop_would_do() {
    let backend = MockBackend::new(100, 60);
    let mut w = Folder;
    assert_eq!(
        backend.dispatch(&mut w, &hover()).accepted_drop,
        Some(DropAction::Copy)
    );

    backend.set_dragging_out(true);
    assert_eq!(
        backend.dispatch(&mut w, &hover()).accepted_drop,
        Some(DropAction::Move)
    );

    backend.set_modifiers(Modifiers {
        control: true,
        ..Modifiers::default()
    });
    assert_eq!(
        backend.dispatch(&mut w, &hover()).accepted_drop,
        Some(DropAction::Copy)
    );
}
