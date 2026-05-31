//! flight_booker — 7GUIs task 3.
//!
//! "A combobox C with two options 'one-way flight' and 'return flight', two
//! textfields T1 and T2 for the start and return date, and a button B for
//! submitting the selected flight. T2 is enabled iff C is 'return flight'.
//! When C is 'return flight' and T2's date is strictly before T1's, B is
//! disabled. When a non-disabled textfield carries an ill-formatted date it is
//! marked invalid and B is disabled. Clicking B shows a confirmation."
//!
//! This is the example that drives the [`Dropdown`] widget — picking a flight
//! type reactively enables / disables the return-date field and re-validates
//! the form. Dates use the 7GUIs `D.M.YYYY` format (e.g. `27.03.2014`).
//!
//! Per the widget set's conventions we surface "ill-formatted" not by coloring
//! the field but with a small red [`Label`] beside it — `TextInput` /
//! `Dropdown` / `Button` instead grow a proper *disabled* state, which is what
//! the return field and the Book button toggle through here.

use std::cell::RefCell;
use std::rc::Rc;

use saudade::{
    App, Button, Color, Column, Container, Dialog, Dropdown, Event, EventCtx, Label, Painter,
    PopupRequest, Rect, TextInput, Theme, Widget, WindowConfig,
};

const W: i32 = 300;
const H: i32 = 170;
/// Index of the "return flight" entry in the dropdown.
const RETURN: usize = 1;
const INITIAL_DATE: &str = "27.03.2014";

fn main() {
    let flight_type = Rc::new(RefCell::new(
        Dropdown::new(Rect::new(16, 16, 268, 24)).with_items(["one-way flight", "return flight"]),
    ));
    let start = Rc::new(RefCell::new(
        TextInput::new(Rect::new(16, 52, 170, 24)).with_text(INITIAL_DATE),
    ));
    let back = Rc::new(RefCell::new(
        // The return field starts disabled — the dropdown defaults to one-way.
        TextInput::new(Rect::new(16, 84, 170, 24))
            .with_text(INITIAL_DATE)
            .with_enabled(false),
    ));
    let start_err = Rc::new(RefCell::new(
        Label::new(Rect::new(192, 57, 92, 16), "").with_color(Color::RED),
    ));
    let back_err = Rc::new(RefCell::new(
        Label::new(Rect::new(192, 89, 92, 16), "").with_color(Color::RED),
    ));
    let dialog = Rc::new(RefCell::new(Dialog::new()));

    // Book button — *not* a default button, so its Enter accelerator never
    // competes with the dropdown's "Enter commits the highlighted row" while
    // the list is open. Clicking it (only possible while enabled) confirms.
    let book = Rc::new(RefCell::new(
        Button::new(Rect::new(16, 124, 268, 26), "Book").on_click({
            let flight_type = flight_type.clone();
            let start = start.clone();
            let back = back.clone();
            let dialog = dialog.clone();
            move |cx| {
                let is_return = flight_type.borrow().selected_index() == Some(RETURN);
                let start_date = start.borrow().text();
                let message = if is_return {
                    format!(
                        "You have booked a return flight\nleaving {}\nand returning {}.",
                        start_date,
                        back.borrow().text(),
                    )
                } else {
                    format!("You have booked a one-way flight\non {}.", start_date)
                };
                dialog.borrow_mut().show_info("Flight booked", message);
                cx.request_paint();
            }
        }),
    ));

    // Re-validate whenever the flight type or either date changes.
    flight_type.borrow_mut().set_on_change({
        let start = start.clone();
        let back = back.clone();
        let book = book.clone();
        let start_err = start_err.clone();
        let back_err = back_err.clone();
        move |cx, idx| {
            let is_return = idx == RETURN;
            // Safe to mutate `back` here: this runs from the dropdown's event,
            // not the return field's.
            back.borrow_mut().set_enabled(is_return);
            let start_text = start.borrow().text();
            let back_text = back.borrow().text();
            revalidate(
                &start_text,
                &back_text,
                is_return,
                &book,
                &start_err,
                &back_err,
            );
            cx.request_paint();
        }
    });

    start.borrow_mut().set_on_change({
        let flight_type = flight_type.clone();
        let back = back.clone();
        let book = book.clone();
        let start_err = start_err.clone();
        let back_err = back_err.clone();
        move |cx, text| {
            let is_return = flight_type.borrow().selected_index() == Some(RETURN);
            let back_text = back.borrow().text();
            revalidate(text, &back_text, is_return, &book, &start_err, &back_err);
            cx.request_paint();
        }
    });

    back.borrow_mut().set_on_change({
        let flight_type = flight_type.clone();
        let start = start.clone();
        let book = book.clone();
        let start_err = start_err.clone();
        let back_err = back_err.clone();
        move |cx, text| {
            let is_return = flight_type.borrow().selected_index() == Some(RETURN);
            let start_text = start.borrow().text();
            revalidate(&start_text, text, is_return, &book, &start_err, &back_err);
            cx.request_paint();
        }
    });

    let content = Container::new(W, H)
        .with_background(Color::WHITE)
        .add(SharedTextInput(start.clone()))
        .add(SharedTextInput(back.clone()))
        .add(SharedLabel(start_err.clone()))
        .add(SharedLabel(back_err.clone()))
        .add(SharedButton(book.clone()))
        // The dropdown is added last so its popup paints above its siblings in
        // the overlay pass.
        .add(SharedDropdown(flight_type.clone()));

    // Like the picker example: wrap the fixed-size form as the fill child and
    // float the dialog as an overlay so the runtime can host it in its own
    // top-level window.
    let root = Column::new()
        .add_fill(content)
        .add_overlay(SharedDialog(dialog.clone()));

    App::new(WindowConfig::new("Flight Booker", W, H), root)
        .with_theme(Theme::windows_31())
        .run();
}

/// Parse a 7GUIs `D.M.YYYY` date into a `(year, month, day)` tuple that orders
/// chronologically. Returns `None` for anything ill-formatted.
fn parse_date(text: &str) -> Option<(u32, u32, u32)> {
    let parts: Vec<&str> = text.trim().split('.').collect();
    if parts.len() != 3 {
        return None;
    }
    let day: u32 = parts[0].parse().ok()?;
    let month: u32 = parts[1].parse().ok()?;
    let year: u32 = parts[2].parse().ok()?;
    if !(1..=31).contains(&day) || !(1..=12).contains(&month) {
        return None;
    }
    Some((year, month, day))
}

/// Recompute the form's validity and push it into the widgets: the Book button
/// is enabled only when every relevant date is well-formed and in order, and a
/// red marker is shown beside any enabled field whose date is ill-formatted.
fn revalidate(
    start_text: &str,
    back_text: &str,
    is_return: bool,
    book: &Rc<RefCell<Button>>,
    start_err: &Rc<RefCell<Label>>,
    back_err: &Rc<RefCell<Label>>,
) {
    let start_date = parse_date(start_text);
    let back_date = parse_date(back_text);

    let start_bad = start_date.is_none();
    // The return date only matters — and can only be "invalid" — for a return
    // flight, when its field is enabled.
    let back_bad = is_return && back_date.is_none();
    let out_of_order =
        matches!((is_return, start_date, back_date), (true, Some(s), Some(b)) if b < s);

    book.borrow_mut()
        .set_enabled(!start_bad && !back_bad && !out_of_order);
    start_err.borrow_mut().text = invalid_text(start_bad);
    back_err.borrow_mut().text = invalid_text(back_bad);
}

fn invalid_text(bad: bool) -> String {
    if bad {
        "invalid date".to_string()
    } else {
        String::new()
    }
}

// ============================================================================
// Shared adapters — the same pattern as the picker / timer examples, letting
// the callbacks and the widget tree both hold the live widget. See the README.
// ============================================================================

struct SharedDropdown(Rc<RefCell<Dropdown>>);

impl Widget for SharedDropdown {
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
    fn focusable(&self) -> bool {
        self.0.borrow().focusable()
    }
    fn set_focused(&mut self, focused: bool) {
        self.0.borrow_mut().set_focused(focused);
    }
    fn layout(&mut self, bounds: Rect) {
        self.0.borrow_mut().layout(bounds);
    }
    fn popup_request(&self) -> Option<PopupRequest> {
        self.0.borrow().popup_request()
    }
}

struct SharedTextInput(Rc<RefCell<TextInput>>);

impl Widget for SharedTextInput {
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
    fn wants_ticks(&self) -> bool {
        self.0.borrow().wants_ticks()
    }
}

struct SharedButton(Rc<RefCell<Button>>);

impl Widget for SharedButton {
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
    fn accepts_accelerators(&self) -> bool {
        self.0.borrow().accepts_accelerators()
    }
    fn layout(&mut self, bounds: Rect) {
        self.0.borrow_mut().layout(bounds);
    }
}

struct SharedLabel(Rc<RefCell<Label>>);

impl Widget for SharedLabel {
    fn bounds(&self) -> Rect {
        self.0.borrow().bounds()
    }
    fn paint(&mut self, painter: &mut Painter, theme: &Theme) {
        self.0.borrow_mut().paint(painter, theme);
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
