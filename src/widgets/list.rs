use std::time::{Duration, Instant};

use crate::event::{Event, EventCtx, Key, Modifiers, MouseButton, NamedKey};
use crate::geometry::{Color, Point, Rect};
use crate::painter::Painter;
use crate::svg::SvgImage;
use crate::theme::Theme;
use crate::widget::Widget;
use crate::widgets::scrollbar::{SCROLLBAR_THICKNESS, ScrollBar};

const ROW_HEIGHT: i32 = 18;
const ICON_SIZE: i32 = 16;
const ICON_PAD: i32 = 4;
const TEXT_PAD_X: i32 = 4;
const TEXT_PAD_Y: i32 = 2;
const DOUBLE_CLICK_MS: u64 = 400;
/// How far (logical px) a press may wander before a pending deferred-collapse is
/// treated as the start of a drag and abandoned, leaving the whole
/// multi-selection intact. Kept below a typical wrapper's drag dead zone so the
/// group survives long enough to be dragged out.
const COLLAPSE_SLOP: i64 = 4;

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

/// The art shown to the left of a [`ListItem`]'s label.
enum IconArt {
    /// A hand-built raster pixel buffer.
    Raster(ListIcon),
    /// A compile-time-baked vector image — crisp at any DPI, drawn straight
    /// into the row. Typically produced by [`include_svg!`](crate::include_svg).
    Svg(SvgImage),
}

/// A single entry inside a [`List`]: a text label and an optional icon shown to
/// its left. The icon may be a raster [`ListIcon`] (via [`with_icon`]) or a
/// compile-time-baked [`SvgImage`] (via [`with_svg_icon`]).
///
/// [`with_icon`]: ListItem::with_icon
/// [`with_svg_icon`]: ListItem::with_svg_icon
pub struct ListItem {
    pub label: String,
    icon: Option<IconArt>,
}

impl ListItem {
    pub fn new(label: impl Into<String>) -> Self {
        Self {
            label: label.into(),
            icon: None,
        }
    }

    /// Show a raster [`ListIcon`] to the left of the label.
    pub fn with_icon(mut self, icon: ListIcon) -> Self {
        self.icon = Some(IconArt::Raster(icon));
        self
    }

    /// Show a compile-time-baked [`SvgImage`] to the left of the label — the
    /// crisp, DPI-independent choice (e.g. the folder / file marks the filer and
    /// file dialog use). [`SvgImage`] is `Copy`, so pass it by value, typically
    /// straight from [`include_svg!`](crate::include_svg).
    pub fn with_svg_icon(mut self, icon: SvgImage) -> Self {
        self.icon = Some(IconArt::Svg(icon));
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
/// With [`multi_select`](List::set_multi_select) enabled the list also accepts
/// Ctrl/Cmd+click to toggle a row and Shift+click (or Shift+Arrow) to select a
/// contiguous range; off — the default — it is single-selection and behaves
/// exactly as it always has.
///
/// The list paints a sunken white field with a 1-px black border and a
/// built-in vertical scrollbar pinned to the right edge — identical chrome to
/// [`TextEditor`](crate::widgets::TextEditor).
pub struct List {
    rect: Rect,
    items: Vec<ListItem>,
    /// Opt-in: when `false` (the default) the list is single-selection and
    /// ignores click modifiers, exactly as it behaved before multi-selection.
    multi_select: bool,
    /// Every selected row, kept sorted ascending and deduplicated. Holds 0 or 1
    /// entries while `multi_select` is off.
    selection: Vec<usize>,
    /// The cursor row: where keyboard navigation moves from and where the focus
    /// rectangle is drawn. Usually a member of `selection`.
    lead: Option<usize>,
    /// The fixed end of a Shift range-selection. Plain clicks/arrows reset it to
    /// the lead; Shift extends from it without moving it.
    anchor: Option<usize>,
    focused: bool,
    enabled: bool,
    v_scrollbar: ScrollBar,
    activated: Option<usize>,
    last_click: Option<(usize, Instant)>,
    /// A plain press on a row that is already part of a multi-selection: the
    /// selection is kept intact for a possible group drag, and `(row, press
    /// position)` is parked here. A release without travelling past
    /// [`COLLAPSE_SLOP`] collapses the selection down to `row`; enough motion
    /// (or the pointer leaving) abandons it, keeping the group. While it is set
    /// the list captures the pointer so the follow-up move/release reach it.
    pending_collapse: Option<(usize, Point)>,
}

impl List {
    pub fn new(rect: Rect) -> Self {
        Self {
            rect,
            items: Vec::new(),
            multi_select: false,
            selection: Vec::new(),
            lead: None,
            anchor: None,
            focused: false,
            enabled: true,
            v_scrollbar: ScrollBar::vertical(Rect::new(0, 0, 0, 0)),
            activated: None,
            last_click: None,
            pending_collapse: None,
        }
    }

    pub fn with_items(mut self, items: Vec<ListItem>) -> Self {
        self.set_items(items);
        self
    }

    pub fn with_enabled(mut self, enabled: bool) -> Self {
        self.set_enabled(enabled);
        self
    }

    /// Enable optional multi-selection at construction time. See
    /// [`set_multi_select`](Self::set_multi_select).
    pub fn with_multi_select(mut self, enabled: bool) -> Self {
        self.set_multi_select(enabled);
        self
    }

    pub fn is_multi_select(&self) -> bool {
        self.multi_select
    }

    /// Enable or disable multi-selection. With it on, Ctrl/Cmd+click toggles a
    /// row, Shift+click and Shift+Arrow select a contiguous range, and a plain
    /// click or arrow still selects a single row. Off (the default) the list is
    /// single-selection and click modifiers are ignored.
    ///
    /// A plain press on a row that is *already* part of a multi-selection does
    /// not collapse the selection immediately: the whole group stays selected
    /// until the button is released (collapsing to that one row) so a wrapper
    /// can start a drag of the entire group from the press instead. See
    /// [`selected_indices`](Self::selected_indices).
    ///
    /// Turning it off collapses any current multi-selection down to the single
    /// lead row (or the first selected row) so a single-selection list never
    /// shows more than one highlighted row.
    pub fn set_multi_select(&mut self, enabled: bool) {
        self.multi_select = enabled;
        if !enabled {
            self.pending_collapse = None;
            let keep = self.lead.or_else(|| self.selection.first().copied());
            self.set_selected(keep);
        }
    }

    pub fn is_enabled(&self) -> bool {
        self.enabled
    }

    /// Enable or disable the list. A disabled list paints its rows greyed with
    /// no selection band, can't take focus, and ignores mouse and keyboard
    /// input (including its scrollbar).
    pub fn set_enabled(&mut self, enabled: bool) {
        self.enabled = enabled;
    }

    /// Replace every row. Resets the scroll position and clears any pending
    /// activation; if the previous selection no longer points at a valid row
    /// it is cleared (otherwise it is preserved by index).
    pub fn set_items(&mut self, items: Vec<ListItem>) {
        self.items = items;
        let len = self.items.len();
        self.selection.retain(|&i| i < len);
        if self.lead.is_some_and(|i| i >= len) {
            self.lead = None;
        }
        if self.anchor.is_some_and(|i| i >= len) {
            self.anchor = None;
        }
        self.activated = None;
        self.last_click = None;
        self.pending_collapse = None;
        self.v_scrollbar.set_value(0);
    }

    pub fn items(&self) -> &[ListItem] {
        &self.items
    }

    /// The lead (cursor) row — the one keyboard navigation moves from and that
    /// draws the focus rectangle. For a single-selection list this is simply
    /// "the selected row"; for a multi-selection list it is the most recently
    /// touched row. Use [`selected_indices`](Self::selected_indices) for the
    /// full set.
    pub fn selected_index(&self) -> Option<usize> {
        self.lead
    }

    /// Every selected row, sorted ascending and deduplicated. Empty when nothing
    /// is selected; at most one entry for a single-selection list.
    pub fn selected_indices(&self) -> &[usize] {
        &self.selection
    }

    /// Select a single row, replacing any existing (multi-)selection, or clear
    /// the selection with `None`. The lead and range anchor both move to `idx`.
    /// Out-of-range indices clear the selection.
    pub fn set_selected(&mut self, idx: Option<usize>) {
        match idx.filter(|&i| i < self.items.len()) {
            Some(i) => self.select_single(i),
            None => self.clear_selection(),
        }
        self.ensure_selection_visible();
    }

    /// Replace the entire selection with `indices`: out-of-range entries are
    /// dropped, the rest sorted and deduplicated. The lead and anchor move to
    /// the first selected row. This is the programmatic way to select several
    /// rows at once and is honored regardless of the multi-selection flag.
    pub fn set_selected_indices(&mut self, indices: impl IntoIterator<Item = usize>) {
        let len = self.items.len();
        let mut sel: Vec<usize> = indices.into_iter().filter(|&i| i < len).collect();
        sel.sort_unstable();
        sel.dedup();
        self.lead = sel.first().copied();
        self.anchor = self.lead;
        self.selection = sel;
        self.ensure_selection_visible();
    }

    /// Clear the selection entirely: no selected rows, no lead, no anchor.
    pub fn clear_selection(&mut self) {
        self.selection.clear();
        self.lead = None;
        self.anchor = None;
    }

    /// Whether `pos` lands on the built-in scrollbar gutter rather than the row
    /// field. A wrapper that layers its own press gesture over the list (e.g.
    /// the filer's drag-out) uses this to yield to the scrollbar — which is
    /// pinned *inside* the list bounds and owns that strip — so grabbing the
    /// thumb scrolls instead of triggering the wrapper's gesture.
    pub fn scrollbar_hit(&self, pos: Point) -> bool {
        let sb = self.v_scrollbar.rect();
        sb.w > 0 && sb.contains(pos)
    }

    /// Consume and return the most recent activation (double-click or Enter).
    /// Wrapper widgets that drive a List call this from their `event` handler
    /// after delegating to `List::event` to discover when the user "opened"
    /// a row.
    pub fn take_activated(&mut self) -> Option<usize> {
        self.activated.take()
    }

    fn text_area(&self) -> Rect {
        // When the scrollbar is present the field overlaps it by 1px so the
        // field's right border lands on the scrollbar's own left-border column,
        // collapsing the divider to a single 1px line instead of stacking the
        // two 1px borders into a 2px band. The scrollbar is painted last, on
        // top, so that shared column reads as the scrollbar's edge.
        let (sb_w, overlap) = if self.v_scrollbar.rect().w > 0 {
            (SCROLLBAR_THICKNESS, 1)
        } else {
            (0, 0)
        };
        Rect::new(
            self.rect.x,
            self.rect.y,
            (self.rect.w - sb_w + overlap).max(0),
            self.rect.h,
        )
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
        let Some(idx) = self.lead else { return };
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
        if row < self.items.len() {
            Some(row)
        } else {
            None
        }
    }

    fn is_selected(&self, row: usize) -> bool {
        self.selection.binary_search(&row).is_ok()
    }

    /// Select exactly `idx`, dropping any other selection, and pin both the lead
    /// and the range anchor to it.
    fn select_single(&mut self, idx: usize) {
        self.selection.clear();
        self.selection.push(idx);
        self.lead = Some(idx);
        self.anchor = Some(idx);
    }

    /// Add `idx` to the selection if absent, remove it if present (keeping the
    /// vector sorted). The lead and anchor move to `idx` either way, so the
    /// cursor stays on the row the user just clicked even when it is deselected.
    fn toggle_at(&mut self, idx: usize) {
        match self.selection.binary_search(&idx) {
            Ok(pos) => {
                self.selection.remove(pos);
            }
            Err(pos) => self.selection.insert(pos, idx),
        }
        self.lead = Some(idx);
        self.anchor = Some(idx);
    }

    /// Replace the selection with the inclusive range between the anchor and
    /// `idx`, leaving the anchor fixed so a subsequent Shift gesture re-extends
    /// from the same end. Falls back to the lead, then `idx`, when no anchor is
    /// set yet.
    fn extend_to(&mut self, idx: usize) {
        let anchor = self.anchor.or(self.lead).unwrap_or(idx);
        let (lo, hi) = (anchor.min(idx), anchor.max(idx));
        self.selection = (lo..=hi).collect();
        self.anchor = Some(anchor);
        self.lead = Some(idx);
    }

    fn select_and_show(&mut self, idx: usize) {
        self.select_single(idx);
        self.ensure_selection_visible();
    }

    fn extend_and_show(&mut self, idx: usize) {
        self.extend_to(idx);
        self.ensure_selection_visible();
    }

    /// The row a navigation key would land on, clamped to the item range.
    /// `None` only when the list is empty.
    fn nav_target(&self, delta: i32) -> Option<usize> {
        if self.items.is_empty() {
            return None;
        }
        let cur = self.lead.unwrap_or(0) as i32;
        Some((cur + delta).clamp(0, self.items.len() as i32 - 1) as usize)
    }

    /// How many rows PageUp/PageDown moves — one screenful minus a row of
    /// overlap, at least one.
    fn page_step(&self) -> i32 {
        (self.visible_rows() - 1).max(1)
    }

    /// Move to `target`, either extending the Shift range (multi-select only) or
    /// replacing the selection with that single row.
    fn apply_nav(&mut self, target: usize, extend: bool) {
        if extend && self.multi_select {
            self.extend_and_show(target);
        } else {
            self.select_and_show(target);
        }
    }

    fn activate_selected(&mut self) {
        if let Some(idx) = self.lead {
            self.activated = Some(idx);
        }
    }

    fn handle_click(&mut self, idx: usize, pos: Point, modifiers: Modifiers) {
        // Ctrl/Cmd toggles a row; Shift range-selects. Both only apply with
        // multi-selection on, and neither counts as an activation gesture — only
        // a plain click feeds the double-click detector below.
        if self.multi_select && modifiers.shift {
            self.extend_and_show(idx);
            self.last_click = None;
            self.pending_collapse = None;
            return;
        }
        if self.multi_select && (modifiers.control || modifiers.logo) {
            self.toggle_at(idx);
            self.ensure_selection_visible();
            self.last_click = None;
            self.pending_collapse = None;
            return;
        }

        let now = Instant::now();
        let threshold = Duration::from_millis(DOUBLE_CLICK_MS);
        let double = self
            .last_click
            .map(|(prev_idx, prev_time)| {
                prev_idx == idx && now.duration_since(prev_time) <= threshold
            })
            .unwrap_or(false);

        if double {
            // A double-click always lands on a single row and activates it,
            // even when it began inside a multi-selection.
            self.select_and_show(idx);
            self.activated = Some(idx);
            self.last_click = None;
            self.pending_collapse = None;
        } else if self.multi_select && self.is_selected(idx) && self.selection.len() > 1 {
            // Plain press on a member of a multi-selection: hold the whole group
            // so it can be dragged, and only collapse to this row on release if
            // the press doesn't turn into a drag (see the pending-collapse arm
            // in `event`). The cursor still moves to the pressed row.
            self.lead = Some(idx);
            self.ensure_selection_visible();
            self.pending_collapse = Some((idx, pos));
            self.last_click = Some((idx, now));
        } else {
            self.select_and_show(idx);
            self.pending_collapse = None;
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

        // Field background stays at the full logical bounds; the chrome edges
        // (sunken bevel + 1-px outer border) self-manage the crisp physical-
        // pixel pass so they don't alias against the row separators or the
        // scrollbar gutter.
        painter.fill_rect(text, Color::WHITE);
        painter.sunken_bevel(text, theme.highlight, theme.shadow);
        painter.stroke_rect(text, theme.border);

        // Confine every row to the field interior so a label wider than the
        // row (or a forced partial row in a field too short for one) is clipped
        // at the border instead of bleeding into the scrollbar gutter or past
        // the widget — the same overflow guard `TextInput`/`Dropdown` apply.
        let saved_clip = painter.push_clip(text.inset(1));

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
            let selected = self.is_selected(row);
            // Active focus → navy/white; inactive (focus elsewhere) → muted
            // gray/black, matching the CUA convention so the user can still
            // see what's picked without the row competing for attention.
            let (text_color, bg_color) = if self.focused {
                (theme.highlight_text, theme.highlight_bg)
            } else {
                (theme.text, theme.face)
            };
            let text_color = if selected { text_color } else { theme.text };
            let text_color = if self.enabled {
                text_color
            } else {
                theme.disabled_text
            };
            if selected && self.enabled {
                painter.fill_rect(Rect::new(text_x, y, row_w.max(0), ROW_HEIGHT), bg_color);
            }

            let item = &self.items[row];
            // The label always starts past a fixed icon gutter, so labels line
            // up whether or not the row actually carries an icon.
            let icon_x = text_x + 2;
            match &item.icon {
                Some(IconArt::Raster(icon)) => {
                    let icon_y = y + (ROW_HEIGHT - icon.height) / 2;
                    draw_icon(painter, icon, icon_x, icon_y);
                }
                Some(IconArt::Svg(svg)) => {
                    let icon_y = y + (ROW_HEIGHT - ICON_SIZE) / 2;
                    painter.draw_svg(svg, Rect::new(icon_x, icon_y, ICON_SIZE, ICON_SIZE));
                }
                None => {}
            }
            let label_x = icon_x + ICON_SIZE + ICON_PAD;
            let label_y = y + (ROW_HEIGHT - theme.font_size as i32) / 2 - 1;
            painter.text(label_x, label_y, &item.label, theme.font_size, text_color);
        }

        if self.focused
            && self.enabled
            && let Some(idx) = self.lead
            && idx >= scroll_top
            && idx < scroll_top + visible
        {
            let y = text_y0 + (idx - scroll_top) as i32 * ROW_HEIGHT;
            painter.focus_rect(Rect::new(text_x, y, row_w.max(0), ROW_HEIGHT), theme.text);
        }

        painter.restore_clip(saved_clip);

        self.v_scrollbar.paint(painter, theme);
    }

    fn event(&mut self, event: &Event, ctx: &mut EventCtx) {
        if !self.enabled {
            return;
        }
        if self.v_scrollbar.captures_pointer() {
            self.v_scrollbar.event(event, ctx);
            return;
        }
        // Deferred-collapse lifecycle. While a press on a member of a
        // multi-selection is pending the list captures the pointer, so the
        // follow-up move/release land here. Resolve them up front — before the
        // scrollbar routing below — so a pending collapse can never be stranded
        // (a stuck one would keep the list capturing the pointer for good).
        if self.pending_collapse.is_some() {
            match event {
                Event::PointerMove { pos } => {
                    if let Some((_, start)) = self.pending_collapse {
                        let (dx, dy) = ((pos.x - start.x) as i64, (pos.y - start.y) as i64);
                        if dx * dx + dy * dy > COLLAPSE_SLOP * COLLAPSE_SLOP {
                            // Travelled far enough to read as a drag: keep the
                            // whole group selected and stop awaiting a collapse.
                            self.pending_collapse = None;
                        }
                    }
                }
                Event::PointerUp {
                    button: MouseButton::Left,
                    ..
                } => {
                    if let Some((row, _)) = self.pending_collapse.take() {
                        self.select_and_show(row);
                        ctx.request_paint();
                    }
                    return;
                }
                Event::PointerLeave => {
                    self.pending_collapse = None;
                    return;
                }
                _ => {}
            }
        }
        // The wheel scrolls the field whenever the pointer is anywhere over
        // it — not just over the scrollbar gutter — without disturbing the
        // selection, matching native list boxes.
        if let Event::Scroll { pos, .. } = event {
            if self.rect.contains(*pos) {
                self.v_scrollbar.event(event, ctx);
            }
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
                modifiers,
            } => {
                ctx.request_focus();
                if let Some(row) = self.row_at(*pos) {
                    self.handle_click(row, *pos, *modifiers);
                }
                ctx.request_paint();
            }
            Event::KeyDown { key, modifiers } if self.focused && !modifiers.has_command() => {
                // Shift extends the range (multi-select only); a bare arrow
                // collapses back to a single row. The `!has_command()` guard
                // lets Shift through (it isn't a command modifier) while still
                // passing Ctrl/Alt/Logo shortcuts up to the app.
                let extend = modifiers.shift;
                let target = match key {
                    Key::Named(NamedKey::Up) => self.nav_target(-1),
                    Key::Named(NamedKey::Down) => self.nav_target(1),
                    Key::Named(NamedKey::Home) => (!self.items.is_empty()).then_some(0),
                    Key::Named(NamedKey::End) => self.items.len().checked_sub(1),
                    Key::Named(NamedKey::PageUp) => self.nav_target(-self.page_step()),
                    Key::Named(NamedKey::PageDown) => self.nav_target(self.page_step()),
                    Key::Named(NamedKey::Enter) => {
                        self.activate_selected();
                        ctx.request_paint();
                        return;
                    }
                    _ => return,
                };
                if let Some(t) = target {
                    self.apply_nav(t, extend);
                }
                ctx.request_paint();
            }
            _ => {}
        }
    }

    fn captures_pointer(&self) -> bool {
        // Hold the pointer while a deferred collapse is pending so the move /
        // release that resolve it are routed here even when the list lives
        // inside a container that hit-tests by position.
        self.v_scrollbar.captures_pointer() || self.pending_collapse.is_some()
    }

    fn focusable(&self) -> bool {
        self.enabled
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
        // Only re-sync the scrollbar's range to the new viewport — don't snap
        // the view to the selection. A layout pass fires for reasons unrelated
        // to selection (on Wayland a window focus change triggers a `configure`
        // and hence a relayout), so scrolling to the lead row here would yank a
        // wheel-scrolled view back to the selection whenever the window lost or
        // regained focus. Selection changes and keyboard nav call
        // `ensure_selection_visible` themselves, so the cursor still follows.
        self.sync_scrollbar();
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

#[cfg(test)]
mod tests {
    use super::*;

    /// A focused, laid-out list of `n` rows whose 200×200 field shows every row
    /// without scrolling (10 visible rows for 8 items).
    fn list_with(n: usize) -> List {
        let items = (0..n).map(|i| ListItem::new(format!("item {i}"))).collect();
        let mut l = List::new(Rect::new(0, 0, 200, 200)).with_items(items);
        l.set_focused(true);
        l.layout(Rect::new(0, 0, 200, 200));
        l
    }

    /// A point inside row `row`'s band (no scroll), left of the scrollbar.
    fn row_point(row: usize) -> Point {
        Point::new(5, TEXT_PAD_Y + row as i32 * ROW_HEIGHT + ROW_HEIGHT / 2)
    }

    fn click(l: &mut List, row: usize, modifiers: Modifiers) {
        let mut ctx = EventCtx::new();
        l.event(
            &Event::PointerDown {
                pos: row_point(row),
                button: MouseButton::Left,
                modifiers,
            },
            &mut ctx,
        );
    }

    fn key(l: &mut List, k: NamedKey, modifiers: Modifiers) {
        let mut ctx = EventCtx::new();
        l.event(
            &Event::KeyDown {
                key: Key::Named(k),
                modifiers,
            },
            &mut ctx,
        );
    }

    fn plain() -> Modifiers {
        Modifiers::default()
    }

    fn shift() -> Modifiers {
        Modifiers {
            shift: true,
            ..Default::default()
        }
    }

    fn ctrl() -> Modifiers {
        Modifiers {
            control: true,
            ..Default::default()
        }
    }

    fn release(l: &mut List, row: usize) {
        let mut ctx = EventCtx::new();
        l.event(
            &Event::PointerUp {
                pos: row_point(row),
                button: MouseButton::Left,
                modifiers: Modifiers::default(),
            },
            &mut ctx,
        );
    }

    fn pointer_move(l: &mut List, pos: Point) {
        let mut ctx = EventCtx::new();
        l.event(&Event::PointerMove { pos }, &mut ctx);
    }

    /// Select rows 1..=3 as a multi-selection (anchor 1, lead 3).
    fn group_123(l: &mut List) {
        click(l, 1, plain());
        click(l, 3, shift());
        assert_eq!(l.selected_indices(), &[1, 2, 3]);
    }

    #[test]
    fn single_select_ignores_click_modifiers() {
        let mut l = list_with(8);
        // Ctrl/Shift on a single-select list behave like a plain click: the
        // selection is replaced, never accumulated.
        click(&mut l, 1, ctrl());
        assert_eq!(l.selected_indices(), &[1]);
        click(&mut l, 3, ctrl());
        assert_eq!(l.selected_indices(), &[3]);
        click(&mut l, 5, shift());
        assert_eq!(l.selected_indices(), &[5]);
        assert_eq!(l.selected_index(), Some(5));
    }

    #[test]
    fn ctrl_click_toggles_in_multi() {
        let mut l = list_with(8).with_multi_select(true);
        click(&mut l, 1, plain());
        assert_eq!(l.selected_indices(), &[1]);
        click(&mut l, 3, ctrl());
        assert_eq!(l.selected_indices(), &[1, 3]);
        click(&mut l, 1, ctrl()); // toggle row 1 back off
        assert_eq!(l.selected_indices(), &[3]);
        // The cursor (lead) stays on the row last clicked, even deselected.
        assert_eq!(l.selected_index(), Some(1));
    }

    #[test]
    fn shift_click_selects_range_from_anchor() {
        let mut l = list_with(8).with_multi_select(true);
        click(&mut l, 2, plain());
        click(&mut l, 5, shift());
        assert_eq!(l.selected_indices(), &[2, 3, 4, 5]);
        assert_eq!(l.selected_index(), Some(5));
        // The anchor stays at 2, so extending the other way re-ranges from it.
        click(&mut l, 0, shift());
        assert_eq!(l.selected_indices(), &[0, 1, 2]);
    }

    #[test]
    fn shift_arrow_extends_plain_arrow_collapses() {
        let mut l = list_with(8).with_multi_select(true);
        click(&mut l, 2, plain());
        key(&mut l, NamedKey::Down, shift());
        key(&mut l, NamedKey::Down, shift());
        assert_eq!(l.selected_indices(), &[2, 3, 4]);
        assert_eq!(l.selected_index(), Some(4));
        // A bare arrow drops the range and selects a single row again.
        key(&mut l, NamedKey::Down, plain());
        assert_eq!(l.selected_indices(), &[5]);
    }

    #[test]
    fn set_selected_round_trips() {
        let mut l = list_with(8);
        l.set_selected(Some(4));
        assert_eq!(l.selected_index(), Some(4));
        assert_eq!(l.selected_indices(), &[4]);
        l.set_selected(None);
        assert_eq!(l.selected_index(), None);
        assert!(l.selected_indices().is_empty());
    }

    #[test]
    fn set_selected_indices_is_sorted_and_deduped() {
        let mut l = list_with(8).with_multi_select(true);
        l.set_selected_indices([5, 1, 3, 1, 99]); // 99 is out of range
        assert_eq!(l.selected_indices(), &[1, 3, 5]);
        assert_eq!(l.selected_index(), Some(1));
    }

    #[test]
    fn set_items_drops_now_invalid_selection() {
        let mut l = list_with(8).with_multi_select(true);
        l.set_selected_indices([1, 3, 5]);
        let items = (0..3).map(|i| ListItem::new(format!("x{i}"))).collect();
        l.set_items(items);
        // Only index 1 still points at a row.
        assert_eq!(l.selected_indices(), &[1]);

        // A lead/anchor past the new end is cleared entirely.
        let mut l = list_with(8);
        l.set_selected(Some(7));
        let items = (0..3).map(|i| ListItem::new(format!("y{i}"))).collect();
        l.set_items(items);
        assert!(l.selected_indices().is_empty());
        assert_eq!(l.selected_index(), None);
    }

    #[test]
    fn disabling_multi_select_collapses_to_one_row() {
        let mut l = list_with(8).with_multi_select(true);
        l.set_selected_indices([1, 3, 5]); // lead becomes the first, 1
        l.set_multi_select(false);
        assert!(!l.is_multi_select());
        assert_eq!(l.selected_indices(), &[1]);
        assert_eq!(l.selected_index(), Some(1));
    }

    #[test]
    fn plain_click_on_group_member_defers_collapse_until_release() {
        let mut l = list_with(8).with_multi_select(true);
        group_123(&mut l);
        // Pressing a member keeps the whole group — and captures the pointer so
        // the release lands back here — instead of collapsing on the press.
        click(&mut l, 2, plain());
        assert_eq!(l.selected_indices(), &[1, 2, 3]);
        assert!(l.captures_pointer());
        assert_eq!(l.selected_index(), Some(2)); // cursor moved to the press
        // Releasing without a drag collapses to just that row and frees capture.
        release(&mut l, 2);
        assert_eq!(l.selected_indices(), &[2]);
        assert!(!l.captures_pointer());
    }

    #[test]
    fn drag_motion_keeps_the_whole_group() {
        let mut l = list_with(8).with_multi_select(true);
        group_123(&mut l);
        click(&mut l, 2, plain());
        // Travel past the slop → read as a drag: the group survives the release.
        let start = row_point(2);
        pointer_move(&mut l, Point::new(start.x + 10, start.y + 10));
        assert!(!l.captures_pointer()); // pending abandoned
        release(&mut l, 2);
        assert_eq!(l.selected_indices(), &[1, 2, 3]);
    }

    #[test]
    fn plain_click_on_unselected_row_collapses_immediately() {
        let mut l = list_with(8).with_multi_select(true);
        group_123(&mut l);
        // A row outside the selection is not deferred — it replaces it on press.
        click(&mut l, 5, plain());
        assert_eq!(l.selected_indices(), &[5]);
        assert!(!l.captures_pointer());
    }

    #[test]
    fn double_click_on_group_member_still_activates() {
        let mut l = list_with(8).with_multi_select(true);
        group_123(&mut l);
        click(&mut l, 2, plain()); // deferred
        release(&mut l, 2); // collapses to [2]
        click(&mut l, 2, plain()); // second click of the pair → activation
        assert_eq!(l.selected_indices(), &[2]);
        assert_eq!(l.take_activated(), Some(2));
    }

    /// A wheel-scrolled list with a selection must not jump back to that
    /// selection on a relayout that isn't a resize. On Wayland a window focus
    /// change triggers a `configure` (and a relayout), so a `layout()` that
    /// snapped to the selection would yank the view back the moment the window
    /// lost or regained focus — the list analogue of the notepad caret jump.
    #[test]
    fn relayout_preserves_wheel_scroll() {
        let rect = Rect::new(0, 0, 200, 120);
        let items = (0..100)
            .map(|i| ListItem::new(format!("item {i}")))
            .collect();
        let mut l = List::new(rect).with_items(items);
        l.layout(rect);

        // Select the top row, then wheel far down so the selection scrolls out
        // of view above the viewport.
        l.set_selected(Some(0));
        assert_eq!(l.scroll_top(), 0);
        let mut ctx = EventCtx::new();
        l.event(
            &Event::Scroll {
                pos: Point::new(100, 60),
                delta_x: 0.0,
                delta_y: 40.0,
            },
            &mut ctx,
        );
        let scrolled = l.scroll_top();
        assert!(scrolled > 0, "wheel should have moved the view down");

        // Window focus change → relayout at the same size. The view stays put.
        l.layout(rect);
        assert_eq!(
            l.scroll_top(),
            scrolled,
            "a relayout (e.g. on window focus change) must not jump to the selection"
        );
    }
}
