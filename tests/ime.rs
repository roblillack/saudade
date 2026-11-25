//! Text from an input method: the focused text widget asks for the input
//! method while it has the focus, and only then; composed text goes in once,
//! as if typed; a composition in progress shows but is not part of the text.

use std::cell::RefCell;
use std::rc::Rc;

use saudade::mock::MockBackend;
use saudade::{
    Button, Column, Container, Event, EventCtx, Font, Ime, Key, Label, Modal, Modifiers,
    MouseButton, NamedKey, NumberInput, Painter, Point, PopupRequest, Rect, Size, TextEditor,
    TextInput, Theme, Widget,
};

const WINDOW: Size = Size::new(240, 200);
const FIELD: Rect = Rect::new(10, 10, 120, 22);
const LABEL: Rect = Rect::new(10, 40, 120, 16);
const BUTTON: Rect = Rect::new(10, 70, 80, 24);

/// A form of a text field, a label and a button, with a handle on the field.
fn form() -> (Container, Rc<RefCell<TextInput>>) {
    let field = Rc::new(RefCell::new(TextInput::new(FIELD)));
    let root = Container::new(WINDOW.w, WINDOW.h)
        .add(Shared(field.clone()))
        .add(Label::new(LABEL, "Just a label"))
        .add(Button::new(BUTTON, "OK"));
    (root, field)
}

fn center(rect: Rect) -> Point {
    Point::new(rect.x + rect.w / 2, rect.y + rect.h / 2)
}

/// Click at `pos`, returning where the input method is anchored after it.
fn click(backend: &MockBackend, root: &mut dyn Widget, pos: Point) -> Option<Rect> {
    let modifiers = Modifiers::default();
    let button = MouseButton::Left;
    backend.dispatch(
        root,
        &Event::PointerDown {
            pos,
            button,
            modifiers,
        },
    );
    backend
        .dispatch(
            root,
            &Event::PointerUp {
                pos,
                button,
                modifiers,
            },
        )
        .ime_area
}

fn key(backend: &MockBackend, root: &mut dyn Widget, key: NamedKey, shift: bool) -> Option<Rect> {
    let modifiers = Modifiers {
        shift,
        ..Modifiers::default()
    };
    let key = Key::Named(key);
    backend.dispatch(root, &Event::KeyDown { key, modifiers });
    backend
        .dispatch(root, &Event::KeyUp { key, modifiers })
        .ime_area
}

fn type_char(backend: &MockBackend, root: &mut dyn Widget, ch: char) {
    let modifiers = Modifiers::default();
    backend.dispatch(root, &Event::Char { ch, modifiers });
}

fn preedit(text: &str) -> Event {
    let len = text.len();
    Event::Ime(Ime::Preedit {
        text: text.to_string(),
        cursor: Some((len, len)),
    })
}

fn commit(text: &str) -> Event {
    Event::Ime(Ime::Commit(text.to_string()))
}

#[test]
fn a_focused_field_asks_for_the_input_method_at_its_caret() {
    let (mut root, _) = form();
    let backend = MockBackend::new(WINDOW.w, WINDOW.h);
    assert_eq!(backend.ime_area(), None, "nothing has the focus yet");

    let area = click(&backend, &mut root, center(FIELD)).expect("the field takes text");
    assert!(FIELD.contains(Point::new(area.x, area.y)), "{area:?}");
    assert!(area.h > 0 && area.bottom() <= FIELD.bottom());
}

#[test]
fn the_input_method_follows_the_focus_on_and_off() {
    let (mut root, _) = form();
    let backend = MockBackend::new(WINDOW.w, WINDOW.h);
    assert!(click(&backend, &mut root, center(FIELD)).is_some());

    // A click on something that takes no focus leaves it with the field.
    assert!(click(&backend, &mut root, center(LABEL)).is_some());

    // Tab moves the focus on to the button, which takes no text …
    assert_eq!(key(&backend, &mut root, NamedKey::Tab, false), None);
    // … and Shift+Tab back to the field, though the field heard of neither.
    assert!(key(&backend, &mut root, NamedKey::Tab, true).is_some());

    // A click on the button gives it the focus.
    assert_eq!(click(&backend, &mut root, center(BUTTON)), None);
}

#[test]
fn the_first_focus_is_asked_about_too() {
    let (mut root, _) = form();
    let backend = MockBackend::new(WINDOW.w, WINDOW.h);
    root.focus_first();
    // What the runtime does once it has given the first focus, so the first
    // key typed goes through the input method too.
    assert!(backend.probe_ime(&mut root).ime_area.is_some());
}

#[test]
fn composed_text_goes_in_once_and_the_composition_is_not_text() {
    let (mut root, field) = form();
    let backend = MockBackend::new(WINDOW.w, WINDOW.h);
    click(&backend, &mut root, center(FIELD));

    type_char(&backend, &mut root, 'a');
    // A dead key: the accent waits for its letter, shown but not typed.
    let outcome = backend.dispatch(&mut root, &preedit("´"));
    assert!(outcome.paint_requested);
    assert!(outcome.ime_area.is_some());
    assert_eq!(field.borrow().text(), "a");
    // A click on the label asks the field again — with the composition
    // that's going on, which changes nothing.
    click(&backend, &mut root, center(LABEL));
    assert_eq!(field.borrow().text(), "a");

    backend.dispatch(&mut root, &preedit(""));
    backend.dispatch(&mut root, &commit("é"));
    assert_eq!(field.borrow().text(), "aé");
    // Whatever follows asks again, and adds nothing.
    type_char(&backend, &mut root, 'b');
    key(&backend, &mut root, NamedKey::Left, false);
    assert_eq!(field.borrow().text(), "aéb");
}

#[test]
fn a_commit_replaces_the_selection() {
    let (mut root, field) = form();
    field.borrow_mut().set_text("abc");
    let backend = MockBackend::new(WINDOW.w, WINDOW.h);
    click(&backend, &mut root, center(FIELD));
    field.borrow_mut().select_all();
    backend.dispatch(&mut root, &commit("日本"));
    assert_eq!(field.borrow().text(), "日本");
}

#[test]
fn a_field_without_the_focus_takes_no_composed_text() {
    let (mut root, field) = form();
    let backend = MockBackend::new(WINDOW.w, WINDOW.h);
    click(&backend, &mut root, center(BUTTON));
    let outcome = backend.dispatch(&mut root, &commit("é"));
    assert_eq!(outcome.ime_area, None);
    assert_eq!(field.borrow().text(), "");
}

/// The bundled DejaVu fonts, so a render has text in it to compare.
fn backend_with_fonts() -> MockBackend {
    let sans = Font::from_sans_bytes(include_bytes!("fonts/DejaVuSans.ttf").to_vec()).unwrap();
    let mono = Font::from_sans_bytes(include_bytes!("fonts/DejaVuSansMono.ttf").to_vec()).unwrap();
    MockBackend::new(WINDOW.w, WINDOW.h)
        .with_sans_font(sans)
        .with_mono_font(mono)
}

#[test]
fn a_composition_shows_in_the_field_and_goes_with_the_focus() {
    let (mut root, field) = form();
    field.borrow_mut().set_text("Caf");
    let backend = backend_with_fonts();
    // Measured, so a click lands on a character.
    backend.render(&mut root);
    click(&backend, &mut root, center(FIELD));
    let plain = field_pixels(&backend, &mut root);

    backend.dispatch(&mut root, &preedit("´"));
    let composing = field_pixels(&backend, &mut root);
    assert!(plain != composing, "the accent shows");
    assert_eq!(field.borrow().text(), "Caf");

    // Off to the button and back: the input method was turned off on the
    // way, and the composition went with it.
    click(&backend, &mut root, center(BUTTON));
    click(&backend, &mut root, center(FIELD));
    assert_eq!(field.borrow().text(), "Caf");
    let back = field_pixels(&backend, &mut root);
    assert!(plain == back, "the accent is gone");
}

/// Render `root` (at a scale of 1) and cut the text field out of it.
fn field_pixels(backend: &MockBackend, root: &mut dyn Widget) -> Vec<u32> {
    let snapshot = backend.render(root);
    let width = snapshot.width() as usize;
    (FIELD.y..FIELD.bottom())
        .flat_map(|y| {
            let row = y as usize * width;
            snapshot.pixels()[row + FIELD.x as usize..row + FIELD.right() as usize].to_vec()
        })
        .collect()
}

/// An editor filling the window, with a handle on it.
fn editor() -> (Column, Rc<RefCell<TextEditor>>) {
    let editor = Rc::new(RefCell::new(TextEditor::new(Rect::new(0, 0, 200, 120))));
    let root = Column::new().add_fill(Shared(editor.clone()));
    (root, editor)
}

fn cmd_z() -> Event {
    let modifiers = Modifiers {
        control: !cfg!(target_os = "macos"),
        logo: cfg!(target_os = "macos"),
        ..Modifiers::default()
    };
    Event::KeyDown {
        key: Key::Char('z'),
        modifiers,
    }
}

#[test]
fn an_editor_takes_composed_text_as_typing() {
    let (mut root, editor) = editor();
    let backend = MockBackend::new(WINDOW.w, WINDOW.h);
    root.layout(Rect::new(0, 0, WINDOW.w, WINDOW.h));
    click(&backend, &mut root, Point::new(50, 30));
    assert!(backend.ime_area().is_some());

    type_char(&backend, &mut root, 'C');
    type_char(&backend, &mut root, 'a');
    type_char(&backend, &mut root, 'f');
    backend.dispatch(&mut root, &preedit("´"));
    assert_eq!(
        editor.borrow().text(),
        "Caf",
        "a composition is no text yet"
    );
    backend.dispatch(&mut root, &commit("é"));
    assert_eq!(editor.borrow().text(), "Café");

    // One step with the typing around it, as a typed é would be.
    backend.dispatch(&mut root, &cmd_z());
    assert_eq!(editor.borrow().text(), "");
}

#[test]
fn an_editor_takes_no_text_while_read_only_or_its_menu_is_open() {
    let (mut root, editor) = editor();
    let backend = MockBackend::new(WINDOW.w, WINDOW.h);
    root.layout(Rect::new(0, 0, WINDOW.w, WINDOW.h));
    click(&backend, &mut root, Point::new(50, 30));
    assert!(backend.ime_area().is_some());

    // A right-click opens the editing menu, which has the keyboard.
    let pos = Point::new(60, 40);
    let modifiers = Modifiers::default();
    let button = MouseButton::Right;
    let outcome = backend.dispatch(
        &mut root,
        &Event::PointerDown {
            pos,
            button,
            modifiers,
        },
    );
    assert_eq!(outcome.ime_area, None);
    // Escape closes it, and the editor takes text again.
    assert!(key(&backend, &mut root, NamedKey::Escape, false).is_some());

    editor.borrow_mut().set_read_only(true);
    assert_eq!(key(&backend, &mut root, NamedKey::Right, false), None);
    backend.dispatch(&mut root, &commit("é"));
    assert_eq!(editor.borrow().text(), "");
}

#[test]
fn a_number_field_takes_full_width_digits_and_nothing_else() {
    let number = Rc::new(RefCell::new(
        NumberInput::new(FIELD).with_range(0.0, 1000.0),
    ));
    let mut root = Container::new(WINDOW.w, WINDOW.h).add(Shared(number.clone()));
    let backend = MockBackend::new(WINDOW.w, WINDOW.h);
    assert!(click(&backend, &mut root, center(FIELD)).is_some());

    // Taking the focus selected the number, so the commit replaces it.
    backend.dispatch(&mut root, &commit("１２"));
    assert_eq!(number.borrow().value(), 12.0);
    backend.dispatch(&mut root, &commit("é"));
    assert_eq!(number.borrow().value(), 12.0);
}

#[test]
fn a_field_in_a_dialog_asks_for_the_input_method_there() {
    let modal = Rc::new(RefCell::new(Modal::new()));
    let mut root = Column::new().add_overlay(Shared(modal.clone()));
    let backend = MockBackend::new(WINDOW.w, WINDOW.h);
    root.layout(Rect::new(0, 0, WINDOW.w, WINDOW.h));
    let content = Container::new(160, 80).add(TextInput::new(Rect::new(10, 10, 140, 22)));
    modal
        .borrow_mut()
        .show("Name", Size::new(160, 80), Box::new(content));
    root.layout(Rect::new(0, 0, WINDOW.w, WINDOW.h));

    let area = backend
        .probe_ime(&mut root)
        .ime_area
        .expect("the dialog's field has the focus");
    let dialog = modal.borrow().bounds();
    assert!(
        dialog.contains(Point::new(area.x, area.y)),
        "{area:?} in {dialog:?}"
    );

    // Escape closes the dialog, and with it the field that took the text.
    assert_eq!(key(&backend, &mut root, NamedKey::Escape, false), None);
}

/// Lets a test keep a handle on a widget the tree owns.
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
    fn focusable(&self) -> bool {
        self.0.borrow().focusable()
    }
    fn set_focused(&mut self, focused: bool) {
        self.0.borrow_mut().set_focused(focused);
    }
    fn focus_first(&mut self) -> bool {
        self.0.borrow_mut().focus_first()
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
    fn wants_ticks(&self) -> bool {
        self.0.borrow().wants_ticks()
    }
}
