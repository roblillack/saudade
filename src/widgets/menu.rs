use crate::accel::{Accel, ModifierScheme};
use crate::event::{Event, EventCtx, Key, Modifiers, MouseButton, NamedKey};
use crate::geometry::{Color, Point, Rect};
use crate::include_svg;
use crate::painter::Painter;
use crate::svg::SvgImage;
use crate::theme::Theme;
use crate::widget::{PopupKind, PopupRequest, Widget};
use crate::widgets::mnemonic::{draw_label_with_mnemonic, parse_label};

const BAR_PADDING: i32 = 8;
/// Top inset for the label baseline inside the bar. Tight enough that the
/// 13-pt menu font fits in a 20-px bar without growing it.
const BAR_LABEL_INSET_Y: i32 = 1;
const POPUP_PADDING_X: i32 = 18;
const POPUP_PADDING_Y: i32 = 3;
const ITEM_HEIGHT: i32 = 18;
/// Gap between an item's label and its right-aligned accelerator hint.
const ACCEL_GAP: i32 = 24;
const ITEM_TEXT_INSET_Y: i32 = 1;
const SEPARATOR_HEIGHT: i32 = 6;
const SHADOW_SIZE: i32 = 2;
/// L-shape drop shadow color: a dark gray with no alpha trickery so it
/// renders crisply on every backend.
const SHADOW_COLOR: Color = Color::rgb(0x40, 0x40, 0x40);
/// Footprint of the checkmark drawn in a checked item's left gutter. Kept a few
/// pixels smaller than the label inset (`POPUP_PADDING_X - 4`) and centered in
/// it, so the tick has breathing room on both sides and never crowds the text —
/// and so unchecked / uncheckable menus keep their exact layout.
const CHECK_SIZE: i32 = 9;
/// A tiny nudge applied to the centered checkmark so it reads as aligned with
/// the label rather than sitting a hair high and left of it.
const CHECK_NUDGE_X: i32 = 1;
const CHECK_NUDGE_Y: i32 = 1;

/// The checkmark glyph for a checked item, shared with the [`Checkbox`] widget
/// so both read identically. Baked black is a placeholder, tinted to the item's
/// text color via [`SvgImage::draw_tinted`].
///
/// [`Checkbox`]: crate::widgets::Checkbox
const CHECK: SvgImage = include_svg!("assets/checkbox/check.svg");

/// One entry inside a drop-down [`Menu`].
pub enum MenuItem {
    Action {
        /// Raw label as supplied; may contain `&X` to mark the mnemonic.
        label: String,
        /// Optional keyboard accelerator. Shown right-aligned in the drop-down
        /// (rendered through the bar's [`ModifierScheme`]) *and* live: a
        /// matching chord against the closed bar fires the item directly — see
        /// [`MenuBar::fire_accel`]. [`MenuItem::with_enabled`] gates it like
        /// any other way of firing.
        accel: Option<Accel>,
        callback: Box<dyn FnMut(&mut EventCtx)>,
        /// Optional predicate gating the item, evaluated live each paint / fire.
        /// `None` means always enabled; `Some(f)` greys the item and blocks
        /// firing (mouse and keyboard) and keyboard navigation when `f()` is
        /// false. See [`MenuItem::with_enabled`].
        enabled: Option<Box<dyn Fn() -> bool>>,
        /// Optional predicate evaluated live each paint: when `Some(f)` and
        /// `f()` is true, a checkmark is drawn in the item's left gutter. `None`
        /// is an ordinary (never-checked) item. See [`MenuItem::with_checked`].
        checked: Option<Box<dyn Fn() -> bool>>,
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
            accel: None,
            callback: Box::new(callback),
            enabled: None,
            checked: None,
        }
    }

    /// Attach a keyboard accelerator to an action item: the chord is shown
    /// right-aligned in the drop-down and fires the item when pressed against
    /// the closed bar. Takes an [`Accel`] or its string form (`"Ctrl+R"`,
    /// `"Ctrl+Enter"` — `Ctrl`/`Cmd` both mean the primary role, resolved per
    /// platform). No-op on separators.
    pub fn with_accel(mut self, accel: impl Into<Accel>) -> Self {
        if let MenuItem::Action { accel: slot, .. } = &mut self {
            *slot = Some(accel.into());
        }
        self
    }

    /// Gate the item on a predicate evaluated live (each paint and each attempt
    /// to fire it). A disabled item renders greyed, can't be clicked or
    /// keyboard-selected, and never fires. No-op on separators. The predicate
    /// typically reads shared application state (an `Rc<RefCell<…>>`), letting a
    /// menu built once reflect changing context.
    pub fn with_enabled<F>(mut self, predicate: F) -> Self
    where
        F: Fn() -> bool + 'static,
    {
        if let MenuItem::Action { enabled, .. } = &mut self {
            *enabled = Some(Box::new(predicate));
        }
        self
    }

    /// Mark the item as checkable: when `predicate` evaluates true a checkmark
    /// is drawn in its left gutter, leaving the label where it is. The predicate
    /// is read live each paint, so a menu built once tracks changing state (e.g.
    /// which mode is active). Use this for toggles and radio-style groups. No-op
    /// on separators. The checkmark is purely a display affordance — the item
    /// still fires its callback when picked, regardless of checked state.
    pub fn with_checked<F>(mut self, predicate: F) -> Self
    where
        F: Fn() -> bool + 'static,
    {
        if let MenuItem::Action { checked, .. } = &mut self {
            *checked = Some(Box::new(predicate));
        }
        self
    }

    pub fn separator() -> Self {
        MenuItem::Separator
    }

    fn is_action(&self) -> bool {
        matches!(self, MenuItem::Action { .. })
    }

    /// Whether the item is currently enabled (separators count as enabled but
    /// are never selectable). An action with no predicate is always enabled.
    fn is_enabled(&self) -> bool {
        match self {
            MenuItem::Action {
                enabled: Some(pred),
                ..
            } => pred(),
            _ => true,
        }
    }

    /// Whether the item can be hovered / fired: an action that is also enabled.
    fn is_selectable(&self) -> bool {
        self.is_action() && self.is_enabled()
    }

    /// Whether a checkmark should be drawn for this item right now. Only a
    /// checkable action with a live-true predicate is checked.
    fn is_checked(&self) -> bool {
        matches!(
            self,
            MenuItem::Action {
                checked: Some(pred),
                ..
            } if pred()
        )
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
///
/// Items carrying an [`Accel`] also fire on their chord while the bar is
/// closed — Win 3.1's accelerator table, without the `.rc` file. The chord is
/// matched (and its hint rendered) through the bar's [`ModifierScheme`],
/// which defaults to the build platform's; [`MenuBar::with_scheme`] pins it,
/// e.g. for snapshot tests that must not drift between hosts.
pub struct MenuBar {
    rect: Rect,
    menus: Vec<Menu>,
    scheme: ModifierScheme,
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
            scheme: ModifierScheme::native(),
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

    /// Pin the [`ModifierScheme`] accelerators are matched and rendered with,
    /// overriding the [`ModifierScheme::native`] default. Mainly for tests
    /// that must behave identically on every host.
    pub fn with_scheme(mut self, scheme: ModifierScheme) -> Self {
        self.scheme = scheme;
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
            let w = painter.measure_text(&parsed.display, theme.font_size).w + BAR_PADDING * 2;
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
        let mut max_accel = 0;
        for item in &menu.items {
            if let MenuItem::Action { label, accel, .. } = item {
                let parsed = parse_label(label);
                let w = painter.measure_text(&parsed.display, theme.font_size).w;
                if w > max_label {
                    max_label = w;
                }
                if let Some(accel) = accel {
                    let aw = painter
                        .measure_text(&accel.label(self.scheme), theme.font_size)
                        .w;
                    if aw > max_accel {
                        max_accel = aw;
                    }
                }
            }
        }
        // The accelerator column only widens the popup when some item carries
        // one, so accelerator-free menus keep their original width.
        let accel_col = if max_accel > 0 {
            ACCEL_GAP + max_accel
        } else {
            0
        };
        let width = max_label + accel_col + POPUP_PADDING_X * 2;
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
                return if item.is_selectable() { Some(i) } else { None };
            }
            y += h;
        }
        None
    }

    fn fire(&mut self, item_idx: usize, ctx: &mut EventCtx) {
        let Some(menu_idx) = self.open else { return };
        // A disabled item never fires, even if reached by a stale hover/index.
        if self.menus[menu_idx]
            .items
            .get(item_idx)
            .is_some_and(|item| !item.is_enabled())
        {
            return;
        }
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
                && item.is_enabled()
                && parse_label(label).mnemonic_char == Some(target)
            {
                return Some(i);
            }
        }
        None
    }
}

impl Widget for MenuBar {
    fn bounds(&self) -> Rect {
        self.rect
    }

    fn paint(&mut self, painter: &mut Painter, theme: &Theme) {
        self.rebuild_label_rects(painter, theme);
        self.cache.popup = self.open.map(|idx| self.compute_popup(idx, painter, theme));

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
            draw_label_with_mnemonic(
                painter,
                lx + BAR_PADDING,
                self.rect.y + BAR_LABEL_INSET_Y,
                1,
                &parsed,
                theme.font_size,
                fg,
            );
        }
    }

    fn paint_overlay(&mut self, painter: &mut Painter, theme: &Theme) {
        // The popup lives in a separate top-level window — only draw it when
        // the painter is running *that* popup pass, so neither the main
        // window nor an unrelated popup in the stack (e.g. a dialog) ends up
        // with a duplicate copy in its framebuffer.
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
        // The cache is populated now; the anchor check has to come after so
        // popup_request can report a rect.
        if painter.popup_anchor() != self.popup_request().map(|r| r.rect) {
            return;
        }

        // L-shape drop shadow drawn first so the popup overlays it on the
        // top/left edges.
        painter.fill_rect(
            Rect::new(popup.x + SHADOW_SIZE, popup.bottom(), popup.w, SHADOW_SIZE),
            SHADOW_COLOR,
        );
        painter.fill_rect(
            Rect::new(popup.right(), popup.y + SHADOW_SIZE, SHADOW_SIZE, popup.h),
            SHADOW_COLOR,
        );

        // White interior + thin black border. No raised bevel — Win 3.1
        // drop-downs are flat panels, the bar holds the chrome.
        painter.fill_rect(popup, theme.background);
        painter.stroke_rect(popup, theme.border);

        let mut y = popup.y + POPUP_PADDING_Y;
        for (i, item) in self.menus[menu_idx].items.iter().enumerate() {
            match item {
                MenuItem::Action { label, accel, .. } => {
                    let row = Rect::new(popup.x + 1, y, popup.w - 2, ITEM_HEIGHT);
                    let parsed = parse_label(label);
                    // A disabled item is greyed and never shows the hover band
                    // (it can't be hovered — `hit_item` skips it).
                    let (bg, fg) = if !item.is_enabled() {
                        (theme.background, theme.disabled_text)
                    } else if self.hovered_item == Some(i) {
                        (theme.highlight_bg, theme.highlight_text)
                    } else {
                        (theme.background, theme.text)
                    };
                    painter.fill_rect(row, bg);
                    // A checked item gets a tick centered in the left gutter,
                    // tinted to match the (possibly greyed / highlighted) label
                    // color. It rides inside the existing label inset, so the
                    // text never shifts whether or not the item is checked. A
                    // 1px nudge down/right sits it more squarely against the
                    // label's optical baseline.
                    if item.is_checked() {
                        let gutter = POPUP_PADDING_X - 4;
                        let cx = row.x + (gutter - CHECK_SIZE) / 2 + CHECK_NUDGE_X;
                        let cy = row.y + (ITEM_HEIGHT - CHECK_SIZE) / 2 + CHECK_NUDGE_Y;
                        let check = Rect::new(cx, cy, CHECK_SIZE, CHECK_SIZE);
                        CHECK.draw_tinted(painter, check, fg);
                    }
                    draw_label_with_mnemonic(
                        painter,
                        row.x + POPUP_PADDING_X - 4,
                        row.y + ITEM_TEXT_INSET_Y,
                        0,
                        &parsed,
                        theme.font_size,
                        fg,
                    );
                    // Accelerator hint, right-aligned with the same inset the
                    // label carries on the left.
                    if let Some(accel) = accel {
                        let hint = accel.label(self.scheme);
                        let aw = painter.measure_text(&hint, theme.font_size).w;
                        let ax = row.right() - (POPUP_PADDING_X - 4) - aw;
                        painter.text(ax, row.y + ITEM_TEXT_INSET_Y, &hint, theme.font_size, fg);
                    }
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
                ..
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
                ..
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
            Event::PointerMove { pos }
                if self.open.is_some() => {
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
            Event::KeyDown { key, modifiers } => {
                // Whether the menu was already open *before* handling this key.
                // It matters because firing an item (Enter / a mnemonic) closes
                // the menu in this same dispatch — and we must still swallow that
                // keystroke (see the consume below) so it doesn't also reach the
                // focused widget behind the bar.
                let was_open = self.open.is_some();
                // A closed bar first consults the accelerator table; while a
                // menu is open it owns the keyboard outright, so chords wait.
                // Accelerators outrank mnemonics (Win32 order), though only a
                // chord of exactly Alt+letter could ever contest one.
                if !was_open && self.fire_accel(*key, *modifiers, ctx) {
                    return;
                }
                match key {
                    Key::Named(NamedKey::Escape) if was_open => {
                        self.open = None;
                        self.hovered_item = None;
                        ctx.request_paint();
                    }
                    Key::Named(NamedKey::Down) if was_open => {
                        self.move_selection(1, ctx);
                    }
                    Key::Named(NamedKey::Up) if was_open => {
                        self.move_selection(-1, ctx);
                    }
                    Key::Named(NamedKey::Right) if was_open => {
                        self.switch_top_level(1, ctx);
                    }
                    Key::Named(NamedKey::Left) if was_open => {
                        self.switch_top_level(-1, ctx);
                    }
                    Key::Named(NamedKey::Home) if was_open => {
                        self.hovered_item = self.first_action();
                        ctx.request_paint();
                    }
                    Key::Named(NamedKey::End) if was_open => {
                        self.hovered_item = self.last_action();
                        ctx.request_paint();
                    }
                    Key::Named(NamedKey::Enter) if was_open => {
                        if let Some(item) = self.hovered_item {
                            self.fire(item, ctx);
                            self.open = None;
                            self.hovered_item = None;
                            // Don't let the Enter that fired the item leak into
                            // whatever it opened (its release / trailing text).
                            ctx.swallow_key_until_release();
                            ctx.request_paint();
                        }
                    }
                    Key::Char(ch) => {
                        self.handle_mnemonic(*ch, *modifiers, ctx);
                    }
                    _ => {}
                }
                // An open menu owns the keyboard: swallow the keystroke so it
                // can't *also* act on the focused widget behind the bar — the
                // bug where Enter both fired the menu item and activated the
                // control underneath. We consume when the menu was already open
                // (it captures every key while up, even ones it ignores) or when
                // this key just opened one (a top-level mnemonic). A key pressed
                // against a closed bar that opens nothing falls through.
                if was_open || self.open.is_some() {
                    ctx.consume_event();
                }
            }
            Event::Char { ch, modifiers }
                // Some platforms route mnemonic characters through Char with
                // Alt held; treat the same way. AltGr is excluded so composed
                // characters still reach the focused text widget.
                if modifiers.mnemonic_alt() =>
            {
                let was_open = self.open.is_some();
                self.handle_mnemonic(*ch, *modifiers, ctx);
                if was_open || self.open.is_some() {
                    ctx.consume_event();
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
    /// Try the pressed chord against every item's accelerator: the first
    /// *enabled* match fires directly — without opening its menu — and the
    /// keystroke is consumed and swallowed through its release, so it can't
    /// also reach the focused widget (or leak into whatever the item opens).
    /// A chord whose only matches are disabled items does **not** consume:
    /// it falls through to the focused widget, keeping an inapplicable
    /// accelerator's ordinary meaning intact (e.g. Ctrl+Left staying
    /// word-jump in an editor). Returns whether an item fired.
    fn fire_accel(&mut self, key: Key, modifiers: Modifiers, ctx: &mut EventCtx) -> bool {
        let scheme = self.scheme;
        for menu in &mut self.menus {
            for item in &mut menu.items {
                let MenuItem::Action {
                    accel: Some(accel),
                    enabled,
                    callback,
                    ..
                } = item
                else {
                    continue;
                };
                if !accel.matches(key, modifiers, scheme)
                    || enabled.as_ref().is_some_and(|enabled| !enabled())
                {
                    continue;
                }
                callback(ctx);
                ctx.consume_event();
                ctx.swallow_key_until_release();
                ctx.request_paint();
                return true;
            }
        }
        false
    }

    /// Index of the first action item in the currently open menu (skipping
    /// separators); `None` if no menu is open or it has no actions.
    fn first_action(&self) -> Option<usize> {
        let menu_idx = self.open?;
        self.menus[menu_idx]
            .items
            .iter()
            .position(|item| item.is_selectable())
    }

    fn last_action(&self) -> Option<usize> {
        let menu_idx = self.open?;
        self.menus[menu_idx]
            .items
            .iter()
            .enumerate()
            .rev()
            .find_map(|(i, item)| item.is_selectable().then_some(i))
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
            .filter(|(_, item)| item.is_selectable())
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
                // The letter that fired the item must not also reach whatever
                // the item just opened (e.g. a dialog's focused field).
                ctx.swallow_key_until_release();
                ctx.request_paint();
                return true;
            }
            return false;
        }
        // Closed bar: only respond to (left) Alt+letter to open a top-level
        // menu. AltGr is excluded so it stays free for composing characters.
        // Keyboard-opened menus pre-highlight the first action so the user
        // can hit Enter or use arrows immediately.
        if modifiers.mnemonic_alt()
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;
    use std::rc::Rc;

    fn bar_with_help(fired: Rc<Cell<bool>>) -> MenuBar {
        MenuBar::new(Rect::new(0, 0, 200, 20)).add_menu(Menu::new(
            "&Help",
            vec![MenuItem::action("&About", move |_| fired.set(true))],
        ))
    }

    fn keydown(key: Key) -> Event {
        Event::KeyDown {
            key,
            modifiers: Modifiers::default(),
        }
    }

    #[test]
    fn open_menu_consumes_the_enter_that_fires_an_item() {
        let fired = Rc::new(Cell::new(false));
        let mut bar = bar_with_help(fired.clone());
        bar.open(0);
        // Highlight the first item …
        let mut ctx = EventCtx::new();
        bar.event(&keydown(Key::Named(NamedKey::Down)), &mut ctx);
        // … then Enter fires it.
        let mut ctx = EventCtx::new();
        bar.event(&keydown(Key::Named(NamedKey::Enter)), &mut ctx);
        assert!(fired.get(), "Enter fires the highlighted item");
        assert!(
            ctx.is_consumed(),
            "an open menu must swallow the Enter so it can't also reach the focused widget"
        );
    }

    #[test]
    fn open_menu_swallows_keys_it_ignores() {
        // Space isn't a menu command, but while the menu is up it must not leak
        // to the focused widget behind the bar either.
        let mut bar = bar_with_help(Rc::new(Cell::new(false)));
        bar.open(0);
        let mut ctx = EventCtx::new();
        bar.event(&keydown(Key::Named(NamedKey::Space)), &mut ctx);
        assert!(ctx.is_consumed());
    }

    #[test]
    fn closed_bar_lets_plain_keys_through() {
        let mut bar = bar_with_help(Rc::new(Cell::new(false)));
        let mut ctx = EventCtx::new();
        bar.event(&keydown(Key::Char('x')), &mut ctx);
        assert!(
            !ctx.is_consumed(),
            "a plain key against a closed bar must reach the focused widget"
        );
    }

    #[test]
    fn firing_an_item_by_mnemonic_swallows_the_key_until_release() {
        let fired = Rc::new(Cell::new(false));
        let mut bar = bar_with_help(fired.clone());
        bar.open(0);
        // While the menu is open, the item's letter fires it.
        let mut ctx = EventCtx::new();
        bar.event(&keydown(Key::Char('a')), &mut ctx); // &About
        assert!(fired.get(), "the mnemonic fires the item");
        assert!(
            ctx.swallow_key,
            "the firing keystroke is swallowed through its release so it can't \
             leak into whatever the item opens (e.g. a dialog's focused field)"
        );
    }

    #[test]
    fn firing_an_item_by_enter_swallows_the_key_until_release() {
        let fired = Rc::new(Cell::new(false));
        let mut bar = bar_with_help(fired.clone());
        bar.open(0);
        let mut ctx = EventCtx::new();
        bar.event(&keydown(Key::Named(NamedKey::Down)), &mut ctx); // highlight About
        let mut ctx = EventCtx::new();
        bar.event(&keydown(Key::Named(NamedKey::Enter)), &mut ctx);
        assert!(fired.get());
        assert!(
            ctx.swallow_key,
            "Enter that fires an item is swallowed until release"
        );
    }

    #[test]
    fn merely_opening_a_menu_does_not_swallow_until_release() {
        // Opening (vs. firing) must not swallow: otherwise a held key's
        // autorepeat couldn't drive arrow-key menu navigation.
        let mut bar = bar_with_help(Rc::new(Cell::new(false)));
        let alt = Modifiers {
            alt: true,
            ..Modifiers::default()
        };
        let mut ctx = EventCtx::new();
        bar.event(
            &Event::KeyDown {
                key: Key::Char('h'),
                modifiers: alt,
            },
            &mut ctx,
        );
        assert!(bar.open.is_some(), "Alt+H opens the Help menu");
        assert!(
            !ctx.swallow_key,
            "opening a menu must not swallow the key press"
        );
    }

    #[test]
    fn disabled_item_does_not_fire_by_mnemonic() {
        let fired = Rc::new(Cell::new(false));
        let f = fired.clone();
        let mut bar = MenuBar::new(Rect::new(0, 0, 200, 20)).add_menu(Menu::new(
            "&Edit",
            vec![MenuItem::action("&Paste", move |_| f.set(true)).with_enabled(|| false)],
        ));
        bar.open(0);
        let mut ctx = EventCtx::new();
        bar.event(&keydown(Key::Char('p')), &mut ctx); // &Paste mnemonic
        assert!(!fired.get(), "a disabled item must not fire");
    }

    #[test]
    fn enabled_predicate_item_fires_by_mnemonic() {
        let fired = Rc::new(Cell::new(false));
        let f = fired.clone();
        let mut bar = MenuBar::new(Rect::new(0, 0, 200, 20)).add_menu(Menu::new(
            "&Edit",
            vec![MenuItem::action("&Paste", move |_| f.set(true)).with_enabled(|| true)],
        ));
        bar.open(0);
        let mut ctx = EventCtx::new();
        bar.event(&keydown(Key::Char('p')), &mut ctx);
        assert!(fired.get(), "an enabled item fires normally");
    }

    #[test]
    fn down_nav_skips_a_disabled_item() {
        // A disabled first item, an enabled second one: Down lands on the
        // enabled item, never highlighting the disabled one.
        let mut bar = MenuBar::new(Rect::new(0, 0, 200, 20)).add_menu(Menu::new(
            "&Edit",
            vec![
                MenuItem::action("&Cut", |_| {}).with_enabled(|| false),
                MenuItem::action("C&opy", |_| {}),
            ],
        ));
        bar.open(0);
        let mut ctx = EventCtx::new();
        bar.event(&keydown(Key::Named(NamedKey::Down)), &mut ctx);
        assert_eq!(
            bar.hovered_item,
            Some(1),
            "Down skips the disabled first item"
        );
    }

    #[test]
    fn checked_predicate_tracks_live_state() {
        // A checkable item reflects its predicate live: flipping the shared cell
        // flips whether a checkmark would be drawn, without rebuilding the menu.
        let on = Rc::new(Cell::new(false));
        let c = on.clone();
        let item = MenuItem::action("&Commit Changes", |_| {}).with_checked(move || c.get());
        assert!(!item.is_checked(), "starts unchecked");
        on.set(true);
        assert!(
            item.is_checked(),
            "follows the predicate once it turns true"
        );
        // A checkmark is display-only: it never blocks selection / firing.
        assert!(item.is_selectable());
    }

    #[test]
    fn plain_action_is_never_checked() {
        assert!(!MenuItem::action("&Reload", |_| {}).is_checked());
    }

    fn chord(control: bool, logo: bool, ch: char) -> Event {
        Event::KeyDown {
            key: Key::Char(ch),
            modifiers: Modifiers {
                control,
                logo,
                ..Modifiers::default()
            },
        }
    }

    /// A bar with one File ▸ Rescan item bound to Ctrl+R, pinned to the PC
    /// scheme so the tests behave identically on a macOS host.
    fn bar_with_accel(fired: Rc<Cell<bool>>, enabled: Option<bool>) -> MenuBar {
        let mut item = MenuItem::action("&Rescan", move |_| fired.set(true)).with_accel("Ctrl+R");
        if let Some(enabled) = enabled {
            item = item.with_enabled(move || enabled);
        }
        MenuBar::new(Rect::new(0, 0, 200, 20))
            .with_scheme(ModifierScheme::Pc)
            .add_menu(Menu::new("&File", vec![item]))
    }

    #[test]
    fn accel_fires_the_matching_item_against_the_closed_bar() {
        let fired = Rc::new(Cell::new(false));
        let mut bar = bar_with_accel(fired.clone(), None);
        let mut ctx = EventCtx::new();
        bar.event(&chord(true, false, 'r'), &mut ctx);
        assert!(
            fired.get(),
            "Ctrl+R fires the item without opening the menu"
        );
        assert!(bar.open.is_none(), "the menu stays closed");
        assert!(
            ctx.is_consumed(),
            "the chord must not also reach the focused widget"
        );
        assert!(
            ctx.swallow_key,
            "the press is swallowed through its release so trailing Char/KeyUp \
             events can't leak into whatever the item opened"
        );
    }

    #[test]
    fn accel_on_a_disabled_item_falls_through_unconsumed() {
        let fired = Rc::new(Cell::new(false));
        let mut bar = bar_with_accel(fired.clone(), Some(false));
        let mut ctx = EventCtx::new();
        bar.event(&chord(true, false, 'r'), &mut ctx);
        assert!(!fired.get(), "a disabled item must not fire on its accel");
        assert!(
            !ctx.is_consumed(),
            "an inapplicable chord falls through to the focused widget"
        );
    }

    #[test]
    fn accel_skips_a_disabled_match_in_favor_of_an_enabled_one() {
        // Two items share the chord; the disabled one must not shadow the
        // enabled one further down.
        let fired = Rc::new(Cell::new(false));
        let f = fired.clone();
        let mut bar = MenuBar::new(Rect::new(0, 0, 200, 20))
            .with_scheme(ModifierScheme::Pc)
            .add_menu(Menu::new(
                "&Edit",
                vec![
                    MenuItem::action("&Off", |_| {})
                        .with_accel("Ctrl+R")
                        .with_enabled(|| false),
                    MenuItem::action("O&n", move |_| f.set(true)).with_accel("Ctrl+R"),
                ],
            ));
        let mut ctx = EventCtx::new();
        bar.event(&chord(true, false, 'r'), &mut ctx);
        assert!(fired.get());
    }

    #[test]
    fn accels_wait_while_a_menu_is_open() {
        // An open menu owns the keyboard: the chord neither fires the item nor
        // leaks through — it's simply swallowed like any other ignored key.
        // The accel letter deliberately differs from the item's mnemonic,
        // which *would* fire on its bare letter while the menu is up.
        let fired = Rc::new(Cell::new(false));
        let f = fired.clone();
        let mut bar = MenuBar::new(Rect::new(0, 0, 200, 20))
            .with_scheme(ModifierScheme::Pc)
            .add_menu(Menu::new(
                "&File",
                vec![MenuItem::action("&Rescan", move |_| f.set(true)).with_accel("Ctrl+T")],
            ));
        bar.open(0);
        let mut ctx = EventCtx::new();
        bar.event(&chord(true, false, 't'), &mut ctx);
        assert!(!fired.get(), "no accel firing while a menu is up");
        assert!(
            ctx.is_consumed(),
            "…but the open menu still swallows the key"
        );
    }

    #[test]
    fn mac_scheme_binds_the_logo_key_as_primary() {
        let fired = Rc::new(Cell::new(false));
        let f = fired.clone();
        let mut bar = MenuBar::new(Rect::new(0, 0, 200, 20))
            .with_scheme(ModifierScheme::Mac)
            .add_menu(Menu::new(
                "&File",
                vec![MenuItem::action("&Rescan", move |_| f.set(true)).with_accel("Ctrl+R")],
            ));
        // On a Mac the primary role is ⌘ — the physical Ctrl key is the
        // secondary role and must not fire a primary chord.
        let mut ctx = EventCtx::new();
        bar.event(&chord(true, false, 'r'), &mut ctx);
        assert!(!fired.get(), "Ctrl+R is not the primary chord on a Mac");
        assert!(!ctx.is_consumed());
        let mut ctx = EventCtx::new();
        bar.event(&chord(false, true, 'r'), &mut ctx);
        assert!(fired.get(), "Cmd+R fires it");
    }

    #[test]
    fn alt_mnemonic_opening_a_menu_is_consumed() {
        let mut bar = bar_with_help(Rc::new(Cell::new(false)));
        let alt = Modifiers {
            alt: true,
            ..Modifiers::default()
        };
        let mut ctx = EventCtx::new();
        bar.event(
            &Event::KeyDown {
                key: Key::Char('h'),
                modifiers: alt,
            },
            &mut ctx,
        );
        assert!(bar.open.is_some(), "Alt+H opens the Help menu");
        assert!(
            ctx.is_consumed(),
            "and that mnemonic keystroke is swallowed"
        );
    }
}
