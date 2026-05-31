//! crud — 7GUIs task 5.
//!
//! "The task is to build a frame containing the following elements: a textfield
//! `Tprefix`, a pair of textfields `Tname` and `Tsurname`, a listbox `L`,
//! buttons `BC`, `BU` and `BD` and the three labels as seen in the screenshot.
//! `L` presents a view of the data in the database that consists of a list of
//! names. At most one entry can be selected in `L` at a time. By entering a
//! string into `Tprefix` the user can filter the names whose surname start with
//! the entered prefix — this should happen immediately without having to submit
//! the prefix with enter. Clicking `BC` will append the resulting name from
//! concatenating the strings in `Tname` and `Tsurname` to `L`. `BU` and `BD`
//! are enabled iff an entry in `L` is selected. In contrast to `BC`, `BU` will
//! not append the resulting name but instead replace the selected entry with
//! the new name. `BD` will remove the selected entry."
//!
//! This is the example that drives the [`List`] widget as a live database
//! view. The "database" is a plain `Vec<Person>` behind a shared cell; the list
//! only ever shows a *filtered projection* of it, so a separate `visible` table
//! maps each on-screen row back to the database index it came from. Every
//! mutation (create / update / delete / filter) recomputes that projection and
//! refills the list.
//!
//! The three action buttons enable themselves reactively. Rather than pushing
//! `set_enabled` into them from every place that could change their state, each
//! button is wrapped in a small [`ActionButton`] that *pulls* its enabled flag
//! from a predicate over the shared state every time it paints or handles an
//! event. Because any state change already triggers a full repaint, the buttons
//! always reflect the current selection and field contents — and a button never
//! has to reach in and mutate itself (which the shared-cell tree forbids while
//! that same button is mid-event). Delete is enabled exactly when a row is
//! selected, per the spec; Create and Update additionally require a non-empty
//! name so the database never grows a blank record. Surname filtering is
//! case-insensitive.

use std::cell::RefCell;
use std::rc::Rc;

use saudade::{
    App, Button, Color, Container, Event, EventCtx, Label, List, ListItem, Painter, Rect,
    TextInput, Theme, Widget, WindowConfig,
};

const W: i32 = 420;
const H: i32 = 268;

const LIST_RECT: Rect = Rect::new(12, 46, 230, 174);

fn main() {
    // The database and its current filtered projection live together so the
    // list rebuild and the button predicates share one source of truth.
    let model = Rc::new(RefCell::new(Model {
        people: vec![
            Person::new("Hans", "Emil"),
            Person::new("Max", "Mustermann"),
            Person::new("Roman", "Tisch"),
        ],
        filter: String::new(),
        visible: Vec::new(),
    }));

    let list = Rc::new(RefCell::new(List::new(LIST_RECT)));
    // Populate the list from the seed data before the first paint. Nothing is
    // selected initially, so Update / Delete start disabled.
    refill(&model, &list, None);

    let name = Rc::new(RefCell::new(TextInput::new(Rect::new(318, 50, 90, 22))));
    let surname = Rc::new(RefCell::new(TextInput::new(Rect::new(318, 80, 90, 22))));

    // Tprefix filters the list live: each keystroke updates the stored prefix
    // and refills, keeping the selected person selected when it still matches.
    let filter = TextInput::new(Rect::new(104, 14, 150, 22)).on_change({
        let model = model.clone();
        let list = list.clone();
        move |cx, text| {
            // Note which database row is selected under the *old* filter before
            // we change it, so it survives the rebuild if it still matches.
            let keep = {
                let m = model.borrow();
                m.selected_db(list.borrow().selected_index())
            };
            model.borrow_mut().filter = text.to_string();
            refill(&model, &list, keep);
            cx.request_paint();
        }
    });

    // BC — append a fresh person built from the two name fields, then select it
    // (it lands in the view only if it matches the current filter).
    let create = ActionButton::new(
        Button::new(Rect::new(12, 232, 80, 26), "Create").on_click({
            let model = model.clone();
            let list = list.clone();
            let name = name.clone();
            let surname = surname.clone();
            move |cx| {
                let person = read_person(&name, &surname);
                let new_db = {
                    let mut m = model.borrow_mut();
                    m.people.push(person);
                    m.people.len() - 1
                };
                refill(&model, &list, Some(new_db));
                cx.request_paint();
            }
        }),
        {
            let name = name.clone();
            let surname = surname.clone();
            move || has_input(&name, &surname)
        },
    );

    // BU — replace the selected person in place, keeping it selected if its new
    // surname still matches the filter.
    let update = ActionButton::new(
        Button::new(Rect::new(100, 232, 80, 26), "Update").on_click({
            let model = model.clone();
            let list = list.clone();
            let name = name.clone();
            let surname = surname.clone();
            move |cx| {
                let target = {
                    let m = model.borrow();
                    m.selected_db(list.borrow().selected_index())
                };
                if let Some(db) = target {
                    let person = read_person(&name, &surname);
                    model.borrow_mut().people[db] = person;
                    refill(&model, &list, Some(db));
                    cx.request_paint();
                }
            }
        }),
        {
            let list = list.clone();
            let name = name.clone();
            let surname = surname.clone();
            move || has_selection(&list) && has_input(&name, &surname)
        },
    );

    // BD — remove the selected person and clear the selection (which disables
    // Update / Delete again until the user picks another row).
    let delete = ActionButton::new(
        Button::new(Rect::new(188, 232, 80, 26), "Delete").on_click({
            let model = model.clone();
            let list = list.clone();
            move |cx| {
                let target = {
                    let m = model.borrow();
                    m.selected_db(list.borrow().selected_index())
                };
                if let Some(db) = target {
                    model.borrow_mut().people.remove(db);
                    refill(&model, &list, None);
                    cx.request_paint();
                }
            }
        }),
        {
            let list = list.clone();
            move || has_selection(&list)
        },
    );

    let root = Container::new(W, H)
        .with_background(Color::LIGHT_GRAY)
        .add(Label::new(Rect::new(12, 18, 90, 16), "Filter prefix:"))
        .add(filter)
        .add(SharedList(list.clone()))
        .add(Label::new(Rect::new(254, 54, 60, 16), "Name:"))
        .add(SharedTextInput(name.clone()))
        .add(Label::new(Rect::new(254, 84, 60, 16), "Surname:"))
        .add(SharedTextInput(surname.clone()))
        .add(create)
        .add(update)
        .add(delete);

    App::new(WindowConfig::new("CRUD", W, H), root)
        .with_theme(Theme::windows_31())
        .run();
}

// ============================================================================
// Model — the database plus its current filtered projection.
// ============================================================================

/// One database record. The list shows it as `"Surname, Name"`.
struct Person {
    name: String,
    surname: String,
}

impl Person {
    fn new(name: impl Into<String>, surname: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            surname: surname.into(),
        }
    }

    fn display(&self) -> String {
        format!("{}, {}", self.surname, self.name)
    }
}

struct Model {
    /// The full database, in insertion order.
    people: Vec<Person>,
    /// The current `Tprefix` value, matched case-insensitively against surnames.
    filter: String,
    /// Database indices currently shown in the list, in display order. Maps
    /// each list row back to the `people` entry it projects.
    visible: Vec<usize>,
}

impl Model {
    /// Rebuild [`Model::visible`] from `people` and the current `filter`.
    fn recompute_visible(&mut self) {
        let prefix = self.filter.to_lowercase();
        self.visible = self
            .people
            .iter()
            .enumerate()
            .filter(|(_, p)| p.surname.to_lowercase().starts_with(&prefix))
            .map(|(i, _)| i)
            .collect();
    }

    /// Translate a list row (display index) into the database index it shows.
    fn selected_db(&self, row: Option<usize>) -> Option<usize> {
        row.and_then(|i| self.visible.get(i).copied())
    }
}

/// Recompute the filtered projection and repopulate `list` from it. The row
/// that ends up selected is whichever one shows database index `keep` — or
/// nothing, if `keep` is `None` or no longer passes the filter.
fn refill(model: &Rc<RefCell<Model>>, list: &Rc<RefCell<List>>, keep: Option<usize>) {
    // Build the new rows and the selection under a single model borrow, then
    // drop it before touching the (separate) list cell.
    let (items, selected) = {
        let mut m = model.borrow_mut();
        m.recompute_visible();
        let items: Vec<ListItem> = m
            .visible
            .iter()
            .map(|&i| ListItem::new(m.people[i].display()))
            .collect();
        let selected = keep.and_then(|db| m.visible.iter().position(|&i| i == db));
        (items, selected)
    };
    let mut l = list.borrow_mut();
    l.set_items(items);
    l.set_selected(selected);
}

/// Read the two name fields into a fresh [`Person`], trimming surrounding
/// whitespace so the database never stores stray padding.
fn read_person(name: &Rc<RefCell<TextInput>>, surname: &Rc<RefCell<TextInput>>) -> Person {
    Person::new(name.borrow().text().trim(), surname.borrow().text().trim())
}

/// `true` once at least one name field carries non-whitespace text.
fn has_input(name: &Rc<RefCell<TextInput>>, surname: &Rc<RefCell<TextInput>>) -> bool {
    !name.borrow().text().trim().is_empty() || !surname.borrow().text().trim().is_empty()
}

/// `true` while a list row is selected.
fn has_selection(list: &Rc<RefCell<List>>) -> bool {
    list.borrow().selected_index().is_some()
}

// ============================================================================
// ActionButton — a Button whose enabled state is *pulled* from a predicate over
// the shared state, re-evaluated on every paint and event. This keeps the
// reactive enable/disable logic in one place and means a button never has to
// mutate itself from inside its own click handler (which would re-borrow the
// shared cell the handler is already running under).
// ============================================================================

struct ActionButton {
    button: Button,
    enabled: Box<dyn Fn() -> bool>,
}

impl ActionButton {
    fn new(button: Button, enabled: impl Fn() -> bool + 'static) -> Self {
        Self {
            button,
            enabled: Box::new(enabled),
        }
    }

    fn sync(&mut self) {
        let enabled = (self.enabled)();
        self.button.set_enabled(enabled);
    }
}

impl Widget for ActionButton {
    fn bounds(&self) -> Rect {
        self.button.bounds()
    }
    fn paint(&mut self, painter: &mut Painter, theme: &Theme) {
        self.sync();
        self.button.paint(painter, theme);
    }
    fn event(&mut self, event: &Event, ctx: &mut EventCtx) {
        self.sync();
        self.button.event(event, ctx);
    }
    fn captures_pointer(&self) -> bool {
        self.button.captures_pointer()
    }
    fn focusable(&self) -> bool {
        // Pull the live flag so a disabled button drops out of Tab cycling
        // immediately, regardless of when it last painted.
        (self.enabled)()
    }
    fn set_focused(&mut self, focused: bool) {
        self.button.set_focused(focused);
    }
    fn layout(&mut self, bounds: Rect) {
        self.button.layout(bounds);
    }
}

// ============================================================================
// Shared adapters — let the callbacks and the widget tree hold the same live
// widget. Identical in shape to the picker / flight_booker examples; see the
// README for the pattern.
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

// ============================================================================
// Tests — the CRUD semantics are pure data manipulation over `Model` + `List`,
// so they run headless (a `List` needs no window). These double as executable
// documentation of the 7GUIs rules. Run with `cargo test --example crud`.
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn seeded() -> (Rc<RefCell<Model>>, Rc<RefCell<List>>) {
        let model = Rc::new(RefCell::new(Model {
            people: vec![
                Person::new("Hans", "Emil"),
                Person::new("Max", "Mustermann"),
                Person::new("Roman", "Tisch"),
            ],
            filter: String::new(),
            visible: Vec::new(),
        }));
        let list = Rc::new(RefCell::new(List::new(LIST_RECT)));
        refill(&model, &list, None);
        (model, list)
    }

    fn labels(list: &Rc<RefCell<List>>) -> Vec<String> {
        list.borrow()
            .items()
            .iter()
            .map(|i| i.label.clone())
            .collect()
    }

    /// The selected database record, mirroring what a button handler reads.
    fn selected_db(model: &Rc<RefCell<Model>>, list: &Rc<RefCell<List>>) -> Option<usize> {
        model.borrow().selected_db(list.borrow().selected_index())
    }

    #[test]
    fn initial_view_lists_everyone_unselected() {
        let (_model, list) = seeded();
        assert_eq!(
            labels(&list),
            ["Emil, Hans", "Mustermann, Max", "Tisch, Roman"]
        );
        assert_eq!(list.borrow().selected_index(), None);
    }

    #[test]
    fn filter_matches_surname_prefix_case_insensitively() {
        let (model, list) = seeded();
        model.borrow_mut().filter = "m".to_string();
        refill(&model, &list, None);
        assert_eq!(labels(&list), ["Mustermann, Max"]);
    }

    #[test]
    fn create_appends_and_selects_the_new_row() {
        let (model, list) = seeded();
        let new_db = {
            let mut m = model.borrow_mut();
            m.people.push(Person::new("Ada", "Lovelace"));
            m.people.len() - 1
        };
        refill(&model, &list, Some(new_db));
        assert_eq!(labels(&list).last().unwrap(), "Lovelace, Ada");
        assert_eq!(selected_db(&model, &list), Some(new_db));
    }

    #[test]
    fn update_replaces_the_selected_entry_in_place() {
        let (model, list) = seeded();
        list.borrow_mut().set_selected(Some(1)); // Mustermann, Max
        let db = selected_db(&model, &list).unwrap();
        model.borrow_mut().people[db] = Person::new("Maria", "Musterfrau");
        refill(&model, &list, Some(db));
        assert_eq!(
            labels(&list),
            ["Emil, Hans", "Musterfrau, Maria", "Tisch, Roman"]
        );
        assert_eq!(list.borrow().selected_index(), Some(1));
    }

    #[test]
    fn delete_removes_the_selected_entry_and_clears_selection() {
        let (model, list) = seeded();
        list.borrow_mut().set_selected(Some(0)); // Emil, Hans
        let db = selected_db(&model, &list).unwrap();
        model.borrow_mut().people.remove(db);
        refill(&model, &list, None);
        assert_eq!(labels(&list), ["Mustermann, Max", "Tisch, Roman"]);
        assert_eq!(list.borrow().selected_index(), None);
    }

    #[test]
    fn selection_survives_a_filter_it_still_matches() {
        let (model, list) = seeded();
        list.borrow_mut().set_selected(Some(1)); // Mustermann, Max
        let keep = selected_db(&model, &list);
        model.borrow_mut().filter = "Must".to_string();
        refill(&model, &list, keep);
        assert_eq!(labels(&list), ["Mustermann, Max"]);
        assert_eq!(list.borrow().selected_index(), Some(0)); // now the only row
    }

    #[test]
    fn selection_is_dropped_when_filtered_out() {
        let (model, list) = seeded();
        list.borrow_mut().set_selected(Some(0)); // Emil, Hans
        let keep = selected_db(&model, &list);
        model.borrow_mut().filter = "T".to_string();
        refill(&model, &list, keep);
        assert_eq!(labels(&list), ["Tisch, Roman"]);
        assert_eq!(list.borrow().selected_index(), None);
    }

    #[test]
    fn input_predicates_track_field_contents() {
        let name = Rc::new(RefCell::new(TextInput::new(LIST_RECT)));
        let surname = Rc::new(RefCell::new(TextInput::new(LIST_RECT)));
        assert!(!has_input(&name, &surname));
        name.borrow_mut().set_text("   "); // whitespace-only still counts as empty
        assert!(!has_input(&name, &surname));
        surname.borrow_mut().set_text("Tisch");
        assert!(has_input(&name, &surname));
        // read_person trims the surrounding whitespace it stored.
        assert_eq!(read_person(&name, &surname).display(), "Tisch, ");
    }
}
