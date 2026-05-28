//! Pixel-snapshot tests for every retrogui widget.
//!
//! Each test renders a small fixed-size widget tree through [`MockBackend`]
//! at 1.0x, 1.25x, 1.5x, and 2.0x, then compares the resulting PNG bytes
//! to a checked-in baseline via `insta::assert_binary_snapshot!`. Review
//! diffs visually with `cargo insta review`.

mod common;

use common::snapshot_at_all_scales;

use retrogui::{
    Bevel, Button, Color, Column, Container, Dialog, Image, Label, Menu, MenuBar, MenuItem,
    Orientation, Rect, ScrollBar, TextEditor, Widget,
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
