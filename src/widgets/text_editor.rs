use crate::event::{Event, EventCtx, Key, MouseButton, NamedKey};
use crate::geometry::{Color, Point, Rect};
use crate::painter::Painter;
use crate::theme::Theme;
use crate::widget::Widget;

const PADDING_X: i32 = 4;
const PADDING_Y: i32 = 2;
const LINE_HEIGHT: i32 = 14;

/// A minimal multi-line text editor — sunken white field, monospace text,
/// cursor, selection with cut/copy/paste. Matches Notepad's behavior closely
/// enough for a system-utility editor; undo and word wrap come later.
///
/// Coordinates and `font_size` are all in logical pixels. The editor stores
/// content as `Vec<String>` (one entry per line); the cursor and the
/// selection anchor are tracked as `(row, col)` *character* indices that
/// always point to a valid position (UTF-8 multi-byte safe).
pub struct TextEditor {
    pub rect: Rect,
    pub font_size: f32,
    lines: Vec<String>,
    cursor: (usize, usize),
    /// Start of the current selection. When `Some` and != cursor, that
    /// range is the selection.
    selection_anchor: Option<(usize, usize)>,
    scroll_top: usize,
    focused: bool,
    /// Per-line cumulative pixel widths from column 0 → N, rebuilt every
    /// paint. `widths[row][col]` is the x-offset (in logical px) where the
    /// caret should sit at character index `col` on that row.
    cumulative_widths: Vec<Vec<i32>>,
    /// True while the user is mouse-dragging to extend the selection.
    drag_active: bool,
    /// Lazily-initialized clipboard handle. `None` once we've tried and
    /// failed to open the OS clipboard (e.g., headless environments).
    clipboard: Option<arboard::Clipboard>,
}

impl TextEditor {
    pub fn new(rect: Rect) -> Self {
        Self {
            rect,
            font_size: 11.0,
            lines: vec![String::new()],
            cursor: (0, 0),
            selection_anchor: None,
            scroll_top: 0,
            focused: false,
            cumulative_widths: Vec::new(),
            drag_active: false,
            clipboard: None,
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

    pub fn text(&self) -> String {
        self.lines.join("\n")
    }

    pub fn set_text(&mut self, text: &str) {
        self.lines = if text.is_empty() {
            vec![String::new()]
        } else {
            text.split('\n').map(String::from).collect()
        };
        self.cursor = (0, 0);
        self.selection_anchor = None;
        self.scroll_top = 0;
    }

    pub fn is_focused(&self) -> bool {
        self.focused
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

    fn has_selection(&self) -> bool {
        self.selection_anchor
            .map(|a| a != self.cursor)
            .unwrap_or(false)
    }

    /// Returns the ordered (start, end) of the current selection, or `None`
    /// if there isn't one.
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
        let mut out = String::new();
        if start.0 == end.0 {
            let line = &self.lines[start.0];
            out.extend(line.chars().skip(start.1).take(end.1 - start.1));
        } else {
            out.extend(self.lines[start.0].chars().skip(start.1));
            out.push('\n');
            for row in (start.0 + 1)..end.0 {
                out.push_str(&self.lines[row]);
                out.push('\n');
            }
            out.extend(self.lines[end.0].chars().take(end.1));
        }
        Some(out)
    }

    fn delete_selection(&mut self) {
        let Some((start, end)) = self.selection_range() else {
            return;
        };
        if start.0 == end.0 {
            let line = &mut self.lines[start.0];
            let bs = char_to_byte(line, start.1);
            let be = char_to_byte(line, end.1);
            line.replace_range(bs..be, "");
        } else {
            let first_prefix: String = self.lines[start.0].chars().take(start.1).collect();
            let last_suffix: String = self.lines[end.0].chars().skip(end.1).collect();
            self.lines[start.0] = format!("{}{}", first_prefix, last_suffix);
            self.lines.drain((start.0 + 1)..=end.0);
        }
        self.cursor = start;
        self.selection_anchor = None;
    }

    /// Update the selection anchor for an impending cursor move. If `extend`
    /// is true (Shift held), we start (or keep) a selection anchored at the
    /// current cursor; if not, any selection collapses.
    fn before_move(&mut self, extend: bool) {
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
        if let Some(text) = self.selected_text() {
            self.clipboard_set(&text);
            self.delete_selection();
        }
    }

    pub fn paste(&mut self) {
        let Some(text) = self.clipboard_get() else { return };
        if self.has_selection() {
            self.delete_selection();
        }
        self.insert_text(&text);
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

    // ---------------------------------------------------------------- editing

    fn visible_rows(&self) -> i32 {
        ((self.rect.h - PADDING_Y * 2) / LINE_HEIGHT).max(1)
    }

    fn ensure_cursor_visible(&mut self) {
        let visible = self.visible_rows() as usize;
        if self.cursor.0 < self.scroll_top {
            self.scroll_top = self.cursor.0;
        } else if self.cursor.0 >= self.scroll_top + visible {
            self.scroll_top = self.cursor.0 + 1 - visible;
        }
    }

    fn clamp_col(&mut self) {
        let line_len = self.lines[self.cursor.0].chars().count();
        if self.cursor.1 > line_len {
            self.cursor.1 = line_len;
        }
    }

    fn insert_char(&mut self, ch: char) {
        let (row, col) = self.cursor;
        let line = &mut self.lines[row];
        let byte_idx = char_to_byte(line, col);
        line.insert(byte_idx, ch);
        self.cursor.1 += 1;
    }

    fn insert_newline(&mut self) {
        let (row, col) = self.cursor;
        let line = self.lines[row].clone();
        let byte_idx = char_to_byte(&line, col);
        let (left, right) = line.split_at(byte_idx);
        self.lines[row] = left.to_string();
        self.lines.insert(row + 1, right.to_string());
        self.cursor = (row + 1, 0);
    }

    fn insert_text(&mut self, text: &str) {
        for ch in text.chars() {
            match ch {
                '\n' => self.insert_newline(),
                '\r' => {} // CRLF: drop the CR, the LF triggers insert_newline
                _ => self.insert_char(ch),
            }
        }
    }

    fn backspace(&mut self) {
        if self.has_selection() {
            self.delete_selection();
            return;
        }
        let (row, col) = self.cursor;
        if col > 0 {
            let line = &mut self.lines[row];
            let prev = col - 1;
            let start = char_to_byte(line, prev);
            let end = char_to_byte(line, col);
            line.replace_range(start..end, "");
            self.cursor.1 = prev;
        } else if row > 0 {
            let prev_len = self.lines[row - 1].chars().count();
            let tail = self.lines.remove(row);
            self.lines[row - 1].push_str(&tail);
            self.cursor = (row - 1, prev_len);
        }
    }

    fn delete_forward(&mut self) {
        if self.has_selection() {
            self.delete_selection();
            return;
        }
        let (row, col) = self.cursor;
        let line_len = self.lines[row].chars().count();
        if col < line_len {
            let line = &mut self.lines[row];
            let start = char_to_byte(line, col);
            let end = char_to_byte(line, col + 1);
            line.replace_range(start..end, "");
        } else if row + 1 < self.lines.len() {
            let next = self.lines.remove(row + 1);
            self.lines[row].push_str(&next);
        }
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

    fn move_home(&mut self) {
        self.cursor.1 = 0;
    }

    fn move_end(&mut self) {
        self.cursor.1 = self.lines[self.cursor.0].chars().count();
    }

    /// Place the cursor at the click position using the per-line widths
    /// cached on the most recent paint.
    fn place_cursor_at(&mut self, pos: Point) {
        if self.lines.is_empty() {
            return;
        }
        let local_y = (pos.y - self.rect.y - PADDING_Y).max(0);
        let row_offset = (local_y / LINE_HEIGHT) as usize;
        let row = (self.scroll_top + row_offset).min(self.lines.len() - 1);
        self.cursor.0 = row;

        let text_x = self.rect.x + PADDING_X;
        let target = (pos.x - text_x).max(0);
        let widths = self
            .cumulative_widths
            .get(row)
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
        // Rebuild cumulative widths against the monospace font.
        self.cumulative_widths.clear();
        for line in &self.lines {
            let n = line.chars().count();
            let mut widths = Vec::with_capacity(n + 1);
            widths.push(0);
            for col in 1..=n {
                let prefix: String = line.chars().take(col).collect();
                widths.push(painter.measure_mono_text(&prefix, self.font_size).w);
            }
            self.cumulative_widths.push(widths);
        }

        // Sunken white field with 1-px black outer border.
        painter.fill_rect(self.rect, Color::WHITE);
        painter.sunken_bevel(self.rect, theme.highlight, theme.shadow);
        painter.stroke_rect(self.rect, theme.border);

        let text_x = self.rect.x + PADDING_X;
        let text_y0 = self.rect.y + PADDING_Y;
        let visible = self.visible_rows() as usize;
        let selection = self.selection_range();

        for row_offset in 0..visible {
            let row = self.scroll_top + row_offset;
            if row >= self.lines.len() {
                break;
            }
            let y = text_y0 + row_offset as i32 * LINE_HEIGHT;
            self.paint_line(painter, theme, row, text_x, y, selection);
        }

        if self.focused {
            let (crow, ccol) = self.cursor;
            if crow >= self.scroll_top && crow < self.scroll_top + visible {
                let prefix_w = self
                    .cumulative_widths
                    .get(crow)
                    .and_then(|widths| widths.get(ccol))
                    .copied()
                    .unwrap_or(0);
                let cx = text_x + prefix_w;
                let cy = text_y0 + (crow - self.scroll_top) as i32 * LINE_HEIGHT;
                painter.v_line(cx, cy, LINE_HEIGHT, theme.text);
            }
        }
    }

    fn event(&mut self, event: &Event, ctx: &mut EventCtx) {
        match event {
            Event::PointerDown {
                pos,
                button: MouseButton::Left,
            } => {
                ctx.request_focus();
                self.place_cursor_at(*pos);
                self.selection_anchor = Some(self.cursor);
                self.drag_active = true;
                ctx.request_paint();
            }
            Event::PointerMove { pos } => {
                if self.drag_active {
                    self.place_cursor_at(*pos);
                    ctx.request_paint();
                }
            }
            Event::PointerUp {
                button: MouseButton::Left,
                ..
            } => {
                if self.drag_active {
                    self.drag_active = false;
                    if self.selection_anchor == Some(self.cursor) {
                        self.selection_anchor = None;
                    }
                    ctx.request_paint();
                }
            }
            Event::Char { ch, modifiers } if !modifiers.has_command() => {
                if !self.focused {
                    return;
                }
                if *ch == '\t' || *ch >= ' ' {
                    if self.has_selection() {
                        self.delete_selection();
                    }
                    self.insert_char(*ch);
                    self.ensure_cursor_visible();
                    ctx.request_paint();
                }
            }
            Event::KeyDown { key, modifiers } if self.focused => {
                // Ctrl shortcuts take precedence over everything else.
                if modifiers.control && let Key::Char(c) = key {
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
                        _ => false,
                    };
                    if consumed {
                        ctx.request_paint();
                        return;
                    }
                }

                match key {
                    Key::Named(NamedKey::Enter) => {
                        if self.has_selection() {
                            self.delete_selection();
                        }
                        self.insert_newline();
                        self.ensure_cursor_visible();
                        ctx.request_paint();
                    }
                    Key::Named(NamedKey::Backspace) => {
                        self.backspace();
                        self.ensure_cursor_visible();
                        ctx.request_paint();
                    }
                    Key::Named(NamedKey::Delete) => {
                        self.delete_forward();
                        self.ensure_cursor_visible();
                        ctx.request_paint();
                    }
                    Key::Named(NamedKey::Left) => {
                        self.before_move(modifiers.shift);
                        self.move_left();
                        self.ensure_cursor_visible();
                        ctx.request_paint();
                    }
                    Key::Named(NamedKey::Right) => {
                        self.before_move(modifiers.shift);
                        self.move_right();
                        self.ensure_cursor_visible();
                        ctx.request_paint();
                    }
                    Key::Named(NamedKey::Up) => {
                        self.before_move(modifiers.shift);
                        self.move_up();
                        self.ensure_cursor_visible();
                        ctx.request_paint();
                    }
                    Key::Named(NamedKey::Down) => {
                        self.before_move(modifiers.shift);
                        self.move_down();
                        self.ensure_cursor_visible();
                        ctx.request_paint();
                    }
                    Key::Named(NamedKey::Home) => {
                        self.before_move(modifiers.shift);
                        self.move_home();
                        ctx.request_paint();
                    }
                    Key::Named(NamedKey::End) => {
                        self.before_move(modifiers.shift);
                        self.move_end();
                        ctx.request_paint();
                    }
                    _ => {}
                }
            }
            _ => {}
        }
    }

    fn captures_pointer(&self) -> bool {
        self.drag_active
    }

    fn focusable(&self) -> bool {
        true
    }

    fn set_focused(&mut self, focused: bool) {
        self.focused = focused;
    }

    fn layout(&mut self, bounds: Rect) {
        self.rect = bounds;
        self.ensure_cursor_visible();
    }
}

impl TextEditor {
    /// Paint a single visible row: line text in the foreground color, with a
    /// navy block + white text drawn over any selected substring.
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
            .get(row)
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

        // Selection band: a NAVY block under the selected glyphs. For rows
        // strictly between the first and last lines of a multi-line
        // selection we extend the band slightly past the line end so it
        // looks continuous.
        if let Some((s, e)) = row_selection {
            let x0 = widths.get(s).copied().unwrap_or(0);
            let x1 = widths.get(e).copied().unwrap_or_else(|| {
                widths.last().copied().unwrap_or(0)
            });
            let extra = if let Some((_start, end)) = selection {
                if row < end.0 { 6 } else { 0 }
            } else {
                0
            };
            painter.fill_rect(
                Rect::new(text_x + x0, y, x1 - x0 + extra, LINE_HEIGHT),
                theme.highlight_bg,
            );
        }

        if let Some((s, e)) = row_selection {
            let before: String = line.chars().take(s).collect();
            let middle: String = line.chars().skip(s).take(e - s).collect();
            let after: String = line.chars().skip(e).collect();
            painter.mono_text(text_x, y, &before, self.font_size, theme.text);
            let middle_x = text_x + widths.get(s).copied().unwrap_or(0);
            painter.mono_text(
                middle_x,
                y,
                &middle,
                self.font_size,
                theme.highlight_text,
            );
            let after_x = text_x + widths.get(e).copied().unwrap_or(0);
            painter.mono_text(after_x, y, &after, self.font_size, theme.text);
        } else {
            painter.mono_text(text_x, y, line, self.font_size, theme.text);
        }
    }
}

/// Convert a logical character index into a byte index inside a UTF-8 line.
/// Saturates at the end of the line.
fn char_to_byte(line: &str, char_idx: usize) -> usize {
    line.char_indices()
        .nth(char_idx)
        .map(|(b, _)| b)
        .unwrap_or(line.len())
}
