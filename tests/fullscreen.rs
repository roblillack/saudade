//! Fullscreen requests: what widgets see through `EventCtx`, and what the mock
//! backend does with them.

use saudade::mock::MockBackend;
use saudade::*;

/// Toggles fullscreen on `f`, and records what `is_fullscreen` said before
/// and after asking.
struct Toggler {
    seen: Vec<(bool, bool)>,
}

impl Widget for Toggler {
    fn bounds(&self) -> Rect {
        Rect::new(0, 0, 100, 60)
    }

    fn paint(&mut self, painter: &mut Painter, _theme: &Theme) {
        painter.fill_rect(self.bounds(), Color::WHITE);
    }

    fn event(&mut self, event: &Event, ctx: &mut EventCtx) {
        match event {
            Event::KeyDown {
                key: Key::Char('f'),
                ..
            } => {
                let before = ctx.is_fullscreen();
                ctx.toggle_fullscreen();
                self.seen.push((before, ctx.is_fullscreen()));
            }
            Event::KeyDown {
                key: Key::Named(NamedKey::Escape),
                ..
            } => ctx.set_fullscreen(false),
            _ => {}
        }
    }

    fn focusable(&self) -> bool {
        true
    }
}

fn key(key: impl Into<Key>) -> Event {
    Event::KeyDown {
        key: key.into(),
        modifiers: Modifiers::default(),
    }
}

#[test]
fn toggling_flips_the_state_the_runtime_reports() {
    let backend = MockBackend::new(100, 60);
    let mut w = Toggler { seen: Vec::new() };
    assert!(!backend.is_fullscreen());

    let out = backend.dispatch(&mut w, &key('f'));
    assert_eq!(out.fullscreen_request, Some(true));
    assert!(out.paint_requested, "a fullscreen switch repaints");
    assert!(backend.is_fullscreen());

    backend.dispatch(&mut w, &key('f'));
    assert!(!backend.is_fullscreen());

    // Within a dispatch, `is_fullscreen` answers with the state being switched to.
    assert_eq!(w.seen, vec![(false, true), (true, false)]);
}

#[test]
fn leaving_when_not_fullscreen_is_harmless() {
    let backend = MockBackend::new(100, 60);
    let mut w = Toggler { seen: Vec::new() };
    let out = backend.dispatch(&mut w, &key(NamedKey::Escape));
    assert_eq!(out.fullscreen_request, Some(false));
    assert!(!backend.is_fullscreen());
}

#[test]
fn requests_travel_up_through_containers() {
    let backend = MockBackend::new(100, 60).with_fullscreen(true);
    let mut root = Column::new().add_fill(Toggler { seen: Vec::new() });
    root.layout(Rect::new(0, 0, 100, 60));
    root.focus_first();
    backend.dispatch(&mut root, &key(NamedKey::Escape));
    assert!(!backend.is_fullscreen());
}

#[test]
fn events_without_a_request_leave_the_state_alone() {
    let backend = MockBackend::new(100, 60).with_fullscreen(true);
    let mut w = Toggler { seen: Vec::new() };
    let out = backend.dispatch(&mut w, &key('x'));
    assert_eq!(out.fullscreen_request, None);
    assert!(backend.is_fullscreen());
}

#[test]
fn a_fullscreen_window_is_rendered_without_chrome() {
    let mut w = Toggler { seen: Vec::new() };
    let chrome = WindowChrome::resizable("Full");
    let windowed = MockBackend::new(100, 60).render_framed(&mut w, &chrome);
    let full = MockBackend::new(100, 60)
        .with_fullscreen(true)
        .render_framed(&mut w, &chrome);
    assert!(windowed.width() > 100 && windowed.height() > 60);
    assert_eq!((full.width(), full.height()), (100, 60));
}

#[test]
fn window_config_can_start_fullscreen() {
    let cfg = WindowConfig::new("Kiosk", 320, 200).fullscreen(true);
    assert!(cfg.fullscreen);
    assert_eq!(cfg.size, Size::new(320, 200), "the windowed size is kept");
    assert!(!WindowConfig::new("Plain", 1, 1).fullscreen);
}
