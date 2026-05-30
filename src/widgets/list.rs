use std::time::{Duration, Instant};

use crate::event::{Event, EventCtx, Key, MouseButton, NamedKey};
use crate::geometry::{Color, Point, Rect};
use crate::painter::Painter;
use crate::theme::Theme;
use crate::widget::Widget;
use crate::widgets::scrollbar::{SCROLLBAR_THICKNESS, ScrollBar};

const ROW_HEIGHT: i32 = 18;
const ICON_SIZE: i32 = 16;
const ICON_PAD: i32 = 4;
const TEXT_PAD_X: i32 = 4;
const TEXT_PAD_Y: i32 = 2;
const DOUBLE_CLICK_MS: u64 = 400;

/// A small ARGB32 pixel buffer drawn next to a list item's label. Pixels with
/// `alpha == 0` are skipped (transparent), so icons keep their outline crisp
/// against the row's selection color.
#[derive(Clone)]
pub struct ListIcon {
    pub width: i32,
    pub height: i32,
    pub pixels: Vec<u32>,
}

impl ListIcon {
    pub fn new(width: i32, height: i32) -> Self {
        let len = (width.max(0) * height.max(0)) as usize;
        Self {
            width,
            height,
            pixels: vec![0; len],
        }
    }

    pub fn from_pixels(width: i32, height: i32, pixels: Vec<u32>) -> Self {
        debug_assert_eq!(pixels.len(), (width * height) as usize);
        Self {
            width,
            height,
            pixels,
        }
    }

    pub fn set_pixel(&mut self, px: i32, py: i32, color: Color) {
        if px < 0 || py < 0 || px >= self.width || py >= self.height {
            return;
        }
        self.pixels[(py * self.width + px) as usize] = color.0;
    }

    pub fn fill_rect(&mut self, rect: Rect, color: Color) {
        let x0 = rect.x.max(0);
        let y0 = rect.y.max(0);
        let x1 = rect.right().min(self.width);
        let y1 = rect.bottom().min(self.height);
        for y in y0..y1 {
            let row = (y * self.width) as usize;
            for x in x0..x1 {
                self.pixels[row + x as usize] = color.0;
            }
        }
    }
}

/// A single entry inside a [`List`]: a text label and an optional icon shown to
/// its left.
pub struct ListItem {
    pub label: String,
    pub icon: Option<ListIcon>,
}

impl ListItem {
    pub fn new(label: impl Into<String>) -> Self {
        Self {
            label: label.into(),
            icon: None,
        }
    }

    pub fn with_icon(mut self, icon: ListIcon) -> Self {
        self.icon = Some(icon);
        self
    }
}

/// A vertically-scrolling list of labeled rows with optional icons.
///
/// Single-click selects a row; double-click on the same row fires an
/// activation that consumers can pick up via [`List::take_activated`].
/// Keyboard navigation mirrors the mouse: Up/Down/Home/End/PageUp/PageDown
/// move the selection, Enter activates the current row.
///
/// The list paints a sunken white field with a 1-px black border and a
/// built-in vertical scrollbar pinned to the right edge — identical chrome to
/// [`TextEditor`](crate::widgets::TextEditor).
pub struct List {
    rect: Rect,
    items: Vec<ListItem>,
    selected: Option<usize>,
    focused: bool,
    v_scrollbar: ScrollBar,
    activated: Option<usize>,
    last_click: Option<(usize, Instant)>,
}

impl List {
    pub fn new(rect: Rect) -> Self {
        Self {
            rect,
            items: Vec::new(),
            selected: None,
            focused: false,
            v_scrollbar: ScrollBar::vertical(Rect::new(0, 0, 0, 0)),
            activated: None,
            last_click: None,
        }
    }

    pub fn with_items(mut self, items: Vec<ListItem>) -> Self {
        self.set_items(items);
        self
    }

    /// Replace every row. Resets the scroll position and clears any pending
    /// activation; if the previous selection no longer points at a valid row
    /// it is cleared (otherwise it is preserved by index).
    pub fn set_items(&mut self, items: Vec<ListItem>) {
        self.items = items;
        if let Some(idx) = self.selected
            && idx >= self.items.len()
        {
            self.selected = None;
        }
        self.activated = None;
        self.last_click = None;
        self.v_scrollbar.set_value(0);
    }

    pub fn items(&self) -> &[ListItem] {
        &self.items
    }

    pub fn selected_index(&self) -> Option<usize> {
        self.selected
    }

    pub fn set_selected(&mut self, idx: Option<usize>) {
        self.selected = idx.filter(|&i| i < self.items.len());
        self.ensure_selection_visible();
    }

    /// Consume and return the most recent activation (double-click or Enter).
    /// Wrapper widgets that drive a List call this from their `event` handler
    /// after delegating to `List::event` to discover when the user "opened"
    /// a row.
    pub fn take_activated(&mut self) -> Option<usize> {
        self.activated.take()
    }

    fn text_area(&self) -> Rect {
        let sb_w = if self.v_scrollbar.rect().w > 0 {
            SCROLLBAR_THICKNESS
        } else {
            0
        };
        Rect::new(self.rect.x, self.rect.y, (self.rect.w - sb_w).max(0), self.rect.h)
    }

    fn visible_rows(&self) -> i32 {
        ((self.text_area().h - TEXT_PAD_Y * 2) / ROW_HEIGHT).max(1)
    }

    fn scroll_top(&self) -> usize {
        self.v_scrollbar.value().max(0) as usize
    }

    fn set_scroll_top(&mut self, top: usize) {
        self.v_scrollbar.set_value(top as i32);
    }

    fn sync_scrollbar(&mut self) {
        let visible = self.visible_rows();
        let max_scroll = (self.items.len() as i32 - visible).max(0);
        self.v_scrollbar.set_range(visible, max_scroll);
    }

    fn ensure_selection_visible(&mut self) {
        self.sync_scrollbar();
        let Some(idx) = self.selected else { return };
        let visible = self.visible_rows() as usize;
        let mut top = self.scroll_top();
        if idx < top {
            top = idx;
        } else if idx >= top + visible {
            top = idx + 1 - visible;
        }
        self.set_scroll_top(top);
    }

    /// Map a logical-coordinate point inside the text area to a row index, if
    /// the point hits an actual item.
    fn row_at(&self, pos: Point) -> Option<usize> {
        let text = self.text_area();
        if !text.contains(pos) {
            return None;
        }
        let local_y = pos.y - text.y - TEXT_PAD_Y;
        if local_y < 0 {
            return None;
        }
        let row_offset = (local_y / ROW_HEIGHT) as usize;
        let row = self.scroll_top() + row_offset;
        if row < self.items.len() { Some(row) } else { None }
    }

    fn select_and_show(&mut self, idx: usize) {
        self.selected = Some(idx);
        self.ensure_selection_visible();
    }

    fn move_selection(&mut self, delta: i32) {
        if self.items.is_empty() {
            return;
        }
        let cur = self.selected.unwrap_or(0) as i32;
        let next = (cur + delta).clamp(0, self.items.len() as i32 - 1);
        self.select_and_show(next as usize);
    }

    fn move_page(&mut self, delta_pages: i32) {
        if self.items.is_empty() {
            return;
        }
        let visible = self.visible_rows();
        let step = (visible - 1).max(1);
        self.move_selection(delta_pages * step);
    }

    fn activate_selected(&mut self) {
        if let Some(idx) = self.selected {
            self.activated = Some(idx);
        }
    }

    fn handle_click(&mut self, idx: usize) {
        let now = Instant::now();
        let threshold = Duration::from_millis(DOUBLE_CLICK_MS);
        let double = self
            .last_click
            .map(|(prev_idx, prev_time)| prev_idx == idx && now.duration_since(prev_time) <= threshold)
            .unwrap_or(false);
        self.select_and_show(idx);
        if double {
            self.activated = Some(idx);
            self.last_click = None;
        } else {
            self.last_click = Some((idx, now));
        }
    }
}

impl Widget for List {
    fn bounds(&self) -> Rect {
        self.rect
    }

    fn paint(&mut self, painter: &mut Painter, theme: &Theme) {
        self.sync_scrollbar();
        let text = self.text_area();

        painter.fill_rect(text, Color::WHITE);
        painter.sunken_bevel(text, theme.highlight, theme.shadow);
        painter.stroke_rect(text, theme.border);

        let text_x = text.x + TEXT_PAD_X;
        let text_y0 = text.y + TEXT_PAD_Y;
        let row_w = text.w - TEXT_PAD_X * 2;
        let visible = self.visible_rows() as usize;
        let scroll_top = self.scroll_top();

        for row_offset in 0..visible {
            let row = scroll_top + row_offset;
            if row >= self.items.len() {
                break;
            }
            let y = text_y0 + row_offset as i32 * ROW_HEIGHT;
            let selected = self.selected == Some(row);
            // Active focus → navy/white; inactive (focus elsewhere) → muted
            // gray/black, matching the CUA convention so the user can still
            // see what's picked without the row competing for attention.
            let (text_color, bg_color) = if self.focused {
                (theme.highlight_text, theme.highlight_bg)
            } else {
                (theme.text, theme.face)
            };
            let text_color = if selected { text_color } else { theme.text };
            if selected {
                painter.fill_rect(
                    Rect::new(text_x, y, row_w.max(0), ROW_HEIGHT),
                    bg_color,
                );
            }

            let item = &self.items[row];
            let mut label_x = text_x + 2;
            if let Some(icon) = &item.icon {
                let icon_y = y + (ROW_HEIGHT - icon.height) / 2;
                draw_icon(painter, icon, label_x, icon_y);
                label_x += ICON_SIZE + ICON_PAD;
            } else {
                label_x += ICON_SIZE + ICON_PAD;
            }
            let label_y = y + (ROW_HEIGHT - theme.font_size as i32) / 2 - 1;
            painter.text(label_x, label_y, &item.label, theme.font_size, text_color);
        }

        if self.focused
            && let Some(idx) = self.selected
            && idx >= scroll_top
            && idx < scroll_top + visible
        {
            let y = text_y0 + (idx - scroll_top) as i32 * ROW_HEIGHT;
            draw_focus_rect(
                painter,
                Rect::new(text_x, y, row_w.max(0), ROW_HEIGHT),
                theme.text,
            );
        }

        self.v_scrollbar.paint(painter, theme);
    }

    fn event(&mut self, event: &Event, ctx: &mut EventCtx) {
        if self.v_scrollbar.captures_pointer() {
            self.v_scrollbar.event(event, ctx);
            return;
        }
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
                if let Some(row) = self.row_at(*pos) {
                    self.handle_click(row);
                }
                ctx.request_paint();
            }
            Event::KeyDown { key, modifiers } if self.focused && !modifiers.has_command() => {
                let consumed = match key {
                    Key::Named(NamedKey::Up) => {
                        self.move_selection(-1);
                        true
                    }
                    Key::Named(NamedKey::Down) => {
                        self.move_selection(1);
                        true
                    }
                    Key::Named(NamedKey::Home) => {
                        if !self.items.is_empty() {
                            self.select_and_show(0);
                        }
                        true
                    }
                    Key::Named(NamedKey::End) => {
                        if let Some(last) = self.items.len().checked_sub(1) {
                            self.select_and_show(last);
                        }
                        true
                    }
                    Key::Named(NamedKey::PageUp) => {
                        self.move_page(-1);
                        true
                    }
                    Key::Named(NamedKey::PageDown) => {
                        self.move_page(1);
                        true
                    }
                    Key::Named(NamedKey::Enter) => {
                        self.activate_selected();
                        true
                    }
                    _ => false,
                };
                if consumed {
                    ctx.request_paint();
                }
            }
            _ => {}
        }
    }

    fn captures_pointer(&self) -> bool {
        self.v_scrollbar.captures_pointer()
    }

    fn focusable(&self) -> bool {
        true
    }

    fn set_focused(&mut self, focused: bool) {
        self.focused = focused;
    }

    fn layout(&mut self, bounds: Rect) {
        self.rect = bounds;
        let sb_rect = Rect::new(
            bounds.right() - SCROLLBAR_THICKNESS,
            bounds.y,
            SCROLLBAR_THICKNESS,
            bounds.h,
        );
        self.v_scrollbar.set_rect(sb_rect);
        self.ensure_selection_visible();
    }
}

/// Blit a [`ListIcon`] into the painter at logical (x, y). Mirrors
/// [`Image`](crate::widgets::Image)'s paint path but at an arbitrary
/// destination, which is what list rows need.
fn draw_icon(painter: &mut Painter, icon: &ListIcon, x: i32, y: i32) {
    for py in 0..icon.height {
        for px in 0..icon.width {
            let color = Color(icon.pixels[(py * icon.width + px) as usize]);
            if color.alpha() == 0 {
                continue;
            }
            painter.pixel(x + px, y + py, color);
        }
    }
}

/// 1-px dotted rectangle around the focused row — the same chrome the Win 3.1
/// list-box used to mark its caret separately from the navy selection band.
fn draw_focus_rect(painter: &mut Painter, rect: Rect, color: Color) {
    if rect.w <= 0 || rect.h <= 0 {
        return;
    }
    let right = rect.right() - 1;
    let bottom = rect.bottom() - 1;
    let mut x = rect.x;
    while x <= right {
        painter.pixel(x, rect.y, color);
        painter.pixel(x, bottom, color);
        x += 2;
    }
    let mut y = rect.y;
    while y <= bottom {
        painter.pixel(rect.x, y, color);
        painter.pixel(right, y, color);
        y += 2;
    }
}
