//! Window-title requests: what a widget asks of `EventCtx`, as the mock backend
//! records it.

use saudade::mock::MockBackend;
use saudade::*;

/// Retitles the window after the key pressed, twice for `d` — the last request
/// of a dispatch is the one that counts.
struct Retitler;

impl Widget for Retitler {
    fn bounds(&self) -> Rect {
        Rect::new(0, 0, 100, 60)
    }

    fn paint(&mut self, painter: &mut Painter, _theme: &Theme) {
        painter.fill_rect(self.bounds(), Color::WHITE);
    }

    fn event(&mut self, event: &Event, ctx: &mut EventCtx) {
        if let Event::KeyDown {
            key: Key::Char(c), ..
        } = event
        {
            if *c == 'd' {
                ctx.set_window_title("overruled");
            }
            ctx.set_window_title(format!("{c} - Test"));
        }
    }

    fn focusable(&self) -> bool {
        true
    }
}

fn key(c: char) -> Event {
    Event::KeyDown {
        key: Key::Char(c),
        modifiers: Modifiers::default(),
    }
}

#[test]
fn the_latest_request_names_the_window() {
    let backend = MockBackend::new(100, 60);
    let mut w = Retitler;
    assert_eq!(backend.window_title(), None);

    backend.dispatch(&mut w, &key('a'));
    assert_eq!(backend.window_title().as_deref(), Some("a - Test"));

    backend.dispatch(&mut w, &key('d'));
    assert_eq!(backend.window_title().as_deref(), Some("d - Test"));
}

#[test]
fn an_event_that_asks_nothing_keeps_the_title() {
    let backend = MockBackend::new(100, 60);
    let mut w = Retitler;
    backend.dispatch(&mut w, &key('a'));
    backend.dispatch(&mut w, &Event::Tick);
    assert_eq!(backend.window_title().as_deref(), Some("a - Test"));
}
