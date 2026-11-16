//! The modifier keys widgets see through `EventCtx` on any event, pointer moves
//! included.

use saudade::mock::MockBackend;
use saudade::*;

/// Records the modifiers each pointer move was dispatched with.
struct Hover {
    seen: Vec<Modifiers>,
}

impl Widget for Hover {
    fn bounds(&self) -> Rect {
        Rect::new(0, 0, 100, 60)
    }

    fn paint(&mut self, painter: &mut Painter, _theme: &Theme) {
        painter.fill_rect(self.bounds(), Color::WHITE);
    }

    fn event(&mut self, event: &Event, ctx: &mut EventCtx) {
        if let Event::PointerMove { .. } = event {
            self.seen.push(ctx.modifiers());
        }
    }
}

fn moved() -> Event {
    Event::PointerMove {
        pos: Point::new(10, 10),
    }
}

#[test]
fn pointer_moves_report_the_modifiers_held() {
    let backend = MockBackend::new(100, 60);
    let mut w = Hover { seen: Vec::new() };
    backend.dispatch(&mut w, &moved());

    let ctrl = Modifiers {
        control: true,
        ..Modifiers::default()
    };
    backend.set_modifiers(ctrl);
    backend.dispatch(&mut w, &moved());

    backend.set_modifiers(Modifiers::default());
    backend.dispatch(&mut w, &moved());

    assert_eq!(
        w.seen,
        vec![Modifiers::default(), ctrl, Modifiers::default()]
    );
}
