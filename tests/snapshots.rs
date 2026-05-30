//! Pixel-snapshot tests for every retrogui widget.
//!
//! Each test renders a small fixed-size widget tree through `MockBackend`
//! at 1.0x, 1.25x, 1.5x, and 2.0x, then compares the result to a checked-in
//! baseline PNG with a small pixel tolerance (see `common::snapshot_at_all_scales`).
//! Regenerate baselines after an intentional change with
//! `UPDATE_SNAPSHOTS=1 cargo test -p retrogui`.

mod common;

use common::snapshot_at_all_scales;

use retrogui::{
    Bevel, Button, Color, Column, Container, Dialog, Event, Image, Key, Label, List, ListIcon,
    ListItem, Menu, MenuBar, MenuItem, Modifiers, NamedKey, Orientation, Rect, Row, ScrollBar,
    TextEditor, Widget,
};

// ---------------------------------------------------------------- Bevel

#[test]
fn bevel_etched_line() {
    snapshot_at_all_scales("bevel_etched_line", 120, 12, || {
        Box::new(
            Container::new(120, 12)
                .with_background(Color::LIGHT_GRAY)
                .add(Bevel::etched_line(8, 5, 104)),
        )
    });
}

#[test]
fn bevel_raised_frame() {
    snapshot_at_all_scales("bevel_raised_frame", 120, 60, || {
        Box::new(
            Container::new(120, 60)
                .with_background(Color::LIGHT_GRAY)
                .add(Bevel::raised(Rect::new(8, 8, 104, 44))),
        )
    });
}

#[test]
fn bevel_sunken_frame() {
    snapshot_at_all_scales("bevel_sunken_frame", 120, 60, || {
        Box::new(
            Container::new(120, 60)
                .with_background(Color::LIGHT_GRAY)
                .add(Bevel::sunken(Rect::new(8, 8, 104, 44))),
        )
    });
}

// ---------------------------------------------------------------- Button

#[test]
fn button_plain() {
    snapshot_at_all_scales("button_plain", 120, 40, || {
        Box::new(
            Container::new(120, 40)
                .with_background(Color::LIGHT_GRAY)
                .add(Button::new(Rect::new(20, 8, 80, 24), "Cancel")),
        )
    });
}

#[test]
fn button_default() {
    snapshot_at_all_scales("button_default", 120, 40, || {
        Box::new(
            Container::new(120, 40)
                .with_background(Color::LIGHT_GRAY)
                .add(Button::new(Rect::new(20, 8, 80, 24), "OK").default(true)),
        )
    });
}

#[test]
fn button_focused() {
    snapshot_at_all_scales("button_focused", 120, 40, || {
        let mut btn = Button::new(Rect::new(20, 8, 80, 24), "Press");
        btn.set_focused(true);
        Box::new(
            Container::new(120, 40)
                .with_background(Color::LIGHT_GRAY)
                .add(btn),
        )
    });
}

// ---------------------------------------------------------------- Label

#[test]
fn label_default() {
    snapshot_at_all_scales("label_default", 140, 30, || {
        Box::new(
            Container::new(140, 30)
                .with_background(Color::WHITE)
                .add(Label::new(8, 8, "Hello, world!")),
        )
    });
}

#[test]
fn label_styled() {
    snapshot_at_all_scales("label_styled", 140, 30, || {
        Box::new(
            Container::new(140, 30)
                .with_background(Color::WHITE)
                .add(
                    Label::new(8, 6, "Big Red")
                        .with_color(Color::RED)
                        .with_size(16.0),
                ),
        )
    });
}

// ---------------------------------------------------------------- Image

#[test]
fn image_swatches() {
    snapshot_at_all_scales("image_swatches", 60, 40, || {
        let mut img = Image::new(10, 8, 40, 24);
        img.fill_rect(Rect::new(0, 0, 20, 12), Color::RED);
        img.fill_rect(Rect::new(20, 0, 20, 12), Color::GREEN);
        img.fill_rect(Rect::new(0, 12, 20, 12), Color::NAVY);
        img.fill_rect(Rect::new(20, 12, 20, 12), Color::YELLOW);
        Box::new(
            Container::new(60, 40)
                .with_background(Color::WHITE)
                .add(img),
        )
    });
}

// ---------------------------------------------------------------- ScrollBar

#[test]
fn scrollbar_vertical_empty() {
    // No scrollable range — thumb fills the track.
    snapshot_at_all_scales("scrollbar_vertical_empty", 20, 140, || {
        let sb = ScrollBar::new(Rect::new(2, 2, 16, 136), Orientation::Vertical);
        Box::new(
            Container::new(20, 140)
                .with_background(Color::WHITE)
                .add(sb),
        )
    });
}

#[test]
fn scrollbar_vertical_mid() {
    snapshot_at_all_scales("scrollbar_vertical_mid", 20, 140, || {
        let mut sb = ScrollBar::new(Rect::new(2, 2, 16, 136), Orientation::Vertical);
        sb.set_range(10, 90);
        sb.set_value(45);
        Box::new(
            Container::new(20, 140)
                .with_background(Color::WHITE)
                .add(sb),
        )
    });
}

#[test]
fn scrollbar_horizontal_mid() {
    snapshot_at_all_scales("scrollbar_horizontal_mid", 160, 20, || {
        let mut sb = ScrollBar::new(Rect::new(2, 2, 156, 16), Orientation::Horizontal);
        sb.set_range(10, 90);
        sb.set_value(45);
        Box::new(
            Container::new(160, 20)
                .with_background(Color::WHITE)
                .add(sb),
        )
    });
}

// ---------------------------------------------------------------- MenuBar

#[test]
fn menubar_closed() {
    snapshot_at_all_scales("menubar_closed", 220, 24, || {
        let bar = MenuBar::new(Rect::new(0, 0, 220, 20))
            .add_menu(Menu::new(
                "&File",
                vec![
                    MenuItem::action("&New", |_| {}),
                    MenuItem::action("&Open…", |_| {}),
                    MenuItem::separator(),
                    MenuItem::action("E&xit", |_| {}),
                ],
            ))
            .add_menu(Menu::new(
                "&Edit",
                vec![
                    MenuItem::action("&Copy", |_| {}),
                    MenuItem::action("&Paste", |_| {}),
                ],
            ));
        Box::new(
            Container::new(220, 24)
                .with_background(Color::WHITE)
                .add(bar),
        )
    });
}

#[test]
fn menubar_file_open() {
    // Big enough below the bar so the dropped-down popup fits inside
    // the window — keeps the snapshot a single self-contained image.
    snapshot_at_all_scales("menubar_file_open", 220, 120, || {
        let mut bar = MenuBar::new(Rect::new(0, 0, 220, 20))
            .add_menu(Menu::new(
                "&File",
                vec![
                    MenuItem::action("&New", |_| {}),
                    MenuItem::action("&Open…", |_| {}),
                    MenuItem::separator(),
                    MenuItem::action("E&xit", |_| {}),
                ],
            ))
            .add_menu(Menu::new(
                "&Edit",
                vec![
                    MenuItem::action("&Copy", |_| {}),
                    MenuItem::action("&Paste", |_| {}),
                ],
            ));
        bar.open(0);
        Box::new(
            Container::new(220, 120)
                .with_background(Color::WHITE)
                .add(bar),
        )
    });
}

// ---------------------------------------------------------------- Dialog

// Dialog bodies fill with the theme's background color (white in the
// default theme) and rely on the compositor to draw the surrounding
// chrome. The tests use a light-gray column background so the white
// dialog body stands out against it.

#[test]
fn dialog_info() {
    snapshot_at_all_scales("dialog_info", 420, 240, || {
        let mut dialog = Dialog::new();
        dialog.show_info("Information", "Operation completed successfully.");
        let column = Column::new()
            .with_background(Color::LIGHT_GRAY)
            .add_overlay(dialog);
        Box::new(column)
    });
}

#[test]
fn dialog_warning() {
    snapshot_at_all_scales("dialog_warning", 420, 240, || {
        let mut dialog = Dialog::new();
        dialog.show_warning("Warning", "Unsaved changes will be lost.");
        let column = Column::new()
            .with_background(Color::LIGHT_GRAY)
            .add_overlay(dialog);
        Box::new(column)
    });
}

#[test]
fn dialog_error() {
    snapshot_at_all_scales("dialog_error", 420, 240, || {
        let mut dialog = Dialog::new();
        dialog.show_error("Error", "Could not open the requested file.");
        let column = Column::new()
            .with_background(Color::LIGHT_GRAY)
            .add_overlay(dialog);
        Box::new(column)
    });
}

#[test]
fn dialog_no_icon() {
    snapshot_at_all_scales("dialog_no_icon", 420, 240, || {
        let mut dialog = Dialog::new();
        dialog.show(
            "Notice",
            "A plain message without any icon decoration.",
            retrogui::DialogIcon::None,
        );
        let column = Column::new()
            .with_background(Color::LIGHT_GRAY)
            .add_overlay(dialog);
        Box::new(column)
    });
}

// ---------------------------------------------------------------- TextEditor

/// TextEditor wraps an internal ScrollBar that's only sized via
/// `layout`. Container doesn't propagate layout (its children own their
/// absolute placement), so editor tests must call `layout` themselves to
/// match the way Column would size the editor in a real window.
fn laid_out_editor(rect: Rect, text: &str) -> TextEditor {
    let mut editor = TextEditor::new(rect);
    if !text.is_empty() {
        editor = editor.with_text(text);
    }
    editor.layout(rect);
    editor
}

#[test]
fn text_editor_empty() {
    snapshot_at_all_scales("text_editor_empty", 200, 120, || {
        let editor = laid_out_editor(Rect::new(8, 8, 184, 104), "");
        Box::new(
            Container::new(200, 120)
                .with_background(Color::LIGHT_GRAY)
                .add(editor),
        )
    });
}

#[test]
fn text_editor_with_text() {
    snapshot_at_all_scales("text_editor_with_text", 200, 120, || {
        let editor = laid_out_editor(
            Rect::new(8, 8, 184, 104),
            "hello world\nthe quick brown fox\njumped over\nthe lazy dog",
        );
        Box::new(
            Container::new(200, 120)
                .with_background(Color::LIGHT_GRAY)
                .add(editor),
        )
    });
}

#[test]
fn text_editor_focused() {
    snapshot_at_all_scales("text_editor_focused", 200, 120, || {
        let mut editor = laid_out_editor(Rect::new(8, 8, 184, 104), "type here");
        editor.set_focused(true);
        Box::new(
            Container::new(200, 120)
                .with_background(Color::LIGHT_GRAY)
                .add(editor),
        )
    });
}

/// Editor with more rows than fit on screen — exercises both the
/// scrollbar's mid-track thumb and the visible-window clipping.
#[test]
fn text_editor_scrolls() {
    snapshot_at_all_scales("text_editor_scrolls", 200, 100, || {
        let mut lines = Vec::new();
        for n in 1..=20 {
            lines.push(format!("line {:>2}", n));
        }
        let editor = laid_out_editor(Rect::new(8, 8, 184, 84), &lines.join("\n"));
        Box::new(
            Container::new(200, 100)
                .with_background(Color::LIGHT_GRAY)
                .add(editor),
        )
    });
}

// ---------------------------------------------------------------- List

/// A small icon used in list-snapshot tests so we exercise icon rendering
/// without depending on the file-browser example's specific glyphs.
fn swatch_icon(color: Color) -> ListIcon {
    let mut icon = ListIcon::new(10, 10);
    icon.fill_rect(Rect::new(0, 0, 10, 10), Color::BLACK);
    icon.fill_rect(Rect::new(1, 1, 8, 8), color);
    icon
}

fn laid_out_list(rect: Rect, items: Vec<ListItem>) -> List {
    let mut list = List::new(rect).with_items(items);
    list.layout(rect);
    list
}

#[test]
fn list_basic() {
    snapshot_at_all_scales("list_basic", 200, 100, || {
        let list = laid_out_list(
            Rect::new(8, 8, 184, 84),
            vec![
                ListItem::new("first").with_icon(swatch_icon(Color::RED)),
                ListItem::new("second").with_icon(swatch_icon(Color::GREEN)),
                ListItem::new("third").with_icon(swatch_icon(Color::NAVY)),
                ListItem::new("plain (no icon)"),
            ],
        );
        Box::new(
            Container::new(200, 100)
                .with_background(Color::LIGHT_GRAY)
                .add(list),
        )
    });
}

#[test]
fn list_selected_focused() {
    snapshot_at_all_scales("list_selected_focused", 200, 100, || {
        let mut list = laid_out_list(
            Rect::new(8, 8, 184, 84),
            vec![
                ListItem::new("alpha").with_icon(swatch_icon(Color::RED)),
                ListItem::new("beta").with_icon(swatch_icon(Color::GREEN)),
                ListItem::new("gamma").with_icon(swatch_icon(Color::NAVY)),
            ],
        );
        list.set_selected(Some(1));
        list.set_focused(true);
        Box::new(
            Container::new(200, 100)
                .with_background(Color::LIGHT_GRAY)
                .add(list),
        )
    });
}

#[test]
fn list_scrolls() {
    snapshot_at_all_scales("list_scrolls", 200, 100, || {
        let mut items = Vec::new();
        for n in 1..=20 {
            items.push(ListItem::new(format!("entry {:>2}", n)).with_icon(swatch_icon(Color::NAVY)));
        }
        let list = laid_out_list(Rect::new(8, 8, 184, 84), items);
        Box::new(
            Container::new(200, 100)
                .with_background(Color::LIGHT_GRAY)
                .add(list),
        )
    });
}

// ---------------------------------------------------------------- Composite

/// A small dialog-style layout that exercises Container, Label, Bevel,
/// and Button in concert — the kind of arrangement the retrofetch about
/// box uses. Catches regressions that affect coordination between
/// widgets, not just an individual one.
#[test]
fn composite_about_box() {
    snapshot_at_all_scales("composite_about_box", 260, 140, || {
        Box::new(
            Container::new(260, 140)
                .with_background(Color::LIGHT_GRAY)
                .add(Label::new(16, 16, "Retrogui Demo").with_size(14.0))
                .add(Label::new(16, 40, "Version 0.1.0"))
                .add(Bevel::etched_line(16, 64, 228))
                .add(Label::new(16, 76, "(c) Nobody in particular"))
                .add(Button::new(Rect::new(90, 104, 80, 24), "OK").default(true)),
        )
    });
}

// ---------------------------------------------------------------- Focus

/// A Tab press fires both `KeyDown(Tab)` and `Char('\t')` from the
/// runtime. The container should treat the pair as a single focus move,
/// not two — otherwise Tab from "A" lands on "C" instead of "B".
#[test]
fn tab_press_moves_focus_exactly_once() {
    use retrogui::mock::MockBackend;
    let mut container = Container::new(300, 60)
        .add(Button::new(Rect::new(10, 10, 60, 24), "A"))
        .add(Button::new(Rect::new(80, 10, 60, 24), "B"))
        .add(Button::new(Rect::new(150, 10, 60, 24), "C"));
    container.focus_first();
    assert_eq!(container.focused_index(), Some(0));

    let backend = MockBackend::new(300, 60).with_scale(1.0);
    let tab = Event::KeyDown {
        key: Key::Named(NamedKey::Tab),
        modifiers: Modifiers::default(),
    };
    let tab_char = Event::Char {
        ch: '\t',
        modifiers: Modifiers::default(),
    };

    backend.dispatch(&mut container, &tab);
    backend.dispatch(&mut container, &tab_char);
    assert_eq!(container.focused_index(), Some(1), "first Tab press should land on B");

    backend.dispatch(&mut container, &tab);
    backend.dispatch(&mut container, &tab_char);
    assert_eq!(container.focused_index(), Some(2), "second Tab press should land on C");

    // Shift+Tab cycles backward.
    let shift_tab = Event::KeyDown {
        key: Key::Named(NamedKey::Tab),
        modifiers: Modifiers {
            shift: true,
            ..Modifiers::default()
        },
    };
    let shift_tab_char = Event::Char {
        ch: '\t',
        modifiers: Modifiers {
            shift: true,
            ..Modifiers::default()
        },
    };
    backend.dispatch(&mut container, &shift_tab);
    backend.dispatch(&mut container, &shift_tab_char);
    assert_eq!(container.focused_index(), Some(1), "Shift+Tab should walk back to B");
}

/// Tab at an outer `Column` that has the inner `Container` as its only
/// focusable child must not swallow the keystroke — it should propagate
/// so the inner container can cycle among *its* focusable widgets. This
/// is the situation the picker example sets up (Column → Container →
/// List/OK/Cancel), so the test pins the contract.
#[test]
fn tab_propagates_through_single_child_outer_container() {
    use std::cell::RefCell;
    use std::rc::Rc;

    use retrogui::mock::MockBackend;

    // Each button records its own label when fired so the test can read
    // back which one was activated by Enter.
    let fired: Rc<RefCell<Option<String>>> = Rc::new(RefCell::new(None));
    let make_btn = |x: i32, label: &'static str| {
        let f = fired.clone();
        Button::new(Rect::new(x, 10, 60, 24), label).on_click(move |_cx| {
            *f.borrow_mut() = Some(label.to_string());
        })
    };

    let inner = Container::new(300, 60)
        .add(make_btn(10, "A"))
        .add(make_btn(80, "B"))
        .add(make_btn(150, "C"));
    let mut outer = Column::new().add_fill(inner);
    outer.layout(Rect::new(0, 0, 300, 60));
    outer.focus_first();

    let backend = MockBackend::new(300, 60).with_scale(1.0);
    let tab = Event::KeyDown {
        key: Key::Named(NamedKey::Tab),
        modifiers: Modifiers::default(),
    };
    let tab_char = Event::Char {
        ch: '\t',
        modifiers: Modifiers::default(),
    };
    let enter = Event::KeyDown {
        key: Key::Named(NamedKey::Enter),
        modifiers: Modifiers::default(),
    };

    // Initial focus on A (Container.focus_first picks first focusable).
    backend.dispatch(&mut outer, &enter);
    assert_eq!(fired.borrow().as_deref(), Some("A"));

    fired.borrow_mut().take();
    backend.dispatch(&mut outer, &tab);
    backend.dispatch(&mut outer, &tab_char);
    backend.dispatch(&mut outer, &enter);
    assert_eq!(
        fired.borrow().as_deref(),
        Some("B"),
        "after one Tab, Enter should fire B"
    );

    fired.borrow_mut().take();
    backend.dispatch(&mut outer, &tab);
    backend.dispatch(&mut outer, &tab_char);
    backend.dispatch(&mut outer, &enter);
    assert_eq!(
        fired.borrow().as_deref(),
        Some("C"),
        "after two Tabs, Enter should fire C"
    );

    fired.borrow_mut().take();
    backend.dispatch(&mut outer, &tab);
    backend.dispatch(&mut outer, &tab_char);
    backend.dispatch(&mut outer, &enter);
    assert_eq!(
        fired.borrow().as_deref(),
        Some("A"),
        "Tab cycling should wrap back to A"
    );
}

/// A default button fires on Enter regardless of which sibling holds
/// focus — the classic Win 3.1 "OK is the Enter target in any dialog"
/// behavior. Once it fires, the focused widget must not also see the
/// Enter (otherwise a focused List would also activate its current
/// item).
#[test]
fn default_button_fires_on_enter_from_any_focus() {
    use std::cell::RefCell;
    use std::rc::Rc;

    use retrogui::mock::MockBackend;

    let fired: Rc<RefCell<Vec<String>>> = Rc::new(RefCell::new(Vec::new()));
    let make_btn = |x: i32, label: &'static str, default: bool| {
        let f = fired.clone();
        let mut b = Button::new(Rect::new(x, 40, 60, 24), label).on_click(move |_cx| {
            f.borrow_mut().push(label.to_string());
        });
        if default {
            b = b.default(true);
        }
        b
    };

    let mut list = List::new(Rect::new(10, 10, 200, 20))
        .with_items(vec![ListItem::new("alpha"), ListItem::new("beta")]);
    list.set_selected(Some(0));

    let mut c = Container::new(300, 80)
        .add(list)
        .add(make_btn(10, "OK", true))
        .add(make_btn(80, "Cancel", false));
    c.layout(Rect::new(0, 0, 300, 80));
    c.focus_first();
    // List is the first focusable widget so it should hold focus initially.
    assert_eq!(c.focused_index(), Some(0));

    let backend = MockBackend::new(300, 80).with_scale(1.0);
    let enter = Event::KeyDown {
        key: Key::Named(NamedKey::Enter),
        modifiers: Modifiers::default(),
    };

    // Enter while the list is focused fires the default button (OK), not
    // the list's own Enter handler.
    backend.dispatch(&mut c, &enter);
    assert_eq!(*fired.borrow(), vec!["OK".to_string()]);

    // Focus the Cancel button via Tab and confirm Enter still fires OK
    // (default beats focused non-default per the explicit request).
    fired.borrow_mut().clear();
    let tab = Event::KeyDown {
        key: Key::Named(NamedKey::Tab),
        modifiers: Modifiers::default(),
    };
    let tab_char = Event::Char {
        ch: '\t',
        modifiers: Modifiers::default(),
    };
    // list -> OK -> Cancel
    backend.dispatch(&mut c, &tab);
    backend.dispatch(&mut c, &tab_char);
    backend.dispatch(&mut c, &tab);
    backend.dispatch(&mut c, &tab_char);
    assert_eq!(c.focused_index(), Some(2), "Cancel should be focused");

    backend.dispatch(&mut c, &enter);
    assert_eq!(
        *fired.borrow(),
        vec!["OK".to_string()],
        "Enter on focused Cancel should still fire default OK"
    );

    // Walk back from Cancel to OK with Shift+Tab and press Enter —
    // the focused-button path must still fire exactly once (no double-
    // fire via the accelerator pass).
    fired.borrow_mut().clear();
    let shift_tab = Event::KeyDown {
        key: Key::Named(NamedKey::Tab),
        modifiers: Modifiers {
            shift: true,
            ..Modifiers::default()
        },
    };
    let shift_tab_char = Event::Char {
        ch: '\t',
        modifiers: Modifiers {
            shift: true,
            ..Modifiers::default()
        },
    };
    backend.dispatch(&mut c, &shift_tab);
    backend.dispatch(&mut c, &shift_tab_char);
    assert_eq!(c.focused_index(), Some(1), "OK should be focused");
    backend.dispatch(&mut c, &enter);
    assert_eq!(*fired.borrow(), vec!["OK".to_string()]);
}

/// Sanity-check that the [`MockBackend`] API itself returns a
/// non-empty image. Distinguishes framework breakage from genuine
/// widget regressions.
#[test]
fn snapshot_facility_smoke_test() {
    use retrogui::mock::MockBackend;
    let mut root: Box<dyn Widget> = Box::new(
        Container::new(40, 20)
            .with_background(Color::WHITE)
            .add(Label::new(2, 2, "ok")),
    );
    let backend = MockBackend::new(40, 20)
        .with_scale(1.0)
        .with_font(common::sans_font())
        .with_mono_font(common::mono_font());
    let snap = backend.render(root.as_mut());
    assert_eq!(snap.width(), 40);
    assert_eq!(snap.height(), 20);
    assert!(!snap.to_png().is_empty());
}

// ---------------------------------------------------------------- Row

/// A `Row` with a fixed-width child on the left and a fill child taking the
/// rest — the horizontal counterpart to the `Column` layout tests.
#[test]
fn row_fixed_and_fill() {
    snapshot_at_all_scales("row_fixed_and_fill", 240, 70, || {
        let left = List::new(Rect::new(0, 0, 0, 0))
            .with_items(vec![ListItem::new("A"), ListItem::new("B")]);
        let right = List::new(Rect::new(0, 0, 0, 0)).with_items(vec![
            ListItem::new("one"),
            ListItem::new("two"),
            ListItem::new("three"),
        ]);
        Box::new(
            Row::new()
                .with_background(Color::LIGHT_GRAY)
                .add_fixed(left, 80)
                .add_fill(right),
        )
    });
}

/// Two equal fill children split the row in half.
#[test]
fn row_two_fills() {
    snapshot_at_all_scales("row_two_fills", 200, 60, || {
        let a = List::new(Rect::new(0, 0, 0, 0)).with_items(vec![ListItem::new("left")]);
        let b = List::new(Rect::new(0, 0, 0, 0)).with_items(vec![ListItem::new("right")]);
        Box::new(
            Row::new()
                .with_background(Color::LIGHT_GRAY)
                .add_fill(a)
                .add_fill(b),
        )
    });
}
