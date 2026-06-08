use crate::event::{Cursor, Event, EventCtx, Key, MouseButton, NamedKey};
use crate::geometry::{Color, Point, Rect};
use crate::include_svg;
use crate::painter::Painter;
use crate::svg::SvgImage;
use crate::theme::Theme;
use crate::widget::{PopupKind, PopupRequest, Widget};
use crate::widgets::scrollbar::{SCROLLBAR_THICKNESS, ScrollBar};

type ChangeHandler = Box<dyn FnMut(&mut EventCtx, usize)>;

/// Height of one row inside the open popup list.
const ITEM_HEIGHT: i32 = 18;
/// Vertical breathing room above the first and below the last popup row.
const POPUP_PAD_Y: i32 = 2;
/// Left inset for text in both the closed field and the popup rows.
const TEXT_PAD_X: i32 = 5;
/// L-shape drop-shadow size, mirroring [`MenuBar`](crate::widgets::MenuBar).
const SHADOW_SIZE: i32 = 2;
/// L-shape drop shadow color: a dark gray that renders crisply on every
/// backend (same value the menu popups use).
const SHADOW_COLOR: Color = Color::DARK_GRAY;
/// Most rows the open popup shows before it scrolls. Beyond this the list caps
/// its height and grows a vertical scrollbar instead of a popup taller than the
/// screen — so a 100-layout list stays usable.
const MAX_POPUP_ITEMS: usize = 12;

/// The drop-arrow glyph, baked from SVG at compile time: the classic combobox
/// down-triangle sitting above a short horizontal bar. Its 16-unit viewBox maps
/// onto the arrow button so it lands centered, and `SvgImage::draw_tinted`
/// re-snaps it crisply at every scale. The baked black is only a placeholder —
/// it is drawn tinted with the theme's text (or disabled-text) color.
const ARROW: SvgImage = include_svg!("assets/dropdown/down.svg");

/// A classic Win 3.1 drop-down list box (combobox).
///
/// Closed, it reads as a sunken white field showing the current selection with
/// a raised drop-arrow button on the right. Clicking the field opens a popup
/// list of the items — hosted in its own borderless top-level window (via
/// [`PopupRequest`]) exactly like [`MenuBar`](crate::widgets::MenuBar)'s
/// dropdowns, so the list can extend past the main window's bottom edge.
///
/// The widget owns its selection. Pick an item with the mouse, or — while
/// focused — navigate with the keyboard (see the table in the crate README).
/// An optional [`on_change`](Self::on_change) handler fires whenever the
/// selected index changes, which is what the 7GUIs flight booker hooks to
/// enable / disable its return-date field.
pub struct Dropdown {
    rect: Rect,
    items: Vec<String>,
    selected: Option<usize>,
    /// Highlighted row while the list is open — tracks the mouse hover and the
    /// keyboard cursor. `None` while closed.
    highlighted: Option<usize>,
    open: bool,
    focused: bool,
    enabled: bool,
    on_change: Option<ChangeHandler>,
    /// Vertical scrollbar for the open popup. Only shown (and only consulted)
    /// once the list is longer than [`MAX_POPUP_ITEMS`]; its `value` is the
    /// index of the first visible row.
    scrollbar: ScrollBar,
}

impl Dropdown {
    pub fn new(rect: Rect) -> Self {
        Self {
            rect,
            items: Vec::new(),
            selected: None,
            highlighted: None,
            open: false,
            focused: false,
            enabled: true,
            on_change: None,
            scrollbar: ScrollBar::vertical(Rect::new(0, 0, 0, 0)),
        }
    }

    /// Populate the list. Accepts anything that iterates into strings, so both
    /// `["a", "b"]` and a `Vec<String>` work. The first item becomes the
    /// initial selection.
    pub fn with_items<I, S>(mut self, items: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.set_items(items.into_iter().map(Into::into).collect());
        self
    }

    /// Replace every item. The current selection is kept if it still points at
    /// a valid row; otherwise it falls back to the first item (or `None` when
    /// the list is now empty). Closes the popup.
    pub fn set_items(&mut self, items: Vec<String>) {
        self.items = items;
        self.selected = match self.selected {
            Some(i) if i < self.items.len() => Some(i),
            _ if self.items.is_empty() => None,
            _ => Some(0),
        };
        self.highlighted = None;
        self.open = false;
        self.scrollbar.set_value(0);
    }

    pub fn with_selected(mut self, idx: usize) -> Self {
        self.set_selected(Some(idx));
        self
    }

    pub fn with_enabled(mut self, enabled: bool) -> Self {
        self.set_enabled(enabled);
        self
    }

    pub fn on_change<F>(mut self, handler: F) -> Self
    where
        F: FnMut(&mut EventCtx, usize) + 'static,
    {
        self.on_change = Some(Box::new(handler));
        self
    }

    /// Install (or replace) the change handler after construction. Mirrors
    /// [`on_change`](Self::on_change) for callers that hold the dropdown behind
    /// an `Rc<RefCell<…>>` and need to wire the callback up once its peers
    /// exist.
    pub fn set_on_change<F>(&mut self, handler: F)
    where
        F: FnMut(&mut EventCtx, usize) + 'static,
    {
        self.on_change = Some(Box::new(handler));
    }

    pub fn items(&self) -> &[String] {
        &self.items
    }

    pub fn selected_index(&self) -> Option<usize> {
        self.selected
    }

    pub fn selected_text(&self) -> Option<&str> {
        self.selected
            .and_then(|i| self.items.get(i))
            .map(String::as_str)
    }

    /// Set the selection by index (clamped to a valid row, or cleared with
    /// `None`). Does **not** fire `on_change` — it mirrors the convention of
    /// the other widgets' setters so programmatic updates can't loop back
    /// through a handler.
    pub fn set_selected(&mut self, idx: Option<usize>) {
        self.selected = idx.filter(|&i| i < self.items.len());
    }

    pub fn is_enabled(&self) -> bool {
        self.enabled
    }

    /// Enable or disable the control. A disabled dropdown paints greyed, drops
    /// keyboard focus eligibility, ignores input, and closes any open popup.
    pub fn set_enabled(&mut self, enabled: bool) {
        self.enabled = enabled;
        if !enabled {
            self.close();
        }
    }

    pub fn is_open(&self) -> bool {
        self.open
    }

    /// Programmatically drop the list open — handy for tests and for wiring
    /// custom application-level keybindings. No-op when the list is empty.
    pub fn open(&mut self) {
        self.open_list();
    }

    pub fn rect(&self) -> Rect {
        self.rect
    }

    pub fn set_rect(&mut self, rect: Rect) {
        self.rect = rect;
    }

    /// The raised drop-arrow button on the right edge. Drawn flush with the
    /// field's outer frame: its top, right and bottom edges land *on* the 1px
    /// black border, so the button's own outline becomes those three sides of
    /// the overall dropdown frame (only its left edge is an interior divider).
    /// The frame's sharp corners show through the button's rounded ones.
    ///
    /// Kept square — its side tracks the field's full height — so the button
    /// reads as a proper combobox button and the baked glyph is never
    /// stretched, whatever height the field is laid out at.
    fn arrow_rect(&self) -> Rect {
        let size = self.rect.h.max(0);
        Rect::new(self.rect.right() - size, self.rect.y, size, size)
    }

    /// The area the selected label is drawn in — everything left of the arrow
    /// button, inset by the sunken border.
    fn text_area(&self) -> Rect {
        let inner = self.rect.inset(2);
        let w = (inner.w - self.arrow_rect().w).max(0);
        Rect::new(inner.x, inner.y, w, inner.h)
    }

    /// Rows the open popup shows at once — every item, capped at
    /// [`MAX_POPUP_ITEMS`].
    fn visible_items(&self) -> usize {
        self.items.len().min(MAX_POPUP_ITEMS)
    }

    /// `true` once the list is too long to show all at once, so the popup grows
    /// a scrollbar.
    fn scrollable(&self) -> bool {
        self.items.len() > MAX_POPUP_ITEMS
    }

    /// Index of the first visible row (0 unless scrolled).
    fn scroll_top(&self) -> usize {
        self.scrollbar.value().max(0) as usize
    }

    /// Logical-coordinate rect of the open popup list (without its shadow), in
    /// the root widget's coordinate space — flush below the field. Capped to
    /// [`MAX_POPUP_ITEMS`] rows tall.
    fn popup_rect(&self) -> Rect {
        let h = POPUP_PAD_Y * 2 + self.visible_items() as i32 * ITEM_HEIGHT;
        Rect::new(self.rect.x, self.rect.bottom(), self.rect.w, h)
    }

    /// The popup area that holds the item rows — the whole popup minus the
    /// scrollbar gutter when one is shown.
    fn popup_rows_rect(&self) -> Rect {
        let p = self.popup_rect();
        let gutter = if self.scrollable() {
            SCROLLBAR_THICKNESS
        } else {
            0
        };
        Rect::new(p.x, p.y, (p.w - gutter).max(0), p.h)
    }

    /// The scrollbar's gutter rect inside the popup's right edge (inside the
    /// 1-px border).
    fn popup_scrollbar_rect(&self) -> Rect {
        let p = self.popup_rect();
        Rect::new(
            p.right() - 1 - SCROLLBAR_THICKNESS,
            p.y + 1,
            SCROLLBAR_THICKNESS,
            (p.h - 2).max(0),
        )
    }

    /// Tell the scrollbar how big its window and travel are. The popup has no
    /// layout pass of its own, so this is called lazily before painting /
    /// handling events while open.
    fn sync_scrollbar(&mut self) {
        let visible = self.visible_items() as i32;
        let max = (self.items.len() as i32 - visible).max(0);
        self.scrollbar.set_range(visible, max);
    }

    /// Position the scrollbar and refresh its range for the current popup.
    fn layout_popup(&mut self) {
        self.scrollbar.set_rect(self.popup_scrollbar_rect());
        self.sync_scrollbar();
    }

    /// Scroll so row `idx` is within the visible window.
    fn ensure_visible(&mut self, idx: usize) {
        self.sync_scrollbar();
        let visible = self.visible_items();
        let mut top = self.scroll_top();
        if idx < top {
            top = idx;
        } else if idx >= top + visible {
            top = idx + 1 - visible;
        }
        self.scrollbar.set_value(top as i32);
    }

    /// Map a pointer position to the popup row under it, if the list is open
    /// and the point lands on an actual item (not the scrollbar gutter).
    fn hit_item(&self, pos: Point) -> Option<usize> {
        if !self.open {
            return None;
        }
        let rows = self.popup_rows_rect();
        if !rows.contains(pos) {
            return None;
        }
        let local = pos.y - (rows.y + POPUP_PAD_Y);
        if local < 0 {
            return None;
        }
        let row_offset = (local / ITEM_HEIGHT) as usize;
        if row_offset >= self.visible_items() {
            return None;
        }
        let idx = self.scroll_top() + row_offset;
        (idx < self.items.len()).then_some(idx)
    }

    fn open_list(&mut self) {
        if self.items.is_empty() {
            return;
        }
        self.open = true;
        // Pre-highlight the current selection so Enter / arrows have a starting
        // point and the user sees what's picked — scrolled into view.
        let start = self.selected.unwrap_or(0);
        self.highlighted = Some(start);
        self.ensure_visible(start);
    }

    fn close(&mut self) {
        self.open = false;
        self.highlighted = None;
        // Drop any thumb-drag state so reopening doesn't grab the thumb.
        self.scrollbar.end_drag();
    }

    /// Commit `idx` as the new selection, firing `on_change` only when the
    /// value actually changes.
    fn commit(&mut self, idx: usize, ctx: &mut EventCtx) {
        if idx >= self.items.len() {
            return;
        }
        let changed = self.selected != Some(idx);
        self.selected = Some(idx);
        if changed && let Some(handler) = self.on_change.as_mut() {
            handler(ctx, idx);
        }
    }

    /// Step the open-list highlight by `delta`, clamped to the ends (Win 3.1
    /// combos don't wrap), scrolling the new highlight into view.
    fn move_highlight(&mut self, delta: i32) {
        if self.items.is_empty() {
            return;
        }
        let n = self.items.len() as i32;
        let cur = self.highlighted.or(self.selected).unwrap_or(0) as i32;
        let next = (cur + delta).clamp(0, n - 1) as usize;
        self.highlighted = Some(next);
        self.ensure_visible(next);
    }

    fn handle_key(&mut self, key: &Key, ctx: &mut EventCtx) {
        if self.open {
            // One keyboard page steps a near-full window (mirrors List).
            let page = (self.visible_items() as i32 - 1).max(1);
            match key {
                Key::Named(NamedKey::Up) => self.move_highlight(-1),
                Key::Named(NamedKey::Down) => self.move_highlight(1),
                Key::Named(NamedKey::PageUp) => self.move_highlight(-page),
                Key::Named(NamedKey::PageDown) => self.move_highlight(page),
                Key::Named(NamedKey::Home) => {
                    self.highlighted = Some(0);
                    self.ensure_visible(0);
                }
                Key::Named(NamedKey::End) => {
                    if let Some(last) = self.items.len().checked_sub(1) {
                        self.highlighted = Some(last);
                        self.ensure_visible(last);
                    }
                }
                Key::Named(NamedKey::Enter) | Key::Named(NamedKey::Space) => {
                    if let Some(idx) = self.highlighted {
                        self.commit(idx, ctx);
                    }
                    self.close();
                }
                Key::Named(NamedKey::Escape) => self.close(),
                _ => return,
            }
        } else {
            // Closed: arrow keys change the selection in place (classic combo
            // behavior), Space drops the list open. Enter is deliberately left
            // alone so a sibling default button keeps its accelerator.
            match key {
                Key::Named(NamedKey::Up) => {
                    let target = self.selected.unwrap_or(0).saturating_sub(1);
                    self.commit(target, ctx);
                }
                Key::Named(NamedKey::Down) => {
                    let target = self.selected.map(|i| i + 1).unwrap_or(0);
                    self.commit(target.min(self.items.len().saturating_sub(1)), ctx);
                }
                Key::Named(NamedKey::Home) => self.commit(0, ctx),
                Key::Named(NamedKey::End) => {
                    if let Some(last) = self.items.len().checked_sub(1) {
                        self.commit(last, ctx);
                    }
                }
                Key::Named(NamedKey::Space) => self.open_list(),
                _ => return,
            }
        }
        ctx.request_paint();
        ctx.consume_event();
    }
}

impl Widget for Dropdown {
    fn bounds(&self) -> Rect {
        self.rect
    }

    fn paint(&mut self, painter: &mut Painter, theme: &Theme) {
        // Sunken field — white when live, light-gray when disabled — with a
        // 1-px black outer border, matching TextInput's chrome.
        let bg = if self.enabled {
            Color::WHITE
        } else {
            theme.face
        };
        let btn = self.arrow_rect();
        let arrow_color = if self.enabled {
            theme.text
        } else {
            theme.disabled_text
        };
        // Field chrome — sunken field + outer border + raised arrow button —
        // self-manages the crisp physical-pixel pass at fractional scales, and
        // `draw_tinted` does the same for the baked-SVG glyph on top.
        painter.fill_rect(self.rect, bg);
        painter.sunken_bevel(self.rect, theme.highlight, theme.shadow);
        painter.stroke_rect(self.rect, theme.border);
        painter.button(btn, theme, self.open, false);
        ARROW.draw_tinted(painter, btn, arrow_color);

        // Selected label, clipped to the area left of the button. Text
        // always renders at the actual scale so glyphs stay legible.
        if let Some(text) = self.selected_text() {
            let area = self.text_area();
            let saved = painter.push_clip(area);
            let th = painter.measure_text(text, theme.font_size).h;
            let ty = self.rect.y + ((self.rect.h - th) / 2).max(0);
            let fg = if self.enabled {
                theme.text
            } else {
                theme.disabled_text
            };
            painter.text(area.x + TEXT_PAD_X, ty, text, theme.font_size, fg);
            painter.restore_clip(saved);
        }

        // Dotted focus rectangle inside the text area, Win 3.1-style.
        if self.focused && self.enabled {
            painter.focus_rect(self.text_area().inset(1), theme.text);
        }
    }

    fn paint_overlay(&mut self, painter: &mut Painter, theme: &Theme) {
        // The list lives in a separate top-level popup window — only draw it
        // when the painter is running *that* popup pass, so neither the main
        // window nor any other popup in the stack (e.g. a dialog hosting this
        // dropdown) ends up with a duplicate copy in its framebuffer.
        if !self.open {
            return;
        }
        let Some(req) = self.popup_request() else {
            return;
        };
        if painter.popup_anchor() != Some(req.rect) {
            return;
        }
        self.layout_popup();
        let popup = self.popup_rect();

        // L-shape drop shadow first, then the white panel overlays its top /
        // left edges.
        painter.fill_rect(
            Rect::new(popup.x + SHADOW_SIZE, popup.bottom(), popup.w, SHADOW_SIZE),
            SHADOW_COLOR,
        );
        painter.fill_rect(
            Rect::new(popup.right(), popup.y + SHADOW_SIZE, SHADOW_SIZE, popup.h),
            SHADOW_COLOR,
        );

        painter.fill_rect(popup, theme.background);
        painter.stroke_rect(popup, theme.border);

        // Only the visible window of rows, offset by the scroll position.
        let row_w = (self.popup_rows_rect().w - 2).max(0);
        let top = self.scroll_top();
        let mut y = popup.y + POPUP_PAD_Y;
        for row_offset in 0..self.visible_items() {
            let i = top + row_offset;
            let Some(item) = self.items.get(i) else { break };
            let row = Rect::new(popup.x + 1, y, row_w, ITEM_HEIGHT);
            let (bg, fg) = if self.highlighted == Some(i) {
                (theme.highlight_bg, theme.highlight_text)
            } else {
                (theme.background, theme.text)
            };
            painter.fill_rect(row, bg);
            let th = painter.measure_text(item, theme.font_size).h;
            let ty = row.y + ((row.h - th) / 2).max(0);
            painter.text(row.x + TEXT_PAD_X, ty, item, theme.font_size, fg);
            y += ITEM_HEIGHT;
        }

        if self.scrollable() {
            self.scrollbar.paint(painter, theme);
        }
    }

    fn event(&mut self, event: &Event, ctx: &mut EventCtx) {
        if !self.enabled {
            return;
        }
        // While the popup is open its scrollbar gets first refusal on drags,
        // the wheel, and gutter clicks — before the click-to-pick / dismiss
        // logic below would otherwise fold the list shut.
        if self.open {
            self.layout_popup();
            if self.scrollbar.captures_pointer() {
                // A thumb drag is in progress: feed it everything until release.
                self.scrollbar.event(event, ctx);
                return;
            }
            match event {
                Event::Scroll { pos, .. } if self.popup_rect().contains(*pos) => {
                    self.scrollbar.event(event, ctx);
                    return;
                }
                Event::PointerDown {
                    pos,
                    button: MouseButton::Left,
                    ..
                } if self.scrollable() && self.popup_scrollbar_rect().contains(*pos) => {
                    self.scrollbar.event(event, ctx);
                    ctx.request_paint();
                    return;
                }
                // Hovering the popup's scrollbar gutter: let the bar pick its
                // own pointer shape (hand over its arrows, grab over its thumb)
                // rather than falling through to the row-highlight logic.
                Event::PointerMove { pos }
                    if self.scrollable() && self.popup_scrollbar_rect().contains(*pos) =>
                {
                    self.scrollbar.event(event, ctx);
                    return;
                }
                _ => {}
            }
        }
        match event {
            Event::PointerDown {
                pos,
                button: MouseButton::Left,
                ..
            } => {
                if self.open {
                    // Released over a row → pick it; over the field → toggle
                    // shut; anywhere else → dismiss. The runtime routes
                    // popup-window clicks back here in root coordinates, so a
                    // single hit-test against the popup rect handles both.
                    if let Some(idx) = self.hit_item(*pos) {
                        self.commit(idx, ctx);
                    }
                    self.close();
                    ctx.request_paint();
                } else if self.rect.contains(*pos) {
                    self.open_list();
                    ctx.request_focus();
                    ctx.request_paint();
                }
            }
            Event::PointerMove { pos } if self.open => {
                // Track the hover only while the cursor is actually over a
                // row; sliding off into the field area keeps the last
                // highlight rather than clearing it.
                if let Some(hit) = self.hit_item(*pos)
                    && self.highlighted != Some(hit)
                {
                    self.highlighted = Some(hit);
                    ctx.request_paint();
                }
            }
            Event::KeyDown { key, modifiers } if self.focused && !modifiers.has_command() => {
                self.handle_key(key, ctx);
            }
            _ => {}
        }

        // Pointer shape: both the closed field (a button that drops the list
        // open) and, while open, the list rows are click targets, so they take
        // the hand. The popup's scrollbar gutter was already routed away above,
        // so it keeps its own shape.
        if let Some(pos) = event.position() {
            let clickable = if self.open {
                self.hit_item(pos).is_some()
            } else {
                self.rect.contains(pos)
            };
            if clickable {
                ctx.set_cursor(Cursor::Hand);
            }
        }
    }

    fn captures_pointer(&self) -> bool {
        // While open, grab every pointer event (popup clicks and click-outside
        // dismissals both route here) until the list closes.
        self.open
    }

    fn focusable(&self) -> bool {
        self.enabled
    }

    fn set_focused(&mut self, focused: bool) {
        self.focused = focused;
        if !focused {
            // Losing focus (Tab away, click elsewhere) folds the list up.
            self.close();
        }
    }

    fn layout(&mut self, bounds: Rect) {
        self.rect = bounds;
    }

    fn popup_request(&self) -> Option<PopupRequest> {
        if !self.open || self.items.is_empty() {
            return None;
        }
        let popup = self.popup_rect();
        // Pad the request by the shadow size so it isn't clipped at the
        // right / bottom edges of the popup window.
        Some(PopupRequest {
            rect: Rect::new(
                popup.x,
                popup.y,
                popup.w + SHADOW_SIZE,
                popup.h + SHADOW_SIZE,
            ),
            kind: PopupKind::Popup,
            title: None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::event::Modifiers;
    use crate::mock::MockBackend;

    fn dropdown_with(n: usize) -> Dropdown {
        let items: Vec<String> = (0..n).map(|i| format!("item {i}")).collect();
        Dropdown::new(Rect::new(0, 0, 120, 20)).with_items(items)
    }

    fn key(named: NamedKey) -> Event {
        Event::KeyDown {
            key: Key::Named(named),
            modifiers: Modifiers::default(),
        }
    }

    #[test]
    fn a_short_list_shows_every_row_with_no_scrollbar() {
        let d = dropdown_with(5);
        assert!(!d.scrollable());
        assert_eq!(d.visible_items(), 5);
        // No gutter: the rows span the full popup width.
        assert_eq!(d.popup_rows_rect().w, d.popup_rect().w);
        // Popup is exactly five rows tall.
        assert_eq!(d.popup_rect().h, POPUP_PAD_Y * 2 + 5 * ITEM_HEIGHT);
    }

    #[test]
    fn a_long_list_caps_its_height_and_grows_a_scrollbar() {
        let d = dropdown_with(40);
        assert!(d.scrollable());
        assert_eq!(d.visible_items(), MAX_POPUP_ITEMS);
        // Height is capped no matter how many items there are.
        assert_eq!(
            d.popup_rect().h,
            POPUP_PAD_Y * 2 + MAX_POPUP_ITEMS as i32 * ITEM_HEIGHT
        );
        // The rows give up a scrollbar gutter on the right.
        assert_eq!(
            d.popup_rows_rect().w,
            d.popup_rect().w - SCROLLBAR_THICKNESS
        );
    }

    #[test]
    fn keyboard_navigation_scrolls_the_window() {
        let mut d = dropdown_with(40);
        d.set_focused(true);
        d.open();
        assert_eq!(d.scroll_top(), 0, "opens at the top");

        let backend = MockBackend::new(200, 200);
        // Step down past the visible window; the top must follow the highlight.
        for _ in 0..MAX_POPUP_ITEMS + 3 {
            backend.dispatch(&mut d, &key(NamedKey::Down));
        }
        assert_eq!(d.highlighted, Some(MAX_POPUP_ITEMS + 3));
        assert!(d.scroll_top() > 0, "the window scrolled to follow");
        assert!(d.scroll_top() <= MAX_POPUP_ITEMS + 3);

        // End jumps to the last row and pins the window to the bottom.
        backend.dispatch(&mut d, &key(NamedKey::End));
        assert_eq!(d.highlighted, Some(39));
        assert_eq!(d.scroll_top(), 40 - MAX_POPUP_ITEMS);

        // Home returns to the very top.
        backend.dispatch(&mut d, &key(NamedKey::Home));
        assert_eq!(d.highlighted, Some(0));
        assert_eq!(d.scroll_top(), 0);
    }

    #[test]
    fn hit_item_accounts_for_the_scroll_offset() {
        let mut d = dropdown_with(40);
        d.open();
        d.layout_popup();
        d.scrollbar.set_value(5);

        let popup = d.popup_rect();
        // A click on the first visible row now lands on item 5.
        let first_row = Point::new(popup.x + 4, popup.y + POPUP_PAD_Y + 1);
        assert_eq!(d.hit_item(first_row), Some(5));
        // A click in the scrollbar gutter is not a row hit.
        let gutter = d.popup_scrollbar_rect();
        assert_eq!(
            d.hit_item(Point::new(gutter.x + 2, gutter.y + 4)),
            None,
            "the scrollbar gutter is not an item"
        );
    }

    #[test]
    fn the_closed_field_requests_the_hand() {
        // Closed, the whole field is a button that drops the list open.
        let mut d = dropdown_with(3);
        let mut ctx = EventCtx::new();
        d.event(
            &Event::PointerMove {
                pos: Point::new(10, 10),
            },
            &mut ctx,
        );
        assert_eq!(ctx.cursor_request, Some(Cursor::Hand));
    }

    #[test]
    fn an_open_list_row_requests_the_hand() {
        let mut d = dropdown_with(3);
        d.open();
        let popup = d.popup_rect();
        let pos = Point::new(popup.x + 4, popup.y + POPUP_PAD_Y + 1);
        let mut ctx = EventCtx::new();
        d.event(&Event::PointerMove { pos }, &mut ctx);
        assert_eq!(ctx.cursor_request, Some(Cursor::Hand));
    }

    #[test]
    fn opening_scrolls_the_selection_into_view() {
        let mut d = dropdown_with(40);
        d.set_selected(Some(35));
        d.open();
        d.layout_popup();
        let top = d.scroll_top();
        assert!(
            top <= 35 && 35 < top + d.visible_items(),
            "selection visible"
        );
    }
}
