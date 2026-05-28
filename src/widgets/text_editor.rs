use crate::event::{Event, EventCtx, Key, MouseButton, NamedKey};
use crate::geometry::{Color, Point, Rect};
use crate::painter::Painter;
use crate::theme::Theme;
use crate::widget::Widget;

const PADDING_X: i32 = 4;
const PADDING_Y: i32 = 2;
const LINE_HEIGHT: i32 = 14;

/// A minimal multi-line text editor — sunken white field, cursor, basic
/// navigation. Matches Notepad's behavior closely enough for a system-utility
/// editor; selections, undo, word-wrap, and clipboard come later.
///
/// Coordinates and `font_size` are all in logical pixels. The editor stores
/// its content as `Vec<String>` (one row per line); the cursor is tracked as
/// a `(row, col)` character index that always points to a valid position.
pub struct TextEditor {
    pub rect: Rect,
    pub font_size: f32,
    lines: Vec<String>,
    cursor: (usize, usize),
    scroll_top: usize,
    focused: bool,
    /// Per-line cumulative pixel widths from column 0 → N, rebuilt every
    /// paint. `widths[row][col]` is the x-offset (in logical px) where the
    /// caret should sit at character index `col` on that row. Used so
    /// pointer-down can map a click into a character position without
    /// needing a `Painter` in the event handler.
    cumulative_widths: Vec<Vec<i32>>,
}

impl TextEditor {
    pub fn new(rect: Rect) -> Self {
        Self {
            rect,
            font_size: 11.0,
            lines: vec![String::new()],
            cursor: (0, 0),
            scroll_top: 0,
            focused: false,
            cumulative_widths: Vec::new(),
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
        self.scroll_top = 0;
    }

    pub fn is_focused(&self) -> bool {
        self.focused
    }

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

    fn backspace(&mut self) {
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
    /// cached on the most recent paint. Picks the column whose caret x is
    /// closest to the click x.
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
        // Rebuild per-character cumulative widths so click→cursor mapping
        // can answer without needing a Painter at event time.
        self.cumulative_widths.clear();
        for line in &self.lines {
            let n = line.chars().count();
            let mut widths = Vec::with_capacity(n + 1);
            widths.push(0);
            for col in 1..=n {
                let prefix: String = line.chars().take(col).collect();
                widths.push(painter.measure_text(&prefix, self.font_size).w);
            }
            self.cumulative_widths.push(widths);
        }

        // Sunken white field — interior background is always white regardless
        // of theme.background, matching Notepad's look.
        painter.fill_rect(self.rect, Color::WHITE);
        painter.sunken_bevel(self.rect, theme.highlight, theme.shadow);
        painter.stroke_rect(self.rect, theme.border);

        let text_x = self.rect.x + PADDING_X;
        let text_y0 = self.rect.y + PADDING_Y;
        let visible = self.visible_rows() as usize;

        for row_offset in 0..visible {
            let row = self.scroll_top + row_offset;
            if row >= self.lines.len() {
                break;
            }
            let y = text_y0 + row_offset as i32 * LINE_HEIGHT;
            painter.text(text_x, y, &self.lines[row], self.font_size, theme.text);
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
                ctx.request_paint();
            }
            Event::Char { ch, modifiers } if !modifiers.has_command() => {
                if !self.focused {
                    return;
                }
                if *ch == '\t' || *ch >= ' ' {
                    self.insert_char(*ch);
                    self.ensure_cursor_visible();
                    ctx.request_paint();
                }
            }
            Event::KeyDown { key, modifiers: _ } if self.focused => match key {
                Key::Named(NamedKey::Enter) => {
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
                    self.move_left();
                    self.ensure_cursor_visible();
                    ctx.request_paint();
                }
                Key::Named(NamedKey::Right) => {
                    self.move_right();
                    self.ensure_cursor_visible();
                    ctx.request_paint();
                }
                Key::Named(NamedKey::Up) => {
                    self.move_up();
                    self.ensure_cursor_visible();
                    ctx.request_paint();
                }
                Key::Named(NamedKey::Down) => {
                    self.move_down();
                    self.ensure_cursor_visible();
                    ctx.request_paint();
                }
                Key::Named(NamedKey::Home) => {
                    self.move_home();
                    ctx.request_paint();
                }
                Key::Named(NamedKey::End) => {
                    self.move_end();
                    ctx.request_paint();
                }
                _ => {}
            },
            _ => {}
        }
    }

    fn focusable(&self) -> bool {
        true
    }

    fn set_focused(&mut self, focused: bool) {
        self.focused = focused;
    }

    fn layout(&mut self, bounds: Rect) {
        self.rect = bounds;
        // Re-clamp scroll so a smaller viewport still shows the cursor.
        self.ensure_cursor_visible();
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
