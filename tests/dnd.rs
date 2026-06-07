//! Drag-and-drop event routing. The platform backends turn OS file drops into
//! [`Event::DragEnter`] / [`Event::DragMove`] / [`Event::DragLeave`] /
//! [`Event::Drop`]; here we check the *widget-facing* contract those events
//! flow through — positional routing, the leave broadcast, and payload
//! delivery — using the offscreen [`MockBackend`] so no real window is needed.

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;

use saudade::mock::MockBackend;
use saudade::{Container, DragData, Event, EventCtx, Painter, Point, Rect, Theme, Widget};

type Log = Rc<RefCell<Vec<String>>>;

/// A drop target that records every drag event it receives (tagged with its id)
/// into a shared log, and tracks whether it currently believes a drag is over
/// it. Lets a test see *which* widget an event reached.
struct Recorder {
    rect: Rect,
    id: &'static str,
    log: Log,
    hovering: bool,
}

impl Recorder {
    fn new(id: &'static str, rect: Rect, log: &Log) -> Self {
        Self {
            rect,
            id,
            log: log.clone(),
            hovering: false,
        }
    }
}

impl Widget for Recorder {
    fn bounds(&self) -> Rect {
        self.rect
    }

    fn paint(&mut self, _p: &mut Painter, _t: &Theme) {}

    fn event(&mut self, event: &Event, _ctx: &mut EventCtx) {
        let note = match event {
            Event::DragEnter { .. } => {
                self.hovering = true;
                "enter".to_string()
            }
            Event::DragMove { .. } => "move".to_string(),
            Event::DragLeave => {
                self.hovering = false;
                "leave".to_string()
            }
            Event::Drop { data, .. } => {
                self.hovering = false;
                let joined = data
                    .paths
                    .iter()
                    .map(|p| p.display().to_string())
                    .collect::<Vec<_>>()
                    .join(",");
                format!("drop:{joined}")
            }
            _ => return,
        };
        self.log.borrow_mut().push(format!("{}:{note}", self.id));
    }
}

fn drop_at(x: i32, paths: &[&str]) -> Event {
    Event::Drop {
        pos: Point::new(x, 10),
        data: DragData::from_paths(paths.iter().map(PathBuf::from)),
    }
}

#[test]
fn drop_routes_to_the_widget_under_the_cursor() {
    let log: Log = Rc::new(RefCell::new(Vec::new()));
    let mut root = Container::new(100, 20)
        .add(Recorder::new("A", Rect::new(0, 0, 50, 20), &log))
        .add(Recorder::new("B", Rect::new(50, 0, 50, 20), &log));

    let backend = MockBackend::new(100, 20);
    backend.dispatch(&mut root, &drop_at(60, &["/tmp/file.txt"]));

    // Only B is under x=60, so only B sees the drop — with its payload intact.
    assert_eq!(
        log.borrow().as_slice(),
        &["B:drop:/tmp/file.txt".to_string()]
    );
}

#[test]
fn drag_enter_then_drop_carries_the_payload_only_on_drop() {
    let log: Log = Rc::new(RefCell::new(Vec::new()));
    let mut root = Container::new(100, 20).add(Recorder::new("A", Rect::new(0, 0, 100, 20), &log));

    let backend = MockBackend::new(100, 20);
    backend.dispatch(
        &mut root,
        &Event::DragEnter {
            pos: Point::new(5, 5),
        },
    );
    backend.dispatch(
        &mut root,
        &Event::DragMove {
            pos: Point::new(8, 5),
        },
    );
    backend.dispatch(&mut root, &drop_at(8, &["/a", "/b"]));

    assert_eq!(
        log.borrow().as_slice(),
        &[
            "A:enter".to_string(),
            "A:move".to_string(),
            "A:drop:/a,/b".to_string(),
        ]
    );
}

#[test]
fn drag_leave_is_broadcast_to_every_child() {
    // Like PointerLeave, DragLeave carries no position, so the container hands
    // it to every child — any drop target can clear its highlight even if the
    // drag never entered it.
    let log: Log = Rc::new(RefCell::new(Vec::new()));
    let mut root = Container::new(100, 20)
        .add(Recorder::new("A", Rect::new(0, 0, 50, 20), &log))
        .add(Recorder::new("B", Rect::new(50, 0, 50, 20), &log));

    let backend = MockBackend::new(100, 20);
    backend.dispatch(&mut root, &Event::DragLeave);

    let entries = log.borrow();
    assert!(entries.contains(&"A:leave".to_string()));
    assert!(entries.contains(&"B:leave".to_string()));
}

#[test]
fn positional_drag_events_expose_a_position_but_leave_does_not() {
    assert_eq!(
        Event::DragEnter {
            pos: Point::new(3, 4)
        }
        .position(),
        Some(Point::new(3, 4))
    );
    assert_eq!(
        Event::DragMove {
            pos: Point::new(7, 8)
        }
        .position(),
        Some(Point::new(7, 8))
    );
    assert_eq!(
        Event::Drop {
            pos: Point::new(1, 2),
            data: DragData::default(),
        }
        .position(),
        Some(Point::new(1, 2))
    );
    assert_eq!(Event::DragLeave.position(), None);

    // None of the drag events count as keyboard events.
    assert!(!Event::DragLeave.is_keyboard());
    assert!(
        !Event::Drop {
            pos: Point::new(0, 0),
            data: DragData::default(),
        }
        .is_keyboard()
    );
}

#[test]
fn drag_data_helpers() {
    let empty = DragData::default();
    assert!(!empty.has_paths());

    let data = DragData::from_paths([PathBuf::from("/x"), PathBuf::from("/y")]);
    assert!(data.has_paths());
    assert_eq!(data.paths.len(), 2);
}
