use std::cell::{Cell, RefCell};
use std::path::{Path, PathBuf};
use std::rc::Rc;

use crate::event::{Event, EventCtx, Key, NamedKey};
use crate::geometry::{Rect, Size};
use crate::include_svg;
use crate::painter::Painter;
use crate::svg::SvgImage;
use crate::theme::Theme;
use crate::widget::{PopupRequest, Widget};
use crate::widgets::list::{List, ListItem};
use crate::widgets::modal::Modal;
use crate::widgets::{Button, Container, Dropdown, TextInput};

/// Row icons for the combined folder / file list, baked from SVG at compile
/// time — no SVG parser in the binary, crisp at any DPI. Shared with the
/// `filer` example, which bakes the same asset files.
const FOLDER_ICON: SvgImage = include_svg!("assets/icons/folder.svg");
const FILE_ICON: SvgImage = include_svg!("assets/icons/file.svg");
const UP_ICON: SvgImage = include_svg!("assets/icons/up.svg");

// ----------------------------------------------------------------------------
// Layout constants. The dialog is a fixed-size modal; everything inside is
// positioned relative to the client rect (see [`Geometry`]), so these are the
// only magic numbers.
// ----------------------------------------------------------------------------

/// Default client size of the dialog. Roomy enough for one tall list plus the
/// File name / File types rows and the OK / Cancel buttons beside them.
const DIALOG_W: i32 = 460;
const DIALOG_H: i32 = 300;
/// Outer padding inside the dialog client rect.
const PAD: i32 = 14;
/// Height of a one-line text row (the path along the top).
const LABEL_H: i32 = 16;
/// Height of the File name field and the File types dropdown.
const FIELD_H: i32 = 22;
/// Gap between the path / list and the rows above and below them.
const LIST_GAP: i32 = 6;
/// Push-button geometry for the OK / Cancel pair.
const BTN_W: i32 = 75;
const BTN_H: i32 = 26;
const BTN_GAP: i32 = 8;
/// Gap between the File name / File types fields and the button column.
const COL_GAP: i32 = 14;
/// Width reserved for the inline "File name:" / "File types:" labels.
const LABEL_COL_W: i32 = 78;

// Child indices within the `Container` (its add order below). Used to recognise
// "list focused" for the Enter-descends shortcut and to point the label
// mnemonics (Alt+N / Alt+T / Alt+L) at the right widget.
const IDX_NAME: usize = 0;
const IDX_LIST: usize = 1;
const IDX_TYPE: usize = 2;

// Section labels, with their `&` mnemonic markers. `Location` focuses the list,
// `name` the File name field, `types` the filter dropdown.
const LABEL_LOCATION: &str = "&Location:";
const LABEL_NAME: &str = "File &name:";
const LABEL_TYPES: &str = "File &types:";

/// A file-type choice shown in the dialog's "File types" dropdown.
///
/// A filter pairs a human label with one or more glob patterns (`*` and `?`
/// wildcards, matched case-insensitively). The file list shows only the names
/// matching the *selected* filter; switching the dropdown re-filters in place.
///
/// ```
/// use saudade::FileFilter;
///
/// let text = FileFilter::new("Text Files (*.txt)", ["*.txt"]);
/// let any = FileFilter::all_files(); // "All Files (*.*)", matches everything
/// ```
#[derive(Clone, Debug)]
pub struct FileFilter {
    label: String,
    patterns: Vec<String>,
}

impl FileFilter {
    /// A named filter matching any of `patterns` (glob syntax, case-insensitive).
    pub fn new<I, S>(label: impl Into<String>, patterns: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        Self {
            label: label.into(),
            patterns: patterns.into_iter().map(Into::into).collect(),
        }
    }

    /// The catch-all "All Files (\*.\*)" filter, which matches every name.
    pub fn all_files() -> Self {
        Self::new("All Files (*.*)", ["*"])
    }

    pub fn label(&self) -> &str {
        &self.label
    }

    pub fn patterns(&self) -> &[String] {
        &self.patterns
    }

    /// True if any of this filter's patterns matches `name` (case-insensitively).
    pub fn matches(&self, name: &str) -> bool {
        self.patterns.iter().any(|p| glob_match(p, name))
    }
}

/// A modern, single-pane **Open / Save** file dialog.
///
/// Built on the general-purpose [`Modal`](crate::Modal), `FileDialog` presents
/// a file browser in its own top-level window: the current path along the top,
/// one combined list of folders (shown first) and files below it, a **File
/// name** field and a **File types** filter dropdown along the bottom, and
/// OK / Cancel to their right. That's the arrangement modern KDE / Windows
/// pickers use, rather than the Win 3.1 two-column "Directories" / "Drives"
/// layout — a single flat pane with no separate drive selector.
///
/// Each section label carries a keyboard accelerator that moves focus to its
/// control: **Alt+L** ("Location") to the list, **Alt+N** to the File name
/// field, **Alt+T** to the File types filter.
///
/// The application owns it as an overlay — typically `Rc<RefCell<FileDialog>>`
/// added with [`Column::add_overlay`](crate::Column::add_overlay), exactly like
/// [`Dialog`](crate::Dialog) — and opens it with [`show_open`](Self::show_open)
/// or [`show_save`](Self::show_save), passing a callback that receives the
/// chosen [`Path`] when the user confirms. Cancelling (the Cancel button,
/// Escape, or the window's close button) simply closes the dialog without
/// calling back.
///
/// ```no_run
/// use std::cell::RefCell;
/// use std::rc::Rc;
/// use saudade::{FileDialog, FileFilter};
///
/// let dialog = Rc::new(RefCell::new(
///     FileDialog::new().with_filters(vec![
///         FileFilter::new("Text Files (*.txt)", ["*.txt"]),
///         FileFilter::all_files(),
///     ]),
/// ));
///
/// // From a menu / button handler:
/// dialog.borrow_mut().show_open(|cx, path| {
///     // load `path` …
///     cx.request_paint();
/// });
/// ```
///
/// Interaction:
///
/// * single-click a file to put its name in the **File name** field;
/// * double-click a file (or select it and press Enter / OK) to open it;
/// * double-click a folder — or `..` — to descend / ascend (with a folder
///   selected in the list, Enter navigates into it too);
/// * type a name and press Enter / OK to accept it; type a wildcard pattern
///   (e.g. `*.rs`) to re-filter the list; type a directory name to descend.
pub struct FileDialog {
    modal: Modal,
    /// Directory the next `show_*` opens at. Defaults to the process's current
    /// working directory.
    directory: PathBuf,
    /// File-type filters for the dropdown. Defaults to a lone "All Files".
    filters: Vec<FileFilter>,
}

impl FileDialog {
    pub fn new() -> Self {
        Self {
            modal: Modal::new(),
            directory: std::env::current_dir().unwrap_or_else(|_| PathBuf::from("/")),
            filters: vec![FileFilter::all_files()],
        }
    }

    /// Set the directory the dialog opens at.
    pub fn with_directory(mut self, dir: impl Into<PathBuf>) -> Self {
        self.directory = dir.into();
        self
    }

    /// Set the directory the next `show_*` opens at, after construction —
    /// handy for a shared dialog that should reopen near the current document.
    pub fn set_directory(&mut self, dir: impl Into<PathBuf>) {
        self.directory = dir.into();
    }

    /// Replace the file-type filters. The first becomes the initial selection;
    /// an empty list falls back to "All Files".
    pub fn with_filters(mut self, filters: Vec<FileFilter>) -> Self {
        self.filters = if filters.is_empty() {
            vec![FileFilter::all_files()]
        } else {
            filters
        };
        self
    }

    /// Open the dialog to pick a file. `on_open` runs with the chosen path when
    /// the user confirms (Open / double-click / Enter); Cancel or Escape close
    /// it without calling back. The focused **File name** field starts empty —
    /// the user picks a file from the list or types a name.
    pub fn show_open<F>(&mut self, on_open: F)
    where
        F: FnMut(&mut EventCtx, &Path) + 'static,
    {
        // No suggested name → the field opens empty and focused.
        self.show("Open", None, Box::new(on_open));
    }

    /// Open the dialog to choose a save destination. `suggested_name` pre-fills
    /// the focused **File name** field (e.g. the current document's name), fully
    /// selected with the cursor at the end so it's ready to keep, edit, or
    /// replace by typing; `on_save` runs with the chosen path on confirm. The
    /// path need not exist yet.
    pub fn show_save<F>(&mut self, suggested_name: impl Into<String>, on_save: F)
    where
        F: FnMut(&mut EventCtx, &Path) + 'static,
    {
        self.show("Save As", Some(suggested_name.into()), Box::new(on_save));
    }

    fn show(&mut self, title: &str, suggested_name: Option<String>, on_accept: AcceptHandler) {
        let body = FileDialogBody::new(
            self.directory.clone(),
            self.filters.clone(),
            suggested_name,
            on_accept,
        );
        self.modal
            .show(title, Size::new(DIALOG_W, DIALOG_H), Box::new(body));
    }

    /// Close the dialog programmatically (no callback runs).
    pub fn dismiss(&mut self) {
        self.modal.dismiss();
    }

    pub fn is_open(&self) -> bool {
        self.modal.is_open()
    }
}

impl Default for FileDialog {
    fn default() -> Self {
        Self::new()
    }
}

// `FileDialog` is a `Modal` with a fixed body, so the `Widget` surface just
// delegates through — the same shape as `Dialog`.
impl Widget for FileDialog {
    fn bounds(&self) -> Rect {
        self.modal.bounds()
    }
    fn layout(&mut self, bounds: Rect) {
        self.modal.layout(bounds);
    }
    fn paint(&mut self, painter: &mut Painter, theme: &Theme) {
        self.modal.paint(painter, theme);
    }
    fn paint_overlay(&mut self, painter: &mut Painter, theme: &Theme) {
        self.modal.paint_overlay(painter, theme);
    }
    fn event(&mut self, event: &Event, ctx: &mut EventCtx) {
        self.modal.event(event, ctx);
    }
    fn captures_pointer(&self) -> bool {
        self.modal.captures_pointer()
    }
    fn accepts_accelerators(&self) -> bool {
        self.modal.accepts_accelerators()
    }
    fn popup_request(&self) -> Option<PopupRequest> {
        self.modal.popup_request()
    }
    fn collect_popups(&self, out: &mut Vec<PopupRequest>) {
        self.modal.collect_popups(out);
    }
    fn wants_ticks(&self) -> bool {
        self.modal.wants_ticks()
    }
}

// ----------------------------------------------------------------------------
// FileDialogBody — the modal content. Hosts the interactive widgets in a
// `Container` (which gives us focus cycling, Tab, pointer capture, and the
// default-button Enter accelerator for free) and owns the navigation state.
//
// The widgets the body needs to read or mutate after dispatch — the list, the
// name field, the filter dropdown — are held behind `Rc<RefCell<…>>`, with a
// `Shared` clone living in the `Container`. Buttons and dropdown changes report
// back through the small `Signals` cells; the body drains them once `Container`
// dispatch returns, where it can borrow everything without contention.
// ----------------------------------------------------------------------------

type AcceptHandler = Box<dyn FnMut(&mut EventCtx, &Path)>;

/// One-shot flags set by callbacks during `Container` dispatch and drained by
/// the body afterwards. Keeping the work out of the callbacks avoids
/// re-borrowing a widget that's mid-`event`.
#[derive(Default)]
struct Signals {
    /// OK pressed (button click, or Enter via the default-button accelerator).
    accept: Cell<bool>,
    /// "File types" filter changed to this index.
    filter: Cell<Option<usize>>,
}

/// One row in the combined list: its display name and whether it is a directory
/// (true for real subdirectories and the `..` parent entry).
struct Entry {
    name: String,
    is_dir: bool,
}

struct FileDialogBody {
    rect: Rect,
    container: Container,
    name: Rc<RefCell<TextInput>>,
    list: Rc<RefCell<List>>,
    signals: Rc<Signals>,
    on_accept: AcceptHandler,

    // Navigation state.
    dir: PathBuf,
    filters: Vec<FileFilter>,
    /// Patterns the file list is currently filtered by — the selected filter's,
    /// or a one-off wildcard the user typed into the File name field.
    active_patterns: Vec<String>,
    /// True for `show_save` (a suggested name was supplied). In Save mode the
    /// typed filename is carried across folder navigation; in Open mode the
    /// field follows the current folder's selection and clears as you move.
    save_mode: bool,
    /// Entries parallel to the list rows, so an activation or selection change
    /// can tell a folder from a file.
    entries: Vec<Entry>,
    /// List selection at the previous dispatch, so a genuine change can be
    /// reflected into the File name field without clobbering typed text.
    last_sel: Option<usize>,
}

impl FileDialogBody {
    fn new(
        dir: PathBuf,
        filters: Vec<FileFilter>,
        suggested_name: Option<String>,
        on_accept: AcceptHandler,
    ) -> Self {
        let geo = Geometry::compute(Rect::new(0, 0, DIALOG_W, DIALOG_H));
        let signals = Rc::new(Signals::default());

        // The interactive widgets, authored at (0,0)-relative positions; the
        // hosting `Container` shifts them to the dialog's centered origin on
        // layout (the same trick `ConfirmBody` uses).
        let name = Rc::new(RefCell::new(
            TextInput::new(geo.name_field).with_font_size(13.0),
        ));
        let list = Rc::new(RefCell::new(List::new(geo.list)));

        let type_dd = Rc::new(RefCell::new(
            Dropdown::new(geo.type_dd).with_items(filters.iter().map(FileFilter::label)),
        ));
        type_dd.borrow_mut().set_on_change({
            let signals = signals.clone();
            move |_cx, idx| signals.filter.set(Some(idx))
        });

        // OK is the default action, so Enter from any field accepts. Cancel
        // just asks the modal to close.
        let ok = Button::new(geo.ok, "OK").default(true).on_click({
            let signals = signals.clone();
            move |_cx| signals.accept.set(true)
        });
        let cancel = Button::new(geo.cancel, "Cancel").on_click(|cx| cx.request_dismiss());

        // Add order == Tab order: name field, list, filter, OK, Cancel. Must
        // match the IDX_LIST constant above.
        let container = Container::new(DIALOG_W, DIALOG_H)
            .add(Shared(name.clone()))
            .add(Shared(list.clone()))
            .add(Shared(type_dd.clone()))
            .add(ok)
            .add(cancel);

        let active_patterns = filters[0].patterns().to_vec();
        let mut body = Self {
            rect: Rect::new(0, 0, 0, 0),
            container,
            name,
            list,
            signals,
            on_accept,
            dir,
            filters,
            active_patterns,
            save_mode: suggested_name.is_some(),
            entries: Vec::new(),
            last_sel: None,
        };

        // Seed the list, then the File name field. Open starts empty (the user
        // picks or types a name); Save starts on the suggested name, fully
        // selected with the cursor at the end so typing replaces it or the end
        // can be edited straight away.
        body.reload_list();
        if let Some(suggested) = suggested_name {
            let mut name = body.name.borrow_mut();
            name.set_text(&suggested);
            name.select_all();
        }
        body
    }

    /// Re-read the current directory and repopulate the combined list: `..`
    /// (when there's a parent), then the subfolders, then the files matching
    /// the active filter.
    fn reload_list(&mut self) {
        let mut items = Vec::new();
        let mut entries = Vec::new();
        if self.dir.parent().is_some() {
            items.push(ListItem::new("..").with_svg_icon(UP_ICON));
            entries.push(Entry {
                name: "..".to_string(),
                is_dir: true,
            });
        }
        for name in read_dir_names(&self.dir, true) {
            items.push(ListItem::new(name.clone()).with_svg_icon(FOLDER_ICON));
            entries.push(Entry { name, is_dir: true });
        }
        for name in read_dir_names(&self.dir, false) {
            if self.matches_active(&name) {
                items.push(ListItem::new(name.clone()).with_svg_icon(FILE_ICON));
                entries.push(Entry {
                    name,
                    is_dir: false,
                });
            }
        }
        self.entries = entries;
        let mut list = self.list.borrow_mut();
        list.set_items(items);
        // Leave the list unselected; the File name field is only filled once the
        // user actually picks (or types) something.
        list.set_selected(None);
        self.last_sel = None;
    }

    fn matches_active(&self, name: &str) -> bool {
        self.active_patterns.iter().any(|p| glob_match(p, name))
    }

    /// Switch to a new directory and refresh everything that depends on it.
    fn set_dir(&mut self, dir: PathBuf) {
        self.dir = dir;
        self.reload_list();
        // In Open mode the field tracks the current folder's selection, so a
        // stale name carried over from the previous folder is cleared. In Save
        // mode the chosen filename rides along as the user navigates to the
        // target folder.
        if !self.save_mode {
            self.name.borrow_mut().set_text("");
        }
    }

    /// Act on list row `idx`: ascend via `..`, descend into a folder, or open a
    /// file. The single place a double-click / Enter on a row funnels through.
    fn navigate_entry(&mut self, idx: usize, ctx: &mut EventCtx) {
        let Some(entry) = self.entries.get(idx) else {
            return;
        };
        let name = entry.name.clone();
        let is_dir = entry.is_dir;

        if name == ".." {
            if let Some(parent) = self.dir.parent().map(Path::to_path_buf) {
                self.set_dir(parent);
                ctx.request_paint();
            }
            return;
        }

        let target = self.dir.join(&name);
        if is_dir && target.is_dir() {
            self.set_dir(target);
            ctx.request_paint();
        } else {
            // A file row → put its name in the field and accept it.
            self.name.borrow_mut().set_text(&name);
            self.accept(ctx);
        }
    }

    /// Act on the File name field: descend into a directory, re-filter on a
    /// wildcard, or accept a concrete path. The single place OK / Enter /
    /// double-click all funnel through.
    fn accept(&mut self, ctx: &mut EventCtx) {
        let raw = self.name.borrow().text().trim().to_string();
        if raw.is_empty() {
            return;
        }

        // A wildcard pattern re-filters the list rather than opening anything.
        if is_pattern(&raw) {
            self.active_patterns = vec![raw];
            self.reload_list();
            ctx.request_paint();
            return;
        }

        let candidate = {
            let p = Path::new(&raw);
            if p.is_absolute() {
                p.to_path_buf()
            } else {
                self.dir.join(p)
            }
        };

        // A directory name descends into it instead of being "opened".
        if candidate.is_dir() {
            self.set_dir(candidate);
            ctx.request_paint();
            return;
        }

        (self.on_accept)(ctx, &candidate);
        ctx.request_dismiss();
    }

    /// Drain the post-dispatch signals and list activations. Returns once any
    /// of them closes the dialog so we don't keep acting on a torn-down body.
    fn process(&mut self, ctx: &mut EventCtx) {
        if let Some(idx) = self.signals.filter.take()
            && let Some(filter) = self.filters.get(idx)
        {
            // Switching the File types filter only re-filters the list; the
            // File name field is left as the user left it.
            self.active_patterns = filter.patterns().to_vec();
            self.reload_list();
            ctx.request_paint();
        }

        // Bind the activation to a local first so the `RefMut` is dropped before
        // the `&mut self` navigation call borrows the body.
        let activated = self.list.borrow_mut().take_activated();
        if let Some(idx) = activated {
            self.navigate_entry(idx, ctx);
            if ctx.is_dismiss_requested() {
                return;
            }
        }

        // Reflect a fresh single-click *file* selection into the name field;
        // folder selections leave the typed name alone.
        let sel = self.list.borrow().selected_index();
        if sel != self.last_sel {
            self.last_sel = sel;
            let label = sel.and_then(|idx| {
                self.entries
                    .get(idx)
                    .filter(|e| !e.is_dir)
                    .map(|e| e.name.clone())
            });
            if let Some(label) = label {
                self.name.borrow_mut().set_text(&label);
                ctx.request_paint();
            }
        }

        if self.signals.accept.take() {
            self.accept(ctx);
        }
    }
}

impl Widget for FileDialogBody {
    fn bounds(&self) -> Rect {
        self.rect
    }

    fn layout(&mut self, bounds: Rect) {
        self.rect = bounds;
        self.container.layout(bounds);
    }

    fn paint(&mut self, painter: &mut Painter, theme: &Theme) {
        let geo = Geometry::compute(self.rect);
        // No interior fill — the dialog rides on the modal's plain background,
        // the way modern (KDE / Windows) pickers present a flat pane. The
        // sunken white fields read against it on their own.

        let th = painter.measure_text("Ag", theme.font_size).h;

        // "Location:" label, then the current directory beside it (indented to
        // line up with the File name / File types fields below), clipped to its
        // column.
        let loc_y = geo.path_text.y + (geo.path_text.h - th) / 2;
        draw_label(painter, geo.label_x, loc_y, LABEL_LOCATION, theme);
        let saved = painter.push_clip(geo.path_text);
        painter.text(
            geo.path_text.x,
            loc_y,
            &self.dir.display().to_string(),
            theme.font_size,
            theme.text,
        );
        painter.restore_clip(saved);

        // Inline field labels, vertically centered against their fields.
        let name_ly = geo.name_field.y + (geo.name_field.h - th) / 2;
        let type_ly = geo.type_dd.y + (geo.type_dd.h - th) / 2;
        draw_label(painter, geo.label_x, name_ly, LABEL_NAME, theme);
        draw_label(painter, geo.label_x, type_ly, LABEL_TYPES, theme);

        self.container.paint(painter, theme);
    }

    fn paint_overlay(&mut self, painter: &mut Painter, theme: &Theme) {
        self.container.paint_overlay(painter, theme);
    }

    fn event(&mut self, event: &Event, ctx: &mut EventCtx) {
        // Label mnemonics: Alt+L focuses the list, Alt+N the File name field,
        // Alt+T the File types filter. Like the menu bar, they just move
        // focus — the CUA way of reaching a labelled control from the keyboard.
        if let Some(ch) = mnemonic_key(event) {
            let target = match ch {
                'l' => Some(IDX_LIST),
                'n' => Some(IDX_NAME),
                't' => Some(IDX_TYPE),
                _ => None,
            };
            if let Some(idx) = target {
                self.container.focus_child(idx);
                ctx.request_paint();
                return;
            }
        }

        // With the list focused and a folder selected, Enter descends into it.
        // The default OK button would otherwise swallow Enter via the
        // accelerator pass, so intercept it first. A selected file falls
        // through to OK, which accepts the name now sitting in the field.
        if let Event::KeyDown {
            key: Key::Named(NamedKey::Enter),
            ..
        } = event
            && self.container.focused_index() == Some(IDX_LIST)
        {
            let folder = self
                .list
                .borrow()
                .selected_index()
                .filter(|&i| self.entries.get(i).is_some_and(|e| e.is_dir));
            if let Some(idx) = folder {
                self.navigate_entry(idx, ctx);
                return;
            }
        }

        self.container.event(event, ctx);
        if ctx.is_dismiss_requested() {
            return;
        }
        self.process(ctx);
    }

    fn on_cancel(&mut self, _ctx: &mut EventCtx) {
        // Nothing to revert — cancelling just closes the dialog.
    }

    fn focusable(&self) -> bool {
        self.container.focusable()
    }

    fn focus_first(&mut self) -> bool {
        self.container.focus_first()
    }

    fn captures_pointer(&self) -> bool {
        self.container.captures_pointer()
    }

    fn popup_request(&self) -> Option<PopupRequest> {
        self.container.popup_request()
    }

    fn collect_popups(&self, out: &mut Vec<PopupRequest>) {
        self.container.collect_popups(out);
    }

    fn wants_ticks(&self) -> bool {
        self.container.wants_ticks()
    }
}

// ----------------------------------------------------------------------------
// Geometry — every rect inside the dialog, derived from the client rect so the
// modal's centered origin (and any future resize) just works. Child widgets are
// built against `Rect::new(0, 0, W, H)`; the path and labels are painted against
// `self.rect`. The container shifts the children to match.
// ----------------------------------------------------------------------------

struct Geometry {
    path_text: Rect,
    list: Rect,
    /// Left edge for both inline labels ("File name:" / "Filter:").
    label_x: i32,
    name_field: Rect,
    type_dd: Rect,
    ok: Rect,
    cancel: Rect,
}

impl Geometry {
    fn compute(base: Rect) -> Self {
        let left = base.x + PAD;
        let right = base.right() - PAD;
        let top = base.y + PAD;
        let bottom = base.bottom() - PAD;
        let content_w = (right - left).max(0);

        // Two buttons on the right, aligned with the two bottom field rows.
        let btn_x = right - BTN_W;
        let filter_row = bottom - BTN_H;
        let name_row = filter_row - BTN_GAP - BTN_H;
        let ok = Rect::new(btn_x, name_row, BTN_W, BTN_H);
        let cancel = Rect::new(btn_x, filter_row, BTN_W, BTN_H);

        // Fields fill the space between the inline label gutter and the button
        // column, vertically centered in their (taller) button-height rows.
        let fields_right = btn_x - COL_GAP;
        let field_x = left + LABEL_COL_W;
        let field_w = (fields_right - field_x).max(0);
        let field_dy = (BTN_H - FIELD_H) / 2;
        let name_field = Rect::new(field_x, name_row + field_dy, field_w, FIELD_H);
        let type_dd = Rect::new(field_x, filter_row + field_dy, field_w, FIELD_H);

        // Path along the top — indented past the "Location:" label so it lines
        // up with the fields below — then the (full-width) list filling
        // everything between it and the bottom field rows.
        let path_text = Rect::new(field_x, top, (right - field_x).max(0), LABEL_H);
        let list_top = top + LABEL_H + LIST_GAP;
        let list_h = (name_row - LIST_GAP - list_top).max(0);
        let list = Rect::new(left, list_top, content_w, list_h);

        Self {
            path_text,
            list,
            label_x: left,
            name_field,
            type_dd,
            ok,
            cancel,
        }
    }
}

// ----------------------------------------------------------------------------
// Label mnemonics. The dialog's section labels carry a Win 3.1 `&X` mnemonic
// marker; we draw the marked glyph underlined and match Alt+X back to the
// labelled control. (The menu bar has its own copy of this for its own labels.)
// ----------------------------------------------------------------------------

/// If `event` is an Alt+letter mnemonic keystroke, the lowercase letter. Some
/// platforms route it as `KeyDown(Char)`, others as a bare `Char`; accept both,
/// and exclude AltGr so composed characters still reach the text fields.
fn mnemonic_key(event: &Event) -> Option<char> {
    match event {
        Event::KeyDown {
            key: Key::Char(ch),
            modifiers,
        }
        | Event::Char { ch, modifiers }
            if modifiers.mnemonic_alt() =>
        {
            Some(ch.to_ascii_lowercase())
        }
        _ => None,
    }
}

/// Draw a `&X`-marked label at `(x, y)` with the `&` stripped and the marked
/// glyph underlined, so the dialog's accelerators are discoverable.
fn draw_label(painter: &mut Painter, x: i32, y: i32, raw: &str, theme: &Theme) {
    let mut display = String::with_capacity(raw.len());
    let mut mnemonic_index = None;
    let mut chars = raw.chars().peekable();
    let mut idx = 0;
    while let Some(c) = chars.next() {
        if c == '&' {
            if chars.peek() == Some(&'&') {
                chars.next();
                display.push('&');
                idx += 1;
            } else if chars.peek().is_some() {
                mnemonic_index = Some(idx);
            }
        } else {
            display.push(c);
            idx += 1;
        }
    }

    painter.text(x, y, &display, theme.font_size, theme.text);
    if let Some(i) = mnemonic_index {
        let prefix: String = display.chars().take(i).collect();
        let glyph: String = display.chars().skip(i).take(1).collect();
        if glyph.is_empty() {
            return;
        }
        let prefix_w = painter.measure_text(&prefix, theme.font_size).w;
        let glyph_w = painter.measure_text(&glyph, theme.font_size).w;
        // One logical pixel below the baseline so it clears the glyph.
        let underline_y = y + theme.font_size as i32 + 1;
        painter.fill_rect(Rect::new(x + prefix_w, underline_y, glyph_w, 1), theme.text);
    }
}

// ----------------------------------------------------------------------------
// Shared — a generic adapter that lets the body keep an `Rc<RefCell<W>>` handle
// to a widget while a clone lives inside the `Container`. Forwards every
// `Widget` method straight through, the same pattern the examples spell out by
// hand for each widget type.
// ----------------------------------------------------------------------------

struct Shared<W>(Rc<RefCell<W>>);

impl<W: Widget> Widget for Shared<W> {
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
    fn on_cancel(&mut self, ctx: &mut EventCtx) {
        self.0.borrow_mut().on_cancel(ctx);
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
    fn focus_first(&mut self) -> bool {
        self.0.borrow_mut().focus_first()
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

// ----------------------------------------------------------------------------
// Filesystem helpers.
// ----------------------------------------------------------------------------

/// Sorted (case-insensitive) names of either the subdirectories (`dirs == true`)
/// or the regular files (`dirs == false`) directly inside `path`. Symlinks are
/// resolved so a link to a directory is listed as one.
fn read_dir_names(path: &Path, dirs: bool) -> Vec<String> {
    let Ok(read) = std::fs::read_dir(path) else {
        return Vec::new();
    };
    let mut names: Vec<String> = read
        .flatten()
        .filter(|e| e.path().is_dir() == dirs)
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    names.sort_by_key(|n| n.to_lowercase());
    names
}

/// Whether `name` reads as a wildcard pattern rather than a literal name.
fn is_pattern(name: &str) -> bool {
    name.contains('*') || name.contains('?')
}

/// Case-insensitive glob match supporting `*` (any run) and `?` (one char).
/// `*` and `*.*` both behave as match-all for the common "all files" case.
fn glob_match(pattern: &str, name: &str) -> bool {
    if pattern == "*" || pattern == "*.*" {
        return true;
    }
    let p: Vec<char> = pattern.to_lowercase().chars().collect();
    let s: Vec<char> = name.to_lowercase().chars().collect();

    let (mut pi, mut si) = (0usize, 0usize);
    // Position to backtrack to on a `*` mismatch, and how far the star has
    // consumed so far.
    let mut star: Option<usize> = None;
    let mut star_si = 0usize;

    while si < s.len() {
        if pi < p.len() && (p[pi] == '?' || p[pi] == s[si]) {
            pi += 1;
            si += 1;
        } else if pi < p.len() && p[pi] == '*' {
            star = Some(pi);
            star_si = si;
            pi += 1;
        } else if let Some(sp) = star {
            // Let the last `*` swallow one more character and retry.
            pi = sp + 1;
            star_si += 1;
            si = star_si;
        } else {
            return false;
        }
    }
    // Trailing `*`s match the empty remainder.
    while pi < p.len() && p[pi] == '*' {
        pi += 1;
    }
    pi == p.len()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::event::{Event, Key, Modifiers, MouseButton, NamedKey};
    use crate::geometry::Point;
    use crate::mock::MockBackend;
    use std::sync::atomic::{AtomicU32, Ordering};

    // -------------------------------------------------------------- glob / filter

    #[test]
    fn glob_matches_simple_extension() {
        assert!(glob_match("*.txt", "notes.txt"));
        assert!(glob_match("*.txt", "NOTES.TXT")); // case-insensitive
        assert!(!glob_match("*.txt", "image.png"));
    }

    #[test]
    fn glob_star_and_question() {
        assert!(glob_match("*", "anything"));
        assert!(glob_match("*.*", "x")); // all-files alias matches dotless names
        assert!(glob_match("a?c", "abc"));
        assert!(!glob_match("a?c", "ac"));
        assert!(glob_match("read*e", "readme"));
        assert!(glob_match("*.rs", "lib.rs"));
        assert!(!glob_match("*.rs", "lib.rs.bak"));
    }

    #[test]
    fn filter_matches_any_pattern() {
        let f = FileFilter::new("Images", ["*.png", "*.jpg"]);
        assert!(f.matches("photo.jpg"));
        assert!(f.matches("photo.PNG"));
        assert!(!f.matches("photo.gif"));
    }

    #[test]
    fn all_files_filter_matches_everything() {
        let f = FileFilter::all_files();
        assert!(f.matches("README"));
        assert!(f.matches("a.tar.gz"));
    }

    #[test]
    fn is_pattern_detects_wildcards() {
        assert!(is_pattern("*.txt"));
        assert!(is_pattern("file?.dat"));
        assert!(!is_pattern("notes.txt"));
    }

    // -------------------------------------------------------------- behaviour

    /// A throwaway directory under the system temp dir, unique per test.
    fn unique_temp() -> PathBuf {
        static N: AtomicU32 = AtomicU32::new(0);
        let n = N.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!("saudade_fd_{}_{}", std::process::id(), n));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("create temp dir");
        dir
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

    /// An Alt+`letter` mnemonic keystroke.
    fn alt(letter: char) -> Event {
        Event::KeyDown {
            key: Key::Char(letter),
            modifiers: Modifiers {
                alt: true,
                ..Default::default()
            },
        }
    }

    /// Type `s` into the focused widget, one `Char` event per character.
    fn type_text(backend: &MockBackend, dlg: &mut FileDialog, s: &str) {
        for ch in s.chars() {
            backend.dispatch(
                dlg,
                &Event::Char {
                    ch,
                    modifiers: Default::default(),
                },
            );
        }
    }

    fn click(backend: &MockBackend, dlg: &mut FileDialog, x: i32, y: i32) {
        let pos = Point::new(x, y);
        backend.dispatch(
            dlg,
            &Event::PointerDown {
                pos,
                button: MouseButton::Left,
            },
        );
        backend.dispatch(
            dlg,
            &Event::PointerUp {
                pos,
                button: MouseButton::Left,
            },
        );
    }

    /// The centered dialog fills the whole backend, so its geometry is computed
    /// against the origin — these are the logical coordinates events use.
    fn geo() -> Geometry {
        Geometry::compute(Rect::new(0, 0, DIALOG_W, DIALOG_H))
    }

    /// Center y of list row `row` for a list at `list_rect`.
    fn row_y(list_rect: Rect, row: i32) -> i32 {
        // Mirrors `List`: rows start at rect.y + TEXT_PAD_Y (2), each 18 tall.
        list_rect.y + 2 + row * 18 + 9
    }

    #[test]
    fn open_then_dismiss_toggles_state() {
        let dir = unique_temp();
        let mut dlg = FileDialog::new().with_directory(&dir);
        assert!(!dlg.is_open());
        dlg.show_open(|_, _| {});
        assert!(dlg.is_open());
        dlg.dismiss();
        assert!(!dlg.is_open());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn renders_populated_dialog_without_panicking() {
        let dir = unique_temp();
        std::fs::write(dir.join("a.txt"), b"x").unwrap();
        std::fs::create_dir(dir.join("sub")).unwrap();
        let mut dlg = FileDialog::new().with_directory(&dir);
        dlg.show_save("draft.txt", |_, _| {});

        let backend = MockBackend::new(DIALOG_W, DIALOG_H);
        let snap = backend.render(&mut dlg);
        assert_eq!(snap.width(), DIALOG_W);
        assert_eq!(snap.height(), DIALOG_H);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn selecting_a_file_and_pressing_enter_accepts_its_path() {
        let dir = unique_temp();
        std::fs::write(dir.join("hello.txt"), b"hi").unwrap();
        std::fs::write(dir.join("image.png"), b"nope").unwrap();
        std::fs::create_dir(dir.join("sub")).unwrap();

        let chosen: Rc<RefCell<Option<PathBuf>>> = Rc::new(RefCell::new(None));
        let mut dlg = FileDialog::new().with_directory(&dir).with_filters(vec![
            FileFilter::new("Text Files (*.txt)", ["*.txt"]),
            FileFilter::all_files(),
        ]);
        {
            let chosen = chosen.clone();
            dlg.show_open(move |_cx, path| *chosen.borrow_mut() = Some(path.to_path_buf()));
        }

        let backend = MockBackend::new(DIALOG_W, DIALOG_H);
        backend.render(&mut dlg); // lays the dialog out, centered at the origin

        // The *.txt filter leaves the combined list as `..`, the `sub` folder,
        // then `hello.txt` (image.png is filtered out). Click the file to load
        // the name field, then Enter fires the default OK button.
        let g = geo();
        click(&backend, &mut dlg, g.list.x + 24, row_y(g.list, 2));
        backend.dispatch(&mut dlg, &key(NamedKey::Enter, true));
        backend.dispatch(&mut dlg, &key(NamedKey::Enter, false));

        assert_eq!(
            chosen.borrow().as_deref(),
            Some(dir.join("hello.txt").as_path()),
            "accepting yields the selected file's full path"
        );
        assert!(!dlg.is_open(), "accepting closes the dialog");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn double_clicking_a_directory_descends_into_it() {
        let dir = unique_temp();
        let sub = dir.join("sub");
        std::fs::create_dir(&sub).unwrap();
        std::fs::write(sub.join("deep.txt"), b"deep").unwrap();

        let chosen: Rc<RefCell<Option<PathBuf>>> = Rc::new(RefCell::new(None));
        let mut dlg = FileDialog::new()
            .with_directory(&dir)
            .with_filters(vec![FileFilter::new("Text Files (*.txt)", ["*.txt"])]);
        {
            let chosen = chosen.clone();
            dlg.show_open(move |_cx, path| *chosen.borrow_mut() = Some(path.to_path_buf()));
        }

        let backend = MockBackend::new(DIALOG_W, DIALOG_H);
        backend.render(&mut dlg);

        // Combined list: row 0 is "..", row 1 is the "sub" folder (no *.txt
        // files sit in `dir`). Two quick presses on "sub" register as a
        // double-click and descend into it.
        let g = geo();
        let (dx, dy) = (g.list.x + 24, row_y(g.list, 1));
        let down = Event::PointerDown {
            pos: Point::new(dx, dy),
            button: MouseButton::Left,
        };
        backend.dispatch(&mut dlg, &down);
        backend.dispatch(&mut dlg, &down);

        // Now inside `sub`: row 0 is "..", row 1 is deep.txt — select it and
        // accept.
        let g = geo();
        click(&backend, &mut dlg, g.list.x + 24, row_y(g.list, 1));
        backend.dispatch(&mut dlg, &key(NamedKey::Enter, true));
        backend.dispatch(&mut dlg, &key(NamedKey::Enter, false));

        assert_eq!(
            chosen.borrow().as_deref(),
            Some(sub.join("deep.txt").as_path()),
            "navigation followed by accept yields a path inside the subdirectory"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn alt_l_focuses_the_list_so_enter_descends() {
        let dir = unique_temp();
        let sub = dir.join("sub");
        std::fs::create_dir(&sub).unwrap();
        std::fs::write(sub.join("deep.txt"), b"deep").unwrap();

        let chosen: Rc<RefCell<Option<PathBuf>>> = Rc::new(RefCell::new(None));
        let mut dlg = FileDialog::new()
            .with_directory(&dir)
            .with_filters(vec![FileFilter::new("Text Files (*.txt)", ["*.txt"])]);
        {
            let chosen = chosen.clone();
            dlg.show_open(move |_cx, path| *chosen.borrow_mut() = Some(path.to_path_buf()));
        }
        let backend = MockBackend::new(DIALOG_W, DIALOG_H);
        backend.render(&mut dlg);

        // The dialog opens with the File name field focused. Alt+L moves focus
        // to the list; only then does Enter-on-a-folder descend (it's the
        // "list focused" shortcut) rather than firing OK. Select the "sub"
        // folder (the first Down moves off the empty selection onto row 1) and
        // descend with Enter.
        backend.dispatch(&mut dlg, &alt('l'));
        backend.dispatch(&mut dlg, &key(NamedKey::Down, true));
        backend.dispatch(&mut dlg, &key(NamedKey::Enter, true));
        backend.dispatch(&mut dlg, &key(NamedKey::Enter, false));

        // Inside `sub`: select deep.txt (row 1) and accept it.
        backend.dispatch(&mut dlg, &key(NamedKey::Down, true));
        backend.dispatch(&mut dlg, &key(NamedKey::Enter, true));
        backend.dispatch(&mut dlg, &key(NamedKey::Enter, false));

        assert_eq!(
            chosen.borrow().as_deref(),
            Some(sub.join("deep.txt").as_path()),
            "Alt+L focused the list, so keyboard navigation descended and accepted"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn typing_a_wildcard_refilters_without_accepting() {
        let dir = unique_temp();
        std::fs::write(dir.join("hello.txt"), b"hi").unwrap();
        std::fs::write(dir.join("lib.rs"), b"//").unwrap();

        let chosen: Rc<RefCell<Option<PathBuf>>> = Rc::new(RefCell::new(None));
        let mut dlg = FileDialog::new().with_directory(&dir);
        {
            let chosen = chosen.clone();
            dlg.show_open(move |_cx, path| *chosen.borrow_mut() = Some(path.to_path_buf()));
        }
        let backend = MockBackend::new(DIALOG_W, DIALOG_H);
        backend.render(&mut dlg);

        // The File name field opens empty and focused. Type a wildcard, then
        // Enter re-filters the list rather than accepting, so the dialog stays
        // open and nothing is chosen.
        type_text(&backend, &mut dlg, "*.rs");
        backend.dispatch(&mut dlg, &key(NamedKey::Enter, true));
        backend.dispatch(&mut dlg, &key(NamedKey::Enter, false));

        assert!(
            chosen.borrow().is_none(),
            "a wildcard pattern is not accepted"
        );
        assert!(dlg.is_open(), "the dialog stays open after re-filtering");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn open_starts_empty_and_save_starts_selected() {
        let dir = unique_temp();
        std::fs::write(dir.join("hello.txt"), b"hi").unwrap();

        // Open: the File name field is empty, so OK / Enter with nothing picked
        // does nothing — the dialog stays open and no path is chosen.
        let chosen: Rc<RefCell<Option<PathBuf>>> = Rc::new(RefCell::new(None));
        let mut dlg = FileDialog::new().with_directory(&dir);
        {
            let chosen = chosen.clone();
            dlg.show_open(move |_cx, path| *chosen.borrow_mut() = Some(path.to_path_buf()));
        }
        let backend = MockBackend::new(DIALOG_W, DIALOG_H);
        backend.render(&mut dlg);
        backend.dispatch(&mut dlg, &key(NamedKey::Enter, true));
        backend.dispatch(&mut dlg, &key(NamedKey::Enter, false));
        assert!(
            chosen.borrow().is_none(),
            "an empty Open field accepts nothing"
        );
        assert!(dlg.is_open(), "Open stays open with an empty field");

        // Save: the field is pre-filled and fully selected, so typing replaces
        // the whole suggested name in one go (proving the selection covered it).
        let chosen: Rc<RefCell<Option<PathBuf>>> = Rc::new(RefCell::new(None));
        let mut dlg = FileDialog::new().with_directory(&dir);
        {
            let chosen = chosen.clone();
            dlg.show_save("draft.txt", move |_cx, path| {
                *chosen.borrow_mut() = Some(path.to_path_buf())
            });
        }
        backend.render(&mut dlg);
        type_text(&backend, &mut dlg, "final.txt");
        backend.dispatch(&mut dlg, &key(NamedKey::Enter, true));
        backend.dispatch(&mut dlg, &key(NamedKey::Enter, false));
        assert_eq!(
            chosen.borrow().as_deref(),
            Some(dir.join("final.txt").as_path()),
            "typing over the fully-selected suggested name replaces it entirely"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }
}
