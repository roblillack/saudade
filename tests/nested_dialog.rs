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

/// A window with a custom dialog open whose content holds a message box, both
/// shared so the test can show them, both open.
fn nested_dialogs() -> (Column, Rc<RefCell<Modal>>, Rc<RefCell<Dialog>>) {
    let inner = Rc::new(RefCell::new(Dialog::new()));
    let body = Column::new().add_overlay(Shared(inner.clone()));
    let outer = Rc::new(RefCell::new(Modal::new()));
    let mut root = Column::new().add_overlay(Shared(outer.clone()));
    root.layout(PARENT); // establish parent bounds
    outer
        .borrow_mut()
        .show("Outer", Size::new(240, 160), Box::new(body));
    root.layout(PARENT); // lay the content out
    inner
        .borrow_mut()
        .show_info("Inner", "Shown over the outer dialog.");
    root.layout(PARENT);
    (root, outer, inner)
}

fn escape() -> Event {
    Event::KeyDown {
        key: Key::Named(NamedKey::Escape),
        modifiers: Modifiers::default(),
    }
}

fn dialogs(root: &Column) -> usize {
    let mut popups = Vec::new();
    root.collect_popups(&mut popups);
    popups
        .iter()
        .filter(|p| p.kind == PopupKind::Dialog)
        .count()
}

#[test]
fn escape_closes_only_the_dialog_on_top() {
    let (mut root, outer, inner) = nested_dialogs();
    let backend = MockBackend::new(PARENT.w, PARENT.h);
    // The message box opens once a paint has measured its message — in the
    // outer dialog's pass, as that is where it is painted.
    assert_eq!(dialogs(&root), 1);
    backend.render(&mut root);
    assert_eq!(dialogs(&root), 2);

    backend.dispatch(&mut root, &escape());
    assert!(!inner.borrow().is_open(), "Escape closes the message box");
    assert!(outer.borrow().is_open(), "the dialog beneath stays open");
    assert_eq!(dialogs(&root), 1);

    backend.dispatch(&mut root, &escape());
    assert!(
        !outer.borrow().is_open(),
        "a second Escape closes that one too"
    );
}

/// Lets the test keep a handle on a dialog the widget tree owns.
struct Shared<W>(Rc<RefCell<W>>);

impl<W: Widget> Widget for Shared<W> {
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
