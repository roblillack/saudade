use crate::event::{Event, EventCtx, Key, MouseButton, NamedKey};
use crate::geometry::{Color, Point, Rect};
use crate::painter::Painter;
use crate::theme::Theme;
use crate::widget::{PopupKind, PopupRequest, Widget};

type ChangeHandler = Box<dyn FnMut(&mut EventCtx, usize)>;

/// Width of the drop-arrow button on the right edge of the closed field.
const ARROW_BTN_W: i32 = 17;
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

    /// The raised drop-arrow button on the right edge, inside the field border.
    fn arrow_rect(&self) -> Rect {
        let inner = self.rect.inset(1);
        Rect::new(
            inner.right() - ARROW_BTN_W,
            inner.y,
            ARROW_BTN_W,
            inner.h.max(0),
        )
    }

    /// The area the selected label is drawn in — everything left of the arrow
    /// button, inset by the sunken border.
    fn text_area(&self) -> Rect {
        let inner = self.rect.inset(2);
        let w = (inner.w - ARROW_BTN_W).max(0);
        Rect::new(inner.x, inner.y, w, inner.h)
    }

    /// Logical-coordinate rect of the open popup list (without its shadow), in
    /// the root widget's coordinate space — flush below the field.
    fn popup_rect(&self) -> Rect {
        let h = POPUP_PAD_Y * 2 + self.items.len() as i32 * ITEM_HEIGHT;
        Rect::new(self.rect.x, self.rect.bottom(), self.rect.w, h)
    }

    /// Map a pointer position to the popup row under it, if the list is open
    /// and the point lands on an actual item.
    fn hit_item(&self, pos: Point) -> Option<usize> {
        if !self.open {
            return None;
        }
        let popup = self.popup_rect();
        if !popup.contains(pos) {
            return None;
        }
        let local = pos.y - (popup.y + POPUP_PAD_Y);
        if local < 0 {
            return None;
        }
        let idx = (local / ITEM_HEIGHT) as usize;
        (idx < self.items.len()).then_some(idx)
    }

    fn open_list(&mut self) {
        if self.items.is_empty() {
            return;
        }
        self.open = true;
        // Pre-highlight the current selection so Enter / arrows have a starting
        // point and the user sees what's picked.
        self.highlighted = self.selected.or(Some(0));
    }

    fn close(&mut self) {
        self.open = false;
        self.highlighted = None;
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
    /// combos don't wrap).
    fn move_highlight(&mut self, delta: i32) {
        if self.items.is_empty() {
            return;
        }
        let n = self.items.len() as i32;
        let cur = self.highlighted.or(self.selected).unwrap_or(0) as i32;
        self.highlighted = Some((cur + delta).clamp(0, n - 1) as usize);
    }

    fn handle_key(&mut self, key: &Key, ctx: &mut EventCtx) {
        if self.open {
            match key {
                Key::Named(NamedKey::Up) => self.move_highlight(-1),
                Key::Named(NamedKey::Down) => self.move_highlight(1),
                Key::Named(NamedKey::Home) => self.highlighted = Some(0),
                Key::Named(NamedKey::End) => {
                    self.highlighted = self.items.len().checked_sub(1);
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
        // self-manages the crisp physical-pixel pass at fractional scales. The
        // arrow glyph still needs a manual pass until `draw_down_arrow` is
        // hoisted onto the painter the way the bevels were.
        painter.fill_rect(self.rect, bg);
        painter.sunken_bevel(self.rect, theme.highlight, theme.shadow);
        painter.stroke_rect(self.rect, theme.border);
        painter.button(btn, theme, self.open, false);
        if painter.wants_crisp_chrome() {
            let phys_btn = painter.rect_to_physical(btn);
            let saved = painter.push_physical_pixels();
            draw_down_arrow(painter, phys_btn, arrow_color);
            painter.restore_scale(saved);
        } else {
            draw_down_arrow(painter, btn, arrow_color);
        }

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

        let mut y = popup.y + POPUP_PAD_Y;
        for (i, item) in self.items.iter().enumerate() {
            let row = Rect::new(popup.x + 1, y, (popup.w - 2).max(0), ITEM_HEIGHT);
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
    }

    fn event(&mut self, event: &Event, ctx: &mut EventCtx) {
        if !self.enabled {
            return;
        }
        match event {
            Event::PointerDown {
                pos,
                button: MouseButton::Left,
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

/// A small downward-pointing triangle (7 px base) centered in `btn`.
fn draw_down_arrow(painter: &mut Painter, btn: Rect, color: Color) {
    let cx = btn.x + btn.w / 2;
    let top = btn.y + (btn.h - 4) / 2;
    // Rows shrink 7 → 5 → 3 → 1, forming the arrowhead.
    for row in 0..4 {
        let half = 3 - row;
        painter.fill_rect(Rect::new(cx - half, top + row, half * 2 + 1, 1), color);
    }
}
