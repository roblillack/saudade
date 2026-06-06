use std::cell::RefCell;
use std::env;
use std::fs;
use std::path::PathBuf;
use std::rc::Rc;

use saudade::{
    App, Column, Dialog, Event, EventCtx, FileDialog, FileFilter, Menu, MenuBar, MenuItem, Painter,
    PopupRequest, Rect, TextEditor, Theme, Widget, WindowConfig,
};

const WINDOW_W: i32 = 520;
const WINDOW_H: i32 = 340;
const MENU_BAR_H: i32 = 20;

fn main() {
    // First positional argument (if any) is the file we open and save to.
    // Notepad has always had exactly one document — so do we, for now. The
    // path is shared because Open / Save As now retarget it at runtime.
    let initial_path: PathBuf = env::args()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("notepad.txt"));

    // Initial editor rect doesn't matter: Column::layout will resize it to
    // fill the window the moment the runtime starts.
    let mut editor = TextEditor::new(Rect::new(0, 0, 0, 0)).with_font_size(11.0);
    if let Ok(content) = fs::read_to_string(&initial_path) {
        editor.set_text(&content);
    }
    let editor = Rc::new(RefCell::new(editor));
    let path = Rc::new(RefCell::new(initial_path));

    // Shared dialog for "not implemented yet" warnings and the About box.
    // Menu callbacks borrow it mutably to show; the OK button inside the
    // dialog calls `dismiss` itself.
    let dialog = Rc::new(RefCell::new(Dialog::new()));

    // The Open / Save As file picker, shared so both menu items drive the same
    // overlay. Notepad edits plain text, so it offers a *.txt filter plus a
    // catch-all.
    let file_dialog = Rc::new(RefCell::new(FileDialog::new().with_filters(vec![
        FileFilter::new("Text Files (*.txt)", ["*.txt"]),
        FileFilter::all_files(),
    ])));

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
                    let file_dialog = file_dialog.clone();
                    let editor = editor.clone();
                    let path = path.clone();
                    move |cx| {
                        open_dialog_at(&file_dialog, &path);
                        let editor = editor.clone();
                        let path = path.clone();
                        file_dialog.borrow_mut().show_open(move |cx, chosen| {
                            if let Ok(content) = fs::read_to_string(chosen) {
                                editor.borrow_mut().set_text(&content);
                                *path.borrow_mut() = chosen.to_path_buf();
                                cx.request_paint();
                            }
                        });
                        cx.request_paint();
                    }
                }),
                MenuItem::action("&Save", {
                    let editor = editor.clone();
                    let path = path.clone();
                    move |_cx| {
                        let _ = fs::write(&*path.borrow(), editor.borrow().text());
                    }
                }),
                MenuItem::action("Save &As...", {
                    let file_dialog = file_dialog.clone();
                    let editor = editor.clone();
                    let path = path.clone();
                    move |cx| {
                        open_dialog_at(&file_dialog, &path);
                        let suggested = path
                            .borrow()
                            .file_name()
                            .map(|n| n.to_string_lossy().into_owned())
                            .unwrap_or_else(|| "Untitled.txt".to_string());
                        let editor = editor.clone();
                        let path = path.clone();
                        file_dialog.borrow_mut().show_save(suggested, move |cx, chosen| {
                            if fs::write(chosen, editor.borrow().text()).is_ok() {
                                *path.borrow_mut() = chosen.to_path_buf();
                            }
                            cx.request_paint();
                        });
                        cx.request_paint();
                    }
                }),
                MenuItem::separator(),
                MenuItem::action("Page Set&up...", warn(&dialog, "Page Setup", UNIMPL)),
                MenuItem::action("&Print...", warn(&dialog, "Print", UNIMPL)),
                MenuItem::separator(),
                MenuItem::action("E&xit", |cx| cx.close()),
            ],
        ))
        .add_menu(Menu::new(
            "&Edit",
            vec![
                MenuItem::action("&Undo", warn(&dialog, "Undo", UNIMPL)),
                MenuItem::action("&Redo", warn(&dialog, "Redo", UNIMPL)),
                MenuItem::separator(),
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
                MenuItem::action("&Find...", warn(&dialog, "Find", UNIMPL)),
                MenuItem::action("Find &Next", warn(&dialog, "Find Next", UNIMPL)),
                MenuItem::action("&Replace...", warn(&dialog, "Replace", UNIMPL)),
                MenuItem::action("&Go To...", warn(&dialog, "Go To Line", UNIMPL)),
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
            "F&ormat",
            vec![
                MenuItem::action("&Word Wrap", warn(&dialog, "Word Wrap", UNIMPL)),
                MenuItem::action("&Font...", warn(&dialog, "Font", UNIMPL)),
            ],
        ))
        .add_menu(Menu::new(
            "&Help",
            vec![MenuItem::action("&About Notepad", {
                let dialog = dialog.clone();
                move |cx| {
                    dialog.borrow_mut().show_info(
                        "About Notepad",
                        "notepad\n\nA saudade demonstration.\n\nBuilt on saudade — a\nminimal Win 3.1-styled\nGUI toolkit in Rust.",
                    );
                    cx.request_paint();
                }
            })],
        ));

    // The Column layout makes the menu bar a fixed strip at the top spanning
    // the full window width, and lets the editor flex to fill the rest. The
    // dialog floats on top as an overlay (no layout slot). The runtime
    // auto-focuses the editor on startup.
    let root = Column::new()
        .add_fixed(menu_bar, MENU_BAR_H)
        .add_fill(SharedEditor(editor.clone()))
        .add_overlay(SharedDialog(dialog.clone()))
        .add_overlay(SharedFileDialog(file_dialog.clone()));

    App::new(
        WindowConfig::new("Notepad", WINDOW_W, WINDOW_H).resizable(true),
        root,
    )
    .with_theme(Theme::windows_31())
    .run();
}

/// Point the file dialog at the directory holding the current document, so it
/// reopens where the user last was rather than at the process's working
/// directory. A no-op for a bare relative filename with no parent.
fn open_dialog_at(file_dialog: &Rc<RefCell<FileDialog>>, path: &Rc<RefCell<PathBuf>>) {
    if let Some(parent) = path.borrow().parent() {
        // Treat an empty parent (a bare filename) as "current directory".
        if !parent.as_os_str().is_empty() {
            file_dialog.borrow_mut().set_directory(parent.to_path_buf());
        }
    }
}

/// Shorthand: turn `(dialog, title, body)` into a menu callback that pops
/// the warning dialog with that title and body.
fn warn(
    dialog: &Rc<RefCell<Dialog>>,
    title: &'static str,
    body: &'static str,
) -> impl FnMut(&mut EventCtx) + 'static {
    let dialog = dialog.clone();
    move |cx| {
        dialog.borrow_mut().show_warning(title, body);
        cx.request_paint();
    }
}

const UNIMPL: &str =
    "This action is not implemented\nyet in saudade's notepad demo.\n\nClick OK to dismiss.";

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
    fn wants_ticks(&self) -> bool {
        self.0.borrow().wants_ticks()
    }
    fn layout(&mut self, bounds: Rect) {
        self.0.borrow_mut().layout(bounds);
    }
    fn popup_request(&self) -> Option<PopupRequest> {
        self.0.borrow().popup_request()
    }
}

/// Same Rc-wrapper trick for the shared `Dialog` so menu callbacks can
/// mutate it while it's still installed in the widget tree as an overlay.
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

/// And once more for the shared `FileDialog`. It forwards a couple more hooks
/// than the message box: `collect_popups` so its "Drives" / "List Files of
/// Type" dropdowns can open their own popup windows, and `wants_ticks` so the
/// File Name field's caret blinks.
struct SharedFileDialog(Rc<RefCell<FileDialog>>);

impl Widget for SharedFileDialog {
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
