use std::cell::RefCell;
use std::env;
use std::fs;
use std::path::PathBuf;
use std::rc::Rc;

use retrogui::{
    App, Color, Column, Event, EventCtx, Menu, MenuBar, MenuItem, Painter, Rect, TextEditor, Theme,
    Widget, WindowConfig,
};

const WINDOW_W: i32 = 520;
const WINDOW_H: i32 = 340;
const MENU_BAR_H: i32 = 20;

fn main() {
    // First positional argument (if any) is the file we open and save to.
    // Notepad has always had exactly one document — so do we, for now.
    let path: PathBuf = env::args()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("notepad.txt"));

    // Initial editor rect doesn't matter: Column::layout will resize it to
    // fill the window the moment the runtime starts.
    let mut editor = TextEditor::new(Rect::new(0, 0, 0, 0)).with_font_size(11.0);
    if let Ok(content) = fs::read_to_string(&path) {
        editor.set_text(&content);
    }
    let editor = Rc::new(RefCell::new(editor));

    let menu_bar = MenuBar::new(Rect::new(0, 0, WINDOW_W, MENU_BAR_H))
        .add_menu(Menu::new(
            "&File",
            vec![
                MenuItem::action("&New", {
                    let editor = editor.clone();
                    move |cx| {
                        editor.borrow_mut().set_text("");
                        cx.request_paint();
                    }
                }),
                MenuItem::action("&Open", {
                    let editor = editor.clone();
                    let path = path.clone();
                    move |cx| {
                        if let Ok(content) = fs::read_to_string(&path) {
                            editor.borrow_mut().set_text(&content);
                            cx.request_paint();
                        }
                    }
                }),
                MenuItem::action("&Save", {
                    let editor = editor.clone();
                    let path = path.clone();
                    move |_cx| {
                        let _ = fs::write(&path, editor.borrow().text());
                    }
                }),
                MenuItem::separator(),
                MenuItem::action("E&xit", |cx| cx.close()),
            ],
        ))
        .add_menu(Menu::new(
            "&Edit",
            vec![
                MenuItem::action("Cu&t", {
                    let editor = editor.clone();
                    move |cx| {
                        editor.borrow_mut().cut();
                        cx.request_paint();
                    }
                }),
                MenuItem::action("&Copy", {
                    let editor = editor.clone();
                    move |_cx| {
                        editor.borrow_mut().copy();
                    }
                }),
                MenuItem::action("&Paste", {
                    let editor = editor.clone();
                    move |cx| {
                        editor.borrow_mut().paste();
                        cx.request_paint();
                    }
                }),
                MenuItem::separator(),
                MenuItem::action("Select &All", {
                    let editor = editor.clone();
                    move |cx| {
                        editor.borrow_mut().select_all();
                        cx.request_paint();
                    }
                }),
            ],
        ))
        .add_menu(Menu::new(
            "&Help",
            vec![MenuItem::action("&About Notepad", |_| {
                eprintln!("notepad — a retrogui demo");
            })],
        ));

    // The Column layout makes the menu bar a fixed strip at the top spanning
    // the full window width, and lets the editor flex to fill the rest.
    let mut root = Column::new()
        .with_background(Color::WHITE)
        .add_fixed(menu_bar, MENU_BAR_H)
        .add_fill(SharedEditor(editor.clone()));
    root.focus_first();

    App::new(
        WindowConfig::new("Notepad", WINDOW_W, WINDOW_H).resizable(true),
        root,
    )
    .with_theme(Theme::windows_31())
    .run();
}

/// Tiny adapter that lets us hold a `TextEditor` in a `Rc<RefCell>` while
/// still satisfying the `Widget` trait. The menu callbacks clone the `Rc` so
/// they can mutate the editor's text in response to File → New / Open / Save.
struct SharedEditor(Rc<RefCell<TextEditor>>);

impl Widget for SharedEditor {
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
}
