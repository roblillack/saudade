//! A right-click menu: the same panel a [`MenuBar`](crate::widgets::MenuBar)
//! drops open, anchored at a point instead of hanging off a bar label.

use crate::accel::ModifierScheme;
use crate::event::{Event, EventCtx, Key, MouseButton, NamedKey};
use crate::geometry::{Point, Rect};
use crate::painter::Painter;
use crate::theme::Theme;
use crate::widget::{PopupKind, PopupRequest, Widget};
use crate::widgets::menu::MenuItem;
use crate::widgets::menu_popup::{self, ITEM_HEIGHT, MenuPopup, POPUP_PADDING_Y};

/// The shortest panel worth drawing: one row plus the padding at both ends. A
/// region too short for even that gets this and clips, rather than a menu with
/// nothing in it.
const MIN_HEIGHT: i32 = POPUP_PADDING_Y * 2 + ITEM_HEIGHT;

/// A pop-up menu anchored at a point — the classic right-click menu.
///
/// It shows the same black-on-white panel a [`MenuBar`](crate::widgets::MenuBar)
/// drop-down does, built from the same [`MenuItem`]s: mnemonics marked with `&`,
/// accelerator hints in a right-aligned column, checkmarks, separators, and
/// `with_enabled` predicates greying rows out. Only the way it opens differs —
/// there is no bar, so the menu is opened by the widget that owns it, wherever
/// the user asked for it.
///
/// Like a menu bar's drop-downs the panel lives in its own borderless top-level
/// window, so it is never clipped by the widget it belongs to.
///
/// ```no_run
/// use saudade::*;
///
/// let mut menu = ContextMenu::new().with_items(vec![
///     MenuItem::action("&Edit", |_| { /* … */ }),
///     MenuItem::action("&Duplicate", |_| { /* … */ }),
///     MenuItem::separator(),
///     MenuItem::action("De&lete", |_| { /* … */ }),
/// ]);
///
/// // …from the owning widget's `event`, on a right-press:
/// # let pos = Point::new(0, 0);
/// menu.open_at(pos);
/// ```
///
/// **Owning one.** A `ContextMenu` takes no space and is not placed by layout:
/// hold it in the widget whose rows it acts on, forward `paint`,
/// `paint_overlay`, `event` and `collect_popups` to it, and report
/// [`captures_pointer`] while it is open so the pointer keeps reaching it once
/// it leaves those rows. Added to a [`Container`](crate::widgets::Container)
/// instead, the container's own routing does all of that.
///
/// **Building items per opening.** A context menu usually acts on whatever was
/// right-clicked, so its items are built fresh each time: call
/// [`set_items`](Self::set_items) before [`open_at`](Self::open_at). Labels are
/// mnemonic-parsed, so an `&` in a name the app did not write (a file, a
/// project) has to be doubled — `label.replace('&', "&&")` — or it disappears
/// and underlines the letter after it.
///
/// **Picking an item** runs its callback, which gets only an
/// [`EventCtx`] — nothing that can reach back into the owning widget. Where the
/// action needs more than the shared application state a closure can capture,
/// have the callback record what was picked (an `Rc<Cell<…>>`) and act on it
/// after the event returns.
///
/// **Mouse behavior.** Move the pointer to highlight rows; a left-click fires
/// one and closes the menu. A press outside dismisses it — and a *right*-press
/// outside is left unconsumed, so it can open a fresh menu on whatever it
/// landed on in one gesture.
///
/// **Keyboard navigation** (active while the menu is open):
///
/// | Key        | Action                                              |
/// |------------|-----------------------------------------------------|
/// | ↑ / ↓      | move the highlight, skipping separators             |
/// | Home / End | first / last item                                   |
/// | Enter      | fire the highlighted item                           |
/// | letter     | fire the item whose mnemonic it is                  |
/// | Esc        | dismiss the menu                                    |
///
/// While the menu is up it owns the keyboard: no keystroke reaches the widget
/// underneath, the way an open drop-down's doesn't.
///
/// **Placement.** The panel opens down and to the right of the anchor, at its
/// full size — the window it lives in is not the app's, so it may hang off the
/// main window's edges, which is what a menu right-clicked near a border should
/// do. [`open_within`](Self::open_within) instead keeps it inside a rect the
/// caller names: there it flips to the other side of the anchor rather than
/// crossing an edge, and a menu taller than that rect is capped to it and
/// scrolls — with the wheel, the arrow keys, or a click on either end's arrow,
/// which also mark what is off-panel.
///
/// [`captures_pointer`]: Widget::captures_pointer
pub struct ContextMenu {
    items: Vec<MenuItem>,
    scheme: ModifierScheme,
    open: bool,
    /// Where the menu was asked to appear, in the root widget's coordinates.
    anchor: Point,
    /// The area the panel is kept inside, from [`Self::open_within`]. Empty for
    /// the [`Self::open_at`] default: the panel lands on the anchor at its full
    /// size, hanging off the main window if that is where it falls.
    region: Rect,
    /// The placed panel, measured on the first paint after opening — the labels
    /// cannot be measured without a painter. `None` until then, which is also
    /// what keeps `popup_request` quiet: there is nothing to anchor a popup
    /// window to yet.
    rect: Option<Rect>,
    /// First item drawn, for a panel too short to show them all.
    scroll: usize,
    /// Leftover fractional wheel travel, banked between events so a trackpad's
    /// stream of small deltas scrolls at the same rate as a wheel's detents.
    wheel_accum: f32,
    hovered: Option<usize>,
}

impl Default for ContextMenu {
    fn default() -> Self {
        Self::new()
    }
}

impl ContextMenu {
    pub fn new() -> Self {
        Self {
            items: Vec::new(),
            scheme: ModifierScheme::native(),
            open: false,
            anchor: Point::new(0, 0),
            region: Rect::new(0, 0, 0, 0),
            rect: None,
            scroll: 0,
            wheel_accum: 0.0,
            hovered: None,
        }
    }

    /// Fill in the menu up front, for a menu whose entries never change.
    pub fn with_items(mut self, items: Vec<MenuItem>) -> Self {
        self.set_items(items);
        self
    }

    /// Pin the [`ModifierScheme`] accelerator hints are rendered with,
    /// overriding the [`ModifierScheme::native`] default. Mainly for tests that
    /// must behave identically on every host.
    pub fn with_scheme(mut self, scheme: ModifierScheme) -> Self {
        self.scheme = scheme;
        self
    }

    /// Replace every item. Call this before [`Self::open_at`] to build a menu
    /// around whatever the user just right-clicked.
    pub fn set_items(&mut self, items: Vec<MenuItem>) {
        self.items = items;
        self.scroll = 0;
        self.hovered = None;
        self.rect = None;
    }

    pub fn is_open(&self) -> bool {
        self.open
    }

    /// Open the menu with its top-left corner at `anchor`, in the root widget's
    /// coordinate space — the space pointer events arrive in.
    ///
    /// The panel is placed at its full size wherever it was asked for: it has a
    /// top-level window of its own, so it is free to hang off the main window's
    /// edges the way a right-click menu near a window border should. Use
    /// [`open_within`](Self::open_within) to keep it inside something instead.
    pub fn open_at(&mut self, anchor: Point) {
        self.open_within(anchor, Rect::new(0, 0, 0, 0));
    }

    /// Open at `anchor`, but keep the panel inside `region` — a pane it should
    /// not leave, or a screen rect the app knows and the widget doesn't. The
    /// panel flips to the other side of the anchor rather than crossing an edge
    /// of the region, and a menu taller than the region is capped to it and
    /// scrolls. An empty `region` constrains nothing, i.e. behaves like
    /// [`open_at`](Self::open_at).
    pub fn open_within(&mut self, anchor: Point, region: Rect) {
        self.open = true;
        self.anchor = anchor;
        self.region = region;
        self.rect = None;
        self.scroll = 0;
        self.wheel_accum = 0.0;
        self.hovered = None;
    }

    /// Dismiss the menu. The items stay, so a menu built once can be reopened.
    pub fn close(&mut self) {
        self.open = false;
        self.rect = None;
        self.hovered = None;
    }

    fn popup(&self) -> MenuPopup<'_> {
        MenuPopup::new(&self.items, self.scheme)
    }

    /// Measure the labels and settle where the panel sits. Runs on every paint
    /// of the main pass, and always against the region the menu was opened with
    /// — a window resized under an open menu doesn't move it.
    fn place(&mut self, painter: &Painter, theme: &Theme) {
        let natural = self.popup().measure(painter, theme);
        let mut rect = Rect::new(self.anchor.x, self.anchor.y, natural.w, natural.h);
        if self.region.w > 0 && self.region.h > 0 {
            rect.w = rect.w.min(self.region.w);
            rect.h = rect.h.min(self.region.h.max(MIN_HEIGHT));
            // Classic placement: down and to the right, flipping to the other
            // side of the anchor when there is no room — and only when the flip
            // actually fits, since a panel wider or taller than the space on
            // either side is better off pinned to the far edge than hanging off
            // the near one.
            if rect.right() > self.region.right() {
                rect.x = if self.anchor.x - rect.w >= self.region.x {
                    self.anchor.x - rect.w
                } else {
                    (self.region.right() - rect.w).max(self.region.x)
                };
            }
            if rect.bottom() > self.region.bottom() {
                rect.y = if self.anchor.y - rect.h >= self.region.y {
                    self.anchor.y - rect.h
                } else {
                    (self.region.bottom() - rect.h).max(self.region.y)
                };
            }
        }
        self.rect = Some(rect);
        self.scroll = self.scroll.min(self.popup().max_scroll(rect.h));
    }

    /// Fire item `idx` and close. The menu is closed *first*, so an item that
    /// opens something of its own — another menu, a dialog — isn't torn down
    /// again on the way out.
    fn fire(&mut self, idx: usize, ctx: &mut EventCtx) {
        // A disabled item never fires, even if reached by a stale index.
        if self.items.get(idx).is_some_and(|item| !item.is_enabled()) {
            return;
        }
        self.close();
        if let Some(MenuItem::Action { callback, .. }) = self.items.get_mut(idx) {
            callback(ctx);
        }
        ctx.request_paint();
    }

    /// Scroll so the highlighted row is on the panel, after the keyboard moved
    /// it past either end of a scrolled menu.
    fn reveal(&mut self, idx: usize) {
        let Some(rect) = self.rect else { return };
        if idx < self.scroll {
            self.scroll = idx;
            return;
        }
        // Walk the offset forward until `idx` falls inside the visible window:
        // rows are not all the same height, so there is no arithmetic for it.
        while self.scroll < self.popup().max_scroll(rect.h)
            && idx >= self.scroll + self.popup().rows(rect.h, self.scroll).count
        {
            self.scroll += 1;
        }
    }

    fn move_highlight(&mut self, delta: i32, ctx: &mut EventCtx) {
        let Some(next) = self.popup().step(self.hovered, delta) else {
            return;
        };
        self.hovered = Some(next);
        self.reveal(next);
        ctx.request_paint();
    }

    fn jump_highlight(&mut self, to: Option<usize>, ctx: &mut EventCtx) {
        let Some(idx) = to else { return };
        self.hovered = Some(idx);
        self.reveal(idx);
        ctx.request_paint();
    }

    /// How far a press at `pos` scrolls, if it landed on one of a clipped
    /// panel's arrow strips: a page's worth in that direction, the way clicking
    /// a scrollbar's gutter moves.
    fn arrow_at(&self, pos: Point) -> Option<f32> {
        let rect = self.rect?;
        let (up, down) = self.popup().arrow_strips(rect, self.scroll);
        let page = self.popup().rows(rect.h, self.scroll).count.max(1) as f32;
        if up.is_some_and(|strip| strip.contains(pos)) {
            return Some(-page);
        }
        down.is_some_and(|strip| strip.contains(pos))
            .then_some(page)
    }

    fn scroll_by(&mut self, lines: f32, ctx: &mut EventCtx) {
        let Some(rect) = self.rect else { return };
        self.wheel_accum += lines;
        let whole = self.wheel_accum.trunc();
        self.wheel_accum -= whole;
        let step = whole as i32;
        if step == 0 {
            return;
        }
        let max = self.popup().max_scroll(rect.h) as i32;
        let next = (self.scroll as i32 + step).clamp(0, max) as usize;
        if next != self.scroll {
            self.scroll = next;
            ctx.request_paint();
        }
    }

    /// Handle a keystroke aimed at the open menu. Returns `true` when it did
    /// something; either way the caller consumes the key, because an open menu
    /// owns the keyboard.
    fn handle_key(&mut self, key: Key, ctx: &mut EventCtx) -> bool {
        match key {
            Key::Named(NamedKey::Escape) => {
                self.close();
                ctx.request_paint();
            }
            Key::Named(NamedKey::Down) => self.move_highlight(1, ctx),
            Key::Named(NamedKey::Up) => self.move_highlight(-1, ctx),
            Key::Named(NamedKey::Home) => {
                let first = self.popup().first_action();
                self.jump_highlight(first, ctx);
            }
            Key::Named(NamedKey::End) => {
                let last = self.popup().last_action();
                self.jump_highlight(last, ctx);
            }
            Key::Named(NamedKey::Enter) => {
                if let Some(idx) = self.hovered {
                    self.fire(idx, ctx);
                    // Don't let the Enter that fired the item leak into
                    // whatever it opened (its release / trailing text).
                    ctx.swallow_key_until_release();
                }
            }
            Key::Char(ch) => {
                if let Some(idx) = self.popup().mnemonic(ch) {
                    self.fire(idx, ctx);
                    ctx.swallow_key_until_release();
                } else {
                    return false;
                }
            }
            _ => return false,
        }
        true
    }
}

impl Widget for ContextMenu {
    /// The open panel, or an empty rect at the anchor while closed — a context
    /// menu occupies nothing when it isn't showing.
    fn bounds(&self) -> Rect {
        self.rect
            .filter(|_| self.open)
            .unwrap_or(Rect::new(self.anchor.x, self.anchor.y, 0, 0))
    }

    /// The panel itself is drawn in `paint_overlay`, into a popup window of its
    /// own. All the main pass does is measure the labels and settle where the
    /// panel goes, so `popup_request` has a rect to open that window at.
    fn paint(&mut self, painter: &mut Painter, theme: &Theme) {
        if self.open && !painter.is_popup_pass() {
            self.place(painter, theme);
        }
    }

    fn paint_overlay(&mut self, painter: &mut Painter, theme: &Theme) {
        if !self.open || !painter.is_popup_pass() {
            return;
        }
        // Nothing has measured the panel yet — do it now rather than skip a
        // frame. (The main pass normally gets there first, via `paint`.)
        if self.rect.is_none() {
            self.place(painter, theme);
        }
        let Some(rect) = self.rect else { return };
        // Only draw into *our* popup window: every other pass in the stack (a
        // dialog, a dropdown opened inside one) would otherwise get a copy of
        // the panel in its framebuffer.
        if painter.popup_anchor() != Some(menu_popup::with_shadow(rect)) {
            return;
        }
        self.popup()
            .paint(painter, theme, rect, self.scroll, self.hovered);
    }

    fn event(&mut self, event: &Event, ctx: &mut EventCtx) {
        if !self.open {
            return;
        }
        let rect = self.rect;
        match event {
            Event::PointerMove { pos } => {
                let hit = rect.and_then(|rect| self.popup().hit(rect, self.scroll, *pos));
                if hit != self.hovered {
                    self.hovered = hit;
                    ctx.request_paint();
                }
                ctx.consume_event();
            }
            Event::Scroll { delta_y, .. } => {
                self.scroll_by(*delta_y, ctx);
                ctx.consume_event();
            }
            Event::PointerDown { pos, button, .. } => {
                ctx.consume_event();
                if !rect.is_some_and(|rect| rect.contains(*pos)) {
                    self.close();
                    ctx.request_paint();
                    // The press is spent on the dismissal — except a
                    // right-press, which is allowed through to open a fresh
                    // menu on whatever it landed on.
                    if *button == MouseButton::Right {
                        ctx.consumed = false;
                    }
                    return;
                }
                if *button != MouseButton::Left {
                    return;
                }
                if let Some(lines) = self.arrow_at(*pos) {
                    self.scroll_by(lines, ctx);
                } else if let Some(idx) = rect.and_then(|r| self.popup().hit(r, self.scroll, *pos))
                {
                    self.fire(idx, ctx);
                }
            }
            // Swallow the release of the press that opened or picked, so it
            // can't also land on whatever is underneath.
            Event::PointerUp { .. } => ctx.consume_event(),
            Event::KeyDown { key, .. } => {
                self.handle_key(*key, ctx);
                // Whether or not the key meant something, an open menu owns
                // the keyboard: it must not *also* reach the widget behind.
                ctx.consume_event();
            }
            Event::Char { .. } => ctx.consume_event(),
            _ => {}
        }
    }

    /// An open menu keeps every pointer event, wherever the cursor goes: that
    /// is what lets a press outside it dismiss it instead of landing on
    /// whatever sits there.
    fn captures_pointer(&self) -> bool {
        self.open
    }

    /// The menu reads the keyboard without ever holding focus, so Escape
    /// closes it however the user got there.
    fn accepts_accelerators(&self) -> bool {
        true
    }

    fn popup_request(&self) -> Option<PopupRequest> {
        if !self.open || self.items.is_empty() {
            return None;
        }
        // `rect` is filled in by the first paint after opening; until then
        // there is nothing measured to anchor a window to.
        Some(PopupRequest {
            rect: menu_popup::with_shadow(self.rect?),
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
    use std::cell::Cell;
    use std::rc::Rc;

    const REGION: Rect = Rect::new(0, 0, 200, 200);

    /// A menu of three actions, the middle one recording that it fired.
    fn menu(fired: Rc<Cell<bool>>) -> ContextMenu {
        ContextMenu::new().with_items(vec![
            MenuItem::action("&Edit", |_| {}),
            MenuItem::action("&Duplicate", move |_| fired.set(true)),
            MenuItem::separator(),
            MenuItem::action("De&lete", |_| {}),
        ])
    }

    /// Paint the menu through the mock backend so its labels get measured and
    /// the panel is placed — everything positional needs this first.
    fn place(menu: &mut ContextMenu) {
        MockBackend::new(REGION.w, REGION.h).render(menu);
    }

    fn keydown(key: Key) -> Event {
        Event::KeyDown {
            key,
            modifiers: Modifiers::default(),
        }
    }

    fn dispatch(menu: &mut ContextMenu, event: &Event) -> EventCtx {
        let mut ctx = EventCtx::new();
        menu.event(event, &mut ctx);
        ctx
    }

    fn press(menu: &mut ContextMenu, pos: Point, button: MouseButton) -> EventCtx {
        dispatch(
            menu,
            &Event::PointerDown {
                pos,
                button,
                modifiers: Modifiers::default(),
            },
        )
    }

    /// Centre of the `idx`-th row of the placed panel. Rows are not all the
    /// same height — a separator is shorter — so the row's own height is what
    /// the midpoint is taken from.
    fn row_center(menu: &ContextMenu, idx: usize) -> Point {
        let rect = menu.rect.expect("the menu must be placed first");
        let mut y = rect.y + POPUP_PADDING_Y;
        for item in menu.items.iter().take(idx) {
            y += item.height();
        }
        Point::new(rect.x + rect.w / 2, y + menu.items[idx].height() / 2)
    }

    #[test]
    fn a_closed_menu_asks_for_no_popup_window() {
        let mut menu = menu(Rc::new(Cell::new(false)));
        place(&mut menu);
        assert!(!menu.is_open());
        assert!(menu.popup_request().is_none());
        assert!(!menu.captures_pointer());
    }

    #[test]
    fn opening_asks_for_a_popup_window_at_the_anchor() {
        let mut menu = menu(Rc::new(Cell::new(false)));
        menu.open_within(Point::new(20, 30), REGION);
        // Nothing is measured before the first paint, so there is nothing to
        // anchor a window to yet.
        assert!(menu.popup_request().is_none());
        place(&mut menu);
        let req = menu.popup_request().expect("a placed menu wants a window");
        assert_eq!((req.rect.x, req.rect.y), (20, 30));
        assert_eq!(req.kind, PopupKind::Popup);
        assert!(menu.captures_pointer(), "an open menu holds the pointer");
    }

    #[test]
    fn clicking_a_row_fires_it_and_closes() {
        let fired = Rc::new(Cell::new(false));
        let mut menu = menu(fired.clone());
        menu.open_at(Point::new(10, 10));
        place(&mut menu);
        let row = row_center(&menu, 1);
        let ctx = press(&mut menu, row, MouseButton::Left);
        assert!(fired.get(), "the row under the pointer fires");
        assert!(!menu.is_open(), "and the menu closes behind it");
        assert!(ctx.is_consumed());
    }

    #[test]
    fn clicking_a_separator_does_nothing() {
        let fired = Rc::new(Cell::new(false));
        let mut menu = menu(fired.clone());
        menu.open_at(Point::new(10, 10));
        place(&mut menu);
        let row = row_center(&menu, 2);
        press(&mut menu, row, MouseButton::Left);
        assert!(!fired.get());
        assert!(menu.is_open(), "a separator is not a pick");
    }

    #[test]
    fn a_press_outside_dismisses_the_menu() {
        let mut menu = menu(Rc::new(Cell::new(false)));
        menu.open_at(Point::new(10, 10));
        place(&mut menu);
        let outside = Point::new(190, 190);
        let ctx = press(&mut menu, outside, MouseButton::Left);
        assert!(!menu.is_open());
        assert!(
            ctx.is_consumed(),
            "the press is spent on the dismissal, not on what was underneath"
        );
    }

    #[test]
    fn a_right_press_outside_falls_through_so_it_can_reopen() {
        let mut menu = menu(Rc::new(Cell::new(false)));
        menu.open_at(Point::new(10, 10));
        place(&mut menu);
        let ctx = press(&mut menu, Point::new(190, 190), MouseButton::Right);
        assert!(!menu.is_open(), "the old menu still goes away");
        assert!(
            !ctx.is_consumed(),
            "…but the press reaches the row underneath, which opens a new menu"
        );
    }

    #[test]
    fn escape_closes_the_menu_and_is_swallowed() {
        let mut menu = menu(Rc::new(Cell::new(false)));
        menu.open_at(Point::new(10, 10));
        place(&mut menu);
        let ctx = dispatch(&mut menu, &keydown(Key::Named(NamedKey::Escape)));
        assert!(!menu.is_open());
        assert!(ctx.is_consumed());
        assert!(menu.popup_request().is_none(), "the window goes with it");
    }

    #[test]
    fn an_open_menu_swallows_keys_it_ignores() {
        // Space isn't a menu command, but while the menu is up it must not leak
        // to the widget behind it either.
        let mut menu = menu(Rc::new(Cell::new(false)));
        menu.open_at(Point::new(10, 10));
        let ctx = dispatch(&mut menu, &keydown(Key::Named(NamedKey::Space)));
        assert!(ctx.is_consumed());
        assert!(menu.is_open());
    }

    #[test]
    fn a_closed_menu_lets_every_key_through() {
        let mut menu = menu(Rc::new(Cell::new(false)));
        let ctx = dispatch(&mut menu, &keydown(Key::Named(NamedKey::Escape)));
        assert!(!ctx.is_consumed());
    }

    #[test]
    fn arrows_skip_the_separator_and_wrap() {
        let mut menu = menu(Rc::new(Cell::new(false)));
        menu.open_at(Point::new(10, 10));
        place(&mut menu);
        assert_eq!(
            menu.hovered, None,
            "a mouse-opened menu starts unhighlighted"
        );
        dispatch(&mut menu, &keydown(Key::Named(NamedKey::Down)));
        assert_eq!(menu.hovered, Some(0));
        dispatch(&mut menu, &keydown(Key::Named(NamedKey::Down)));
        dispatch(&mut menu, &keydown(Key::Named(NamedKey::Down)));
        assert_eq!(menu.hovered, Some(3), "index 2 is the separator");
        dispatch(&mut menu, &keydown(Key::Named(NamedKey::Down)));
        assert_eq!(menu.hovered, Some(0), "and it wraps");
        dispatch(&mut menu, &keydown(Key::Named(NamedKey::Up)));
        assert_eq!(menu.hovered, Some(3));
    }

    #[test]
    fn enter_fires_the_highlighted_item() {
        let fired = Rc::new(Cell::new(false));
        let mut menu = menu(fired.clone());
        menu.open_at(Point::new(10, 10));
        place(&mut menu);
        dispatch(&mut menu, &keydown(Key::Named(NamedKey::Down)));
        dispatch(&mut menu, &keydown(Key::Named(NamedKey::Down)));
        let ctx = dispatch(&mut menu, &keydown(Key::Named(NamedKey::Enter)));
        assert!(fired.get());
        assert!(!menu.is_open());
        assert!(
            ctx.swallow_key,
            "the Enter that fired must not leak into whatever the item opened"
        );
    }

    #[test]
    fn a_mnemonic_fires_its_item() {
        let fired = Rc::new(Cell::new(false));
        let mut menu = menu(fired.clone());
        menu.open_at(Point::new(10, 10));
        place(&mut menu);
        let ctx = dispatch(&mut menu, &keydown(Key::Char('d'))); // &Duplicate
        assert!(fired.get());
        assert!(ctx.swallow_key);
    }

    #[test]
    fn a_disabled_item_never_fires() {
        let fired = Rc::new(Cell::new(false));
        let f = fired.clone();
        let mut menu = ContextMenu::new().with_items(vec![
            MenuItem::action("&Paste", move |_| f.set(true)).with_enabled(|| false),
        ]);
        menu.open_at(Point::new(10, 10));
        place(&mut menu);
        let row = row_center(&menu, 0);
        press(&mut menu, row, MouseButton::Left);
        assert!(!fired.get(), "a click on a greyed row is not a pick");
        dispatch(&mut menu, &keydown(Key::Char('p')));
        assert!(!fired.get(), "nor is its mnemonic");
    }

    #[test]
    fn the_panel_flips_left_and_up_at_the_far_corner() {
        let mut menu = menu(Rc::new(Cell::new(false)));
        // A right-click in the bottom-right corner: the panel has to open the
        // other way to stay on screen.
        let anchor = Point::new(REGION.right() - 2, REGION.bottom() - 2);
        menu.open_within(anchor, REGION);
        place(&mut menu);
        let rect = menu.rect.unwrap();
        assert_eq!(rect.right(), anchor.x, "flipped to the left of the anchor");
        assert_eq!(rect.bottom(), anchor.y, "and above it");
    }

    #[test]
    fn an_unconstrained_menu_lands_on_the_anchor_at_its_full_size() {
        let mut menu = menu(Rc::new(Cell::new(false)));
        // Right at the far corner of the area that was rendered: with nothing
        // to keep it inside, the panel opens down and right regardless, into
        // the window of its own that it will be drawn in.
        let anchor = Point::new(REGION.right() - 2, REGION.bottom() - 2);
        menu.open_at(anchor);
        place(&mut menu);
        let rect = menu.rect.unwrap();
        assert_eq!((rect.x, rect.y), (anchor.x, anchor.y));
        assert!(rect.bottom() > REGION.bottom(), "and reaches past the edge");
    }

    /// A menu of `n` plain actions, for the overflow cases.
    fn long_menu(n: usize) -> ContextMenu {
        ContextMenu::new().with_items(
            (0..n)
                .map(|i| MenuItem::action(format!("Item {i}"), |_| {}))
                .collect(),
        )
    }

    /// A region only tall enough for `rows` items.
    fn short_region(rows: i32) -> Rect {
        Rect::new(0, 0, 200, POPUP_PADDING_Y * 2 + rows * ITEM_HEIGHT)
    }

    #[test]
    fn a_menu_taller_than_its_region_is_capped_and_scrolls() {
        let mut menu = long_menu(20);
        let region = short_region(5);
        menu.open_within(Point::new(0, 0), region);
        place(&mut menu);
        let rect = menu.rect.unwrap();
        assert_eq!(rect.h, region.h, "capped to the region");
        assert_eq!(
            menu.popup().rows(rect.h, 0).count,
            4,
            "one row goes to the down arrow"
        );

        // A wheel detent down moves the window; it stops at the last page.
        menu.scroll_by(3.0, &mut EventCtx::new());
        assert_eq!(menu.scroll, 3);
        menu.scroll_by(100.0, &mut EventCtx::new());
        assert_eq!(
            menu.scroll, 16,
            "the end of a 20-item menu, 4 rows of which fit once the up arrow \
             has claimed one"
        );
        menu.scroll_by(-100.0, &mut EventCtx::new());
        assert_eq!(menu.scroll, 0);
    }

    #[test]
    fn keyboard_navigation_scrolls_the_highlight_into_view() {
        let mut menu = long_menu(20);
        menu.open_within(Point::new(0, 0), short_region(5));
        place(&mut menu);
        // End jumps to the last item, which has to be on the panel to be seen.
        dispatch(&mut menu, &keydown(Key::Named(NamedKey::End)));
        assert_eq!(menu.hovered, Some(19));
        assert_eq!(menu.scroll, 16);
        dispatch(&mut menu, &keydown(Key::Named(NamedKey::Home)));
        assert_eq!(menu.hovered, Some(0));
        assert_eq!(menu.scroll, 0);
    }

    #[test]
    fn hit_testing_follows_the_scroll_offset() {
        let mut menu = long_menu(20);
        menu.open_within(Point::new(0, 0), short_region(5));
        place(&mut menu);
        let rect = menu.rect.unwrap();
        let first_row = Point::new(rect.x + 5, rect.y + POPUP_PADDING_Y + ITEM_HEIGHT / 2);
        assert_eq!(menu.popup().hit(rect, 0, first_row), Some(0));
        assert_eq!(menu.popup().hit(rect, 7, first_row), Some(7));
    }
}
