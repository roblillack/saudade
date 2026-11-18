use std::collections::HashMap;
use std::time::{Duration, Instant};

use crate::accel::ModifierScheme;
use crate::event::{Cursor, Event, EventCtx, Key, MouseButton, NamedKey};
use crate::font::{FontFamily, FontStyle};
use crate::geometry::{Color, Point, Rect};
use crate::painter::{Merged, Painter};
use crate::theme::Theme;
use crate::widget::Widget;
use crate::widgets::scrollbar::{SCROLLBAR_THICKNESS, ScrollBar};

const PADDING_X: i32 = 4;
const PADDING_Y: i32 = 2;
const LINE_HEIGHT: i32 = 14;
const MULTI_CLICK_MS: u64 = 400;
const MULTI_CLICK_SLOP: i32 = 3;
/// How long each half of the caret blink lasts while the widget is focused.
const BLINK_HALF_MS: u64 = 500;
/// How far a read-only editor's paper goes from the theme's background
/// towards its face: enough to tell the box from an editable one beside it
/// without greying it like a disabled one.
const READ_ONLY_TINT: f32 = 0.2;
/// How many steps back [`TextEditor::undo`] can go. Past this the oldest
/// step is forgotten, so a long session can't grow the history without end.
const UNDO_LIMIT: usize = 1000;

/// A caret position: `(row, column)`, the column counted in characters.
type Pos = (usize, usize);

/// Where the caret and the selection anchor stood at some moment.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Caret {
    cursor: Pos,
    anchor: Option<Pos>,
}

/// What kind of edit a history step came from. Steps of the same kind that
/// follow one another without the caret moving in between grow into a single
/// step, so one undo takes back a run of typing rather than a letter.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum EditKind {
    /// A typed character.
    Typing,
    /// Backspace over one character.
    Backspace,
    /// Delete of one character.
    Delete,
    /// Anything else — Enter, cut, paste, a replaced selection. Always a
    /// step of its own.
    Other,
}

/// One step of the undo history: at `at`, `removed` gave way to `inserted`.
#[derive(Debug)]
struct Edit {
    at: Pos,
    removed: String,
    inserted: String,
    before: Caret,
    after: Caret,
    kind: EditKind,
}

/// A minimal multi-line text editor — sunken white field, monospace text,
/// cursor, selection with cut/copy/paste, undo/redo, and a built-in vertical
/// scrollbar.
///
/// Lines are never wrapped. One wider than the field is clipped at its edge,
/// and the view scrolls sideways to keep the caret in sight;
/// [`with_horizontal_scrollbar`](Self::with_horizontal_scrollbar) adds a
/// scrollbar along the bottom to do the same by hand.
///
/// Only the visible rows are drawn on each paint, and a line is measured once
/// until it changes — text that's scrolled out of view contributes no work to
/// the render loop. The scrollbars own the canonical scroll position;
/// `TextEditor` reads it via `scrollbar.value()`.
pub struct TextEditor {
    pub rect: Rect,
    pub font_size: f32,
    lines: Vec<String>,
    cursor: Pos,
    selection_anchor: Option<Pos>,
    /// The user's edits, oldest first; [`undo`](Self::undo) takes the last.
    undo_stack: Vec<Edit>,
    /// The steps undone since the last edit, for [`redo`](Self::redo).
    redo_stack: Vec<Edit>,
    /// Whether the next edit may grow the last history step instead of adding
    /// one. Cleared by whatever ends a run: moving the caret, undo and redo.
    coalesce: bool,
    focused: bool,
    enabled: bool,
    read_only: bool,
    scheme: ModifierScheme,
    /// Per-visible-row cumulative pixel widths, keyed by absolute row index.
    /// `widths[col]` is the x-offset (in logical px) where the caret sits
    /// at character index `col`. Rebuilt every paint; only visible rows are
    /// populated so big files stay cheap.
    cumulative_widths: HashMap<usize, Vec<i32>>,
    drag_active: bool,
    last_click: Option<(Instant, Point)>,
    click_count: u32,
    clipboard: Option<arboard::Clipboard>,
    v_scrollbar: ScrollBar,
    /// How far the text is scrolled sideways, in pixels. Kept by a scrollbar
    /// whether or not one is shown, so both cases share the clamping and the
    /// wheel handling.
    h_scrollbar: ScrollBar,
    show_h_scrollbar: bool,
    /// Pixel width of each line, `None` until measured or once the line has
    /// changed. What the horizontal scroll range is made of.
    line_widths: Vec<Option<i32>>,
    /// The widest of `line_widths`, `None` when any of them changed.
    widest_line: Option<i32>,
    /// The font size `line_widths` were measured at.
    measured_font_size: f32,
    /// Set when the caret may have left the view sideways. Where it stands in
    /// pixels takes the font to know, so the next paint does the scrolling.
    reveal_caret: bool,
    /// When the current half of the blink cycle started. Reset on every
    /// user action so the caret stays visible (and "on") while the user is
    /// actively typing.
    blink_since: Instant,
    /// Cached on/off state of the focused caret. Updated by `Event::Tick`.
    blink_on: bool,
}

impl TextEditor {
    pub fn new(rect: Rect) -> Self {
        Self {
            rect,
            font_size: 11.0,
            lines: vec![String::new()],
            cursor: (0, 0),
            selection_anchor: None,
            undo_stack: Vec::new(),
            redo_stack: Vec::new(),
            coalesce: false,
            focused: false,
            enabled: true,
            read_only: false,
            scheme: ModifierScheme::native(),
            cumulative_widths: HashMap::new(),
            drag_active: false,
            last_click: None,
            click_count: 0,
            clipboard: None,
            v_scrollbar: ScrollBar::vertical(Rect::new(0, 0, 0, 0)),
            h_scrollbar: ScrollBar::horizontal(Rect::new(0, 0, 0, 0)),
            show_h_scrollbar: false,
            line_widths: vec![None],
            widest_line: None,
            measured_font_size: 0.0,
            reveal_caret: false,
            blink_since: Instant::now(),
            blink_on: true,
        }
    }

    pub fn with_text(mut self, text: &str) -> Self {
        self.set_text(text);
        self
    }

    pub fn with_font_size(mut self, size: f32) -> Self {
        self.font_size = size;
        self
    }

    /// Pin the [`ModifierScheme`] the editing shortcuts (Copy, Cut, Paste,
    /// Select All, word moves) are matched with, overriding the
    /// [`ModifierScheme::native`] default. Mainly for tests that must behave
    /// identically on every host.
    pub fn with_scheme(mut self, scheme: ModifierScheme) -> Self {
        self.scheme = scheme;
        self
    }

    pub fn with_enabled(mut self, enabled: bool) -> Self {
        self.set_enabled(enabled);
        self
    }

    pub fn is_enabled(&self) -> bool {
        self.enabled
    }

    /// Enable or disable the editor. A disabled editor paints on the grey
    /// button face with greyed text, hides its caret, can't take focus, and
    /// ignores all input.
    pub fn set_enabled(&mut self, enabled: bool) {
        self.enabled = enabled;
        if !enabled {
            self.drag_active = false;
            self.last_click = None;
            self.click_count = 0;
        }
    }

    /// Show a horizontal scrollbar along the bottom edge, for text whose lines
    /// may be wider than the field. Off by default: the view then still
    /// follows the caret sideways, with nothing but the caret to move it.
    pub fn with_horizontal_scrollbar(mut self, show: bool) -> Self {
        self.set_horizontal_scrollbar(show);
        self
    }

    pub fn set_horizontal_scrollbar(&mut self, show: bool) {
        self.show_h_scrollbar = show;
        self.layout(self.rect);
    }

    pub fn with_read_only(mut self, read_only: bool) -> Self {
        self.set_read_only(read_only);
        self
    }

    pub fn is_read_only(&self) -> bool {
        self.read_only
    }

    /// Make the editor read-only, or editable again. A read-only editor shows
    /// text the user may read, select and copy but not change: it still takes
    /// the focus, moves its caret and scrolls, while typing, Enter, Backspace,
    /// Delete, [`cut`](Self::cut), [`paste`](Self::paste), [`undo`](Self::undo)
    /// and [`redo`](Self::redo) do nothing.
    /// [`set_text`](Self::set_text) is the program's own doing and keeps
    /// working. Its paper is a touch towards the theme's `face`, enough to
    /// tell it from an editable box beside it; the text stays as legible as
    /// ever, unlike in a [disabled](Self::set_enabled) editor.
    pub fn set_read_only(&mut self, read_only: bool) {
        self.read_only = read_only;
    }

    pub fn text(&self) -> String {
        self.lines.join("\n")
    }

    /// Replace the whole text, putting the caret at the start. The undo
    /// history goes with the old text: this is a new document, not an edit.
    pub fn set_text(&mut self, text: &str) {
        self.undo_stack.clear();
        self.redo_stack.clear();
        self.lines = if text.is_empty() {
            vec![String::new()]
        } else {
            text.split('\n').map(String::from).collect()
        };
        self.line_widths = vec![None; self.lines.len()];
        self.widest_line = None;
        self.cursor = (0, 0);
        self.selection_anchor = None;
        self.v_scrollbar.set_value(0);
        self.h_scrollbar.set_value(0);
    }

    pub fn is_focused(&self) -> bool {
        self.focused
    }

    fn scroll_top(&self) -> usize {
        self.v_scrollbar.value().max(0) as usize
    }

    fn set_scroll_top(&mut self, top: usize) {
        self.v_scrollbar.set_value(top as i32);
    }

    /// The rectangle used to render text — everything except the column the
    /// vertical scrollbar occupies on the right edge and the row the
    /// horizontal one, if shown, occupies at the bottom.
    fn text_area(&self) -> Rect {
        // Where a scrollbar is present the field overlaps it by 1px so the
        // field's border lands on the scrollbar's own border line, collapsing
        // the divider to a single 1px line instead of stacking the two 1px
        // borders into a 2px band. The scrollbars are painted last, on top, so
        // that shared line reads as the scrollbar's edge.
        let across = |bar: i32| if bar > 0 { bar - 1 } else { 0 };
        Rect::new(
            self.rect.x,
            self.rect.y,
            (self.rect.w - across(self.v_scrollbar.rect().w)).max(0),
            (self.rect.h - across(self.h_scrollbar.rect().h)).max(0),
        )
    }

    /// The square in the bottom right corner that neither scrollbar reaches
    /// when both are shown.
    fn corner(&self) -> Option<Rect> {
        let (v, h) = (self.v_scrollbar.rect(), self.h_scrollbar.rect());
        (v.w > 0 && h.h > 0).then(|| Rect::new(v.x, h.y, v.w, h.h))
    }

    /// Which edges of the field lie on a scrollbar's border line.
    fn merged_edges(&self) -> Merged {
        Merged {
            right: self.v_scrollbar.rect().w > 0,
            bottom: self.h_scrollbar.rect().h > 0,
            ..Merged::NONE
        }
    }

    /// Width of the room the text has between the field's paddings.
    fn view_width(&self) -> i32 {
        (self.text_area().w - PADDING_X * 2).max(1)
    }

    /// Measure the lines that changed since the last paint and size the
    /// horizontal scroll range to the widest line of all.
    fn sync_h_scrollbar(&mut self, painter: &Painter) {
        if self.measured_font_size != self.font_size || self.line_widths.len() != self.lines.len() {
            self.line_widths = vec![None; self.lines.len()];
            self.widest_line = None;
            self.measured_font_size = self.font_size;
        }
        let size = self.font_size;
        let line_width = |line: &str| {
            painter
                .cumulative_widths(line, size, FontFamily::Mono, FontStyle::Regular)
                .last()
                .copied()
                .unwrap_or(0)
        };
        let widest = match self.widest_line {
            Some(widest) => widest,
            None => {
                let mut widest = 0;
                for (line, width) in self.lines.iter().zip(&mut self.line_widths) {
                    widest = widest.max(*width.get_or_insert_with(|| line_width(line)));
                }
                self.widest_line = Some(widest);
                widest
            }
        };
        let view = self.view_width();
        // One pixel past the text, for the caret at the end of the widest line.
        self.h_scrollbar.set_range(view, widest + 1 - view);
        // An arrow click or a wheel line moves by a column.
        self.h_scrollbar.set_line_step(line_width("0"));
    }

    /// Scroll sideways just far enough that the caret is inside the view,
    /// given its x-offset from the start of its line.
    fn reveal_caret_x(&mut self, caret: i32) {
        let view = self.view_width();
        let scrolled = self.h_scrollbar.value();
        if caret < scrolled {
            self.h_scrollbar.set_value(caret);
        } else if caret >= scrolled + view {
            self.h_scrollbar.set_value(caret - view + 1);
        }
    }

    // ---------------------------------------------------------------- selection

    pub fn select_all(&mut self) {
        if self.lines.is_empty() {
            return;
        }
        self.selection_anchor = Some((0, 0));
        let last_row = self.lines.len() - 1;
        let last_col = self.lines[last_row].chars().count();
        self.cursor = (last_row, last_col);
    }

    fn selection_range(&self) -> Option<((usize, usize), (usize, usize))> {
        let anchor = self.selection_anchor?;
        if anchor == self.cursor {
            return None;
        }
        if anchor < self.cursor {
            Some((anchor, self.cursor))
        } else {
            Some((self.cursor, anchor))
        }
    }

    fn selected_text(&self) -> Option<String> {
        let (start, end) = self.selection_range()?;
        Some(self.text_between(start, end))
    }

    fn text_between(&self, start: Pos, end: Pos) -> String {
        let mut out = String::new();
        if start.0 == end.0 {
            let line = &self.lines[start.0];
            out.extend(line.chars().skip(start.1).take(end.1 - start.1));
        } else {
            out.extend(self.lines[start.0].chars().skip(start.1));
            out.push('\n');
            for line in &self.lines[start.0 + 1..end.0] {
                out.push_str(line);
                out.push('\n');
            }
            out.extend(self.lines[end.0].chars().take(end.1));
        }
        out
    }

    /// Delete the selected text, if any, and drop the selection anchor. An
    /// empty selection (anchor on the caret) has nothing to delete, but
    /// leaving its anchor behind would select whatever the edit inserts — or
    /// point at a row that a Backspace joining two lines removed.
    fn delete_selection(&mut self) -> bool {
        let selected = self.selection_range().is_some();
        self.replace_selection("", EditKind::Other);
        selected
    }

    fn before_move(&mut self, extend: bool) {
        // Moving the caret ends a run of typing, even if it comes back.
        self.coalesce = false;
        if extend {
            if self.selection_anchor.is_none() {
                self.selection_anchor = Some(self.cursor);
            }
        } else {
            self.selection_anchor = None;
        }
    }

    // ---------------------------------------------------------------- clipboard

    pub fn copy(&mut self) {
        if let Some(text) = self.selected_text() {
            self.clipboard_set(&text);
        }
    }

    pub fn cut(&mut self) {
        if self.read_only {
            return;
        }
        if let Some(text) = self.selected_text() {
            self.clipboard_set(&text);
            self.delete_selection();
        }
    }

    pub fn paste(&mut self) {
        if self.read_only {
            return;
        }
        let Some(text) = self.clipboard_get() else {
            return;
        };
        self.replace_selection(&text.replace('\r', ""), EditKind::Other);
    }

    fn clipboard(&mut self) -> Option<&mut arboard::Clipboard> {
        if self.clipboard.is_none() {
            self.clipboard = arboard::Clipboard::new().ok();
        }
        self.clipboard.as_mut()
    }

    fn clipboard_set(&mut self, text: &str) {
        if let Some(cb) = self.clipboard() {
            let _ = cb.set_text(text.to_owned());
        }
    }

    fn clipboard_get(&mut self) -> Option<String> {
        self.clipboard().and_then(|cb| cb.get_text().ok())
    }

    // ---------------------------------------------------------------- scrolling

    fn visible_rows(&self) -> i32 {
        ((self.text_area().h - PADDING_Y * 2) / LINE_HEIGHT).max(1)
    }

    fn sync_scrollbar(&mut self) {
        let visible = self.visible_rows();
        let max_scroll = (self.lines.len() as i32 - visible).max(0);
        self.v_scrollbar.set_range(visible, max_scroll);
    }

    fn ensure_cursor_visible(&mut self) {
        self.reveal_caret = true;
        self.sync_scrollbar();
        let visible = self.visible_rows() as usize;
        let mut top = self.scroll_top();
        if self.cursor.0 < top {
            top = self.cursor.0;
        } else if self.cursor.0 >= top + visible {
            top = self.cursor.0 + 1 - visible;
        }
        self.set_scroll_top(top);
    }

    fn clamp_col(&mut self) {
        let line_len = self.lines[self.cursor.0].chars().count();
        if self.cursor.1 > line_len {
            self.cursor.1 = line_len;
        }
    }

    // ---------------------------------------------------------------- history

    fn caret(&self) -> Caret {
        Caret {
            cursor: self.cursor,
            anchor: self.selection_anchor,
        }
    }

    fn set_caret(&mut self, caret: Caret) {
        self.cursor = caret.cursor;
        self.selection_anchor = caret.anchor;
    }

    /// Put `text` in place of what lies between `start` and `end`, and return
    /// what was there. Every change to the text after [`set_text`] comes
    /// through here.
    ///
    /// [`set_text`]: Self::set_text
    fn splice(&mut self, start: Pos, end: Pos, text: &str) -> String {
        let removed = self.text_between(start, end);
        let head = char_to_byte(&self.lines[start.0], start.1);
        let tail = char_to_byte(&self.lines[end.0], end.1);
        let mut lines: Vec<String> = text.split('\n').map(String::from).collect();
        lines[0].insert_str(0, &self.lines[start.0][..head]);
        let last = lines.len() - 1;
        lines[last].push_str(&self.lines[end.0][tail..]);
        // The lines put in have yet to be measured. The cache may be behind
        // the text already if nothing was painted since `font_size` changed;
        // the next paint starts over then.
        if self.line_widths.len() == self.lines.len() {
            self.line_widths
                .splice(start.0..=end.0, std::iter::repeat_n(None, lines.len()));
        }
        self.widest_line = None;
        self.lines.splice(start.0..=end.0, lines);
        removed
    }

    /// One edit by the user: replace `start..end` with `text`, leave the caret
    /// behind the new text, and remember the step for [`undo`](Self::undo).
    fn edit(&mut self, start: Pos, end: Pos, text: &str, kind: EditKind) {
        let before = self.caret();
        let removed = self.splice(start, end, text);
        self.cursor = end_of(start, text);
        self.selection_anchor = None;
        if removed.is_empty() && text.is_empty() {
            return;
        }
        self.record(Edit {
            at: start,
            removed,
            inserted: text.to_string(),
            before,
            after: self.caret(),
            kind,
        });
    }

    /// Replace the selection with `text`, or insert it at the caret when
    /// nothing is selected.
    fn replace_selection(&mut self, text: &str, kind: EditKind) {
        let (start, end) = self.selection_range().unwrap_or((self.cursor, self.cursor));
        self.edit(start, end, text, kind);
    }

    fn record(&mut self, edit: Edit) {
        self.redo_stack.clear();
        // A run of typing, of Backspaces or of Deletes is one step — as long as
        // nothing ended it and the caret is still where the last of them left
        // it (`select_all` and friends move it without saying so).
        if self.coalesce
            && let Some(last) = self.undo_stack.last_mut()
            && last.kind == edit.kind
            && last.after == edit.before
        {
            match edit.kind {
                EditKind::Typing if edit.removed.is_empty() => {
                    last.inserted.push_str(&edit.inserted);
                    last.after = edit.after;
                    return;
                }
                EditKind::Backspace => {
                    last.removed.insert_str(0, &edit.removed);
                    last.at = edit.at;
                    last.after = edit.after;
                    return;
                }
                EditKind::Delete => {
                    last.removed.push_str(&edit.removed);
                    return;
                }
                _ => {}
            }
        }
        self.undo_stack.push(edit);
        if self.undo_stack.len() > UNDO_LIMIT {
            self.undo_stack.remove(0);
        }
        self.coalesce = true;
    }

    /// `true` if [`undo`](Self::undo) has an edit to take back — what an Edit
    /// menu greys its Undo item by.
    pub fn can_undo(&self) -> bool {
        !self.read_only && !self.undo_stack.is_empty()
    }

    /// `true` if [`redo`](Self::redo) has an undone edit to restore.
    pub fn can_redo(&self) -> bool {
        !self.read_only && !self.redo_stack.is_empty()
    }

    /// Take back the user's last edit, putting the caret and the selection
    /// back where they were before it. A run of typing counts as one edit, as
    /// does a run of Backspaces or Deletes.
    pub fn undo(&mut self) {
        if self.read_only {
            return;
        }
        let Some(edit) = self.undo_stack.pop() else {
            return;
        };
        self.splice(edit.at, end_of(edit.at, &edit.inserted), &edit.removed);
        self.set_caret(edit.before);
        self.redo_stack.push(edit);
        self.coalesce = false;
    }

    /// Restore the edit the last [`undo`](Self::undo) took back. Editing the
    /// text after an undo drops what could have been redone.
    pub fn redo(&mut self) {
        if self.read_only {
            return;
        }
        let Some(edit) = self.redo_stack.pop() else {
            return;
        };
        self.splice(edit.at, end_of(edit.at, &edit.removed), &edit.inserted);
        self.set_caret(edit.after);
        self.undo_stack.push(edit);
        self.coalesce = false;
    }

    // ---------------------------------------------------------------- editing

    fn insert_char(&mut self, ch: char) {
        self.replace_selection(ch.encode_utf8(&mut [0; 4]), EditKind::Typing);
    }

    fn insert_newline(&mut self) {
        self.replace_selection("\n", EditKind::Other);
    }

    fn backspace(&mut self) {
        if self.delete_selection() {
            return;
        }
        let (row, col) = self.cursor;
        let start = if col > 0 {
            (row, col - 1)
        } else if row > 0 {
            (row - 1, self.lines[row - 1].chars().count())
        } else {
            return;
        };
        self.edit(start, (row, col), "", EditKind::Backspace);
    }

    fn delete_forward(&mut self) {
        if self.delete_selection() {
            return;
        }
        let (row, col) = self.cursor;
        let end = if col < self.lines[row].chars().count() {
            (row, col + 1)
        } else if row + 1 < self.lines.len() {
            (row + 1, 0)
        } else {
            return;
        };
        self.edit((row, col), end, "", EditKind::Delete);
    }

    fn move_left(&mut self) {
        if self.cursor.1 > 0 {
            self.cursor.1 -= 1;
        } else if self.cursor.0 > 0 {
            self.cursor.0 -= 1;
            self.cursor.1 = self.lines[self.cursor.0].chars().count();
        }
    }

    fn move_right(&mut self) {
        let line_len = self.lines[self.cursor.0].chars().count();
        if self.cursor.1 < line_len {
            self.cursor.1 += 1;
        } else if self.cursor.0 + 1 < self.lines.len() {
            self.cursor.0 += 1;
            self.cursor.1 = 0;
        }
    }

    fn move_up(&mut self) {
        if self.cursor.0 > 0 {
            self.cursor.0 -= 1;
            self.clamp_col();
        }
    }

    fn move_down(&mut self) {
        if self.cursor.0 + 1 < self.lines.len() {
            self.cursor.0 += 1;
            self.clamp_col();
        }
    }

    fn is_word_char(ch: char) -> bool {
        ch.is_alphanumeric() || ch == '_'
    }

    /// Skip whitespace/punct, then the current word — Ctrl+Left semantics.
    /// At the start of a line this steps to the end of the previous line,
    /// mirroring a plain left-arrow's line wrap.
    fn move_word_left(&mut self) {
        if self.cursor.1 == 0 {
            self.move_left();
            return;
        }
        let chars: Vec<char> = self.lines[self.cursor.0].chars().collect();
        let mut i = self.cursor.1;
        while i > 0 && !Self::is_word_char(chars[i - 1]) {
            i -= 1;
        }
        while i > 0 && Self::is_word_char(chars[i - 1]) {
            i -= 1;
        }
        self.cursor.1 = i;
    }

    fn move_word_right(&mut self) {
        let chars: Vec<char> = self.lines[self.cursor.0].chars().collect();
        let n = chars.len();
        if self.cursor.1 >= n {
            self.move_right();
            return;
        }
        let mut i = self.cursor.1;
        while i < n && Self::is_word_char(chars[i]) {
            i += 1;
        }
        while i < n && !Self::is_word_char(chars[i]) {
            i += 1;
        }
        self.cursor.1 = i;
    }

    /// Word boundaries around a caret column on `row`, matching how a
    /// double-click expands selection — runs of word characters and runs of
    /// non-word characters are each their own "word".
    fn word_bounds_at(&self, row: usize, col: usize) -> (usize, usize) {
        let chars: Vec<char> = self.lines[row].chars().collect();
        if chars.is_empty() {
            return (0, 0);
        }
        // A caret at len has no glyph to its right; look at the one to the left.
        let target = if col >= chars.len() {
            chars.len() - 1
        } else {
            col
        };
        let is_word = Self::is_word_char(chars[target]);
        let mut start = target;
        let mut end = target + 1;
        while start > 0 && Self::is_word_char(chars[start - 1]) == is_word {
            start -= 1;
        }
        while end < chars.len() && Self::is_word_char(chars[end]) == is_word {
            end += 1;
        }
        (start, end)
    }

    /// Bump the multi-click counter on a fresh left-button press. Returns the
    /// run length: 1 for a regular click, 2 for double, 3 for triple. Any
    /// further click resets back to 1.
    fn register_click(&mut self, pos: Point) -> u32 {
        let now = Instant::now();
        let threshold = Duration::from_millis(MULTI_CLICK_MS);
        let continues = self.last_click.is_some_and(|(t, p)| {
            now.duration_since(t) <= threshold
                && (p.x - pos.x).abs() <= MULTI_CLICK_SLOP
                && (p.y - pos.y).abs() <= MULTI_CLICK_SLOP
        });
        self.click_count = if continues {
            (self.click_count + 1).min(3)
        } else {
            1
        };
        self.last_click = Some((now, pos));
        self.click_count
    }

    /// Keep the caret visible while the user is actively interacting — every
    /// edit / movement restarts the blink cycle from its "on" phase.
    fn reset_blink(&mut self) {
        self.blink_since = Instant::now();
        self.blink_on = true;
    }

    fn move_home(&mut self) {
        self.cursor.1 = 0;
    }

    fn move_end(&mut self) {
        self.cursor.1 = self.lines[self.cursor.0].chars().count();
    }

    fn move_page(&mut self, delta_pages: i32) {
        let visible = self.visible_rows() as usize;
        let step = visible.saturating_sub(1).max(1);
        let target = if delta_pages > 0 {
            (self.cursor.0 + step * delta_pages as usize).min(self.lines.len().saturating_sub(1))
        } else {
            self.cursor.0.saturating_sub(step * (-delta_pages) as usize)
        };
        self.cursor.0 = target;
        self.clamp_col();
    }

    fn place_cursor_at(&mut self, pos: Point) {
        if self.lines.is_empty() {
            return;
        }
        let text = self.text_area();
        let local_y = (pos.y - text.y - PADDING_Y).max(0);
        let row_offset = (local_y / LINE_HEIGHT) as usize;
        let row = (self.scroll_top() + row_offset).min(self.lines.len() - 1);
        self.cursor.0 = row;

        let text_x = text.x + PADDING_X - self.h_scrollbar.value();
        let target = (pos.x - text_x).max(0);
        let widths = self
            .cumulative_widths
            .get(&row)
            .cloned()
            .unwrap_or_else(|| vec![0]);
        let mut best_col = 0;
        let mut best_delta = i32::MAX;
        for (col, w) in widths.iter().enumerate() {
            let delta = (*w - target).abs();
            if delta < best_delta {
                best_delta = delta;
                best_col = col;
            }
        }
        self.cursor.1 = best_col;
    }
}

impl Widget for TextEditor {
    fn bounds(&self) -> Rect {
        self.rect
    }

    fn paint(&mut self, painter: &mut Painter, theme: &Theme) {
        self.sync_scrollbar();
        let text = self.text_area();

        // Sunken field with 1-px outer border around the whole widget (text
        // area + scrollbar live inside) — the background when live, the face
        // when disabled, and a touch of the face when read-only.
        let field_bg = if !self.enabled {
            theme.face
        } else if self.read_only {
            theme.background.mix(theme.face, READ_ONLY_TINT)
        } else {
            theme.background
        };
        painter.fill_rect(text, field_bg);
        // The bevel + border self-manage the crisp physical-pixel pass at
        // fractional scales; an edge that sits on a scrollbar's own border
        // (see `text_area`) is merged — the field's line lands on the very
        // device pixels the scrollbar's line occupies.
        let merged = self.merged_edges();
        painter.sunken_bevel_merged(text, merged, theme.highlight, theme.shadow);
        painter.stroke_rect_merged(text, merged, theme.border);

        let text_y0 = text.y + PADDING_Y;
        let visible = self.visible_rows() as usize;
        let scroll_top = self.scroll_top();

        // Rebuild cumulative caret offsets only for visible rows. Each row is a
        // single O(n) pass over the font's cached per-glyph advances — not the
        // old O(n²) "remeasure every prefix" loop, which made a 500-column line
        // cost ~125k glyph lookups *per frame* and dominated scroll/resize.
        self.cumulative_widths.clear();
        for row_offset in 0..visible {
            let row = scroll_top + row_offset;
            if row >= self.lines.len() {
                break;
            }
            let widths = painter.cumulative_widths(
                &self.lines[row],
                self.font_size,
                FontFamily::Mono,
                FontStyle::Regular,
            );
            self.cumulative_widths.insert(row, widths);
        }

        // Settle the horizontal scroll before anything is placed by it.
        self.sync_h_scrollbar(painter);
        if std::mem::take(&mut self.reveal_caret)
            && let Some(widths) = self.cumulative_widths.get(&self.cursor.0)
        {
            let caret = widths.get(self.cursor.1).or(widths.last()).copied();
            self.reveal_caret_x(caret.unwrap_or(0));
        }
        let text_x = text.x + PADDING_X - self.h_scrollbar.value();
        // Text scrolled sideways stops at the bevel, not at the padding and
        // not, as a long line would otherwise, somewhere past the field.
        let clip = painter.push_clip_frame(text, 2, merged);

        let selection = self.selection_range();
        for row_offset in 0..visible {
            let row = scroll_top + row_offset;
            if row >= self.lines.len() {
                break;
            }
            let y = text_y0 + row_offset as i32 * LINE_HEIGHT;
            if self.enabled {
                self.paint_line(painter, theme, row, text_x, y, selection);
            } else {
                // Disabled: plain greyed text, no selection band.
                painter.text_styled(
                    text_x,
                    y,
                    &self.lines[row],
                    self.font_size,
                    theme.disabled_text,
                    FontFamily::Mono,
                    FontStyle::Regular,
                );
            }
        }

        let (crow, ccol) = self.cursor;
        if self.enabled && crow >= scroll_top && crow < scroll_top + visible {
            let prefix_w = self
                .cumulative_widths
                .get(&crow)
                .and_then(|widths| widths.get(ccol))
                .copied()
                .unwrap_or(0);
            let cx = text_x + prefix_w;
            let cy = text_y0 + (crow - scroll_top) as i32 * LINE_HEIGHT;
            if self.focused {
                if self.blink_on {
                    painter.v_line(cx, cy, LINE_HEIGHT, theme.text);
                }
            } else {
                // Unfocused: a small wedge at the bottom of the line marks
                // where the caret would land if the user clicked back in.
                draw_unfocused_caret(painter, cx, cy + LINE_HEIGHT - 2, theme.text);
            }
        }
        painter.restore_clip(clip);

        // The corner the two scrollbars leave open is a blank square in their
        // frame; its top and left lines are the scrollbars' own.
        if let Some(corner) = self.corner() {
            painter.fill_rect(corner, theme.face);
            let merged = Merged {
                left: true,
                top: true,
                ..Merged::NONE
            };
            painter.stroke_rect_merged(corner, merged, theme.border);
        }
        // Scrollbars last so they sit on top of the field's edges.
        self.v_scrollbar.paint(painter, theme);
        if self.show_h_scrollbar {
            self.h_scrollbar.paint(painter, theme);
        }
    }

    fn event(&mut self, event: &Event, ctx: &mut EventCtx) {
        if !self.enabled {
            return;
        }
        // Once a scrollbar is dragging it gets every event until release.
        for bar in [&mut self.v_scrollbar, &mut self.h_scrollbar] {
            if bar.captures_pointer() {
                bar.event(event, ctx);
                return;
            }
        }
        // The wheel scrolls the view whenever the pointer is over the editor,
        // leaving the caret and selection where they are. Each bar takes the
        // delta along its own axis.
        if let Event::Scroll { pos, .. } = event {
            if self.rect.contains(*pos) {
                self.v_scrollbar.event(event, ctx);
                self.h_scrollbar.event(event, ctx);
            }
            return;
        }
        // Otherwise route positional events that land in a scrollbar.
        if let Some(pos) = event.position() {
            for bar in [&mut self.v_scrollbar, &mut self.h_scrollbar] {
                if bar.rect().contains(pos) {
                    bar.event(event, ctx);
                    return;
                }
            }
        }

        // Past the scrollbar routing above, a pointer move is over the text
        // area — show the I-beam so the user sees they can place the caret or
        // select. Covers a selection drag too (the scrollbar would have
        // captured first if one were in progress there).
        if matches!(event, Event::PointerMove { .. }) {
            ctx.set_cursor(Cursor::Text);
        }

        match event {
            // A press in the corner between the scrollbars is nobody's.
            Event::PointerDown {
                pos,
                button: MouseButton::Left,
                ..
            } if self.text_area().contains(*pos) => {
                ctx.request_focus();
                self.coalesce = false;
                let clicks = self.register_click(*pos);
                self.place_cursor_at(*pos);
                match clicks {
                    1 => {
                        self.selection_anchor = Some(self.cursor);
                        self.drag_active = true;
                    }
                    2 => {
                        let row = self.cursor.0;
                        let (s, e) = self.word_bounds_at(row, self.cursor.1);
                        self.selection_anchor = Some((row, s));
                        self.cursor = (row, e);
                    }
                    _ => {
                        self.select_all();
                    }
                }
                self.reset_blink();
                ctx.request_paint();
            }
            Event::PointerMove { pos } if self.drag_active => {
                self.place_cursor_at(*pos);
                // Dragging past the left or right edge pulls the view along.
                self.reveal_caret = true;
                self.reset_blink();
                ctx.request_paint();
            }
            Event::PointerUp {
                button: MouseButton::Left,
                ..
            } if self.drag_active => {
                self.drag_active = false;
                if self.selection_anchor == Some(self.cursor) {
                    self.selection_anchor = None;
                }
                ctx.request_paint();
            }
            Event::Char { ch, modifiers } if !modifiers.has_command() => {
                if !self.focused || self.read_only {
                    return;
                }
                if *ch == '\t' || *ch >= ' ' {
                    self.insert_char(*ch);
                    self.ensure_cursor_visible();
                    self.reset_blink();
                    ctx.request_paint();
                }
            }
            Event::KeyDown { key, modifiers } if self.focused => {
                if self.scheme.primary_held(*modifiers)
                    && let Key::Char(c) = key
                {
                    let consumed = match c.to_ascii_lowercase() {
                        'c' => {
                            self.copy();
                            true
                        }
                        'x' => {
                            self.cut();
                            self.ensure_cursor_visible();
                            true
                        }
                        'v' => {
                            self.paste();
                            self.ensure_cursor_visible();
                            true
                        }
                        'a' => {
                            self.select_all();
                            true
                        }
                        // Shift+Z redoes everywhere; Ctrl+Y is the PC's own
                        // chord for it, and no shortcut at all on a Mac.
                        'z' if modifiers.shift => {
                            self.redo();
                            self.ensure_cursor_visible();
                            true
                        }
                        'z' => {
                            self.undo();
                            self.ensure_cursor_visible();
                            true
                        }
                        'y' if self.scheme == ModifierScheme::Pc => {
                            self.redo();
                            self.ensure_cursor_visible();
                            true
                        }
                        _ => false,
                    };
                    if consumed {
                        self.reset_blink();
                        ctx.request_paint();
                        return;
                    }
                }

                match key {
                    Key::Named(NamedKey::Enter | NamedKey::Backspace | NamedKey::Delete)
                        if self.read_only => {}
                    Key::Named(NamedKey::Enter) => {
                        self.insert_newline();
                        self.ensure_cursor_visible();
                        self.reset_blink();
                        ctx.request_paint();
                    }
                    Key::Named(NamedKey::Backspace) => {
                        self.backspace();
                        self.ensure_cursor_visible();
                        self.reset_blink();
                        ctx.request_paint();
                    }
                    Key::Named(NamedKey::Delete) => {
                        self.delete_forward();
                        self.ensure_cursor_visible();
                        self.reset_blink();
                        ctx.request_paint();
                    }
                    Key::Named(NamedKey::Left) => {
                        self.before_move(modifiers.shift);
                        if self.scheme.word_move_held(*modifiers) {
                            self.move_word_left();
                        } else {
                            self.move_left();
                        }
                        self.ensure_cursor_visible();
                        self.reset_blink();
                        ctx.request_paint();
                    }
                    Key::Named(NamedKey::Right) => {
                        self.before_move(modifiers.shift);
                        if self.scheme.word_move_held(*modifiers) {
                            self.move_word_right();
                        } else {
                            self.move_right();
                        }
                        self.ensure_cursor_visible();
                        self.reset_blink();
                        ctx.request_paint();
                    }
                    Key::Named(NamedKey::Up) => {
                        self.before_move(modifiers.shift);
                        self.move_up();
                        self.ensure_cursor_visible();
                        self.reset_blink();
                        ctx.request_paint();
                    }
                    Key::Named(NamedKey::Down) => {
                        self.before_move(modifiers.shift);
                        self.move_down();
                        self.ensure_cursor_visible();
                        self.reset_blink();
                        ctx.request_paint();
                    }
                    Key::Named(NamedKey::Home) => {
                        self.before_move(modifiers.shift);
                        self.move_home();
                        self.ensure_cursor_visible();
                        self.reset_blink();
                        ctx.request_paint();
                    }
                    Key::Named(NamedKey::End) => {
                        self.before_move(modifiers.shift);
                        self.move_end();
                        self.ensure_cursor_visible();
                        self.reset_blink();
                        ctx.request_paint();
                    }
                    Key::Named(NamedKey::PageUp) => {
                        self.before_move(modifiers.shift);
                        self.move_page(-1);
                        self.ensure_cursor_visible();
                        self.reset_blink();
                        ctx.request_paint();
                    }
                    Key::Named(NamedKey::PageDown) => {
                        self.before_move(modifiers.shift);
                        self.move_page(1);
                        self.ensure_cursor_visible();
                        self.reset_blink();
                        ctx.request_paint();
                    }
                    _ => {}
                }
            }
            Event::Tick => {
                if !self.focused {
                    return;
                }
                let elapsed_ms = self.blink_since.elapsed().as_millis() as u64;
                let on = (elapsed_ms / BLINK_HALF_MS).is_multiple_of(2);
                if on != self.blink_on {
                    self.blink_on = on;
                    ctx.request_paint();
                }
            }
            _ => {}
        }
    }

    fn captures_pointer(&self) -> bool {
        self.drag_active
            || self.v_scrollbar.captures_pointer()
            || self.h_scrollbar.captures_pointer()
    }

    fn focusable(&self) -> bool {
        self.enabled
    }

    fn set_focused(&mut self, focused: bool) {
        let was_focused = self.focused;
        self.focused = focused;
        if focused {
            if !was_focused {
                self.reset_blink();
            }
        } else {
            // Don't carry a stale click history across focus losses — that
            // would let a re-entry click count as a "double" even though the
            // intervening focus change broke the user's gesture.
            self.last_click = None;
            self.click_count = 0;
            self.drag_active = false;
        }
    }

    fn wants_ticks(&self) -> bool {
        // Steady-state ticks for the caret blink. The embedded scrollbar's
        // own auto-repeat doesn't need forwarding here — it pushes a tick
        // request through the `EventCtx` itself (see `EventCtx::request_tick`).
        self.focused && self.enabled
    }

    fn layout(&mut self, bounds: Rect) {
        self.rect = bounds;
        // Carve the rightmost column for the vertical scrollbar and, if it is
        // shown, the bottom row for the horizontal one; the rest is the text
        // area. Where both are shown each stops a pixel into the corner square
        // between them, so its frame can share their end lines.
        let across = if self.show_h_scrollbar {
            SCROLLBAR_THICKNESS - 1
        } else {
            0
        };
        self.v_scrollbar.set_rect(Rect::new(
            bounds.right() - SCROLLBAR_THICKNESS,
            bounds.y,
            SCROLLBAR_THICKNESS,
            bounds.h - across,
        ));
        self.h_scrollbar.set_rect(if self.show_h_scrollbar {
            Rect::new(
                bounds.x,
                bounds.bottom() - SCROLLBAR_THICKNESS,
                bounds.w - across,
                SCROLLBAR_THICKNESS,
            )
        } else {
            Rect::new(0, 0, 0, 0)
        });
        // Re-sync the scrollbar's range to the new viewport height and clamp the
        // current scroll into it — but DON'T snap the view to the caret. A
        // layout pass fires for reasons unrelated to editing: on Wayland the
        // compositor sends a `configure` (hence a relayout) every time the
        // window's activation state changes, so forcing the cursor into view
        // here would yank a wheel-scrolled view back to the caret (line 0 for a
        // freshly opened file) the moment the window lost or regained focus.
        self.sync_scrollbar();
    }
}

impl TextEditor {
    fn paint_line(
        &self,
        painter: &mut Painter,
        theme: &Theme,
        row: usize,
        text_x: i32,
        y: i32,
        selection: Option<((usize, usize), (usize, usize))>,
    ) {
        let line = &self.lines[row];
        let widths = self
            .cumulative_widths
            .get(&row)
            .map(|v| v.as_slice())
            .unwrap_or(&[]);
        let total_chars = line.chars().count();

        let row_selection = selection.and_then(|(start, end)| {
            if row < start.0 || row > end.0 {
                return None;
            }
            let s = if row == start.0 { start.1 } else { 0 };
            let e = if row == end.0 { end.1 } else { total_chars };
            if s == e { None } else { Some((s, e)) }
        });

        // An unfocused field still draws its selection so the user can see
        // what's selected when keyboard focus is elsewhere — but in the muted
        // "inactive" palette (black-on-gray) rather than the active
        // navy-on-white, matching CUA convention.
        let (sel_bg, sel_text) = if self.focused {
            (theme.highlight_bg, theme.highlight_text)
        } else {
            (theme.face, theme.text)
        };

        if let Some((s, e)) = row_selection {
            let x0 = widths.get(s).copied().unwrap_or(0);
            let x1 = widths
                .get(e)
                .copied()
                .unwrap_or_else(|| widths.last().copied().unwrap_or(0));
            let extra = if let Some((_start, end)) = selection {
                if row < end.0 { 6 } else { 0 }
            } else {
                0
            };
            painter.fill_rect(
                Rect::new(text_x + x0, y, x1 - x0 + extra, LINE_HEIGHT),
                sel_bg,
            );
        }

        if let Some((s, e)) = row_selection {
            let before: String = line.chars().take(s).collect();
            let middle: String = line.chars().skip(s).take(e - s).collect();
            let after: String = line.chars().skip(e).collect();
            painter.text_styled(
                text_x,
                y,
                &before,
                self.font_size,
                theme.text,
                FontFamily::Mono,
                FontStyle::Regular,
            );
            let middle_x = text_x + widths.get(s).copied().unwrap_or(0);
            painter.text_styled(
                middle_x,
                y,
                &middle,
                self.font_size,
                sel_text,
                FontFamily::Mono,
                FontStyle::Regular,
            );
            let after_x = text_x + widths.get(e).copied().unwrap_or(0);
            painter.text_styled(
                after_x,
                y,
                &after,
                self.font_size,
                theme.text,
                FontFamily::Mono,
                FontStyle::Regular,
            );
        } else {
            painter.text_styled(
                text_x,
                y,
                line,
                self.font_size,
                theme.text,
                FontFamily::Mono,
                FontStyle::Regular,
            );
        }
    }
}

/// Where text inserted at `at` ends: on the same row unless it has line
/// breaks, in which case on the row of its last line.
fn end_of(at: Pos, text: &str) -> Pos {
    match text.rsplit_once('\n') {
        None => (at.0, at.1 + text.chars().count()),
        Some((head, last)) => (at.0 + head.matches('\n').count() + 1, last.chars().count()),
    }
}

fn char_to_byte(line: &str, char_idx: usize) -> usize {
    line.char_indices()
        .nth(char_idx)
        .map(|(b, _)| b)
        .unwrap_or(line.len())
}

/// Small upward wedge: a one-pixel apex with a three-pixel base, with `cx`
/// as the horizontal center and `top_y` the row that holds the apex.
fn draw_unfocused_caret(painter: &mut Painter, cx: i32, top_y: i32, color: Color) {
    painter.pixel(cx, top_y, color);
    painter.pixel(cx - 1, top_y + 1, color);
    painter.pixel(cx, top_y + 1, color);
    painter.pixel(cx + 1, top_y + 1, color);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::event::Modifiers;

    fn dispatch(editor: &mut TextEditor, event: &Event) {
        let mut ctx = EventCtx::new();
        editor.event(event, &mut ctx);
    }

    /// A wheel-scrolled view must survive a relayout that isn't a resize. On
    /// Wayland the compositor sends a `configure` — and the runtime a relayout —
    /// every time the window's activation state changes, so a `layout()` that
    /// snapped the view to the caret made the notepad example jump back to the
    /// first line the instant the window lost or regained focus.
    #[test]
    fn relayout_preserves_wheel_scroll() {
        let rect = Rect::new(0, 0, 200, 120);
        let text = (0..100)
            .map(|i| format!("line {i}"))
            .collect::<Vec<_>>()
            .join("\n");

        // A freshly opened document: the caret sits at the top (set_text leaves
        // the cursor on line 0) and the view starts at the top.
        let mut editor = TextEditor::new(rect).with_text(&text);
        editor.layout(rect);
        assert_eq!(editor.cursor, (0, 0));
        assert_eq!(editor.scroll_top(), 0);

        // Wheel down past the first page. The wheel scrolls the view and leaves
        // the caret where it is, so the caret ends up offscreen above the view.
        dispatch(
            &mut editor,
            &Event::Scroll {
                pos: Point::new(100, 60),
                delta_x: 0.0,
                delta_y: 40.0,
            },
        );
        let scrolled = editor.scroll_top();
        assert!(scrolled > 0, "wheel should have moved the view down");
        assert_eq!(editor.cursor, (0, 0), "wheel must not move the caret");

        // The window loses focus: the runtime relayouts at the same size. The
        // view must stay put — not snap back to the caret on line 0.
        editor.layout(rect);
        assert_eq!(
            editor.scroll_top(),
            scrolled,
            "a relayout (e.g. on window focus change) must not jump to the caret"
        );
    }

    fn focused(text: &str) -> TextEditor {
        let mut editor = TextEditor::new(Rect::new(0, 0, 200, 120)).with_text(text);
        editor.set_focused(true);
        editor
    }

    fn type_text(editor: &mut TextEditor, text: &str) {
        for ch in text.chars() {
            dispatch(
                editor,
                &Event::Char {
                    ch,
                    modifiers: Modifiers::default(),
                },
            );
        }
    }

    fn press(editor: &mut TextEditor, key: NamedKey, shift: bool) {
        dispatch(
            editor,
            &Event::KeyDown {
                key: Key::Named(key),
                modifiers: Modifiers {
                    shift,
                    ..Modifiers::default()
                },
            },
        );
    }

    fn chord(editor: &mut TextEditor, key: Key, modifiers: Modifiers) {
        dispatch(editor, &Event::KeyDown { key, modifiers });
    }

    #[test]
    fn mac_shortcuts_take_command_and_option() {
        let ctrl = Modifiers {
            control: true,
            ..Modifiers::default()
        };
        let cmd = Modifiers {
            logo: true,
            ..Modifiers::default()
        };
        let option = Modifiers {
            alt: true,
            ..Modifiers::default()
        };
        let mut editor = focused("foo bar").with_scheme(ModifierScheme::Mac);
        press(&mut editor, NamedKey::End, false);
        chord(&mut editor, Key::Named(NamedKey::Left), option);
        type_text(&mut editor, "x");
        assert_eq!(editor.text(), "foo xbar");

        chord(&mut editor, Key::Char('a'), ctrl);
        type_text(&mut editor, "y");
        assert_eq!(editor.text(), "foo xybar");
        chord(&mut editor, Key::Char('a'), cmd);
        type_text(&mut editor, "z");
        assert_eq!(editor.text(), "z");
    }

    #[test]
    fn read_only_paper_is_a_touch_towards_the_face() {
        let theme = Theme::default();
        let tint = theme.background.mix(theme.face, READ_ONLY_TINT);
        assert!(tint != theme.background && tint != theme.face);

        let backend = crate::mock::MockBackend::new(200, 120);
        let shows_tint =
            |editor: &mut TextEditor| backend.render(editor).pixels().contains(&tint.0);
        let new = || TextEditor::new(Rect::new(0, 0, 200, 120));
        assert!(shows_tint(&mut new().with_read_only(true)));
        assert!(!shows_tint(&mut new()));
        assert!(!shows_tint(&mut new().with_enabled(false)));
        assert!(
            !shows_tint(&mut new().with_read_only(true).with_enabled(false)),
            "disabled wins"
        );
    }

    #[test]
    fn read_only_editor_ignores_every_edit() {
        let cmd = Modifiers {
            logo: true,
            ..Modifiers::default()
        };
        let mut editor = focused("ab\ncd")
            .with_scheme(ModifierScheme::Mac)
            .with_read_only(true);
        assert!(
            editor.focusable(),
            "a read-only editor still takes the focus"
        );

        type_text(&mut editor, "x\t");
        press(&mut editor, NamedKey::Enter, false);
        press(&mut editor, NamedKey::Delete, false);
        press(&mut editor, NamedKey::End, false);
        press(&mut editor, NamedKey::Backspace, false);
        chord(&mut editor, Key::Char('a'), cmd);
        chord(&mut editor, Key::Char('x'), cmd);
        chord(&mut editor, Key::Char('v'), cmd);
        press(&mut editor, NamedKey::Delete, false);
        editor.cut();
        editor.paste();
        assert_eq!(editor.text(), "ab\ncd");
        assert_eq!(
            editor.selected_text().as_deref(),
            Some("ab\ncd"),
            "selecting still works"
        );

        // The program itself may still replace the text.
        editor.set_text("ef");
        assert_eq!(editor.text(), "ef");

        editor.set_read_only(false);
        type_text(&mut editor, "x");
        assert_eq!(editor.text(), "xef");
    }

    fn mac(text: &str) -> TextEditor {
        focused(text).with_scheme(ModifierScheme::Mac)
    }

    /// A 200×120 window to paint an editor in, with the bundled monospace
    /// font — horizontal scrolling is measured in its pixels. The fixtures are
    /// excluded from the published crate, so a test that needs them stands
    /// aside when they are not there rather than failing.
    fn backend() -> Option<crate::mock::MockBackend> {
        let mono = std::fs::read("tests/fonts/DejaVuSansMono.ttf").ok()?;
        let mono = crate::font::Font::from_sans_bytes(mono)?;
        Some(crate::mock::MockBackend::new(200, 120).with_mono_font(mono))
    }

    fn wheel(editor: &mut TextEditor, delta_x: f32) {
        dispatch(
            editor,
            &Event::Scroll {
                pos: Point::new(100, 60),
                delta_x,
                delta_y: 0.0,
            },
        );
    }

    fn click(editor: &mut TextEditor, x: i32, y: i32) {
        let (pos, button) = (Point::new(x, y), MouseButton::Left);
        let modifiers = Modifiers::default();
        dispatch(
            editor,
            &Event::PointerDown {
                pos,
                button,
                modifiers,
            },
        );
        dispatch(
            editor,
            &Event::PointerUp {
                pos,
                button,
                modifiers,
            },
        );
    }

    const LONG: &str = "0123456789012345678901234567890123456789012345678901234567890123456789";

    #[test]
    fn view_follows_the_caret_along_a_long_line() {
        let Some(backend) = backend() else { return };
        let mut editor = focused(&format!("short\n{LONG}"));
        backend.render(&mut editor);
        assert_eq!(editor.h_scrollbar.value(), 0);
        let (view, widest) = (editor.view_width(), editor.widest_line.unwrap());
        assert!(widest > view, "the second line is wider than the field");
        assert_eq!(editor.h_scrollbar.max(), widest + 1 - view);

        // End of the long line: scrolled all the way, caret on the last pixel.
        press(&mut editor, NamedKey::Down, false);
        press(&mut editor, NamedKey::End, false);
        backend.render(&mut editor);
        assert_eq!(editor.h_scrollbar.value(), editor.h_scrollbar.max());

        // A few columns back the caret is still in view: nothing moves.
        for _ in 0..5 {
            press(&mut editor, NamedKey::Left, false);
        }
        backend.render(&mut editor);
        assert_eq!(editor.h_scrollbar.value(), editor.h_scrollbar.max());

        // The short line above ends left of the view, which follows.
        press(&mut editor, NamedKey::Up, false);
        backend.render(&mut editor);
        let caret = editor.cumulative_widths[&0][5];
        assert_eq!(editor.h_scrollbar.value(), caret);

        press(&mut editor, NamedKey::Home, false);
        backend.render(&mut editor);
        assert_eq!(editor.h_scrollbar.value(), 0);
    }

    #[test]
    fn wheel_scrolls_sideways_without_moving_the_caret() {
        let Some(backend) = backend() else { return };
        let mut editor = focused(LONG);
        backend.render(&mut editor);

        wheel(&mut editor, 3.0);
        let column = editor.cumulative_widths[&0][1];
        assert_eq!(editor.h_scrollbar.value(), 3 * column, "a line is a column");
        assert_eq!(editor.cursor, (0, 0));
        // A repaint for any other reason must not snap back to the caret.
        backend.render(&mut editor);
        assert_eq!(editor.h_scrollbar.value(), 3 * column);

        wheel(&mut editor, 1000.0);
        assert_eq!(editor.h_scrollbar.value(), editor.h_scrollbar.max());
        wheel(&mut editor, -1000.0);
        assert_eq!(editor.h_scrollbar.value(), 0);
    }

    #[test]
    fn click_lands_on_the_column_under_it_when_scrolled() {
        let Some(backend) = backend() else { return };
        let mut editor = focused(LONG);
        backend.render(&mut editor);
        wheel(&mut editor, 10.0);
        backend.render(&mut editor);

        // The left end of the text now shows column 10.
        click(&mut editor, PADDING_X, PADDING_Y + 2);
        assert_eq!(editor.cursor, (0, 10));
    }

    #[test]
    fn scroll_range_tracks_the_widest_line_through_edits() {
        let Some(backend) = backend() else { return };
        let mut editor = mac(&format!("{LONG}\n{LONG}{LONG}"));
        backend.render(&mut editor);
        let end = |editor: &TextEditor, row| editor.cumulative_widths[&row].last().copied();
        let (one, two) = (end(&editor, 0), end(&editor, 1));
        assert_eq!(editor.widest_line, two);

        // Typing at the end of the widest line widens the range…
        press(&mut editor, NamedKey::Down, false);
        press(&mut editor, NamedKey::End, false);
        type_text(&mut editor, "xx");
        backend.render(&mut editor);
        let wider = end(&editor, 1);
        assert!(wider > two);
        assert_eq!(editor.widest_line, wider);
        assert_eq!(editor.h_scrollbar.value(), editor.h_scrollbar.max());

        // …and with that line gone the other one sets it, pulling the view
        // back inside.
        press(&mut editor, NamedKey::Home, true);
        press(&mut editor, NamedKey::Backspace, false);
        backend.render(&mut editor);
        assert_eq!(editor.widest_line, one);
        assert!(editor.h_scrollbar.value() <= editor.h_scrollbar.max());

        // Undo puts the line back, and its width with it.
        editor.undo();
        backend.render(&mut editor);
        assert_eq!(editor.widest_line, wider);

        editor.set_text("short");
        backend.render(&mut editor);
        assert_eq!(editor.h_scrollbar.max(), 0);
        assert_eq!(editor.h_scrollbar.value(), 0);
    }

    #[test]
    fn horizontal_scrollbar_takes_the_bottom_row() {
        let rect = Rect::new(0, 0, 200, 120);
        let mut editor = TextEditor::new(rect);
        editor.layout(rect);
        assert_eq!(editor.text_area(), Rect::new(0, 0, 185, 120));
        assert_eq!(editor.corner(), None);
        let rows = editor.visible_rows();

        editor.set_horizontal_scrollbar(true);
        assert_eq!(editor.text_area(), Rect::new(0, 0, 185, 105));
        assert_eq!(editor.v_scrollbar.rect(), Rect::new(184, 0, 16, 105));
        assert_eq!(editor.h_scrollbar.rect(), Rect::new(0, 104, 185, 16));
        assert_eq!(editor.corner(), Some(Rect::new(184, 104, 16, 16)));
        assert_eq!(editor.visible_rows(), rows - 1);

        // A press in the corner neither focuses the editor nor moves the caret.
        let mut editor = editor.with_text("ab\ncd");
        let mut ctx = EventCtx::new();
        editor.event(
            &Event::PointerDown {
                pos: Point::new(190, 110),
                button: MouseButton::Left,
                modifiers: Modifiers::default(),
            },
            &mut ctx,
        );
        assert!(!ctx.is_focus_requested());
        assert_eq!(editor.cursor, (0, 0));
    }

    fn cmd(shift: bool) -> Modifiers {
        Modifiers {
            logo: true,
            shift,
            ..Modifiers::default()
        }
    }

    #[test]
    fn undo_takes_back_a_run_of_typing_at_once() {
        let mut editor = mac("");
        assert!(!editor.can_undo());
        type_text(&mut editor, "hello");
        assert!(editor.can_undo() && !editor.can_redo());

        chord(&mut editor, Key::Char('z'), cmd(false));
        assert_eq!(editor.text(), "");
        assert!(!editor.can_undo() && editor.can_redo());

        chord(&mut editor, Key::Char('z'), cmd(true));
        assert_eq!(editor.text(), "hello");
        assert_eq!(editor.cursor, (0, 5));
        assert!(editor.can_undo() && !editor.can_redo());
    }

    #[test]
    fn moving_the_caret_ends_a_run() {
        let mut editor = mac("");
        type_text(&mut editor, "ab");
        press(&mut editor, NamedKey::Left, false);
        press(&mut editor, NamedKey::Right, false);
        type_text(&mut editor, "cd");
        press(&mut editor, NamedKey::Enter, false);
        type_text(&mut editor, "ef");
        assert_eq!(editor.text(), "abcd\nef");

        // "ef", the line break, "cd" and "ab" are four steps.
        for expected in ["abcd\n", "abcd", "ab", ""] {
            editor.undo();
            assert_eq!(editor.text(), expected);
        }
        editor.undo();
        assert_eq!(editor.text(), "", "nothing left to undo");
    }

    #[test]
    fn undo_puts_a_replaced_selection_back_selected() {
        let mut editor = mac("one two");
        press(&mut editor, NamedKey::End, false);
        for _ in 0..3 {
            press(&mut editor, NamedKey::Left, true);
        }
        type_text(&mut editor, "2!");
        assert_eq!(editor.text(), "one 2!");

        editor.undo();
        assert_eq!(editor.text(), "one two");
        assert_eq!(editor.selected_text().as_deref(), Some("two"));

        editor.redo();
        assert_eq!(editor.text(), "one 2!");
        assert_eq!(editor.cursor, (0, 6));
        assert_eq!(editor.selected_text(), None);
    }

    #[test]
    fn runs_of_backspace_and_delete_are_one_step_each() {
        let mut editor = mac("ab\ncd\nef");
        press(&mut editor, NamedKey::Down, false);
        press(&mut editor, NamedKey::End, false);
        // Backspace through "cd" and the line break before it…
        for _ in 0..3 {
            press(&mut editor, NamedKey::Backspace, false);
        }
        assert_eq!(editor.text(), "ab\nef");
        // …then Delete through the line break after it and the "e".
        for _ in 0..2 {
            press(&mut editor, NamedKey::Delete, false);
        }
        assert_eq!(editor.text(), "abf");

        editor.undo();
        assert_eq!(editor.text(), "ab\nef");
        assert_eq!(editor.cursor, (0, 2));
        editor.undo();
        assert_eq!(editor.text(), "ab\ncd\nef");
        assert_eq!(editor.cursor, (1, 2));
        editor.redo();
        editor.redo();
        assert_eq!(editor.text(), "abf");
    }

    #[test]
    fn an_edit_after_undo_drops_the_redo() {
        let mut editor = mac("");
        type_text(&mut editor, "a");
        editor.undo();
        type_text(&mut editor, "b");
        assert!(!editor.can_redo());
        editor.redo();
        assert_eq!(editor.text(), "b");
        // The undone "a" must not have grown into the new step either.
        editor.undo();
        assert_eq!(editor.text(), "");
        assert!(!editor.can_undo());
    }

    #[test]
    fn typing_after_an_undo_is_a_step_of_its_own() {
        let mut editor = mac("");
        type_text(&mut editor, "a");
        press(&mut editor, NamedKey::Enter, false);
        // Back on the first line, right where the "a" step left the caret.
        editor.undo();
        type_text(&mut editor, "b");
        editor.undo();
        assert_eq!(editor.text(), "a");
    }

    #[test]
    fn a_multi_line_replacement_undoes_whole() {
        let mut editor = mac("ab\ncd\nef");
        press(&mut editor, NamedKey::Right, false);
        press(&mut editor, NamedKey::Down, true);
        press(&mut editor, NamedKey::Down, true);
        // What a paste does with the clipboard's text.
        editor.replace_selection("1\n2\n3\n4", EditKind::Other);
        assert_eq!(editor.text(), "a1\n2\n3\n4f");
        assert_eq!(editor.cursor, (3, 1));

        editor.undo();
        assert_eq!(editor.text(), "ab\ncd\nef");
        assert_eq!(editor.selected_text().as_deref(), Some("b\ncd\ne"));
        editor.redo();
        assert_eq!(editor.text(), "a1\n2\n3\n4f");
        assert_eq!(editor.cursor, (3, 1));
    }

    #[test]
    fn set_text_forgets_the_history() {
        let mut editor = mac("");
        type_text(&mut editor, "a");
        editor.undo();
        editor.set_text("new");
        assert!(!editor.can_undo() && !editor.can_redo());
        editor.redo();
        editor.undo();
        assert_eq!(editor.text(), "new");
    }

    #[test]
    fn read_only_editor_keeps_its_history_untouched() {
        let mut editor = mac("");
        type_text(&mut editor, "a");
        editor.set_read_only(true);
        assert!(!editor.can_undo());
        chord(&mut editor, Key::Char('z'), cmd(false));
        assert_eq!(editor.text(), "a");

        editor.set_read_only(false);
        editor.undo();
        assert_eq!(editor.text(), "");
    }

    #[test]
    fn ctrl_y_redoes_on_a_pc_only() {
        let ctrl = Modifiers {
            control: true,
            ..Modifiers::default()
        };
        let mut editor = focused("").with_scheme(ModifierScheme::Pc);
        type_text(&mut editor, "a");
        chord(&mut editor, Key::Char('z'), ctrl);
        assert_eq!(editor.text(), "");
        chord(&mut editor, Key::Char('y'), ctrl);
        assert_eq!(editor.text(), "a");

        let mut editor = mac("");
        type_text(&mut editor, "a");
        editor.undo();
        chord(&mut editor, Key::Char('y'), cmd(false));
        assert_eq!(editor.text(), "", "⌘Y is not a shortcut on a Mac");
    }

    /// Whatever the edits, undoing all of them leads back through every text
    /// the editor showed on the way, and redoing them forward again.
    #[test]
    fn undo_and_redo_retrace_any_edits() {
        // A small deterministic generator — no need for a dependency.
        let mut seed = 0x2545_f491_4f6c_dd1d_u64;
        let mut next = |n: u64| {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            seed % n
        };

        let mut editor = mac("héllo wörld\n\nsecond line\nthird");
        let mut history = vec![editor.text()];
        for _ in 0..400 {
            for _ in 0..next(4) {
                let key = [
                    NamedKey::Left,
                    NamedKey::Right,
                    NamedKey::Up,
                    NamedKey::Down,
                    NamedKey::Home,
                    NamedKey::End,
                ][next(6) as usize];
                press(&mut editor, key, next(3) == 0);
            }
            let steps = editor.undo_stack.len();
            match next(6) {
                0 => type_text(&mut editor, ["x", "é", "\t"][next(3) as usize]),
                1 => press(&mut editor, NamedKey::Enter, false),
                2 => press(&mut editor, NamedKey::Backspace, false),
                3 => press(&mut editor, NamedKey::Delete, false),
                4 => editor.replace_selection("pasted\ntext\n", EditKind::Other),
                _ => editor.replace_selection("", EditKind::Other),
            }
            // A step that grew the previous one overwrites its text; a new
            // step adds one; an edit that changed nothing does neither.
            if editor.undo_stack.len() > steps {
                history.push(editor.text());
            } else if let Some(last) = history.last_mut() {
                *last = editor.text();
            }
        }
        assert_eq!(history.len(), editor.undo_stack.len() + 1);

        for expected in history.iter().rev().skip(1) {
            editor.undo();
            assert_eq!(&editor.text(), expected);
        }
        assert!(!editor.can_undo());
        for expected in history.iter().skip(1) {
            editor.redo();
            assert_eq!(&editor.text(), expected);
        }
        assert!(!editor.can_redo());
    }

    /// An empty selection — anchor and caret at the same spot — must not
    /// outlive the next edit. Otherwise the caret moves on past the inserted
    /// text, the anchor stays behind, and what was just typed ends up selected
    /// and replaced by the next keystroke.
    #[test]
    fn typing_after_select_all_on_empty_text_keeps_every_character() {
        let mut editor = focused("");
        editor.select_all();
        type_text(&mut editor, "rename");
        assert_eq!(editor.text(), "rename");
    }

    #[test]
    fn enter_after_select_all_on_empty_text_keeps_the_newline() {
        let mut editor = focused("");
        editor.select_all();
        press(&mut editor, NamedKey::Enter, false);
        type_text(&mut editor, "x");
        assert_eq!(editor.text(), "\nx");
    }

    #[test]
    fn backspace_drops_an_empty_selection() {
        let mut editor = focused("ab\ncd");
        press(&mut editor, NamedKey::Down, false);
        press(&mut editor, NamedKey::Left, true);
        press(&mut editor, NamedKey::Right, true);
        // Joins the lines, which would leave the anchor on a row that's gone.
        press(&mut editor, NamedKey::Backspace, false);
        type_text(&mut editor, "x");
        assert_eq!(editor.text(), "abxcd");
    }
}
