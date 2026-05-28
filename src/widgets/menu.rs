use crate::event::{Event, EventCtx, Key, Modifiers, MouseButton, NamedKey};
use crate::geometry::{Color, Point, Rect};
use crate::painter::Painter;
use crate::theme::Theme;
use crate::widget::{PopupKind, PopupRequest, Widget};

const BAR_PADDING: i32 = 8;
/// Top inset for the label baseline inside the bar. Tight enough that the
/// 13-pt menu font fits in a 20-px bar without growing it.
const BAR_LABEL_INSET_Y: i32 = 1;
const POPUP_PADDING_X: i32 = 18;
const POPUP_PADDING_Y: i32 = 3;
const ITEM_HEIGHT: i32 = 18;
const ITEM_TEXT_INSET_Y: i32 = 1;
const SEPARATOR_HEIGHT: i32 = 6;
const SHADOW_SIZE: i32 = 2;
/// L-shape drop shadow color: a dark gray with no alpha trickery so it
/// renders crisply on every backend.
const SHADOW_COLOR: Color = Color::rgb(0x40, 0x40, 0x40);

/// One entry inside a drop-down [`Menu`].
pub enum MenuItem {
    Action {
        /// Raw label as supplied; may contain `&X` to mark the mnemonic.
        label: String,
        callback: Box<dyn FnMut(&mut EventCtx)>,
    },
    Separator,
}

impl MenuItem {
    pub fn action<F>(label: impl Into<String>, callback: F) -> Self
    where
        F: FnMut(&mut EventCtx) + 'static,
    {
        MenuItem::Action {
            label: label.into(),
            callback: Box::new(callback),
        }
    }

    pub fn separator() -> Self {
        MenuItem::Separator
    }

    fn is_action(&self) -> bool {
        matches!(self, MenuItem::Action { .. })
    }

    fn height(&self) -> i32 {
        match self {
            MenuItem::Action { .. } => ITEM_HEIGHT,
            MenuItem::Separator => SEPARATOR_HEIGHT,
        }
    }
}

pub struct Menu {
    pub label: String,
    pub items: Vec<MenuItem>,
}

impl Menu {
    pub fn new(label: impl Into<String>, items: Vec<MenuItem>) -> Self {
        Self {
            label: label.into(),
            items,
        }
    }
}

/// Parsed view of a label like `"&File"`: the visible text with the `&`
/// stripped, and the (logical) character index of the mnemonic glyph.
#[derive(Clone)]
struct ParsedLabel {
    display: String,
    mnemonic_index: Option<usize>,
    mnemonic_char: Option<char>,
}

fn parse_label(raw: &str) -> ParsedLabel {
    let mut display = String::with_capacity(raw.len());
    let mut mnemonic_index = None;
    let mut mnemonic_char = None;
    let mut chars = raw.chars().peekable();
    let mut idx = 0;
    while let Some(c) = chars.next() {
        if c == '&' {
            if chars.peek() == Some(&'&') {
                chars.next();
                display.push('&');
                idx += 1;
            } else if let Some(&next) = chars.peek() {
                mnemonic_index = Some(idx);
                mnemonic_char = Some(next.to_ascii_lowercase());
                // do not push the '&' itself; the next loop iteration pushes
                // the actual character at idx.
            }
        } else {
            display.push(c);
            idx += 1;
        }
    }
    ParsedLabel {
        display,
        mnemonic_index,
        mnemonic_char,
    }
}

#[derive(Default, Clone)]
struct Cache {
    /// (x, width) per top-level menu label.
    label_rects: Vec<(i32, i32)>,
    /// Popup rect for the currently open menu, if any.
    popup: Option<Rect>,
}

/// A classic Win 3.1 menu bar.
///
/// `MenuBar::new` takes the bounding rect for the bar itself. The bar paints
/// in the normal pass; any open drop-down is rendered in the overlay pass so
/// it floats over every sibling widget.
///
/// Labels may include an `&` immediately before a character to declare a
/// mnemonic: `&File` displays "File" with **F** underlined and binds Alt+F
/// (top-level) or just F (when the menu is already open) to that entry.
/// Escape closes the open menu.
pub struct MenuBar {
    rect: Rect,
    menus: Vec<Menu>,
    open: Option<usize>,
    hovered_item: Option<usize>,
    /// True between the press-on-the-bar that opened the current menu and
    /// the matching release. Lets us implement classic drag-to-pick: press
    /// on `File`, drag down into the popup, release on an item → fire it.
    /// After the first release without a fire, we drop back into click
    /// mode where a separate click fires an item.
    drag_armed: bool,
    cache: Cache,
}

impl MenuBar {
    pub fn new(rect: Rect) -> Self {
        Self {
            rect,
            menus: Vec::new(),
            open: None,
            hovered_item: None,
            drag_armed: false,
            cache: Cache::default(),
        }
    }

    pub fn add_menu(mut self, menu: Menu) -> Self {
        self.menus.push(menu);
        self
    }

    pub fn push_menu(&mut self, menu: Menu) {
        self.menus.push(menu);
    }

    /// Programmatically open the menu at `index` — useful for tests and
    /// for hooking up custom keyboard shortcuts at the application level.
    pub fn open(&mut self, index: usize) {
        if index < self.menus.len() {
            self.open = Some(index);
            self.hovered_item = None;
        }
    }

    fn rebuild_label_rects(&mut self, painter: &Painter, theme: &Theme) {
        // First label butts up against the bar's left edge so its highlight
        // reaches the window edge when active; subsequent labels follow with
        // their own internal padding. Every label still carries `BAR_PADDING`
        // on both sides of its text for visual breathing room.
        self.cache.label_rects.clear();
        let mut x = self.rect.x;
        for menu in &self.menus {
            let parsed = parse_label(&menu.label);
            let w = painter.measure_text(&parsed.display, theme.menu_font_size).w + BAR_PADDING * 2;
            self.cache.label_rects.push((x, w));
            x += w;
        }
    }

    fn compute_popup(&self, menu_idx: usize, painter: &Painter, theme: &Theme) -> Rect {
        let (lx, _lw) = self
            .cache
            .label_rects
            .get(menu_idx)
            .copied()
            .unwrap_or((self.rect.x, 0));
        let menu = &self.menus[menu_idx];

        let mut max_label = 0;
        for item in &menu.items {
            if let MenuItem::Action { label, .. } = item {
                let parsed = parse_label(label);
                let w = painter.measure_text(&parsed.display, theme.menu_font_size).w;
                if w > max_label {
                    max_label = w;
                }
            }
        }
        let width = max_label + POPUP_PADDING_X * 2;
        let mut height = POPUP_PADDING_Y * 2;
        for item in &menu.items {
            height += item.height();
        }
        Rect::new(lx, self.rect.y + self.rect.h, width, height)
    }

    fn hit_label(&self, pos: Point) -> Option<usize> {
        if pos.y < self.rect.y || pos.y >= self.rect.y + self.rect.h {
            return None;
        }
        self.cache
            .label_rects
            .iter()
            .position(|(x, w)| pos.x >= *x && pos.x < *x + *w)
    }

    fn hit_item(&self, pos: Point) -> Option<usize> {
        let popup = self.cache.popup?;
        if !popup.contains(pos) {
            return None;
        }
        let menu_idx = self.open?;
        let mut y = popup.y + POPUP_PADDING_Y;
        for (i, item) in self.menus[menu_idx].items.iter().enumerate() {
            let h = item.height();
            if pos.y >= y && pos.y < y + h {
                return if item.is_action() { Some(i) } else { None };
            }
            y += h;
        }
        None
    }

    fn fire(&mut self, item_idx: usize, ctx: &mut EventCtx) {
        let Some(menu_idx) = self.open else { return };
        if let Some(MenuItem::Action { callback, .. }) =
            self.menus[menu_idx].items.get_mut(item_idx)
        {
            callback(ctx);
        }
    }

    /// Find a top-level menu whose mnemonic matches the typed character.
    fn top_level_mnemonic(&self, ch: char) -> Option<usize> {
        let target = ch.to_ascii_lowercase();
        for (i, menu) in self.menus.iter().enumerate() {
            if parse_label(&menu.label).mnemonic_char == Some(target) {
                return Some(i);
            }
        }
        None
    }

    /// Find an action item in the currently-open menu whose mnemonic matches.
    fn item_mnemonic(&self, ch: char) -> Option<usize> {
        let menu_idx = self.open?;
        let target = ch.to_ascii_lowercase();
        for (i, item) in self.menus[menu_idx].items.iter().enumerate() {
            if let MenuItem::Action { label, .. } = item
                && parse_label(label).mnemonic_char == Some(target)
            {
                return Some(i);
            }
        }
        None
    }

    /// Draw text with the mnemonic glyph underlined. `dy_phys` lets the
    /// caller nudge both the text and the underline by a physical-pixel
    /// amount independent of any logical-pixel inset (the menu bar uses
    /// this to drop its labels exactly one physical pixel without growing
    /// the bar by a whole logical pixel).
    fn draw_label_with_mnemonic(
        painter: &mut Painter,
        x: i32,
        y: i32,
        dy_phys: i32,
        parsed: &ParsedLabel,
        size: f32,
        color: Color,
    ) {
        painter.text_with_phys_offset(x, y, 0, dy_phys, &parsed.display, size, color);
        if let Some(idx) = parsed.mnemonic_index {
            let prefix: String = parsed.display.chars().take(idx).collect();
            let mnemonic_ch: String = parsed.display.chars().skip(idx).take(1).collect();
            if mnemonic_ch.is_empty() {
                return;
            }
            let prefix_w = painter.measure_text(&prefix, size).w;
            let glyph_w = painter.measure_text(&mnemonic_ch, size).w;
            // Drop the underline 1 logical pixel below the baseline so it
            // doesn't kiss the bottom of the letter (and doesn't fight any
            // descender on the rare lowercase mnemonic).
            let underline_y = y + (size as i32) + 1;
            painter.fill_rect_with_phys_offset(
                Rect::new(x + prefix_w, underline_y, glyph_w, 1),
                0,
                dy_phys,
                color,
            );
        }
    }
}

impl Widget for MenuBar {
    fn bounds(&self) -> Rect {
        self.rect
    }

    fn paint(&mut self, painter: &mut Painter, theme: &Theme) {
        self.rebuild_label_rects(painter, theme);
        self.cache.popup = self
            .open
            .map(|idx| self.compute_popup(idx, painter, theme));

        // Bar background + 1-px shadow line along the bottom. The bar is
        // white to match Win 3.1's program-manager chrome — only the labels
        // and dropdowns carry color.
        painter.fill_rect(self.rect, theme.background);
        painter.h_line(
            self.rect.x,
            self.rect.bottom() - 1,
            self.rect.w,
            theme.shadow,
        );

        for (i, menu) in self.menus.iter().enumerate() {
            let (lx, lw) = self.cache.label_rects[i];
            let label_rect = Rect::new(lx, self.rect.y, lw, self.rect.h - 1);
            let parsed = parse_label(&menu.label);
            let (fg, draw_bg) = if self.open == Some(i) {
                (theme.highlight_text, true)
            } else {
                (theme.text, false)
            };
            if draw_bg {
                painter.fill_rect(label_rect, theme.highlight_bg);
            }
            // Bar labels are nudged down by one physical pixel so the cap
            // height has visible breathing room above without growing the
            // bar by a whole logical pixel.
            Self::draw_label_with_mnemonic(
                painter,
                lx + BAR_PADDING,
                self.rect.y + BAR_LABEL_INSET_Y,
                1,
                &parsed,
                theme.menu_font_size,
                fg,
            );
        }
    }

    fn paint_overlay(&mut self, painter: &mut Painter, theme: &Theme) {
        // The popup lives in a separate top-level window — only draw it
        // when the painter is running in popup-pass mode. In the main
        // window's overlay pass we deliberately leave the popup area
        // untouched so the runtime can place a real popup window there.
        if !painter.is_popup_pass() {
            return;
        }
        let Some(menu_idx) = self.open else { return };
        let popup = match self.cache.popup {
            Some(p) => p,
            None => {
                let p = self.compute_popup(menu_idx, painter, theme);
                self.cache.popup = Some(p);
                p
            }
        };

        // L-shape drop shadow drawn first so the popup overlays it on the
        // top/left edges.
        painter.fill_rect(
            Rect::new(
                popup.x + SHADOW_SIZE,
                popup.bottom(),
                popup.w,
                SHADOW_SIZE,
            ),
            SHADOW_COLOR,
        );
        painter.fill_rect(
            Rect::new(
                popup.right(),
                popup.y + SHADOW_SIZE,
                SHADOW_SIZE,
                popup.h,
            ),
            SHADOW_COLOR,
        );

        // White interior + thin black border. No raised bevel — Win 3.1
        // drop-downs are flat panels, the bar holds the chrome.
        painter.fill_rect(popup, theme.background);
        painter.stroke_rect(popup, theme.border);

        let mut y = popup.y + POPUP_PADDING_Y;
        for (i, item) in self.menus[menu_idx].items.iter().enumerate() {
            match item {
                MenuItem::Action { label, .. } => {
                    let row = Rect::new(popup.x + 1, y, popup.w - 2, ITEM_HEIGHT);
                    let parsed = parse_label(label);
                    let (bg, fg) = if self.hovered_item == Some(i) {
                        (theme.highlight_bg, theme.highlight_text)
                    } else {
                        (theme.background, theme.text)
                    };
                    painter.fill_rect(row, bg);
                    Self::draw_label_with_mnemonic(
                        painter,
                        row.x + POPUP_PADDING_X - 4,
                        row.y + ITEM_TEXT_INSET_Y,
                        0,
                        &parsed,
                        theme.menu_font_size,
                        fg,
                    );
                    y += ITEM_HEIGHT;
                }
                MenuItem::Separator => {
                    let mid = y + SEPARATOR_HEIGHT / 2;
                    painter.etched_h_line(popup.x + 4, mid, popup.w - 8, theme);
                    y += SEPARATOR_HEIGHT;
                }
            }
        }
    }

    fn event(&mut self, event: &Event, ctx: &mut EventCtx) {
        match event {
            Event::PointerDown {
                pos,
                button: MouseButton::Left,
            } => {
                if let Some(menu_idx) = self.open {
                    if let Some(item) = self.hit_item(*pos) {
                        self.fire(item, ctx);
                        self.open = None;
                        self.hovered_item = None;
                        self.drag_armed = false;
                        ctx.request_paint();
                        return;
                    }
                    if let Some(label_idx) = self.hit_label(*pos) {
                        if label_idx == menu_idx {
                            // Press on the open label — close on release.
                            self.open = None;
                            self.hovered_item = None;
                        } else {
                            self.open = Some(label_idx);
                            self.hovered_item = None;
                            self.drag_armed = true;
                        }
                        ctx.request_paint();
                        return;
                    }
                    // Click outside — dismiss.
                    self.open = None;
                    self.hovered_item = None;
                    self.drag_armed = false;
                    ctx.request_paint();
                } else if let Some(label_idx) = self.hit_label(*pos) {
                    self.open = Some(label_idx);
                    self.hovered_item = None;
                    // The press might be the start of a drag-to-pick gesture.
                    self.drag_armed = true;
                    ctx.request_paint();
                }
            }
            Event::PointerUp {
                pos,
                button: MouseButton::Left,
            } => {
                if !self.drag_armed {
                    return;
                }
                self.drag_armed = false;
                // Released over an item → fire it.
                if let Some(item) = self.hit_item(*pos) {
                    self.fire(item, ctx);
                    self.open = None;
                    self.hovered_item = None;
                    ctx.request_paint();
                    return;
                }
                // Click-without-drag (released back over the bar, no item
                // ever hovered) → pre-highlight the first action so the
                // user can fire it with Enter or keep arrow-navigating.
                if self.hovered_item.is_none() && self.hit_label(*pos).is_some() {
                    self.hovered_item = self.first_action();
                    ctx.request_paint();
                }
                // Released somewhere else (dragged outside, then released):
                // just disarm and leave the menu in its current state.
            }
            Event::PointerMove { pos } => {
                if self.open.is_some() {
                    let item = self.hit_item(*pos);
                    if item != self.hovered_item {
                        self.hovered_item = item;
                        ctx.request_paint();
                    }
                    if let Some(label_idx) = self.hit_label(*pos)
                        && self.open != Some(label_idx)
                    {
                        self.open = Some(label_idx);
                        self.hovered_item = None;
                        ctx.request_paint();
                    }
                }
            }
            Event::KeyDown { key, modifiers } => match key {
                Key::Named(NamedKey::Escape) if self.open.is_some() => {
                    self.open = None;
                    self.hovered_item = None;
                    ctx.request_paint();
                }
                Key::Named(NamedKey::Down) if self.open.is_some() => {
                    self.move_selection(1, ctx);
                }
                Key::Named(NamedKey::Up) if self.open.is_some() => {
                    self.move_selection(-1, ctx);
                }
                Key::Named(NamedKey::Right) if self.open.is_some() => {
                    self.switch_top_level(1, ctx);
                }
                Key::Named(NamedKey::Left) if self.open.is_some() => {
                    self.switch_top_level(-1, ctx);
                }
                Key::Named(NamedKey::Home) if self.open.is_some() => {
                    self.hovered_item = self.first_action();
                    ctx.request_paint();
                }
                Key::Named(NamedKey::End) if self.open.is_some() => {
                    self.hovered_item = self.last_action();
                    ctx.request_paint();
                }
                Key::Named(NamedKey::Enter) if self.open.is_some() => {
                    if let Some(item) = self.hovered_item {
                        self.fire(item, ctx);
                        self.open = None;
                        self.hovered_item = None;
                        ctx.request_paint();
                    }
                }
                Key::Char(ch) => {
                    if self.handle_mnemonic(*ch, *modifiers, ctx) {
                        // consumed
                    }
                }
                _ => {}
            },
            Event::Char { ch, modifiers } => {
                // Some platforms route mnemonic characters through Char with
                // Alt held; treat the same way.
                if modifiers.alt {
                    self.handle_mnemonic(*ch, *modifiers, ctx);
                }
            }
            _ => {}
        }
    }

    fn captures_pointer(&self) -> bool {
        self.open.is_some()
    }

    fn accepts_accelerators(&self) -> bool {
        true
    }

    fn layout(&mut self, bounds: Rect) {
        self.rect = bounds;
        // Force the cached label rects to be rebuilt on the next paint —
        // they were measured against the previous width.
        self.cache = Cache::default();
    }

    fn popup_request(&self) -> Option<PopupRequest> {
        // Cache.popup is populated during paint; until the first paint
        // completes after the menu opens, we have nothing to anchor.
        let _ = self.open?;
        let popup = self.cache.popup?;
        // Include the L-shape drop shadow inside the popup window's bounds
        // so it doesn't clip at the right/bottom edges.
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

impl MenuBar {
    /// Index of the first action item in the currently open menu (skipping
    /// separators); `None` if no menu is open or it has no actions.
    fn first_action(&self) -> Option<usize> {
        let menu_idx = self.open?;
        self.menus[menu_idx]
            .items
            .iter()
            .position(|item| item.is_action())
    }

    fn last_action(&self) -> Option<usize> {
        let menu_idx = self.open?;
        self.menus[menu_idx]
            .items
            .iter()
            .enumerate()
            .rev()
            .find_map(|(i, item)| item.is_action().then_some(i))
    }

    /// Step hovered_item by ±1, skipping separators, wrapping at the ends.
    /// `delta` should be +1 (Down) or -1 (Up).
    fn move_selection(&mut self, delta: i32, ctx: &mut EventCtx) {
        let Some(menu_idx) = self.open else { return };
        let n = self.menus[menu_idx].items.len();
        if n == 0 {
            return;
        }
        let actions: Vec<usize> = self.menus[menu_idx]
            .items
            .iter()
            .enumerate()
            .filter(|(_, item)| item.is_action())
            .map(|(i, _)| i)
            .collect();
        if actions.is_empty() {
            return;
        }
        let current = self
            .hovered_item
            .and_then(|h| actions.iter().position(|&a| a == h));
        let next = match (current, delta) {
            (None, 1) => 0,
            (None, _) => actions.len() - 1,
            (Some(i), d) => {
                let len = actions.len() as i32;
                ((i as i32 + d).rem_euclid(len)) as usize
            }
        };
        self.hovered_item = Some(actions[next]);
        ctx.request_paint();
    }

    /// Move to the previous / next top-level menu, keeping a dropdown open.
    /// Always pre-highlights the first action of the newly opened menu — the
    /// previous highlight position doesn't carry over.
    fn switch_top_level(&mut self, delta: i32, ctx: &mut EventCtx) {
        let Some(current) = self.open else { return };
        let n = self.menus.len() as i32;
        if n == 0 {
            return;
        }
        let next = ((current as i32 + delta).rem_euclid(n)) as usize;
        if next != current {
            self.open = Some(next);
            self.hovered_item = self.first_action();
            ctx.request_paint();
        }
    }

    /// Translate a typed character into a menu-open or item-fire action. Returns
    /// `true` if the keystroke was consumed.
    fn handle_mnemonic(&mut self, ch: char, modifiers: Modifiers, ctx: &mut EventCtx) -> bool {
        if self.open.is_some() {
            // No modifier required while a menu is open — typing a letter
            // fires its mnemonic item.
            if let Some(item) = self.item_mnemonic(ch) {
                self.fire(item, ctx);
                self.open = None;
                self.hovered_item = None;
                ctx.request_paint();
                return true;
            }
            return false;
        }
        // Closed bar: only respond to Alt+letter to open a top-level menu.
        // Keyboard-opened menus pre-highlight the first action so the user
        // can hit Enter or use arrows immediately.
        if modifiers.alt
            && let Some(menu_idx) = self.top_level_mnemonic(ch)
        {
            self.open = Some(menu_idx);
            self.hovered_item = self.first_action();
            ctx.request_paint();
            return true;
        }
        false
    }
}
