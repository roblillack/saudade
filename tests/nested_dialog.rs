//! A dialog opened over another one — a message box shown from a custom dialog
//! — closes on Escape on its own: the dialog beneath stays open until it gets
//! an Escape of its own.

use std::cell::RefCell;
use std::rc::Rc;

use saudade::mock::MockBackend;
use saudade::{
    Column, Dialog, Event, EventCtx, Key, Modal, Modifiers, NamedKey, Painter, PopupKind,
    PopupRequest, Rect, Size, Theme, Widget,
};

const PARENT: Rect = Rect::new(0, 0, 400, 320);

/// A custom dialog whose content holds a message box, shared so the test can
/// show it, both open.
fn nested_dialogs() -> (Modal, Rc<RefCell<Dialog>>) {
    let inner = Rc::new(RefCell::new(Dialog::new()));
    let body = Column::new().add_overlay(Shared(inner.clone()));
    let mut outer = Modal::new();
    outer.layout(PARENT); // establish parent bounds
    outer.show("Outer", Size::new(240, 160), Box::new(body));
    outer.layout(PARENT); // lay the content out
    inner
        .borrow_mut()
        .show_info("Inner", "Shown over the outer dialog.");
    outer.layout(PARENT);
    (outer, inner)
}

fn escape() -> Event {
    Event::KeyDown {
        key: Key::Named(NamedKey::Escape),
        modifiers: Modifiers::default(),
    }
}

fn dialogs(outer: &Modal) -> usize {
    let mut popups = Vec::new();
    outer.collect_popups(&mut popups);
    popups
        .iter()
        .filter(|p| p.kind == PopupKind::Dialog)
        .count()
}

#[test]
fn escape_closes_only_the_dialog_on_top() {
    let (mut outer, inner) = nested_dialogs();
    let backend = MockBackend::new(PARENT.w, PARENT.h);
    assert_eq!(dialogs(&outer), 2);

    backend.dispatch(&mut outer, &escape());
    assert!(!inner.borrow().is_open(), "Escape closes the message box");
    assert!(outer.is_open(), "the dialog beneath stays open");
    assert_eq!(dialogs(&outer), 1);

    backend.dispatch(&mut outer, &escape());
    assert!(!outer.is_open(), "a second Escape closes that one too");
}

/// Lets the test keep a handle on a dialog the widget tree owns.
struct Shared(Rc<RefCell<Dialog>>);

impl Widget for Shared {
    fn bounds(&self) -> Rect {
        self.0.borrow().bounds()
    }
    fn layout(&mut self, bounds: Rect) {
        self.0.borrow_mut().layout(bounds);
    }
    fn paint(&mut self, painter: &mut Painter, theme: &Theme) {
        self.0.borrow_mut().paint(painter, theme);
    }
    fn paint_overlay(&mut self, painter: &mut Painter, theme: &Theme) {
        self.0.borrow_mut().paint_overlay(painter, theme);
    }
    fn event(&mut self, event: &Event, ctx: &mut EventCtx) {
        self.0.borrow_mut().event(event, ctx);
    }
    fn captures_pointer(&self) -> bool {
        self.0.borrow().captures_pointer()
    }
    fn accepts_accelerators(&self) -> bool {
        self.0.borrow().accepts_accelerators()
    }
    fn popup_request(&self) -> Option<PopupRequest> {
        self.0.borrow().popup_request()
    }
    fn collect_popups(&self, out: &mut Vec<PopupRequest>) {
        self.0.borrow().collect_popups(out);
    }
}
