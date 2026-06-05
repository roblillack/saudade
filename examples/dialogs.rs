//! dialogs — a gallery of saudade's dialog facilities.
//!
//! A launcher window with one button per dialog kind:
//!
//! * **Message boxes** — [`Dialog::show_info`] / [`show_warning`] /
//!   [`show_error`] / [`show`] (a plain, icon-less notice). One shared
//!   [`Dialog`] drives them all; each just OKs away.
//! * **A confirmation box** — [`Dialog::show_confirm`], whose affirmative button
//!   runs a handler and whose Cancel / Escape / window-close back out. The
//!   dialog's [`on_dismiss`](Dialog::on_dismiss) reports which happened.
//! * **A custom dialog** — a general-purpose [`Modal`] hosting our own body. The
//!   body is just a [`Container`] of standard widgets (a slider + OK / Cancel),
//!   so no bespoke widget is needed; it previews a value live in the launcher
//!   while open, commits on OK, and — via [`Widget::on_cancel`] — reverts on
//!   Cancel, Escape, or the window's close button.
//!
//! [`show_warning`]: Dialog::show_warning
//! [`show_error`]: Dialog::show_error
//! [`show`]: Dialog::show

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use saudade::{
    App, Button, Column, Container, Dialog, DialogIcon, Event, EventCtx, Label, Modal, Painter,
    PopupRequest, Rect, Size, Slider, Theme, Widget, WindowConfig,
};

const W: i32 = 320;
const H: i32 = 320;
/// Logical size of the custom dialog.
const CUSTOM_W: i32 = 280;
const CUSTOM_H: i32 = 160;
/// Standard launcher-button geometry.
const BTN_X: i32 = 16;
const BTN_W: i32 = W - 32;
const BTN_H: i32 = 26;

type SharedStatus = Rc<RefCell<String>>;
type SharedValue = Rc<Cell<i32>>;

fn main() {
    let (.., root) = build_app();
    App::new(WindowConfig::new("Dialogs", W, H), root)
        .with_theme(Theme::windows_31())
        .run();
}

/// Assemble the launcher plus the shared state behind it. Returns the shared
/// handles too, so tests can show each dialog against the same tree the mock
/// backend renders.
#[allow(clippy::type_complexity)]
fn build_app() -> (
    Rc<RefCell<Dialog>>,
    Rc<RefCell<Dialog>>,
    Rc<RefCell<Modal>>,
    SharedValue,
    SharedStatus,
    Column,
) {
    let status: SharedStatus = Rc::new(RefCell::new("Pick a dialog to open.".to_string()));
    let value: SharedValue = Rc::new(Cell::new(42));
    // Set by the confirm box's affirmative handler; read (and cleared) by its
    // on_dismiss to tell a confirm from a cancel.
    let confirmed = Rc::new(Cell::new(false));

    // One message box, reused for the four icon variants.
    let msg = Rc::new(RefCell::new(Dialog::new()));

    // A separate confirm box: its on_dismiss runs on *every* close, so it reads
    // the `confirmed` flag the affirmative handler sets to report the outcome.
    let confirm = Rc::new(RefCell::new(Dialog::new().on_dismiss({
        let status = status.clone();
        let confirmed = confirmed.clone();
        move |cx| {
            *status.borrow_mut() = if confirmed.take() {
                "Confirmed — the file was deleted.".to_string()
            } else {
                "Cancelled — nothing was deleted.".to_string()
            };
            cx.request_paint();
        }
    })));

    // The custom dialog is a general-purpose Modal we fill with our own body.
    let custom = Rc::new(RefCell::new(Modal::new()));

    let mut y = 70;
    let mut next_row = || {
        let r = Rect::new(BTN_X, y, BTN_W, BTN_H);
        y += BTN_H + 6;
        r
    };

    let info = Button::new(next_row(), "Information message").on_click({
        let msg = msg.clone();
        let status = status.clone();
        move |cx| {
            msg.borrow_mut()
                .show_info("Information", "Your changes have been saved.");
            *status.borrow_mut() = "Showed an information box.".to_string();
            cx.request_paint();
        }
    });
    let warning = Button::new(next_row(), "Warning message").on_click({
        let msg = msg.clone();
        let status = status.clone();
        move |cx| {
            msg.borrow_mut()
                .show_warning("Warning", "This may overwrite existing data.");
            *status.borrow_mut() = "Showed a warning box.".to_string();
            cx.request_paint();
        }
    });
    let error = Button::new(next_row(), "Error message").on_click({
        let msg = msg.clone();
        let status = status.clone();
        move |cx| {
            msg.borrow_mut()
                .show_error("Error", "The file could not be opened.");
            *status.borrow_mut() = "Showed an error box.".to_string();
            cx.request_paint();
        }
    });
    let plain = Button::new(next_row(), "Plain message (no icon)").on_click({
        let msg = msg.clone();
        let status = status.clone();
        move |cx| {
            msg.borrow_mut().show(
                "Notice",
                "A plain message with no icon decoration.",
                DialogIcon::None,
            );
            *status.borrow_mut() = "Showed a plain box.".to_string();
            cx.request_paint();
        }
    });
    let confirm_btn = Button::new(next_row(), "Confirmation\u{2026}").on_click({
        let confirm = confirm.clone();
        let confirmed = confirmed.clone();
        let status = status.clone();
        move |cx| {
            confirmed.set(false);
            let confirmed = confirmed.clone();
            confirm.borrow_mut().show_confirm(
                "Delete file?",
                "This permanently deletes the file.\nThis cannot be undone.",
                "Delete",
                move |_| confirmed.set(true),
            );
            *status.borrow_mut() = "Confirm or cancel the deletion\u{2026}".to_string();
            cx.request_paint();
        }
    });
    let custom_btn = Button::new(next_row(), "Custom dialog\u{2026}").on_click({
        let custom = custom.clone();
        let value = value.clone();
        let status = status.clone();
        move |cx| {
            custom.borrow_mut().show(
                "Set value",
                Size::new(CUSTOM_W, CUSTOM_H),
                Box::new(NumberDialog::new(value.clone(), status.clone())),
            );
            cx.request_paint();
        }
    });

    let buttons = Container::new(W, H)
        .add(info)
        .add(warning)
        .add(error)
        .add(plain)
        .add(confirm_btn)
        .add(custom_btn);

    let launcher = Launcher {
        value: value.clone(),
        status: status.clone(),
        buttons,
        rect: Rect::new(0, 0, 0, 0),
    };

    // The dialogs float as overlays so the runtime can host each in its own
    // top-level window; only one is ever open at a time.
    let root = Column::new()
        .add_fill(launcher)
        .add_overlay(SharedDialog(msg.clone()))
        .add_overlay(SharedDialog(confirm.clone()))
        .add_overlay(SharedModal(custom.clone()));

    (msg, confirm, custom, value, status, root)
}

// ============================================================================
// Launcher — the main window: a heading, the live value, the launcher buttons,
// and a status line. A custom widget so the value / status text can be read
// straight from the shared state at paint time.
// ============================================================================

struct Launcher {
    value: SharedValue,
    status: SharedStatus,
    buttons: Container,
    rect: Rect,
}

impl Widget for Launcher {
    fn bounds(&self) -> Rect {
        self.rect
    }

    fn layout(&mut self, bounds: Rect) {
        self.rect = bounds;
        self.buttons.layout(bounds);
    }

    fn paint(&mut self, painter: &mut Painter, theme: &Theme) {
        painter.text(
            self.rect.x + 16,
            self.rect.y + 14,
            "Dialogs",
            16.0,
            theme.text,
        );
        painter.text(
            self.rect.x + 16,
            self.rect.y + 40,
            &format!("Current value: {}", self.value.get()),
            theme.font_size,
            theme.text,
        );
        self.buttons.paint(painter, theme);
        painter.text(
            self.rect.x + 16,
            self.rect.bottom() - 26,
            &self.status.borrow(),
            theme.font_size,
            theme.disabled_text,
        );
    }

    fn event(&mut self, event: &Event, ctx: &mut EventCtx) {
        self.buttons.event(event, ctx);
    }

    fn focusable(&self) -> bool {
        self.buttons.focusable()
    }

    fn focus_first(&mut self) -> bool {
        self.buttons.focus_first()
    }

    fn captures_pointer(&self) -> bool {
        self.buttons.captures_pointer()
    }
}

// ============================================================================
// NumberDialog — a custom modal body. Just a Container of standard widgets that
// previews `value` live (the launcher reads it while the dialog is open), keeps
// the change on OK, and reverts it on Cancel / Escape / window-close.
// ============================================================================

struct NumberDialog {
    value: SharedValue,
    /// The value the dialog opened at — what a cancel restores.
    original: i32,
    status: SharedStatus,
    body: Container,
    rect: Rect,
}

impl NumberDialog {
    fn new(value: SharedValue, status: SharedStatus) -> Self {
        let original = value.get();
        let local = Rect::new(0, 0, CUSTOM_W, CUSTOM_H);

        // The slider writes the shared value live, so both this dialog's readout
        // and the launcher behind it update as it moves.
        let slider = Slider::new(Self::slider_rect(local), 0, 100)
            .with_value(original)
            .on_change({
                let value = value.clone();
                move |cx, v| {
                    value.set(v);
                    cx.request_paint();
                }
            });
        let ok = Button::new(Self::ok_rect(local), "OK")
            .default(true)
            .on_click({
                let value = value.clone();
                let status = status.clone();
                move |cx| {
                    *status.borrow_mut() = format!("Kept the value at {}.", value.get());
                    cx.request_dismiss();
                }
            });
        let cancel = Button::new(Self::cancel_rect(local), "Cancel").on_click({
            let value = value.clone();
            let status = status.clone();
            move |cx| {
                revert(&value, original, &status);
                cx.request_paint();
                cx.request_dismiss();
            }
        });

        let body = Container::new(CUSTOM_W, CUSTOM_H)
            .add(Label::new(
                Rect::new(16, 14, CUSTOM_W - 32, 16),
                "Drag to set the value (previews live):",
            ))
            .add(slider)
            .add(ok)
            .add(cancel);

        Self {
            value,
            original,
            status,
            body,
            rect: Rect::new(0, 0, 0, 0),
        }
    }

    fn slider_rect(base: Rect) -> Rect {
        Rect::new(base.x + 16, base.y + 84, base.w - 32, 20)
    }

    fn ok_rect(base: Rect) -> Rect {
        Rect::new(
            base.right() - 16 - 70 - 10 - 70,
            base.bottom() - 16 - 26,
            70,
            26,
        )
    }

    fn cancel_rect(base: Rect) -> Rect {
        Rect::new(base.right() - 16 - 70, base.bottom() - 16 - 26, 70, 26)
    }
}

/// Restore the value the dialog opened at and note it in the status line.
fn revert(value: &SharedValue, original: i32, status: &SharedStatus) {
    value.set(original);
    *status.borrow_mut() = format!("Reverted to {original}.");
}

impl Widget for NumberDialog {
    fn bounds(&self) -> Rect {
        self.rect
    }

    fn layout(&mut self, bounds: Rect) {
        self.rect = bounds;
        self.body.layout(bounds);
    }

    fn paint(&mut self, painter: &mut Painter, theme: &Theme) {
        painter.text_centered(
            Rect::new(self.rect.x, self.rect.y + 36, self.rect.w, 32),
            &self.value.get().to_string(),
            28.0,
            theme.text,
        );
        self.body.paint(painter, theme);
    }

    fn event(&mut self, event: &Event, ctx: &mut EventCtx) {
        self.body.event(event, ctx);
    }

    /// Cancelling — Escape, or the window's close button — reverts the live
    /// preview, exactly like the Cancel button. OK doesn't go through here, so
    /// the kept value survives.
    fn on_cancel(&mut self, ctx: &mut EventCtx) {
        revert(&self.value, self.original, &self.status);
        ctx.request_paint();
    }

    fn focusable(&self) -> bool {
        self.body.focusable()
    }

    fn focus_first(&mut self) -> bool {
        self.body.focus_first()
    }

    fn captures_pointer(&self) -> bool {
        self.body.captures_pointer()
    }
}

// ============================================================================
// Adapters that let the button callbacks and the widget tree share one Dialog /
// Modal — the same Rc<RefCell<…>> trick the other examples use.
// ============================================================================

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
    fn collect_popups(&self, out: &mut Vec<PopupRequest>) {
        self.0.borrow().collect_popups(out);
    }
    fn wants_ticks(&self) -> bool {
        self.0.borrow().wants_ticks()
    }
}

struct SharedModal(Rc<RefCell<Modal>>);

impl Widget for SharedModal {
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
    fn collect_popups(&self, out: &mut Vec<PopupRequest>) {
        self.0.borrow().collect_popups(out);
    }
    fn wants_ticks(&self) -> bool {
        self.0.borrow().wants_ticks()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use saudade::mock::MockBackend;
    use saudade::{Key, NamedKey};

    fn open_custom(custom: &Rc<RefCell<Modal>>, value: &SharedValue, status: &SharedStatus) {
        custom.borrow_mut().show(
            "Set value",
            Size::new(CUSTOM_W, CUSTOM_H),
            Box::new(NumberDialog::new(value.clone(), status.clone())),
        );
    }

    fn key(named: NamedKey, down: bool) -> Event {
        let key = Key::Named(named);
        let modifiers = Default::default();
        if down {
            Event::KeyDown { key, modifiers }
        } else {
            Event::KeyUp { key, modifiers }
        }
    }

    /// Every dialog kind lays out and renders (main pass + popup pass) without
    /// panicking — including the confirm box, which nothing else exercises.
    #[test]
    fn every_dialog_kind_renders() {
        let (msg, confirm, custom, value, status, mut root) = build_app();
        let backend = MockBackend::new(W, H);
        backend.render(&mut root);

        msg.borrow_mut().show_info("Information", "Saved.");
        backend.render(&mut root);
        msg.borrow_mut().dismiss();

        msg.borrow_mut().show_warning("Warning", "Careful.");
        backend.render(&mut root);
        msg.borrow_mut().dismiss();

        msg.borrow_mut().show_error("Error", "Failed.");
        backend.render(&mut root);
        msg.borrow_mut().dismiss();

        msg.borrow_mut().show("Notice", "Plain.", DialogIcon::None);
        backend.render(&mut root);
        msg.borrow_mut().dismiss();

        confirm
            .borrow_mut()
            .show_confirm("Delete?", "Sure?", "Delete", |_| {});
        backend.render(&mut root);
        confirm.borrow_mut().dismiss();

        open_custom(&custom, &value, &status);
        backend.render(&mut root);
        custom.borrow_mut().dismiss();
    }

    /// Escape cancels the custom dialog and reverts the live-previewed value —
    /// the `on_cancel` revert, driven through the `Modal` by a real Escape.
    #[test]
    fn custom_dialog_reverts_on_escape() {
        let (.., custom, value, status, mut root) = build_app();
        let backend = MockBackend::new(W, H);
        open_custom(&custom, &value, &status);
        backend.render(&mut root); // lays the dialog out

        value.set(80); // stand in for a slider drag
        backend.dispatch(&mut root, &key(NamedKey::Escape, true));

        assert_eq!(value.get(), 42, "Escape reverts to the opening value");
        assert!(!custom.borrow().is_open(), "Escape closes the dialog");
    }

    /// Enter fires the default OK button on release, which keeps the value.
    #[test]
    fn custom_dialog_keeps_value_on_ok() {
        let (.., custom, value, status, mut root) = build_app();
        let backend = MockBackend::new(W, H);
        open_custom(&custom, &value, &status);
        backend.render(&mut root);

        value.set(80);
        backend.dispatch(&mut root, &key(NamedKey::Enter, true)); // arms OK
        backend.dispatch(&mut root, &key(NamedKey::Enter, false)); // fires it

        assert_eq!(value.get(), 80, "OK keeps the previewed value");
        assert!(!custom.borrow().is_open(), "OK closes the dialog");
    }
}
