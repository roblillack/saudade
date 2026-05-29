//! picker — a tiny retrogui demo that shows focus cycling across multiple
//! focusable widgets. The window holds a list and two buttons; Tab and
//! Shift+Tab move focus between them, Enter / Space activate the focused
//! button, and the list's own keyboard handlers work whenever it has focus.
//!
//! Clicking (or activating) "OK" pops a confirmation dialog with the
//! currently-picked item; "Cancel" pops a "Cancelled" dialog. Dismissing
//! either dialog closes the window.

use std::cell::RefCell;
use std::rc::Rc;

use retrogui::{
    App, Button, Checkbox, Column, Container, Dialog, Event, EventCtx, Label, List, ListItem,
    Painter, PopupRequest, Rect, Theme, Widget, WindowConfig,
};

const WINDOW_W: i32 = 280;
const WINDOW_H: i32 = 260;

fn main() {
    let items = vec![
        ListItem::new("Anchovy"),
        ListItem::new("Basil"),
        ListItem::new("Capers"),
        ListItem::new("Dill"),
        ListItem::new("Endive"),
        ListItem::new("Fennel"),
        ListItem::new("Garlic"),
    ];
    let mut list = List::new(Rect::new(16, 32, 248, 130)).with_items(items);
    list.set_selected(Some(0));
    let list = Rc::new(RefCell::new(list));

    let favorite = Rc::new(RefCell::new(
        Checkbox::new(Rect::new(16, 170, 200, 16), "Add to favorites"),
    ));

    // The dialog is shared between OK and Cancel. Either one pops it; the
    // shared on_dismiss closes the window when the user clicks the OK
    // inside the dialog (or presses Enter / Escape).
    let dialog = Rc::new(RefCell::new(Dialog::new().on_dismiss(|cx| cx.close())));

    let ok = Button::new(Rect::new(96, 200, 80, 24), "OK")
        .default(true)
        .on_click({
            let list = list.clone();
            let favorite = favorite.clone();
            let dialog = dialog.clone();
            move |cx| {
                let label = {
                    let l = list.borrow();
                    l.selected_index()
                        .and_then(|i| l.items().get(i).map(|x| x.label.clone()))
                };
                let starred = favorite.borrow().is_checked();
                let body = match label {
                    Some(name) if starred => {
                        format!("You picked:\n\n{}\n\nSaved to favorites.", name)
                    }
                    Some(name) => format!("You picked:\n\n{}", name),
                    None => "Nothing was selected.".to_string(),
                };
                dialog.borrow_mut().show_info("Confirmed", body);
                cx.request_paint();
            }
        });

    let cancel = Button::new(Rect::new(184, 200, 80, 24), "Cancel").on_click({
        let dialog = dialog.clone();
        move |cx| {
            dialog
                .borrow_mut()
                .show_info("Cancelled", "No selection was saved.");
            cx.request_paint();
        }
    });

    let content = Container::new(WINDOW_W, WINDOW_H)
        .add(Label::new(16, 12, "Pick an ingredient (Tab to cycle):"))
        .add(SharedList(list.clone()))
        .add(SharedCheckbox(favorite.clone()))
        .add(ok)
        .add(cancel);

    // Column wraps the fixed-size content as its only fill child and adds
    // the dialog as a floating overlay, so the dialog can sit on top of
    // everything else and the runtime can host it in its own top-level
    // window via PopupRequest.
    let root = Column::new()
        .add_fill(content)
        .add_overlay(SharedDialog(dialog.clone()));

    App::new(WindowConfig::new("Picker", WINDOW_W, WINDOW_H), root)
        .with_theme(Theme::windows_31())
        .run();
}

// ============================================================================
// Adapters that let the menu/button callbacks mutate the same List/Dialog
// instances the widget tree owns. Identical in shape to notepad's
// SharedEditor / SharedDialog — see retrogui's README for the pattern.
// ============================================================================

struct SharedList(Rc<RefCell<List>>);

impl Widget for SharedList {
    fn bounds(&self) -> Rect {
        self.0.borrow().bounds()
    }
    fn paint(&mut self, painter: &mut Painter, theme: &Theme) {
        self.0.borrow_mut().paint(painter, theme);
    }
    fn event(&mut self, event: &Event, ctx: &mut EventCtx) {
        self.0.borrow_mut().event(event, ctx);
    }
    fn captures_pointer(&self) -> bool {
        self.0.borrow().captures_pointer()
    }
    fn focusable(&self) -> bool {
        self.0.borrow().focusable()
    }
    fn set_focused(&mut self, focused: bool) {
        self.0.borrow_mut().set_focused(focused);
    }
    fn layout(&mut self, bounds: Rect) {
        self.0.borrow_mut().layout(bounds);
    }
}

struct SharedCheckbox(Rc<RefCell<Checkbox>>);

impl Widget for SharedCheckbox {
    fn bounds(&self) -> Rect {
        self.0.borrow().bounds()
    }
    fn paint(&mut self, painter: &mut Painter, theme: &Theme) {
        self.0.borrow_mut().paint(painter, theme);
    }
    fn event(&mut self, event: &Event, ctx: &mut EventCtx) {
        self.0.borrow_mut().event(event, ctx);
    }
    fn captures_pointer(&self) -> bool {
        self.0.borrow().captures_pointer()
    }
    fn focusable(&self) -> bool {
        self.0.borrow().focusable()
    }
    fn set_focused(&mut self, focused: bool) {
        self.0.borrow_mut().set_focused(focused);
    }
    fn layout(&mut self, bounds: Rect) {
        self.0.borrow_mut().layout(bounds);
    }
}

struct SharedDialog(Rc<RefCell<Dialog>>);

impl Widget for SharedDialog {
    fn bounds(&self) -> Rect {
        self.0.borrow().bounds()
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
    fn layout(&mut self, bounds: Rect) {
        self.0.borrow_mut().layout(bounds);
    }
    fn popup_request(&self) -> Option<PopupRequest> {
        self.0.borrow().popup_request()
    }
}
