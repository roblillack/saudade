use std::cell::{Cell, RefCell};
use std::path::{Path, PathBuf};
use std::rc::Rc;

use crate::event::{Event, EventCtx, Key, NamedKey};
use crate::geometry::{Color, Rect, Size};
use crate::painter::Painter;
use crate::theme::Theme;
use crate::widget::{PopupRequest, Widget};
use crate::widgets::list::{List, ListIcon, ListItem};
use crate::widgets::modal::Modal;
use crate::widgets::{Button, Container, Dropdown, TextInput};

// ----------------------------------------------------------------------------
// Layout constants. The dialog is a fixed-size modal; everything inside is
// positioned relative to the client rect (see [`Geometry`]), so these are the
// only magic numbers.
// ----------------------------------------------------------------------------

/// Default client size of the dialog. Roomy enough for two tall list columns
/// plus the OK / Cancel button stack on the right.
const DIALOG_W: i32 = 460;
const DIALOG_H: i32 = 300;
/// Outer padding inside the dialog client rect.
const PAD: i32 = 14;
/// Height of a section label ("File Name:", "Directories:", …).
const LABEL_H: i32 = 16;
/// Height of the text field and the two dropdowns.
const FIELD_H: i32 = 22;
/// Gap between a field/label and the list below it.
const LIST_GAP: i32 = 6;
/// Push-button geometry for the OK / Cancel stack.
const BTN_W: i32 = 75;
const BTN_H: i32 = 26;
const BTN_GAP: i32 = 8;
/// Gap between the two list columns, and before the button stack.
const COL_GAP: i32 = 14;

// Index of the directory list among the `Container`'s children (its add order
// below). Used to recognise "directory list focused" for the Enter-navigates
// shortcut; the other children don't need naming.
const IDX_DIRS: usize = 3;

/// A file-type choice shown in the dialog's "List Files of Type" dropdown.
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

/// A classic Win 3.1-style **Open / Save** file dialog.
///
/// Built on the general-purpose [`Modal`](crate::Modal), `FileDialog` presents
/// the familiar two-column file browser in its own top-level window: a
/// directory list and "Drives" picker on the left, a file list with a "File
/// Name" field and "List Files of Type" filter on the right, and OK / Cancel on
/// the far right. (The column order is the one swap from the Windows 3.1
/// original — folders sit on the left, the way modern file pickers arrange
/// them.)
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
/// Interaction mirrors the original:
///
/// * single-click a file to put its name in the **File Name** field;
/// * double-click a file (or select it and press Enter / OK) to open it;
/// * double-click a directory — or `..` — to descend / ascend (with the
///   directory list focused, Enter navigates too);
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
    /// it without calling back. The **File Name** field starts on the selected
    /// filter's pattern, the Win 3.1 way.
    pub fn show_open<F>(&mut self, on_open: F)
    where
        F: FnMut(&mut EventCtx, &Path) + 'static,
    {
        // No suggested name → the body seeds the field with the active filter
        // pattern (e.g. `*.txt`).
        self.show("Open", None, Box::new(on_open));
    }

    /// Open the dialog to choose a save destination. `suggested_name` pre-fills
    /// the **File Name** field (e.g. the current document's name); `on_save`
    /// runs with the chosen path on confirm. The path need not exist yet.
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
// The widgets the body needs to read or mutate after dispatch — the lists, the
// name field, the dropdowns — are held behind `Rc<RefCell<…>>`, with a
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
    /// "List Files of Type" changed to this index.
    filter: Cell<Option<usize>>,
    /// "Drives" changed to this index.
    drive: Cell<Option<usize>>,
}

struct FileDialogBody {
    rect: Rect,
    container: Container,
    name: Rc<RefCell<TextInput>>,
    files: Rc<RefCell<List>>,
    dirs: Rc<RefCell<List>>,
    drive_dd: Rc<RefCell<Dropdown>>,
    signals: Rc<Signals>,
    on_accept: AcceptHandler,

    // Navigation state.
    dir: PathBuf,
    filters: Vec<FileFilter>,
    /// Patterns the file list is currently filtered by — the selected filter's,
    /// or a one-off wildcard the user typed into the File Name field.
    active_patterns: Vec<String>,
    drives: Vec<PathBuf>,
    icons: Icons,
    /// File-list selection at the previous dispatch, so a genuine change can be
    /// reflected into the File Name field without clobbering typed text.
    last_file_sel: Option<usize>,
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
        let files = Rc::new(RefCell::new(List::new(geo.file_list)));
        let dirs = Rc::new(RefCell::new(List::new(geo.dir_list)));

        let type_dd = Rc::new(RefCell::new(
            Dropdown::new(geo.type_dd).with_items(filters.iter().map(FileFilter::label)),
        ));
        type_dd.borrow_mut().set_on_change({
            let signals = signals.clone();
            move |_cx, idx| signals.filter.set(Some(idx))
        });

        let drives = list_drives(&dir);
        let drive_dd = Rc::new(RefCell::new(
            Dropdown::new(geo.drive_dd).with_items(drives.iter().map(|d| drive_label(d))),
        ));
        drive_dd.borrow_mut().set_on_change({
            let signals = signals.clone();
            move |_cx, idx| signals.drive.set(Some(idx))
        });

        // OK is the default action, so Enter from any field accepts. Cancel
        // just asks the modal to close.
        let ok = Button::new(geo.ok, "OK").default(true).on_click({
            let signals = signals.clone();
            move |_cx| signals.accept.set(true)
        });
        let cancel = Button::new(geo.cancel, "Cancel").on_click(|cx| cx.request_dismiss());

        // Add order == Tab order: name field, file list, type filter, dir list,
        // drives, OK, Cancel. Must match the IDX_* constants above.
        let container = Container::new(DIALOG_W, DIALOG_H)
            .add(Shared(name.clone()))
            .add(Shared(files.clone()))
            .add(Shared(type_dd.clone()))
            .add(Shared(dirs.clone()))
            .add(Shared(drive_dd.clone()))
            .add(ok)
            .add(cancel);

        let active_patterns = filters[0].patterns().to_vec();
        let mut body = Self {
            rect: Rect::new(0, 0, 0, 0),
            container,
            name,
            files,
            dirs,
            drive_dd,
            signals,
            on_accept,
            dir,
            filters,
            active_patterns,
            drives,
            icons: Icons::new(),
            last_file_sel: None,
        };

        // Seed the file list / dir list and the File Name field.
        body.reload_all();
        let initial = suggested_name.unwrap_or_else(|| body.pattern_hint());
        body.name.borrow_mut().set_text(&initial);
        body.sync_drive_selection();
        body
    }

    /// The pattern shown in the File Name field when no real name is entered —
    /// the active filter's first pattern (e.g. `*.txt`), or `*` as a fallback.
    fn pattern_hint(&self) -> String {
        self.active_patterns
            .first()
            .cloned()
            .unwrap_or_else(|| "*".to_string())
    }

    /// Re-read the current directory and repopulate both lists.
    fn reload_all(&mut self) {
        self.reload_dirs();
        self.reload_files();
    }

    fn reload_dirs(&mut self) {
        let mut items = Vec::new();
        if self.dir.parent().is_some() {
            items.push(ListItem::new("..").with_icon(self.icons.up.clone()));
        }
        for name in read_dir_names(&self.dir, true) {
            items.push(ListItem::new(name).with_icon(self.icons.folder.clone()));
        }
        let mut dirs = self.dirs.borrow_mut();
        dirs.set_items(items);
        if !dirs.items().is_empty() {
            dirs.set_selected(Some(0));
        }
    }

    fn reload_files(&mut self) {
        let items: Vec<ListItem> = read_dir_names(&self.dir, false)
            .into_iter()
            .filter(|n| self.matches_active(n))
            .map(|n| ListItem::new(n).with_icon(self.icons.file.clone()))
            .collect();
        let mut files = self.files.borrow_mut();
        files.set_items(items);
        // Leave the file list unselected so the File Name field keeps showing
        // the pattern hint until the user actually picks something.
        files.set_selected(None);
        self.last_file_sel = None;
    }

    fn matches_active(&self, name: &str) -> bool {
        self.active_patterns.iter().any(|p| glob_match(p, name))
    }

    /// Point the "Drives" dropdown at whichever root the current directory
    /// lives under, without firing its `on_change`.
    fn sync_drive_selection(&mut self) {
        let idx = self
            .drives
            .iter()
            .enumerate()
            .filter(|(_, root)| self.dir.starts_with(root))
            // Prefer the longest matching root (e.g. `/Volumes/Disk` over `/`).
            .max_by_key(|(_, root)| root.as_os_str().len())
            .map(|(i, _)| i);
        if let Some(i) = idx {
            self.drive_dd.borrow_mut().set_selected(Some(i));
        }
    }

    /// Switch to a new directory and refresh everything that depends on it.
    fn set_dir(&mut self, dir: PathBuf) {
        self.dir = dir;
        self.reload_all();
        self.sync_drive_selection();
        // Re-seed the field with the pattern unless the user has a real name in
        // it (Save mode), matching the original's behavior.
        let cur = self.name.borrow().text();
        if cur.is_empty() || is_pattern(&cur) {
            self.name.borrow_mut().set_text(&self.pattern_hint());
        }
    }

    /// Resolve the directory-list row `idx` to a path and navigate to it.
    fn navigate_dir(&mut self, idx: usize, ctx: &mut EventCtx) {
        let label = self.dirs.borrow().items().get(idx).map(|i| i.label.clone());
        let Some(label) = label else { return };
        let target = if label == ".." {
            self.dir.parent().map(Path::to_path_buf)
        } else {
            Some(self.dir.join(&label))
        };
        if let Some(target) = target.filter(|t| t.is_dir()) {
            self.set_dir(target);
            ctx.request_paint();
        }
    }

    /// Open the file-list row `idx` (a double-click / Enter on a file).
    fn open_file(&mut self, idx: usize, ctx: &mut EventCtx) {
        let label = self
            .files
            .borrow()
            .items()
            .get(idx)
            .map(|i| i.label.clone());
        if let Some(label) = label {
            self.name.borrow_mut().set_text(&label);
            self.accept(ctx);
        }
    }

    /// Act on the File Name field: descend into a directory, re-filter on a
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
            self.reload_files();
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
            self.active_patterns = filter.patterns().to_vec();
            self.reload_files();
            let cur = self.name.borrow().text();
            if cur.is_empty() || is_pattern(&cur) {
                self.name.borrow_mut().set_text(&self.pattern_hint());
            }
            ctx.request_paint();
        }

        if let Some(idx) = self.signals.drive.take()
            && let Some(root) = self.drives.get(idx).cloned()
        {
            self.set_dir(root);
            ctx.request_paint();
        }

        // Bind the activations to locals first so the `RefMut` is dropped
        // before the `&mut self` navigation / open calls borrow the body.
        let dir_activated = self.dirs.borrow_mut().take_activated();
        if let Some(idx) = dir_activated {
            self.navigate_dir(idx, ctx);
        }

        let file_activated = self.files.borrow_mut().take_activated();
        if let Some(idx) = file_activated {
            self.open_file(idx, ctx);
            if ctx.is_dismiss_requested() {
                return;
            }
        }

        // Reflect a fresh single-click file selection into the name field.
        let sel = self.files.borrow().selected_index();
        if sel != self.last_file_sel {
            self.last_file_sel = sel;
            let label = sel.and_then(|idx| {
                self.files
                    .borrow()
                    .items()
                    .get(idx)
                    .map(|i| i.label.clone())
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
        // The Win 3.1 dialog interior is the gray button face, not the white
        // the modal fills with — repaint it so the sunken white fields read
        // against it.
        painter.fill_rect(self.rect, theme.face);

        // Section labels.
        label(painter, geo.dir_label, "&Directories:", theme);
        label(painter, geo.file_label, "File &Name:", theme);
        label(painter, geo.drive_label, "Dri&ves:", theme);
        label(painter, geo.type_label, "List Files of &Type:", theme);

        // Current path under the Directories label, clipped to its column.
        let saved = painter.push_clip(geo.path_text);
        painter.text(
            geo.path_text.x,
            geo.path_text.y,
            &self.dir.display().to_string(),
            theme.font_size,
            theme.text,
        );
        painter.restore_clip(saved);

        self.container.paint(painter, theme);
    }

    fn paint_overlay(&mut self, painter: &mut Painter, theme: &Theme) {
        self.container.paint_overlay(painter, theme);
    }

    fn event(&mut self, event: &Event, ctx: &mut EventCtx) {
        // With the directory list focused, Enter navigates into the selected
        // folder. The default OK button would otherwise swallow Enter via the
        // accelerator pass, so intercept it before forwarding.
        if let Event::KeyDown {
            key: Key::Named(NamedKey::Enter),
            ..
        } = event
            && self.container.focused_index() == Some(IDX_DIRS)
        {
            let sel = self.dirs.borrow().selected_index();
            if let Some(idx) = sel {
                self.navigate_dir(idx, ctx);
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

/// Draw a section label at the top-left of `rect`.
fn label(painter: &mut Painter, rect: Rect, text: &str, theme: &Theme) {
    // Mnemonics aren't wired through to focus here, so strip the '&' marker and
    // just draw the plain text.
    let plain: String = text.replace('&', "");
    painter.text(rect.x, rect.y, &plain, theme.font_size, theme.text);
}

// ----------------------------------------------------------------------------
// Geometry — every rect inside the dialog, derived from the client rect so the
// modal's centered origin (and any future resize) just works. Child widgets are
// built against `Rect::new(0, 0, W, H)`; labels are painted against `self.rect`.
// The container shifts the children to match.
// ----------------------------------------------------------------------------

struct Geometry {
    dir_label: Rect,
    path_text: Rect,
    dir_list: Rect,
    drive_label: Rect,
    drive_dd: Rect,
    file_label: Rect,
    name_field: Rect,
    file_list: Rect,
    type_label: Rect,
    type_dd: Rect,
    ok: Rect,
    cancel: Rect,
}

impl Geometry {
    fn compute(base: Rect) -> Self {
        let top = base.y + PAD;

        // Button stack on the far right.
        let btn_x = base.right() - PAD - BTN_W;
        let ok = Rect::new(btn_x, top, BTN_W, BTN_H);
        let cancel = Rect::new(btn_x, ok.bottom() + BTN_GAP, BTN_W, BTN_H);

        // Two equal list columns fill the space left of the button stack.
        let left = base.x + PAD;
        let cols_right = btn_x - COL_GAP;
        let col_w = ((cols_right - left) - COL_GAP) / 2;
        let dir_x = left;
        let file_x = left + col_w + COL_GAP;

        // Bottom row: a labeled dropdown under each column.
        let dd_y = base.bottom() - PAD - FIELD_H;
        let dd_label_y = dd_y - LABEL_H - 2;

        // Lists run from below the top label/field down to the bottom dropdowns.
        let list_top = top + LABEL_H + 2 + FIELD_H + LIST_GAP;
        let list_h = (dd_label_y - LIST_GAP - list_top).max(0);

        Self {
            dir_label: Rect::new(dir_x, top, col_w, LABEL_H),
            path_text: Rect::new(dir_x, top + LABEL_H + 4, col_w, LABEL_H),
            dir_list: Rect::new(dir_x, list_top, col_w, list_h),
            drive_label: Rect::new(dir_x, dd_label_y, col_w, LABEL_H),
            drive_dd: Rect::new(dir_x, dd_y, col_w, FIELD_H),
            file_label: Rect::new(file_x, top, col_w, LABEL_H),
            name_field: Rect::new(file_x, top + LABEL_H + 2, col_w, FIELD_H),
            file_list: Rect::new(file_x, list_top, col_w, list_h),
            type_label: Rect::new(file_x, dd_label_y, col_w, LABEL_H),
            type_dd: Rect::new(file_x, dd_y, col_w, FIELD_H),
            ok,
            cancel,
        }
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

/// Display string for a drive/root in the "Drives" dropdown.
fn drive_label(root: &Path) -> String {
    let s = root.display().to_string();
    if s.is_empty() { "/".to_string() } else { s }
}

/// The filesystem roots offered in the "Drives" dropdown, always including the
/// root that contains `current`.
#[cfg(windows)]
fn list_drives(current: &Path) -> Vec<PathBuf> {
    let mut roots = Vec::new();
    for letter in b'A'..=b'Z' {
        let root = PathBuf::from(format!("{}:\\", letter as char));
        if root.exists() {
            roots.push(root);
        }
    }
    ensure_contains_root(&mut roots, current);
    roots
}

#[cfg(not(windows))]
fn list_drives(current: &Path) -> Vec<PathBuf> {
    // Unix has a single tree rooted at `/`; surface any mounted volumes
    // (macOS `/Volumes`) as extra "drives" so the picker can hop between them.
    let mut roots = vec![PathBuf::from("/")];
    if let Ok(read) = std::fs::read_dir("/Volumes") {
        for entry in read.flatten() {
            if entry.path().is_dir() {
                roots.push(entry.path());
            }
        }
    }
    roots.sort();
    roots.dedup();
    ensure_contains_root(&mut roots, current);
    roots
}

/// Make sure some root in `roots` is an ancestor of `current`; if none is, add
/// `current`'s own root component so the drives list always reflects where we
/// are.
fn ensure_contains_root(roots: &mut Vec<PathBuf>, current: &Path) {
    if roots.iter().any(|r| current.starts_with(r)) {
        return;
    }
    let root = current
        .ancestors()
        .last()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| current.to_path_buf());
    if !roots.contains(&root) {
        roots.push(root);
    }
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

// ----------------------------------------------------------------------------
// Icons — 16x16 procedural glyphs for the list rows. Mirrors the `filer`
// example's icons so the look matches the file manager.
// ----------------------------------------------------------------------------

struct Icons {
    folder: ListIcon,
    file: ListIcon,
    up: ListIcon,
}

impl Icons {
    fn new() -> Self {
        Self {
            folder: folder_icon(),
            file: file_icon(),
            up: up_icon(),
        }
    }
}

fn folder_icon() -> ListIcon {
    let mut icon = ListIcon::new(16, 16);
    let line = Color::BLACK;
    let body = Color::YELLOW;
    icon.fill_rect(Rect::new(1, 3, 6, 1), line);
    icon.fill_rect(Rect::new(1, 4, 1, 2), line);
    icon.fill_rect(Rect::new(6, 4, 1, 1), line);
    icon.fill_rect(Rect::new(7, 5, 7, 1), line);
    icon.fill_rect(Rect::new(1, 6, 13, 1), line);
    icon.fill_rect(Rect::new(1, 6, 1, 8), line);
    icon.fill_rect(Rect::new(13, 6, 1, 8), line);
    icon.fill_rect(Rect::new(1, 13, 13, 1), line);
    icon.fill_rect(Rect::new(2, 4, 4, 2), body);
    icon.fill_rect(Rect::new(2, 7, 11, 6), body);
    icon
}

fn file_icon() -> ListIcon {
    let mut icon = ListIcon::new(16, 16);
    let line = Color::BLACK;
    let body = Color::WHITE;
    icon.fill_rect(Rect::new(3, 1, 7, 1), line);
    icon.fill_rect(Rect::new(3, 1, 1, 13), line);
    icon.fill_rect(Rect::new(3, 13, 9, 1), line);
    icon.fill_rect(Rect::new(11, 5, 1, 9), line);
    icon.set_pixel(10, 1, line);
    icon.set_pixel(10, 2, line);
    icon.set_pixel(11, 2, line);
    icon.set_pixel(11, 3, line);
    icon.set_pixel(12, 3, line);
    icon.set_pixel(12, 4, line);
    icon.set_pixel(11, 4, line);
    icon.fill_rect(Rect::new(9, 4, 3, 1), line);
    icon.fill_rect(Rect::new(4, 2, 6, 3), body);
    icon.fill_rect(Rect::new(4, 5, 7, 8), body);
    icon.fill_rect(Rect::new(5, 7, 5, 1), line);
    icon.fill_rect(Rect::new(5, 9, 5, 1), line);
    icon.fill_rect(Rect::new(5, 11, 4, 1), line);
    icon
}

fn up_icon() -> ListIcon {
    let mut icon = ListIcon::new(16, 16);
    let line = Color::BLACK;
    for y in 0..5 {
        let half = y + 1;
        let cx = 7;
        let xs = cx - half + 1;
        let xe = cx + half;
        icon.fill_rect(Rect::new(xs, 3 + y, xe - xs + 1, 1), line);
    }
    icon.fill_rect(Rect::new(6, 8, 4, 5), line);
    icon
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::event::{Event, Key, MouseButton, NamedKey};
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

        // The *.txt filter leaves a single file — click it to load the name
        // field, then Enter fires the default OK button.
        let g = geo();
        click(
            &backend,
            &mut dlg,
            g.file_list.x + 24,
            row_y(g.file_list, 0),
        );
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

        // Directory list: row 0 is "..", row 1 is "sub". Two quick presses on
        // "sub" register as a double-click and descend into it.
        let g = geo();
        let (dx, dy) = (g.dir_list.x + 24, row_y(g.dir_list, 1));
        let down = Event::PointerDown {
            pos: Point::new(dx, dy),
            button: MouseButton::Left,
        };
        backend.dispatch(&mut dlg, &down);
        backend.dispatch(&mut dlg, &down);

        // Now inside `sub`: the only *.txt is deep.txt — select it and accept.
        let g = geo();
        click(
            &backend,
            &mut dlg,
            g.file_list.x + 24,
            row_y(g.file_list, 0),
        );
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
    fn typing_a_wildcard_refilters_without_accepting() {
        let dir = unique_temp();
        std::fs::write(dir.join("hello.txt"), b"hi").unwrap();

        let chosen: Rc<RefCell<Option<PathBuf>>> = Rc::new(RefCell::new(None));
        let mut dlg = FileDialog::new().with_directory(&dir);
        {
            let chosen = chosen.clone();
            dlg.show_open(move |_cx, path| *chosen.borrow_mut() = Some(path.to_path_buf()));
        }
        let backend = MockBackend::new(DIALOG_W, DIALOG_H);
        backend.render(&mut dlg);

        // The File Name field opens on the "*" pattern; Enter on a wildcard
        // re-filters rather than accepting, so the dialog stays open.
        backend.dispatch(&mut dlg, &key(NamedKey::Enter, true));
        backend.dispatch(&mut dlg, &key(NamedKey::Enter, false));

        assert!(
            chosen.borrow().is_none(),
            "a wildcard pattern is not accepted"
        );
        assert!(dlg.is_open(), "the dialog stays open after re-filtering");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
