//! Scrollbar thumb-drag vs. window-leave, and the list/scrollbar gutter split.
//!
//! These guard the filer's drag'n'drop-vs-scroll collision: a press on the
//! scrollbar must scroll, never arm a wrapper's drag-out, and a thumb drag must
//! not survive the pointer leaving the window (which an outbound drag does by
//! revoking pointer focus) — otherwise the thumb chases the cursor on return.

use saudade::mock::MockBackend;
use saudade::{Event, List, ListItem, MouseButton, Point, Rect, ScrollBar, Slider, Widget};

fn down(x: i32, y: i32) -> Event {
    Event::PointerDown {
        pos: Point::new(x, y),
        button: MouseButton::Left,
        modifiers: Default::default(),
    }
}

fn moved(x: i32, y: i32) -> Event {
    Event::PointerMove {
        pos: Point::new(x, y),
    }
}

#[test]
fn scrollbar_drag_ends_on_pointer_leave() {
    let backend = MockBackend::new(64, 220);
    // Tall vertical bar with a scrollable range, so there's a real, movable
    // thumb sitting just below the top arrow button (track starts at y = 16).
    let mut sb = ScrollBar::vertical(Rect::new(0, 0, 16, 200));
    sb.set_range(5, 20);

    // Grab the thumb and drag it down — value moves, and the bar captures the
    // pointer for the duration of the drag.
    backend.dispatch(&mut sb, &down(8, 20));
    assert!(sb.captures_pointer(), "press on the thumb starts a drag");
    backend.dispatch(&mut sb, &moved(8, 60));
    assert!(sb.value() > 0, "dragging the thumb scrolls");
    let parked = sb.value();

    // The pointer leaves the window mid-drag (what an outbound drag-and-drop
    // does by revoking pointer focus). The drag must end here…
    backend.dispatch(&mut sb, &Event::PointerLeave);
    assert!(
        !sb.captures_pointer(),
        "leaving the window ends the thumb drag"
    );

    // …so a later move with the button no longer held doesn't drag the thumb.
    backend.dispatch(&mut sb, &moved(8, 140));
    assert_eq!(sb.value(), parked, "the thumb no longer chases the cursor");
}

#[test]
fn slider_drag_ends_on_pointer_leave() {
    let backend = MockBackend::new(220, 32);
    // A horizontal trackbar over [0, 100]; the thumb starts at the left (value
    // 0) and travels across the widget's width.
    let mut slider = Slider::new(Rect::new(0, 0, 200, 24), 0, 100);

    // Grab the thumb and drag right — value moves and the slider captures.
    backend.dispatch(&mut slider, &down(10, 12));
    assert!(slider.captures_pointer(), "press starts a drag");
    backend.dispatch(&mut slider, &moved(120, 12));
    assert!(slider.value() > 0, "dragging the thumb moves the value");
    let parked = slider.value();

    // The pointer leaves the window mid-drag (what an outbound drag-and-drop
    // does by revoking pointer focus). The drag must end…
    backend.dispatch(&mut slider, &Event::PointerLeave);
    assert!(
        !slider.captures_pointer(),
        "leaving the window ends the drag"
    );

    // …so a later move with the button no longer held doesn't move the thumb.
    backend.dispatch(&mut slider, &moved(180, 12));
    assert_eq!(
        slider.value(),
        parked,
        "the thumb no longer jumps to the cursor"
    );
}

#[test]
fn list_scrollbar_hit_splits_gutter_from_rows() {
    let mut list = List::new(Rect::new(0, 0, 0, 0)).with_items(
        (0..40)
            .map(|i| ListItem::new(format!("item {i}")))
            .collect(),
    );
    // Layout pins the 16px-wide scrollbar to the right edge.
    list.layout(Rect::new(0, 0, 200, 120));

    assert!(
        list.scrollbar_hit(Point::new(195, 40)),
        "the right gutter belongs to the scrollbar"
    );
    assert!(
        !list.scrollbar_hit(Point::new(40, 40)),
        "the row field is not the scrollbar"
    );
}
