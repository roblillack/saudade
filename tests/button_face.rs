//! Push buttons set their labels in the theme's button face — and nothing else
//! does.
//!
//! `Theme::button_style` is bold by default, and a family the host ships no
//! bold for falls back to its regular face — which is exactly what the bundled
//! DejaVu Sans does, so simply asking it for bold would prove nothing. Each
//! case therefore draws against a sans font carrying a *stand-in* bold face
//! that could never be mistaken for its regular one (the bundled monospace),
//! and renders the same tree twice: once under the default theme, once with the
//! button face forced back to `Regular`. A widget that reads the theme renders
//! the two differently; one that draws in the regular face regardless renders
//! them identically.

use saudade::mock::MockBackend;
use saudade::{
    Button, Checkbox, Color, Container, ContextMenu, Font, FontStyle, Menu, MenuBar, MenuItem,
    ModifierScheme, Point, Rect, Theme, Widget,
};

/// The family every case draws with: DejaVu Sans, with DejaVu Sans Mono standing
/// in for the bold face the repo ships no file for.
fn sans_with_stand_in_bold() -> Font {
    Font::from_sans_bytes(include_bytes!("fonts/DejaVuSans.ttf").to_vec())
        .expect("bundled DejaVuSans.ttf failed to load")
        .with_bold_bytes(include_bytes!("fonts/DejaVuSansMono.ttf").to_vec())
}

fn render(style: FontStyle, w: i32, h: i32, root: &mut dyn Widget) -> Vec<u32> {
    let theme = Theme {
        button_style: style,
        ..Theme::default()
    };
    MockBackend::new(w, h)
        .with_theme(theme)
        .with_sans_font(sans_with_stand_in_bold())
        .render(root)
        .pixels()
        .to_vec()
}

/// Whether the tree `build` puts together comes out differently with the theme's
/// button face bold than with it forced to regular — i.e. whether it draws
/// through `Theme::button_style` at all.
fn follows_the_button_face<F>(w: i32, h: i32, mut build: F) -> bool
where
    F: FnMut() -> Box<dyn Widget>,
{
    render(FontStyle::Bold, w, h, build().as_mut())
        != render(FontStyle::Regular, w, h, build().as_mut())
}

#[test]
fn the_default_theme_asks_for_bold() {
    assert_eq!(Theme::default().button_style, FontStyle::Bold);
}

#[test]
fn a_button_label_follows_the_button_face() {
    assert!(follows_the_button_face(120, 40, || {
        Box::new(
            Container::new(120, 40)
                .with_background(Color::LIGHT_GRAY)
                .add(Button::new(Rect::new(20, 8, 80, 24), "Cancel")),
        )
    }));
}

#[test]
fn a_disabled_button_label_follows_it_too() {
    // The greyed label is drawn twice (the engraved white copy under the grey
    // one) — both copies have to pick up the same face.
    assert!(follows_the_button_face(120, 40, || {
        Box::new(
            Container::new(120, 40)
                .with_background(Color::LIGHT_GRAY)
                .add(Button::new(Rect::new(20, 8, 80, 24), "Cancel").with_enabled(false)),
        )
    }));
}

#[test]
fn a_menu_bar_and_the_menu_it_drops_do_not() {
    // Menus keep the regular face at their regular width: the bar labels, the
    // rows of the drop-down, and the accelerator hints alongside them.
    assert!(!follows_the_button_face(220, 120, || {
        let mut bar = MenuBar::new(Rect::new(0, 0, 220, 20))
            .with_scheme(ModifierScheme::Pc)
            .add_menu(Menu::new(
                "&File",
                vec![
                    MenuItem::action("&New", |_| {}),
                    MenuItem::action("&Open…", |_| {}).with_accel("Ctrl+O"),
                    MenuItem::separator(),
                    MenuItem::action("E&xit", |_| {}),
                ],
            ))
            .add_menu(Menu::new("&Edit", vec![MenuItem::action("&Copy", |_| {})]));
        bar.open(0);
        Box::new(
            Container::new(220, 120)
                .with_background(Color::WHITE)
                .add(bar),
        )
    }));
}

#[test]
fn a_context_menu_does_not_either() {
    assert!(!follows_the_button_face(200, 130, || {
        let mut menu = ContextMenu::new()
            .with_scheme(ModifierScheme::Pc)
            .with_items(vec![
                MenuItem::action("&Edit", |_| {}),
                MenuItem::action("D&uplicate", |_| {}).with_accel("Ctrl+D"),
                MenuItem::separator(),
                MenuItem::action("De&lete", |_| {}).with_enabled(|| false),
            ]);
        menu.open_at(Point::new(20, 18));
        Box::new(
            Container::new(200, 130)
                .with_background(Color::LIGHT_GRAY)
                .add(menu),
        )
    }));
}

#[test]
fn an_ordinary_caption_does_not() {
    // The knob is deliberately narrow: it is the push button's face, not a
    // weight for the chrome at large.
    assert!(!follows_the_button_face(200, 28, || {
        Box::new(
            Container::new(200, 28)
                .with_background(Color::LIGHT_GRAY)
                .add(Checkbox::new(Rect::new(8, 6, 184, 16), "Add to favorites")),
        )
    }));
}
