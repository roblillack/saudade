//! Pixel-snapshot tests for every saudade widget.
//!
//! Each test renders a small fixed-size widget tree through `MockBackend`
//! at 1.0x, 1.25x, 1.5x, and 2.0x, then compares the result to a checked-in
//! baseline PNG with a small pixel tolerance (see `common::snapshot_at_all_scales`).
//! Regenerate baselines after an intentional change with
//! `UPDATE_SNAPSHOTS=1 cargo test -p saudade`.

mod common;

use common::{snapshot_at_all_scales, snapshot_framed_at_all_scales};

use saudade::{
    Bevel, Button, Checkbox, Color, Column, Container, Dialog, Dropdown, Event, Image, Key, Label,
    List, ListIcon, ListItem, Menu, MenuBar, MenuItem, Modifiers, NamedKey, Orientation,
    ProgressBar, Rect, Row, ScrollBar, Slider, TextEditor, TextInput, Widget, WindowChrome,
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

#[test]
fn button_disabled() {
    snapshot_at_all_scales("button_disabled", 120, 40, || {
        Box::new(
            Container::new(120, 40)
                .with_background(Color::LIGHT_GRAY)
                .add(Button::new(Rect::new(20, 8, 80, 24), "Book").with_enabled(false)),
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
                .add(Label::new(Rect::new(8, 8, 124, 16), "Hello, world!")),
        )
    });
}

#[test]
fn label_styled() {
    snapshot_at_all_scales("label_styled", 140, 30, || {
        Box::new(
            Container::new(140, 30).with_background(Color::WHITE).add(
                Label::new(Rect::new(8, 6, 124, 22), "Big Red")
                    .with_color(Color::RED)
                    .with_size(16.0),
            ),
        )
    });
}

#[test]
fn label_multiline() {
    snapshot_at_all_scales("label_multiline", 140, 60, || {
        Box::new(
            Container::new(140, 60)
                .with_background(Color::WHITE)
                .add(Label::new(
                    Rect::new(8, 8, 124, 48),
                    "First line\nSecond line\nThird line",
                )),
        )
    });
}

#[test]
fn label_wrapped() {
    snapshot_at_all_scales("label_wrapped", 140, 80, || {
        Box::new(
            Container::new(140, 80)
                .with_background(Color::WHITE)
                .add(Label::new(
                    Rect::new(8, 8, 124, 64),
                    "The quick brown fox jumps over the lazy dog.",
                )),
        )
    });
}

#[test]
fn label_wrapped_with_breaks() {
    snapshot_at_all_scales("label_wrapped_with_breaks", 160, 100, || {
        Box::new(
            Container::new(160, 100)
                .with_background(Color::WHITE)
                .add(Label::new(
                    Rect::new(8, 8, 144, 88),
                    "Paragraph one wraps across several lines.\n\nParagraph two follows a blank line.",
                )),
        )
    });
}

#[test]
fn label_clipped() {
    // The box is too small for the text: it wraps to the box width and
    // everything below the box's bottom edge is clipped away.
    snapshot_at_all_scales("label_clipped", 160, 50, || {
        Box::new(
            Container::new(160, 50)
                .with_background(Color::WHITE)
                .add(Label::new(
                    Rect::new(8, 8, 80, 20),
                    "This text is wider and taller than its small box.",
                )),
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

/// The mouse wheel drives the scroll position directly: positive `delta_y`
/// moves toward the end, negative back toward the start, sub-line deltas bank
/// until they make a whole line, and the value saturates at both ends. A
/// vertical bar reads `delta_y`; a horizontal bar reads `delta_x`.
#[test]
fn scrollbar_mouse_wheel_moves_value() {
    use saudade::mock::MockBackend;

    let backend = MockBackend::new(40, 160);
    let scroll = |dx: f32, dy: f32| Event::Scroll {
        pos: saudade::Point::new(10, 80),
        delta_x: dx,
        delta_y: dy,
    };

    let mut bar = ScrollBar::vertical(Rect::new(2, 2, 16, 156));
    bar.set_range(/* viewport */ 5, /* max */ 40);

    backend.dispatch(&mut bar, &scroll(0.0, 3.0));
    assert_eq!(bar.value(), 3, "positive delta_y scrolls toward the end");
    backend.dispatch(&mut bar, &scroll(0.0, -1.0));
    assert_eq!(bar.value(), 2, "negative delta_y scrolls back");
    backend.dispatch(&mut bar, &scroll(7.0, 0.0));
    assert_eq!(bar.value(), 2, "a vertical bar ignores delta_x");

    backend.dispatch(&mut bar, &scroll(0.0, 0.5));
    assert_eq!(bar.value(), 2, "half a line banks without moving yet");
    backend.dispatch(&mut bar, &scroll(0.0, 0.5));
    assert_eq!(bar.value(), 3, "the second half completes a whole line");

    backend.dispatch(&mut bar, &scroll(0.0, -100.0));
    assert_eq!(bar.value(), 0, "value saturates at the start");
    backend.dispatch(&mut bar, &scroll(0.0, 100.0));
    assert_eq!(bar.value(), 40, "value saturates at the end");

    let mut hbar = ScrollBar::horizontal(Rect::new(2, 2, 156, 16));
    hbar.set_range(5, 40);
    backend.dispatch(&mut hbar, &scroll(2.0, 9.0));
    assert_eq!(
        hbar.value(),
        2,
        "a horizontal bar reads delta_x, not delta_y"
    );
}

// ---------------------------------------------------------------- ProgressBar

#[test]
fn progressbar_empty() {
    snapshot_at_all_scales("progressbar_empty", 160, 28, || {
        Box::new(
            Container::new(160, 28)
                .with_background(Color::LIGHT_GRAY)
                .add(ProgressBar::new(Rect::new(8, 6, 144, 16))),
        )
    });
}

#[test]
fn progressbar_mid() {
    snapshot_at_all_scales("progressbar_mid", 160, 28, || {
        Box::new(
            Container::new(160, 28)
                .with_background(Color::LIGHT_GRAY)
                .add(ProgressBar::new(Rect::new(8, 6, 144, 16)).with_fraction(0.45)),
        )
    });
}

#[test]
fn progressbar_full() {
    snapshot_at_all_scales("progressbar_full", 160, 28, || {
        Box::new(
            Container::new(160, 28)
                .with_background(Color::LIGHT_GRAY)
                .add(ProgressBar::new(Rect::new(8, 6, 144, 16)).with_fraction(1.0)),
        )
    });
}

/// Grey fill with the rounded percentage drawn centered over the bar.
#[test]
fn progressbar_percentage() {
    snapshot_at_all_scales("progressbar_percentage", 160, 28, || {
        Box::new(
            Container::new(160, 28)
                .with_background(Color::LIGHT_GRAY)
                .add(
                    ProgressBar::new(Rect::new(8, 6, 144, 16))
                        .with_fraction(0.6)
                        .with_percentage(true),
                ),
        )
    });
}

// ---------------------------------------------------------------- Slider

#[test]
fn slider_min() {
    snapshot_at_all_scales("slider_min", 160, 32, || {
        Box::new(
            Container::new(160, 32)
                .with_background(Color::LIGHT_GRAY)
                .add(Slider::new(Rect::new(8, 4, 144, 24), 0, 100)),
        )
    });
}

#[test]
fn slider_mid() {
    snapshot_at_all_scales("slider_mid", 160, 32, || {
        Box::new(
            Container::new(160, 32)
                .with_background(Color::LIGHT_GRAY)
                .add(Slider::new(Rect::new(8, 4, 144, 24), 0, 100).with_value(50)),
        )
    });
}

#[test]
fn slider_max() {
    snapshot_at_all_scales("slider_max", 160, 32, || {
        Box::new(
            Container::new(160, 32)
                .with_background(Color::LIGHT_GRAY)
                .add(Slider::new(Rect::new(8, 4, 144, 24), 0, 100).with_value(100)),
        )
    });
}

#[test]
fn slider_focused() {
    snapshot_at_all_scales("slider_focused", 160, 32, || {
        let mut slider = Slider::new(Rect::new(8, 4, 144, 24), 0, 100).with_value(50);
        slider.set_focused(true);
        Box::new(
            Container::new(160, 32)
                .with_background(Color::LIGHT_GRAY)
                .add(slider),
        )
    });
}

#[test]
fn slider_disabled() {
    snapshot_at_all_scales("slider_disabled", 160, 32, || {
        Box::new(
            Container::new(160, 32)
                .with_background(Color::LIGHT_GRAY)
                .add(
                    Slider::new(Rect::new(8, 4, 144, 24), 0, 100)
                        .with_value(50)
                        .with_enabled(false),
                ),
        )
    });
}

// ---------------------------------------------------------------- Dropdown

fn flight_dropdown(rect: Rect) -> Dropdown {
    Dropdown::new(rect).with_items(["one-way flight", "return flight"])
}

#[test]
fn dropdown_closed() {
    snapshot_at_all_scales("dropdown_closed", 200, 40, || {
        Box::new(
            Container::new(200, 40)
                .with_background(Color::LIGHT_GRAY)
                .add(flight_dropdown(Rect::new(16, 8, 168, 24))),
        )
    });
}

#[test]
fn dropdown_focused() {
    snapshot_at_all_scales("dropdown_focused", 200, 40, || {
        let mut dd = flight_dropdown(Rect::new(16, 8, 168, 24));
        dd.set_focused(true);
        Box::new(
            Container::new(200, 40)
                .with_background(Color::LIGHT_GRAY)
                .add(dd),
        )
    });
}

#[test]
fn dropdown_disabled() {
    snapshot_at_all_scales("dropdown_disabled", 200, 40, || {
        Box::new(
            Container::new(200, 40)
                .with_background(Color::LIGHT_GRAY)
                .add(flight_dropdown(Rect::new(16, 8, 168, 24)).with_enabled(false)),
        )
    });
}

/// Open list — the popup is composited below the closed field, exactly as the
/// runtime hosts it in a separate window. The window is tall enough to hold the
/// dropped-down rows so the snapshot stays a single self-contained image.
#[test]
fn dropdown_open() {
    snapshot_at_all_scales("dropdown_open", 200, 84, || {
        let mut dd = flight_dropdown(Rect::new(16, 8, 168, 24));
        dd.open();
        Box::new(
            Container::new(200, 84)
                .with_background(Color::LIGHT_GRAY)
                .add(dd),
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
            saudade::DialogIcon::None,
        );
        let column = Column::new()
            .with_background(Color::LIGHT_GRAY)
            .add_overlay(dialog);
        Box::new(column)
    });
}

/// A multi-line message must grow the box so the text clears the OK button —
/// the clash the picker example surfaced once both the chrome font and the
/// button grew. Mirrors picker's "<who> picked: … Saved to favorites." body.
#[test]
fn dialog_multiline() {
    snapshot_at_all_scales("dialog_multiline", 420, 240, || {
        let mut dialog = Dialog::new();
        dialog.show_info(
            "Confirmed",
            "Alice picked:\n\nThe blue option\n\nSaved to favorites.",
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
            items
                .push(ListItem::new(format!("entry {:>2}", n)).with_icon(swatch_icon(Color::NAVY)));
        }
        let list = laid_out_list(Rect::new(8, 8, 184, 84), items);
        Box::new(
            Container::new(200, 100)
                .with_background(Color::LIGHT_GRAY)
                .add(list),
        )
    });
}

/// The same overflowing list as `list_scrolls`, but after a few mouse-wheel
/// notches have rolled the viewport down to later entries.
#[test]
fn list_scrolls_with_wheel() {
    snapshot_at_all_scales("list_scrolls_with_wheel", 200, 100, || {
        let mut items = Vec::new();
        for n in 1..=20 {
            items
                .push(ListItem::new(format!("entry {:>2}", n)).with_icon(swatch_icon(Color::NAVY)));
        }
        let mut list = laid_out_list(Rect::new(8, 8, 184, 84), items);
        // Six lines down: the top of the field now starts a few rows in.
        saudade::mock::MockBackend::new(200, 100).dispatch(
            &mut list,
            &Event::Scroll {
                pos: saudade::Point::new(60, 40),
                delta_x: 0.0,
                delta_y: 6.0,
            },
        );
        Box::new(
            Container::new(200, 100)
                .with_background(Color::LIGHT_GRAY)
                .add(list),
        )
    });
}

/// The wheel scrolls the list only while the pointer is over it, and never
/// disturbs the selection.
#[test]
fn list_mouse_wheel_scrolls_when_hovered() {
    use saudade::mock::MockBackend;

    let rect = Rect::new(8, 8, 184, 84);
    let items: Vec<ListItem> = (0..40).map(|n| ListItem::new(format!("row {n}"))).collect();
    let mut list = laid_out_list(rect, items);
    let backend = MockBackend::new(200, 100);

    let over = backend.dispatch(
        &mut list,
        &Event::Scroll {
            pos: saudade::Point::new(50, 40),
            delta_x: 0.0,
            delta_y: 9.0,
        },
    );
    assert!(over.paint_requested, "scrolling over the list repaints it");
    assert_eq!(
        list.selected_index(),
        None,
        "a wheel scroll leaves the selection alone"
    );

    let away = backend.dispatch(
        &mut list,
        &Event::Scroll {
            pos: saudade::Point::new(300, 300),
            delta_x: 0.0,
            delta_y: 9.0,
        },
    );
    assert!(
        !away.paint_requested,
        "a scroll outside the list's bounds is ignored"
    );
}

#[test]
fn list_disabled() {
    snapshot_at_all_scales("list_disabled", 200, 100, || {
        let mut list = laid_out_list(
            Rect::new(8, 8, 184, 84),
            vec![
                ListItem::new("alpha").with_icon(swatch_icon(Color::RED)),
                ListItem::new("beta").with_icon(swatch_icon(Color::GREEN)),
                ListItem::new("gamma").with_icon(swatch_icon(Color::NAVY)),
            ],
        );
        list.set_selected(Some(1));
        list.set_enabled(false);
        Box::new(
            Container::new(200, 100)
                .with_background(Color::LIGHT_GRAY)
                .add(list),
        )
    });
}

#[test]
fn list_clips_long_labels() {
    snapshot_at_all_scales("list_clips_long_labels", 200, 100, || {
        let mut list = laid_out_list(
            Rect::new(8, 8, 184, 84),
            vec![
                ListItem::new("a label far too wide to fit inside this narrow list field")
                    .with_icon(swatch_icon(Color::RED)),
                ListItem::new("another entry whose text also overruns the right edge"),
            ],
        );
        list.set_selected(Some(0));
        list.set_focused(true);
        Box::new(
            Container::new(200, 100)
                .with_background(Color::LIGHT_GRAY)
                .add(list),
        )
    });
}

// ---------------------------------------------------------------- Checkbox

#[test]
fn checkbox_unchecked() {
    snapshot_at_all_scales("checkbox_unchecked", 200, 28, || {
        Box::new(
            Container::new(200, 28)
                .with_background(Color::LIGHT_GRAY)
                .add(Checkbox::new(Rect::new(8, 6, 184, 16), "Add to favorites")),
        )
    });
}

#[test]
fn checkbox_checked() {
    snapshot_at_all_scales("checkbox_checked", 200, 28, || {
        Box::new(
            Container::new(200, 28)
                .with_background(Color::LIGHT_GRAY)
                .add(Checkbox::new(Rect::new(8, 6, 184, 16), "Add to favorites").checked(true)),
        )
    });
}

#[test]
fn checkbox_focused() {
    snapshot_at_all_scales("checkbox_focused", 200, 28, || {
        let mut cb = Checkbox::new(Rect::new(8, 6, 184, 16), "Tab landed here").checked(true);
        cb.set_focused(true);
        Box::new(
            Container::new(200, 28)
                .with_background(Color::LIGHT_GRAY)
                .add(cb),
        )
    });
}

#[test]
fn checkbox_disabled() {
    snapshot_at_all_scales("checkbox_disabled", 200, 28, || {
        Box::new(
            Container::new(200, 28)
                .with_background(Color::LIGHT_GRAY)
                .add(
                    Checkbox::new(Rect::new(8, 6, 184, 16), "Can't toggle me").with_enabled(false),
                ),
        )
    });
}

#[test]
fn checkbox_disabled_checked() {
    snapshot_at_all_scales("checkbox_disabled_checked", 200, 28, || {
        Box::new(
            Container::new(200, 28)
                .with_background(Color::LIGHT_GRAY)
                .add(
                    Checkbox::new(Rect::new(8, 6, 184, 16), "Locked on")
                        .checked(true)
                        .with_enabled(false),
                ),
        )
    });
}

// ---------------------------------------------------------------- TextInput

#[test]
fn text_input_empty() {
    snapshot_at_all_scales("text_input_empty", 200, 32, || {
        Box::new(
            Container::new(200, 32)
                .with_background(Color::LIGHT_GRAY)
                .add(TextInput::new(Rect::new(8, 6, 184, 20))),
        )
    });
}

#[test]
fn text_input_with_text() {
    snapshot_at_all_scales("text_input_with_text", 200, 32, || {
        Box::new(
            Container::new(200, 32)
                .with_background(Color::LIGHT_GRAY)
                .add(TextInput::new(Rect::new(8, 6, 184, 20)).with_text("Anonymous")),
        )
    });
}

/// Focused, blinking caret in its "on" phase (the freshly-constructed state).
#[test]
fn text_input_focused() {
    snapshot_at_all_scales("text_input_focused", 200, 32, || {
        let mut input = TextInput::new(Rect::new(8, 6, 184, 20)).with_text("type here");
        input.set_focused(true);
        Box::new(
            Container::new(200, 32)
                .with_background(Color::LIGHT_GRAY)
                .add(input),
        )
    });
}

#[test]
fn text_input_disabled() {
    snapshot_at_all_scales("text_input_disabled", 200, 32, || {
        Box::new(
            Container::new(200, 32)
                .with_background(Color::LIGHT_GRAY)
                .add(
                    TextInput::new(Rect::new(8, 6, 184, 20))
                        .with_text("greyed out")
                        .with_enabled(false),
                ),
        )
    });
}

/// All text selected while focused — exercises the active highlight band.
#[test]
fn text_input_selection() {
    snapshot_at_all_scales("text_input_selection", 200, 32, || {
        let mut input = TextInput::new(Rect::new(8, 6, 184, 20)).with_text("selected text");
        input.set_focused(true);
        input.select_all();
        Box::new(
            Container::new(200, 32)
                .with_background(Color::LIGHT_GRAY)
                .add(input),
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
                .add(Label::new(Rect::new(16, 16, 228, 20), "Retrogui Demo").with_size(14.0))
                .add(Label::new(Rect::new(16, 40, 228, 16), "Version 0.1.0"))
                .add(Bevel::etched_line(16, 64, 228))
                .add(Label::new(
                    Rect::new(16, 76, 228, 16),
                    "(c) Nobody in particular",
                ))
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
    use saudade::mock::MockBackend;
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
    assert_eq!(
        container.focused_index(),
        Some(1),
        "first Tab press should land on B"
    );

    backend.dispatch(&mut container, &tab);
    backend.dispatch(&mut container, &tab_char);
    assert_eq!(
        container.focused_index(),
        Some(2),
        "second Tab press should land on C"
    );

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
    assert_eq!(
        container.focused_index(),
        Some(1),
        "Shift+Tab should walk back to B"
    );
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

    use saudade::mock::MockBackend;

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
    let enter_down = Event::KeyDown {
        key: Key::Named(NamedKey::Enter),
        modifiers: Modifiers::default(),
    };
    let enter_up = Event::KeyUp {
        key: Key::Named(NamedKey::Enter),
        modifiers: Modifiers::default(),
    };
    let press_enter = |w: &mut dyn saudade::Widget| {
        backend.dispatch(w, &enter_down);
        backend.dispatch(w, &enter_up);
    };

    // Initial focus on A (Container.focus_first picks first focusable).
    press_enter(&mut outer);
    assert_eq!(fired.borrow().as_deref(), Some("A"));

    fired.borrow_mut().take();
    backend.dispatch(&mut outer, &tab);
    backend.dispatch(&mut outer, &tab_char);
    press_enter(&mut outer);
    assert_eq!(
        fired.borrow().as_deref(),
        Some("B"),
        "after one Tab, Enter should fire B"
    );

    fired.borrow_mut().take();
    backend.dispatch(&mut outer, &tab);
    backend.dispatch(&mut outer, &tab_char);
    press_enter(&mut outer);
    assert_eq!(
        fired.borrow().as_deref(),
        Some("C"),
        "after two Tabs, Enter should fire C"
    );

    fired.borrow_mut().take();
    backend.dispatch(&mut outer, &tab);
    backend.dispatch(&mut outer, &tab_char);
    press_enter(&mut outer);
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

    use saudade::mock::MockBackend;

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
    let enter_down = Event::KeyDown {
        key: Key::Named(NamedKey::Enter),
        modifiers: Modifiers::default(),
    };
    let enter_up = Event::KeyUp {
        key: Key::Named(NamedKey::Enter),
        modifiers: Modifiers::default(),
    };
    let press_enter = |w: &mut dyn saudade::Widget| {
        backend.dispatch(w, &enter_down);
        backend.dispatch(w, &enter_up);
    };

    // Enter while the list is focused fires the default button (OK), not
    // the list's own Enter handler.
    press_enter(&mut c);
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

    press_enter(&mut c);
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
    press_enter(&mut c);
    assert_eq!(*fired.borrow(), vec!["OK".to_string()]);
}

/// Drive a `Dropdown` through both input paths: a click opens the list and a
/// click on a row selects it; while focused, Space opens, the arrows move the
/// highlight, and Enter commits. `on_change` must fire exactly on real changes.
#[test]
fn dropdown_open_select_and_keyboard() {
    use std::cell::RefCell;
    use std::rc::Rc;

    use saudade::mock::MockBackend;

    let changes: Rc<RefCell<Vec<usize>>> = Rc::new(RefCell::new(Vec::new()));
    let mut dd = Dropdown::new(Rect::new(0, 0, 100, 24)).with_items(["a", "b", "c"]);
    dd.set_on_change({
        let changes = changes.clone();
        move |_cx, idx| changes.borrow_mut().push(idx)
    });
    assert_eq!(dd.selected_index(), Some(0));

    let backend = MockBackend::new(100, 100).with_scale(1.0);
    let down = |x, y| Event::PointerDown {
        pos: saudade::Point::new(x, y),
        button: saudade::MouseButton::Left,
        modifiers: Modifiers::default(),
    };
    let key = |k| Event::KeyDown {
        key: Key::Named(k),
        modifiers: Modifiers::default(),
    };

    // Click the field to open; popup rows sit flush below it.
    backend.dispatch(&mut dd, &down(50, 12));
    assert!(dd.is_open());
    // Row 2 ("c"): popup top = 24, pad 2, row height 18 → row 2 spans y 62..80.
    backend.dispatch(&mut dd, &down(50, 70));
    assert_eq!(dd.selected_index(), Some(2));
    assert!(!dd.is_open(), "selecting a row closes the list");

    // Keyboard: focus, Space to open, Up to highlight "b", Enter to commit.
    dd.set_focused(true);
    backend.dispatch(&mut dd, &key(NamedKey::Space));
    assert!(dd.is_open());
    backend.dispatch(&mut dd, &key(NamedKey::Up));
    backend.dispatch(&mut dd, &key(NamedKey::Enter));
    assert_eq!(dd.selected_index(), Some(1));
    assert!(!dd.is_open());

    // Escape leaves the selection untouched.
    backend.dispatch(&mut dd, &key(NamedKey::Space));
    backend.dispatch(&mut dd, &key(NamedKey::Down));
    backend.dispatch(&mut dd, &key(NamedKey::Escape));
    assert_eq!(
        dd.selected_index(),
        Some(1),
        "Escape cancels without committing"
    );
    assert!(!dd.is_open());

    assert_eq!(
        *changes.borrow(),
        vec![2, 1],
        "on_change fires once per real change"
    );
}

/// A disabled `Dropdown` accepts no focus and ignores clicks.
#[test]
fn dropdown_disabled_is_inert() {
    use saudade::mock::MockBackend;

    let mut dd = Dropdown::new(Rect::new(0, 0, 100, 24))
        .with_items(["a", "b"])
        .with_enabled(false);
    assert!(!dd.focusable());

    let backend = MockBackend::new(100, 100).with_scale(1.0);
    backend.dispatch(
        &mut dd,
        &Event::PointerDown {
            pos: saudade::Point::new(50, 12),
            button: saudade::MouseButton::Left,
            modifiers: Modifiers::default(),
        },
    );
    assert!(!dd.is_open(), "a disabled dropdown does not open on click");
}

/// An open dropdown owns the keyboard even beside a default button: Enter
/// commits the highlighted row instead of firing the default action. Once the
/// list closes, Enter books as usual. Pins the container's accelerator
/// suppression for a focused, capturing child.
#[test]
fn open_dropdown_blocks_default_button_enter() {
    use std::cell::RefCell;
    use std::rc::Rc;

    use saudade::mock::MockBackend;

    let booked = Rc::new(RefCell::new(0u32));
    let changes: Rc<RefCell<Vec<usize>>> = Rc::new(RefCell::new(Vec::new()));

    let mut dd = Dropdown::new(Rect::new(10, 10, 120, 24)).with_items(["a", "b", "c"]);
    dd.set_on_change({
        let changes = changes.clone();
        move |_cx, idx| changes.borrow_mut().push(idx)
    });
    let book = Button::new(Rect::new(10, 50, 80, 24), "Book")
        .default(true)
        .on_click({
            let booked = booked.clone();
            move |_cx| *booked.borrow_mut() += 1
        });

    let mut container = Container::new(160, 90).add(dd).add(book);
    container.layout(Rect::new(0, 0, 160, 90));
    container.focus_first(); // the dropdown is the first focusable child

    let backend = MockBackend::new(160, 90).with_scale(1.0);
    let down = |x, y| Event::PointerDown {
        pos: saudade::Point::new(x, y),
        button: saudade::MouseButton::Left,
        modifiers: Modifiers::default(),
    };
    let key = |k| Event::KeyDown {
        key: Key::Named(k),
        modifiers: Modifiers::default(),
    };
    let key_up = |k| Event::KeyUp {
        key: Key::Named(k),
        modifiers: Modifiers::default(),
    };

    // Click the field to open, move the highlight to "b", then Enter.
    backend.dispatch(&mut container, &down(20, 20));
    backend.dispatch(&mut container, &key(NamedKey::Down));
    backend.dispatch(&mut container, &key(NamedKey::Enter));
    backend.dispatch(&mut container, &key_up(NamedKey::Enter));
    assert_eq!(
        *changes.borrow(),
        vec![1],
        "Enter commits the open dropdown"
    );
    assert_eq!(
        *booked.borrow(),
        0,
        "the default button must not fire while the list is open"
    );

    // List closed now → Enter fires the default Book button.
    backend.dispatch(&mut container, &key(NamedKey::Enter));
    backend.dispatch(&mut container, &key_up(NamedKey::Enter));
    assert_eq!(*booked.borrow(), 1, "Enter books once the list is closed");
}

/// Sanity-check that the [`MockBackend`] API itself returns a
/// non-empty image. Distinguishes framework breakage from genuine
/// widget regressions.
#[test]
fn snapshot_facility_smoke_test() {
    use saudade::mock::MockBackend;
    let mut root: Box<dyn Widget> = Box::new(
        Container::new(40, 20)
            .with_background(Color::WHITE)
            .add(Label::new(Rect::new(2, 2, 36, 16), "ok")),
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

// ------------------------------------------------------ Window chrome
//
// `render_framed` wraps a client area in Canoe-style window chrome — a teal
// desktop, a soft drop shadow, a title bar, and a frame. The three styles
// differ in their window controls and border; all are drawn active.

/// A simple client area shared by the chrome tests: a label and an OK button on
/// a white background, so the framed window has recognizable content.
fn chrome_content() -> Box<dyn Widget> {
    Box::new(
        Container::new(200, 96)
            .with_background(Color::WHITE)
            .add(Label::new(Rect::new(16, 16, 168, 16), "Document ready."))
            .add(Button::new(Rect::new(60, 56, 80, 24), "OK").default(true)),
    )
}

/// Resizable window: minimize + maximize buttons and the full multi-layer
/// resize border.
#[test]
fn chrome_resizable() {
    let chrome = WindowChrome::resizable("Untitled — Notepad");
    snapshot_framed_at_all_scales("chrome_resizable", 200, 96, &chrome, chrome_content);
}

/// Fixed-size window: a minimize button only, collapsed to a 1px outline border.
#[test]
fn chrome_fixed() {
    let chrome = WindowChrome::fixed("System Monitor");
    snapshot_framed_at_all_scales("chrome_fixed", 200, 96, &chrome, chrome_content);
}

/// Dialog window: no minimize / maximize buttons, dialog-style frame.
#[test]
fn chrome_dialog() {
    let chrome = WindowChrome::dialog("Confirm");
    snapshot_framed_at_all_scales("chrome_dialog", 200, 96, &chrome, chrome_content);
}

/// The desktop color and margin are overridable; everything else stays Canoe's.
#[test]
fn chrome_custom_desktop() {
    let chrome = WindowChrome::resizable("Preferences")
        .with_desktop_background(Color::rgb(0x5A, 0x5A, 0x80))
        .with_margin(20);
    snapshot_framed_at_all_scales("chrome_custom_desktop", 200, 96, &chrome, chrome_content);
}
