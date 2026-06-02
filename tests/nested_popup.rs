//! A [`Dropdown`] opened *inside* a [`Modal`] must surface its list as a second
//! popup nested on top of the dialog — that's what lets the runtime host it in
//! its own window instead of clipping it to the dialog.

use saudade::mock::MockBackend;
use saudade::{
    Color, Column, Dropdown, Event, EventCtx, Modal, Painter, PopupKind, PopupRequest, Rect, Size,
    Theme, Widget,
};

/// Modal content hosting a single dropdown; delegates the popup request so the
/// hosting `Modal` can surface it.
struct Body {
    dd: Dropdown,
    rect: Rect,
}

impl Body {
    fn new() -> Self {
        let mut dd = Dropdown::new(Rect::new(0, 0, 10, 10))
            .with_items(["1920 x 1080", "1280 x 720", "800 x 600"])
            .with_selected(0);
        dd.open();
        Self {
            dd,
            rect: Rect::new(0, 0, 0, 0),
        }
    }
}

impl Widget for Body {
    fn bounds(&self) -> Rect {
        self.rect
    }
    fn layout(&mut self, b: Rect) {
        self.rect = b;
        self.dd.set_rect(Rect::new(b.x + 16, b.y + 16, 160, 24));
    }
    fn paint(&mut self, p: &mut Painter, t: &Theme) {
        self.dd.paint(p, t);
    }
    fn paint_overlay(&mut self, p: &mut Painter, t: &Theme) {
        self.dd.paint_overlay(p, t);
    }
    fn event(&mut self, e: &Event, c: &mut EventCtx) {
        self.dd.event(e, c);
    }
    fn focusable(&self) -> bool {
        true
    }
    fn set_focused(&mut self, f: bool) {
        self.dd.set_focused(f);
    }
    fn captures_pointer(&self) -> bool {
        self.dd.captures_pointer()
    }
    fn popup_request(&self) -> Option<PopupRequest> {
        self.dd.popup_request()
    }
}

fn open_modal() -> Modal {
    let mut modal = Modal::new();
    modal.layout(Rect::new(0, 0, 400, 320)); // establish parent bounds
    modal.show("Display", Size::new(240, 160), Box::new(Body::new()));
    modal.layout(Rect::new(0, 0, 400, 320)); // lay the content out
    modal
}

#[test]
fn dropdown_in_dialog_surfaces_two_nested_popups() {
    let modal = open_modal();
    let mut popups = Vec::new();
    modal.collect_popups(&mut popups);

    assert_eq!(popups.len(), 2, "dialog window + dropdown list window");
    assert_eq!(popups[0].kind, PopupKind::Dialog);
    assert_eq!(popups[1].kind, PopupKind::Popup);

    // The dropdown's list nests below its field, which sits inside the dialog.
    assert!(
        popups[1].rect.y >= popups[0].rect.y,
        "the list should hang off the dialog, not float above it"
    );
}

#[test]
fn closed_dropdown_surfaces_only_the_dialog() {
    let mut modal = open_modal();
    // Esc reaches the dialog content via the runtime; here just dismiss the
    // dropdown by clicking its field (the runtime routes popup clicks in root
    // coords). Simpler: rebuild with a closed dropdown.
    modal.dismiss();
    modal.layout(Rect::new(0, 0, 400, 320));
    modal.show("Display", Size::new(240, 160), {
        let mut dd = Dropdown::new(Rect::new(0, 0, 10, 10)).with_items(["a", "b"]);
        dd.set_selected(Some(0));
        Box::new(ClosedBody {
            dd,
            rect: Rect::new(0, 0, 0, 0),
        })
    });
    modal.layout(Rect::new(0, 0, 400, 320));

    let mut popups = Vec::new();
    modal.collect_popups(&mut popups);
    assert_eq!(
        popups.len(),
        1,
        "only the dialog when the dropdown is closed"
    );
    assert_eq!(popups[0].kind, PopupKind::Dialog);
}

struct ClosedBody {
    dd: Dropdown,
    rect: Rect,
}
impl Widget for ClosedBody {
    fn bounds(&self) -> Rect {
        self.rect
    }
    fn layout(&mut self, b: Rect) {
        self.rect = b;
        self.dd.set_rect(Rect::new(b.x + 16, b.y + 16, 160, 24));
    }
    fn paint(&mut self, p: &mut Painter, t: &Theme) {
        self.dd.paint(p, t);
    }
    fn popup_request(&self) -> Option<PopupRequest> {
        self.dd.popup_request()
    }
}

#[test]
fn nested_dropdown_renders_without_panicking() {
    // Drive the whole compositing path (main pass + both popup passes) through
    // the mock backend, with the modal hosted as a Column overlay exactly like
    // a real app, to make sure the stack renders cleanly.
    let mut root = Column::new().add_overlay(open_modal());
    let snap = MockBackend::new(400, 320).render(&mut root);
    // Sanity: the dialog + dropdown drew something over the plain background.
    assert!(snap.pixels().iter().any(|&p| p != Color::WHITE.0));
}
