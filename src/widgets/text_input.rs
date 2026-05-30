use std::time::{Duration, Instant};

use crate::event::{Event, EventCtx, Key, MouseButton, NamedKey};
use crate::geometry::{Color, Point, Rect};
use crate::painter::Painter;
use crate::theme::Theme;
use crate::widget::Widget;

const PADDING_X: i32 = 4;
const MULTI_CLICK_MS: u64 = 400;
const MULTI_CLICK_SLOP: i32 = 3;
/// How long each half of the caret blink lasts while the widget is focused.
const BLINK_HALF_MS: u64 = 500;

type ChangeHandler = Box<dyn FnMut(&mut EventCtx, &str)>;

/// Single-line text input — sunken white field with proportional text, caret,
/// range selection, clipboard, double-click word select / triple-click
/// select-all and horizontal scrolling when the text doesn't fit. Companion
/// to [`TextEditor`](crate::widgets::TextEditor) for the common case where a
/// single line is enough.
pub struct TextInput {
    pub rect: Rect,
    font_size: Option<f32>,
    chars: Vec<char>,
    cursor: usize,
    selection_anchor: Option<usize>,
    focused: bool,
    /// Cumulative pixel widths for the current text: `widths[i]` is the caret
    /// x-offset (in logical px) at character index `i`. Rebuilt every paint
    /// and reused by event handlers to map pointer x ↔ char index.
    cumulative_widths: Vec<i32>,
    scroll_x: i32,
    drag_active: bool,
    last_click: Option<(Instant, Point)>,
    click_count: u32,
    clipboard: Option<arboard::Clipboard>,
    on_change: Option<ChangeHandler>,
    /// When the current half of the blink cycle started. Reset on every
    /// user action so the caret stays visible (and "on") while the user is
    /// actively typing.
    blink_since: Instant,
    /// Cached on/off state of the focused caret. Updated by `Event::Tick`.
    blink_on: bool,
}

impl TextInput {
    pub fn new(rect: Rect) -> Self {
        Self {
            rect,
            font_size: None,
            chars: Vec::new(),
            cursor: 0,
            selection_anchor: None,
            focused: false,
            cumulative_widths: vec![0],
            scroll_x: 0,
            drag_active: false,
            last_click: None,
            click_count: 0,
            clipboard: None,
            on_change: None,
            blink_since: Instant::now(),
            blink_on: true,
        }
    }

    pub fn with_text(mut self, text: impl AsRef<str>) -> Self {
        self.set_text(text.as_ref());
        self
    }

    pub fn with_font_size(mut self, size: f32) -> Self {
        self.font_size = Some(size);
        self
    }

    pub fn on_change<F>(mut self, handler: F) -> Self
    where
        F: FnMut(&mut EventCtx, &str) + 'static,
    {
        self.on_change = Some(Box::new(handler));
        self
    }

    pub fn text(&self) -> String {
        self.chars.iter().collect()
    }

    pub fn set_text(&mut self, text: &str) {
        // Strip newlines: this is a single-line widget. Tabs are kept — the
        // proportional font will render them as a wider glyph (or whatever
        // the font defines), and Tab as a focus-cycle key never reaches us.
        self.chars = text.chars().filter(|&c| c != '\n' && c != '\r').collect();
        self.cursor = self.chars.len();
        self.selection_anchor = None;
        self.scroll_x = 0;
    }

    pub fn is_focused(&self) -> bool {
        self.focused
    }

    fn font_size_for(&self, theme: &Theme) -> f32 {
        self.font_size.unwrap_or(theme.font_size)
    }

    // ---------------------------------------------------------------- selection

    pub fn select_all(&mut self) {
        self.selection_anchor = Some(0);
        self.cursor = self.chars.len();
    }

    fn has_selection(&self) -> bool {
        self.selection_anchor
            .map(|a| a != self.cursor)
            .unwrap_or(false)
    }

    fn selection_range(&self) -> Option<(usize, usize)> {
        let a = self.selection_anchor?;
        if a == self.cursor {
            return None;
        }
        Some((a.min(self.cursor), a.max(self.cursor)))
    }

    fn selected_text(&self) -> Option<String> {
        let (s, e) = self.selection_range()?;
        Some(self.chars[s..e].iter().collect())
    }

    fn delete_selection(&mut self) -> bool {
        let Some((s, e)) = self.selection_range() else {
            return false;
        };
        self.chars.drain(s..e);
        self.cursor = s;
        self.selection_anchor = None;
        true
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

    pub fn cut(&mut self) -> bool {
        if let Some(text) = self.selected_text() {
            self.clipboard_set(&text);
            self.delete_selection();
            true
        } else {
            false
        }
    }

    pub fn paste(&mut self) -> bool {
        let Some(text) = self.clipboard_get() else {
            return false;
        };
        if self.has_selection() {
            self.delete_selection();
        }
        let cleaned: String = text.chars().filter(|&c| c != '\n' && c != '\r').collect();
        if cleaned.is_empty() {
            return false;
        }
        self.insert_text(&cleaned);
        true
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

    fn insert_char(&mut self, ch: char) {
        self.chars.insert(self.cursor, ch);
        self.cursor += 1;
    }

    fn insert_text(&mut self, text: &str) {
        for ch in text.chars() {
            self.insert_char(ch);
        }
    }

    fn backspace(&mut self) -> bool {
        if self.delete_selection() {
            return true;
        }
        if self.cursor == 0 {
            return false;
        }
        self.cursor -= 1;
        self.chars.remove(self.cursor);
        true
    }

    fn delete_forward(&mut self) -> bool {
        if self.delete_selection() {
            return true;
        }
        if self.cursor >= self.chars.len() {
            return false;
        }
        self.chars.remove(self.cursor);
        true
    }

    // ---------------------------------------------------------------- movement

    fn move_left(&mut self) {
        if self.cursor > 0 {
            self.cursor -= 1;
        }
    }

    fn move_right(&mut self) {
        if self.cursor < self.chars.len() {
            self.cursor += 1;
        }
    }

    fn is_word_char(ch: char) -> bool {
        ch.is_alphanumeric() || ch == '_'
    }

    /// Skip whitespace/punct, then the current word — Ctrl+Left semantics.
    fn move_word_left(&mut self) {
        let mut i = self.cursor;
        while i > 0 && !Self::is_word_char(self.chars[i - 1]) {
            i -= 1;
        }
        while i > 0 && Self::is_word_char(self.chars[i - 1]) {
            i -= 1;
        }
        self.cursor = i;
    }

    fn move_word_right(&mut self) {
        let n = self.chars.len();
        let mut i = self.cursor;
        while i < n && Self::is_word_char(self.chars[i]) {
            i += 1;
        }
        while i < n && !Self::is_word_char(self.chars[i]) {
            i += 1;
        }
        self.cursor = i;
    }

    fn move_home(&mut self) {
        self.cursor = 0;
    }

    fn move_end(&mut self) {
        self.cursor = self.chars.len();
    }

    /// Word boundaries around a caret position, matching how a double-click
    /// expands selection — runs of word characters and runs of non-word
    /// characters are each their own "word".
    fn word_bounds_at(&self, pos: usize) -> (usize, usize) {
        if self.chars.is_empty() {
            return (0, 0);
        }
        // A caret at len has no glyph to its right; look at the one to the left.
        let target = if pos >= self.chars.len() {
            self.chars.len() - 1
        } else {
            pos
        };
        let is_word = Self::is_word_char(self.chars[target]);
        let mut start = target;
        let mut end = target + 1;
        while start > 0 && Self::is_word_char(self.chars[start - 1]) == is_word {
            start -= 1;
        }
        while end < self.chars.len() && Self::is_word_char(self.chars[end]) == is_word {
            end += 1;
        }
        (start, end)
    }

    // ---------------------------------------------------------------- hit test

    /// Map a widget-local x coordinate (in logical px) to the char index
    /// whose caret position is closest.
    fn char_index_at_x(&self, local_x: i32) -> usize {
        let target = (local_x - PADDING_X + self.scroll_x).max(0);
        let widths = &self.cumulative_widths;
        if widths.len() <= 1 {
            return 0;
        }
        let mut best = 0;
        let mut best_delta = i32::MAX;
        for (i, w) in widths.iter().enumerate() {
            let d = (*w - target).abs();
            if d < best_delta {
                best_delta = d;
                best = i;
            }
        }
        best
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

    // ---------------------------------------------------------------- layout

    fn rebuild_widths(&mut self, painter: &Painter, font_size: f32) {
        let n = self.chars.len();
        self.cumulative_widths.clear();
        self.cumulative_widths.reserve(n + 1);
        self.cumulative_widths.push(0);
        let mut prefix = String::new();
        for ch in &self.chars {
            prefix.push(*ch);
            let w = painter.measure_text(&prefix, font_size).w;
            self.cumulative_widths.push(w);
        }
    }

    fn caret_x_offset(&self) -> i32 {
        self.cumulative_widths
            .get(self.cursor)
            .copied()
            .unwrap_or(0)
    }

    fn adjust_scroll(&mut self, visible_w: i32) {
        if visible_w <= 0 {
            self.scroll_x = 0;
            return;
        }
        let total_w = self.cumulative_widths.last().copied().unwrap_or(0);
        let caret = self.caret_x_offset();
        if caret < self.scroll_x {
            self.scroll_x = caret;
        }
        if caret > self.scroll_x + visible_w - 1 {
            self.scroll_x = caret - visible_w + 1;
        }
        // Pull back if we have empty room on the right.
        let max_scroll = (total_w - visible_w).max(0);
        if self.scroll_x > max_scroll {
            self.scroll_x = max_scroll;
        }
        if self.scroll_x < 0 {
            self.scroll_x = 0;
        }
    }

    /// Keep the caret visible while the user is actively interacting — every
    /// edit / movement restarts the blink cycle from its "on" phase.
    fn reset_blink(&mut self) {
        self.blink_since = Instant::now();
        self.blink_on = true;
    }

    fn fire_change(&mut self, ctx: &mut EventCtx) {
        if self.on_change.is_none() {
            return;
        }
        let text: String = self.chars.iter().collect();
        if let Some(handler) = self.on_change.as_mut() {
            handler(ctx, &text);
        }
    }
}

impl Widget for TextInput {
    fn bounds(&self) -> Rect {
        self.rect
    }

    fn paint(&mut self, painter: &mut Painter, theme: &Theme) {
        let font_size = self.font_size_for(theme);

        painter.fill_rect(self.rect, Color::WHITE);
        painter.sunken_bevel(self.rect, theme.highlight, theme.shadow);
        painter.stroke_rect(self.rect, theme.border);

        self.rebuild_widths(painter, font_size);
        let visible_w = (self.rect.w - PADDING_X * 2).max(0);
        self.adjust_scroll(visible_w);

        let text_x0 = self.rect.x + PADDING_X - self.scroll_x;
        let text_h = painter.measure_text("Ag", font_size).h;
        let text_y = self.rect.y + ((self.rect.h - text_h) / 2).max(0);

        let inner = self.rect.inset(2);
        let selection = self.selection_range();

        // Clip text, selection band, and caret to the inner area so glyphs
        // don't leak past the field's chrome when the content is wider than
        // the field (horizontal scroll) or the caret is right at an edge.
        let saved_clip = painter.push_clip(inner);

        // An unfocused field still draws its selection so the user can see
        // what's selected when keyboard focus is elsewhere — but in the
        // muted "inactive" palette (black-on-gray) rather than the active
        // navy-on-white, matching CUA convention.
        let (sel_bg, sel_text) = if self.focused {
            (theme.highlight_bg, theme.highlight_text)
        } else {
            (theme.face, theme.text)
        };

        if let Some((s, e)) = selection {
            let sx = self.cumulative_widths.get(s).copied().unwrap_or(0);
            let ex = self.cumulative_widths.get(e).copied().unwrap_or(0);
            let x0 = text_x0 + sx;
            let x1 = text_x0 + ex;
            if x1 > x0 {
                painter.fill_rect(Rect::new(x0, inner.y, x1 - x0, inner.h), sel_bg);
            }
        }

        if !self.chars.is_empty() {
            if let Some((s, e)) = selection {
                let before: String = self.chars[..s].iter().collect();
                let middle: String = self.chars[s..e].iter().collect();
                let after: String = self.chars[e..].iter().collect();
                let middle_x =
                    text_x0 + self.cumulative_widths.get(s).copied().unwrap_or(0);
                let after_x =
                    text_x0 + self.cumulative_widths.get(e).copied().unwrap_or(0);
                painter.text(text_x0, text_y, &before, font_size, theme.text);
                painter.text(middle_x, text_y, &middle, font_size, sel_text);
                painter.text(after_x, text_y, &after, font_size, theme.text);
            } else {
                let text: String = self.chars.iter().collect();
                painter.text(text_x0, text_y, &text, font_size, theme.text);
            }
        }

        let cx = text_x0 + self.caret_x_offset();
        if self.focused {
            if self.blink_on {
                painter.v_line(cx, inner.y, inner.h, theme.text);
            }
        } else {
            // Unfocused: a small wedge at the bottom of the line marks
            // where the caret would land if the user clicked back in.
            draw_unfocused_caret(painter, cx, inner.bottom() - 2, theme.text);
        }

        painter.restore_clip(saved_clip);
    }

    fn event(&mut self, event: &Event, ctx: &mut EventCtx) {
        match event {
            Event::PointerDown {
                pos,
                button: MouseButton::Left,
            } => {
                if !self.rect.contains(*pos) {
                    return;
                }
                ctx.request_focus();
                let clicks = self.register_click(*pos);
                let local_x = pos.x - self.rect.x;
                let idx = self.char_index_at_x(local_x);
                match clicks {
                    1 => {
                        self.cursor = idx;
                        self.selection_anchor = Some(idx);
                        self.drag_active = true;
                    }
                    2 => {
                        let (s, e) = self.word_bounds_at(idx);
                        self.selection_anchor = Some(s);
                        self.cursor = e;
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
                    let local_x = pos.x - self.rect.x;
                    self.cursor = self.char_index_at_x(local_x);
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
                if *ch >= ' ' {
                    if self.has_selection() {
                        self.delete_selection();
                    }
                    self.insert_char(*ch);
                    self.fire_change(ctx);
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
                            if self.cut() {
                                self.fire_change(ctx);
                            }
                            true
                        }
                        'v' => {
                            if self.paste() {
                                self.fire_change(ctx);
                            }
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
                    Key::Named(NamedKey::Backspace) => {
                        if self.backspace() {
                            self.fire_change(ctx);
                        }
                        self.reset_blink();
                        ctx.request_paint();
                    }
                    Key::Named(NamedKey::Delete) => {
                        if self.delete_forward() {
                            self.fire_change(ctx);
                        }
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
    }
}

/// Small upward wedge: a one-pixel apex with a three-pixel base, with `cx`
/// as the horizontal center and `top_y` the row that holds the apex.
fn draw_unfocused_caret(painter: &mut Painter, cx: i32, top_y: i32, color: Color) {
    painter.pixel(cx, top_y, color);
    painter.pixel(cx - 1, top_y + 1, color);
    painter.pixel(cx, top_y + 1, color);
    painter.pixel(cx + 1, top_y + 1, color);
}
