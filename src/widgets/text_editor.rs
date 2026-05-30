use std::collections::HashMap;
use std::time::{Duration, Instant};

use crate::event::{Event, EventCtx, Key, MouseButton, NamedKey};
use crate::geometry::{Color, Point, Rect};
use crate::painter::Painter;
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

/// A minimal multi-line text editor — sunken white field, monospace text,
/// cursor, selection with cut/copy/paste, and a built-in vertical scrollbar.
///
/// Only the visible rows are measured and drawn on each paint — text that's
/// scrolled out of view contributes no work to the render loop. The scrollbar
/// owns the canonical scroll position; `TextEditor` reads it via
/// `scrollbar.value()`.
pub struct TextEditor {
    pub rect: Rect,
    pub font_size: f32,
    lines: Vec<String>,
    cursor: (usize, usize),
    selection_anchor: Option<(usize, usize)>,
    focused: bool,
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
            focused: false,
            cumulative_widths: HashMap::new(),
            drag_active: false,
            last_click: None,
            click_count: 0,
            clipboard: None,
            v_scrollbar: ScrollBar::vertical(Rect::new(0, 0, 0, 0)),
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
        self.v_scrollbar.set_value(0);
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
    /// scrollbar occupies on the right edge.
    fn text_area(&self) -> Rect {
        let sb_w = if self.v_scrollbar.rect().w > 0 {
            SCROLLBAR_THICKNESS
        } else {
            0
        };
        Rect::new(self.rect.x, self.rect.y, (self.rect.w - sb_w).max(0), self.rect.h)
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
        ((self.text_area().h - PADDING_Y * 2) / LINE_HEIGHT).max(1)
    }

    fn sync_scrollbar(&mut self) {
        let visible = self.visible_rows();
        let max_scroll = (self.lines.len() as i32 - visible).max(0);
        self.v_scrollbar.set_range(visible, max_scroll);
    }

    fn ensure_cursor_visible(&mut self) {
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
                '\r' => {}
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
            self.cursor
                .0
                .saturating_sub(step * (-delta_pages) as usize)
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

        let text_x = text.x + PADDING_X;
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

        // Sunken white field with 1-px black outer border around the whole
        // widget (text area + scrollbar live inside).
        painter.fill_rect(text, Color::WHITE);
        painter.sunken_bevel(text, theme.highlight, theme.shadow);
        painter.stroke_rect(text, theme.border);

        let text_x = text.x + PADDING_X;
        let text_y0 = text.y + PADDING_Y;
        let visible = self.visible_rows() as usize;
        let scroll_top = self.scroll_top();

        // Rebuild cumulative widths only for visible rows.
        self.cumulative_widths.clear();
        for row_offset in 0..visible {
            let row = scroll_top + row_offset;
            if row >= self.lines.len() {
                break;
            }
            let line = &self.lines[row];
            let n = line.chars().count();
            let mut widths = Vec::with_capacity(n + 1);
            widths.push(0);
            for col in 1..=n {
                let prefix: String = line.chars().take(col).collect();
                widths.push(painter.measure_mono_text(&prefix, self.font_size).w);
            }
            self.cumulative_widths.insert(row, widths);
        }

        let selection = self.selection_range();
        for row_offset in 0..visible {
            let row = scroll_top + row_offset;
            if row >= self.lines.len() {
                break;
            }
            let y = text_y0 + row_offset as i32 * LINE_HEIGHT;
            self.paint_line(painter, theme, row, text_x, y, selection);
        }

        let (crow, ccol) = self.cursor;
        if crow >= scroll_top && crow < scroll_top + visible {
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

        // Scrollbar last so it sits on top of the field's right edge.
        self.v_scrollbar.paint(painter, theme);
    }

    fn event(&mut self, event: &Event, ctx: &mut EventCtx) {
        // Once the scrollbar is dragging it gets every event until release.
        if self.v_scrollbar.captures_pointer() {
            self.v_scrollbar.event(event, ctx);
            return;
        }
        // Otherwise route positional events that land in the scrollbar.
        if let Some(pos) = event.position()
            && self.v_scrollbar.rect().contains(pos)
        {
            self.v_scrollbar.event(event, ctx);
            return;
        }

        match event {
            Event::PointerDown {
                pos,
                button: MouseButton::Left,
            } => {
                ctx.request_focus();
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
            Event::PointerMove { pos }
                if self.drag_active => {
                    self.place_cursor_at(*pos);
                    self.reset_blink();
                    ctx.request_paint();
                }
            Event::PointerUp {
                button: MouseButton::Left,
                ..
            }
                if self.drag_active => {
                    self.drag_active = false;
                    if self.selection_anchor == Some(self.cursor) {
                        self.selection_anchor = None;
                    }
                    ctx.request_paint();
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
                    self.reset_blink();
                    ctx.request_paint();
                }
            }
            Event::KeyDown { key, modifiers } if self.focused => {
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
                        self.reset_blink();
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
                        if modifiers.control {
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
                        if modifiers.control {
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
                        self.reset_blink();
                        ctx.request_paint();
                    }
                    Key::Named(NamedKey::End) => {
                        self.before_move(modifiers.shift);
                        self.move_end();
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
        self.drag_active || self.v_scrollbar.captures_pointer()
    }

    fn focusable(&self) -> bool {
        true
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
        self.focused
    }

    fn layout(&mut self, bounds: Rect) {
        self.rect = bounds;
        // Carve the rightmost column for the scrollbar; the rest is the
        // text area.
        let sb_rect = Rect::new(
            bounds.right() - SCROLLBAR_THICKNESS,
            bounds.y,
            SCROLLBAR_THICKNESS,
            bounds.h,
        );
        self.v_scrollbar.set_rect(sb_rect);
        self.ensure_cursor_visible();
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
            painter.mono_text(text_x, y, &before, self.font_size, theme.text);
            let middle_x = text_x + widths.get(s).copied().unwrap_or(0);
            painter.mono_text(
                middle_x,
                y,
                &middle,
                self.font_size,
                sel_text,
            );
            let after_x = text_x + widths.get(e).copied().unwrap_or(0);
            painter.mono_text(after_x, y, &after, self.font_size, theme.text);
        } else {
            painter.mono_text(text_x, y, line, self.font_size, theme.text);
        }
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
