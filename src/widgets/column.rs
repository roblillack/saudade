use crate::event::{Event, EventCtx};
use crate::geometry::{Color, Rect};
use crate::painter::Painter;
use crate::theme::Theme;
use crate::widget::{PopupRequest, Widget};
use crate::widgets::{TabAction, tab_action};

/// Vertical layout container. Each child is given a horizontal slice of the
/// column's bounds: either a *fixed* height it asked for, or it shares the
/// space left after every fixed child has been laid out (a *fill* child).
/// Optional *overlay* children sit on top of everything else — useful for
/// modal dialogs that should float over the menu bar / editor.
///
/// `Column` propagates `layout` to its children whenever its own bounds
/// change, which makes it the building block for windows whose chrome (menu
/// bar, status bar) sits at fixed sizes around a content widget that flexes
/// with the window — exactly what Notepad needs.
///
/// Like `Container`, it handles pointer capture, keyboard focus, accelerator
/// routing, and the overlay paint pass.
pub struct Column {
    bounds: Rect,
    pub background: Option<Color>,
    children: Vec<Child>,
    /// Widgets that live on top of the column's normal layout. They
    /// receive the column's full bounds via `layout`, paint last (so they
    /// appear above siblings), and pre-empt event dispatch whenever they
    /// report `captures_pointer() == true` — the mechanism that makes
    /// modal dialogs actually modal.
    overlays: Vec<Box<dyn Widget>>,
    captured: Option<usize>,
    focused: Option<usize>,
    /// True while the focused child's focus is *visually* suspended because a
    /// menu or a modal overlay is up and owns the keyboard. The focus index in
    /// `focused` is kept so it can be handed straight back when the menu or
    /// overlay goes away; only the child's `set_focused` flag is toggled. See
    /// [`Column::sync_focus_suspend`].
    focus_suspended: bool,
}

struct Child {
    widget: Box<dyn Widget>,
    mode: SizeMode,
}

#[derive(Clone, Copy)]
enum SizeMode {
    Fixed(i32),
    Fill,
}

impl Column {
    pub fn new() -> Self {
        Self {
            bounds: Rect::new(0, 0, 0, 0),
            // Transparent by default (see `Container::new`): the runtime fills
            // the window with `theme.background`, so an opaque white here is
            // redundant. Set one only to override what shows behind the column.
            background: None,
            children: Vec::new(),
            overlays: Vec::new(),
            captured: None,
            focused: None,
            focus_suspended: false,
        }
    }

    pub fn with_background(mut self, color: Color) -> Self {
        self.background = Some(color);
        self
    }

    /// Add a child with a *fixed* logical-pixel height. Width is always the
    /// full column width.
    pub fn add_fixed(mut self, widget: impl Widget + 'static, height: i32) -> Self {
        self.push_fixed(widget, height);
        self
    }

    pub fn push_fixed(&mut self, widget: impl Widget + 'static, height: i32) {
        self.children.push(Child {
            widget: Box::new(widget),
            mode: SizeMode::Fixed(height),
        });
    }

    /// Add a child that fills the leftover height. Multiple fill children
    /// split the remaining space equally.
    pub fn add_fill(mut self, widget: impl Widget + 'static) -> Self {
        self.push_fill(widget);
        self
    }

    pub fn push_fill(&mut self, widget: impl Widget + 'static) {
        self.children.push(Child {
            widget: Box::new(widget),
            mode: SizeMode::Fill,
        });
    }

    /// Add a widget that floats over the column, receives the column's
    /// *full* bounds on layout, and pre-empts event dispatch while it
    /// reports `captures_pointer() == true`. Use this for modal dialogs.
    pub fn add_overlay(mut self, widget: impl Widget + 'static) -> Self {
        self.push_overlay(widget);
        self
    }

    pub fn push_overlay(&mut self, widget: impl Widget + 'static) {
        self.overlays.push(Box::new(widget));
    }

    /// Direct keyboard focus to a specific child by index (its position among
    /// all children — fixed and fill — in add order). Clears any previous
    /// focus and delegates into the child via `focus_first`, so wrapper
    /// widgets pick the right nested leaf. Returns `true` if the index named a
    /// focusable child. Use it to choose a non-default initial focus target
    /// (e.g. focus a content list rather than a leading toolbar field).
    pub fn focus_child(&mut self, index: usize) -> bool {
        if self.children.get(index).map(|c| c.widget.focusable()) != Some(true) {
            return false;
        }
        if let Some(old) = self.focused
            && old != index
            && let Some(c) = self.children.get_mut(old)
        {
            c.widget.set_focused(false);
        }
        let focused = self.children[index].widget.focus_first();
        if focused {
            self.focused = Some(index);
        }
        focused
    }

    fn choose_target(&self, event: &Event) -> Option<usize> {
        if event.is_keyboard() {
            return self.focused;
        }
        if let Some(idx) = self.captured {
            return Some(idx);
        }
        // A child holding an open popup (e.g. a `MenuBar` opened from the
        // keyboard) owns pointer events even though no press ever established
        // capture — otherwise a click would fall through to whatever sits behind
        // the popup that's drawn on top (the icon underneath an open menu).
        if let Some(idx) = self
            .children
            .iter()
            .position(|c| c.widget.accepts_accelerators() && c.widget.captures_pointer())
        {
            return Some(idx);
        }
        let pos = event.position()?;
        (0..self.children.len())
            .rev()
            .find(|&i| self.children[i].widget.bounds().contains(pos))
    }

    /// True while a child menu is open and owns the keyboard.
    fn menu_capturing(&self) -> bool {
        self.children
            .iter()
            .any(|c| c.widget.accepts_accelerators() && c.widget.captures_pointer())
    }

    /// Suspend the focused child's focus visual while a menu or modal overlay
    /// owns the keyboard, and hand it back when they go away — so the content
    /// behind an open menu / dialog reads as unfocused (e.g. a gray rather than
    /// blue selection) and, crucially, *also* drops out of keyboard-handling
    /// paths gated on its `focused` flag. Without this, an Enter that an open
    /// modal handles can — when the dispatcher or the windowing layer routes
    /// it twice (e.g. on X11, where focus across our own pop-up windows is
    /// the WM's call) — also reach the iconbox / list behind the dialog and
    /// reactivate it. The focus *index* is preserved throughout; only the
    /// child's `set_focused` flag is toggled, so the same widget regains
    /// focus the moment the menu / overlay goes away. Idempotent: the
    /// `focus_suspended` latch means each transition fires `set_focused`
    /// exactly once.
    fn sync_focus_suspend(&mut self) {
        let suspended = self.menu_capturing() || self.active_overlay().is_some();
        if suspended == self.focus_suspended {
            return;
        }
        self.focus_suspended = suspended;
        if let Some(idx) = self.focused
            && let Some(child) = self.children.get_mut(idx)
        {
            child.widget.set_focused(!suspended);
        }
    }

    /// Index of the first overlay that's currently asserting pre-emptive
    /// capture (typically: a dialog that's just been shown).
    fn active_overlay(&self) -> Option<usize> {
        self.overlays.iter().position(|o| o.captures_pointer())
    }

    fn change_focus(&mut self, new_focus: Option<usize>, ctx: &mut EventCtx) {
        if new_focus == self.focused {
            return;
        }
        if let Some(old) = self.focused
            && let Some(c) = self.children.get_mut(old)
        {
            c.widget.set_focused(false);
        }
        if let Some(new) = new_focus
            && let Some(c) = self.children.get_mut(new)
        {
            // Use `focus_first` so wrapper widgets that delegate focus to a
            // nested target get a chance to set up the right leaf.
            c.widget.focus_first();
        }
        self.focused = new_focus;
        ctx.request_paint();
    }

    fn focusable_count(&self) -> usize {
        self.children
            .iter()
            .filter(|c| c.widget.focusable())
            .count()
    }

    fn cycle_focus(&mut self, dir: i32, ctx: &mut EventCtx) -> bool {
        let n = self.children.len();
        if n == 0 {
            return false;
        }
        let candidates: Vec<usize> = (0..n)
            .filter(|&i| self.children[i].widget.focusable())
            .collect();
        if candidates.is_empty() {
            return false;
        }
        let next = next_in_cycle(&candidates, self.focused, dir);
        if Some(next) == self.focused {
            return false;
        }
        self.change_focus(Some(next), ctx);
        true
    }
}

fn next_in_cycle(candidates: &[usize], current: Option<usize>, dir: i32) -> usize {
    let n = candidates.len() as i32;
    let cur_pos = current.and_then(|c| candidates.iter().position(|&i| i == c));
    match cur_pos {
        None => {
            if dir > 0 {
                candidates[0]
            } else {
                candidates[(n - 1) as usize]
            }
        }
        Some(p) => {
            let np = ((p as i32) + dir).rem_euclid(n) as usize;
            candidates[np]
        }
    }
}

impl Default for Column {
    fn default() -> Self {
        Self::new()
    }
}

impl Widget for Column {
    fn bounds(&self) -> Rect {
        self.bounds
    }

    fn layout(&mut self, bounds: Rect) {
        self.bounds = bounds;

        let total_fixed: i32 = self
            .children
            .iter()
            .filter_map(|c| match c.mode {
                SizeMode::Fixed(h) => Some(h),
                SizeMode::Fill => None,
            })
            .sum();
        let fill_count = self
            .children
            .iter()
            .filter(|c| matches!(c.mode, SizeMode::Fill))
            .count() as i32;

        let leftover = (bounds.h - total_fixed).max(0);
        let fill_each = if fill_count > 0 {
            leftover / fill_count
        } else {
            0
        };
        // Award any rounding slack to the last fill child so we exactly
        // cover the column's bounds.
        let fill_last_extra = if fill_count > 0 {
            leftover - fill_each * fill_count
        } else {
            0
        };

        let mut y = bounds.y;
        let mut fill_seen = 0;
        for child in &mut self.children {
            let h = match child.mode {
                SizeMode::Fixed(h) => h,
                SizeMode::Fill => {
                    fill_seen += 1;
                    if fill_seen == fill_count {
                        fill_each + fill_last_extra
                    } else {
                        fill_each
                    }
                }
            };
            child.widget.layout(Rect::new(bounds.x, y, bounds.w, h));
            y += h;
        }

        // Overlays float over the whole column, so they receive the
        // column's bounds rather than a slot.
        for overlay in &mut self.overlays {
            overlay.layout(bounds);
        }
    }

    fn paint(&mut self, painter: &mut Painter, theme: &Theme) {
        // Reconcile the focus visual with menu / overlay state just before
        // drawing, so an open menu or modal's keyboard ownership is reflected
        // (suspended content focus) regardless of which event-dispatch path
        // opened or closed it.
        self.sync_focus_suspend();
        if let Some(bg) = self.background {
            painter.fill_rect(self.bounds, bg);
        }
        for child in &mut self.children {
            child.widget.paint(painter, theme);
        }
        for child in &mut self.children {
            child.widget.paint_overlay(painter, theme);
        }
        for overlay in &mut self.overlays {
            overlay.paint(painter, theme);
            overlay.paint_overlay(painter, theme);
        }
    }

    fn paint_overlay(&mut self, painter: &mut Painter, theme: &Theme) {
        for child in &mut self.children {
            child.widget.paint_overlay(painter, theme);
        }
        for overlay in &mut self.overlays {
            overlay.paint_overlay(painter, theme);
        }
    }

    fn event(&mut self, event: &Event, ctx: &mut EventCtx) {
        // Reconcile the focus visual *before* dispatching, so widgets that gate
        // their handlers on `self.focused` (the iconbox's Enter activation, the
        // list's keyboard nav) stop firing the moment a menu / overlay takes
        // over the keyboard — even if the windowing layer happens to leak the
        // same key event back to them after the overlay handled it.
        self.sync_focus_suspend();
        // Modal capture: any overlay that's actively capturing swallows
        // every event before normal dispatch can see it. Returns must
        // happen before any borrow of self.children is taken.
        if let Some(idx) = self.active_overlay() {
            self.overlays[idx].event(event, ctx);
            return;
        }

        if !event.is_keyboard() && event.position().is_none() && self.captured.is_none() {
            for child in &mut self.children {
                child.widget.event(event, ctx);
            }
            return;
        }

        // A focused child that's itself capturing the pointer — an open
        // `Dropdown`, or a `Container` whose open dropdown it forwards — owns
        // the keyboard while it's up. Skip the accelerator pass (so a sibling
        // default button's Enter can't pre-empt it) and Tab cycling; the
        // focused dispatch below still delivers the key.
        let focused_capturing = self
            .focused
            .and_then(|i| self.children.get(i))
            .is_some_and(|c| c.widget.captures_pointer());

        if event.is_keyboard() && !focused_capturing {
            let mut accelerator_blocking = false;
            for (idx, child) in self.children.iter_mut().enumerate() {
                if child.widget.accepts_accelerators() && Some(idx) != self.focused {
                    child.widget.event(event, ctx);
                    if ctx.is_consumed() {
                        return;
                    }
                    if child.widget.captures_pointer() {
                        accelerator_blocking = true;
                    }
                }
            }
            if accelerator_blocking {
                return;
            }

            // Tab / Shift+Tab cycle focus between sibling focusable
            // children before the event reaches the focused widget. The
            // matching `Char('\t')` is swallowed so a single Tab press
            // doesn't move focus twice; when this column has fewer than
            // two focusable children we let both events fall through so
            // a sole `TextEditor` can still receive `'\t'`.
            match tab_action(event) {
                Some(TabAction::Cycle(dir)) if self.cycle_focus(dir, ctx) => {
                    return;
                }
                Some(TabAction::Swallow) if self.focusable_count() >= 2 => return,
                _ => {}
            }
        }

        let Some(idx) = self.choose_target(event) else {
            return;
        };

        let captured_was_set = self.captured == Some(idx);
        {
            let child = &mut self.children[idx];
            child.widget.event(event, ctx);

            if !event.is_keyboard() {
                if child.widget.captures_pointer() {
                    self.captured = Some(idx);
                } else if captured_was_set {
                    self.captured = None;
                }
            }
        }

        if ctx.focus_requested {
            ctx.focus_requested = false;
            self.change_focus(Some(idx), ctx);
        }
        if ctx.focus_released {
            ctx.focus_released = false;
            if self.focused == Some(idx) {
                self.change_focus(None, ctx);
            }
        }
    }

    fn captures_pointer(&self) -> bool {
        self.captured.is_some() || self.active_overlay().is_some()
    }

    fn focusable(&self) -> bool {
        self.children.iter().any(|c| c.widget.focusable())
            || self.overlays.iter().any(|o| o.focusable())
    }

    fn focus_first(&mut self) -> bool {
        for (idx, child) in self.children.iter_mut().enumerate() {
            if child.widget.focus_first() {
                self.focused = Some(idx);
                return true;
            }
        }
        false
    }

    fn popup_request(&self) -> Option<PopupRequest> {
        for overlay in &self.overlays {
            if let Some(req) = overlay.popup_request() {
                return Some(req);
            }
        }
        for child in &self.children {
            if let Some(req) = child.widget.popup_request() {
                return Some(req);
            }
        }
        None
    }

    fn collect_popups(&self, out: &mut Vec<PopupRequest>) {
        // Overlays first (a modal dialog sits over the page), then the page
        // children — so a dropdown opened inside a modal nests after it.
        for overlay in &self.overlays {
            overlay.collect_popups(out);
        }
        for child in &self.children {
            child.widget.collect_popups(out);
        }
    }

    fn wants_ticks(&self) -> bool {
        self.children.iter().any(|c| c.widget.wants_ticks())
            || self.overlays.iter().any(|o| o.wants_ticks())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::event::{Key, Modifiers, MouseButton, NamedKey};
    use crate::geometry::Point;
    use crate::painter::Painter;
    use crate::theme::Theme;
    use crate::widgets::{Menu, MenuBar, MenuItem};
    use std::cell::Cell;
    use std::rc::Rc;

    /// A focusable leaf that records whether it ever received a pointer-down
    /// and tracks its current focus state.
    struct Sensor {
        rect: Rect,
        hit: Rc<Cell<bool>>,
        focused: Rc<Cell<bool>>,
    }

    impl Widget for Sensor {
        fn bounds(&self) -> Rect {
            self.rect
        }
        fn layout(&mut self, bounds: Rect) {
            self.rect = bounds;
        }
        fn paint(&mut self, _: &mut Painter, _: &Theme) {}
        fn event(&mut self, event: &Event, _: &mut EventCtx) {
            if let Event::PointerDown { .. } = event {
                self.hit.set(true);
            }
        }
        fn focusable(&self) -> bool {
            true
        }
        fn set_focused(&mut self, focused: bool) {
            self.focused.set(focused);
        }
    }

    /// A `Column` of a one-menu bar above a [`Sensor`] fill child, plus the
    /// cells the sensor reports through. Laid out and focused (focus lands on
    /// the sensor — the bar isn't focusable).
    fn menu_over_sensor() -> (Column, Rc<Cell<bool>>, Rc<Cell<bool>>) {
        let hit = Rc::new(Cell::new(false));
        let focused = Rc::new(Cell::new(false));
        let bar = MenuBar::new(Rect::new(0, 0, 200, 20))
            .add_menu(Menu::new("&File", vec![MenuItem::action("&New", |_| {})]));
        let sensor = Sensor {
            rect: Rect::new(0, 0, 0, 0),
            hit: hit.clone(),
            focused: focused.clone(),
        };
        let mut col = Column::new().add_fixed(bar, 20).add_fill(sensor);
        col.layout(Rect::new(0, 0, 200, 200));
        col.focus_first();
        (col, hit, focused)
    }

    fn open_file_menu(col: &mut Column) {
        // Alt+F. No mouse ever touches the bar, so the old code never marked it
        // as capturing the pointer.
        let alt = Modifiers {
            alt: true,
            ..Modifiers::default()
        };
        let mut ctx = EventCtx::new();
        col.event(
            &Event::KeyDown {
                key: Key::Char('f'),
                modifiers: alt,
            },
            &mut ctx,
        );
    }

    fn press(col: &mut Column, x: i32, y: i32) {
        let mut ctx = EventCtx::new();
        col.event(
            &Event::PointerDown {
                pos: Point::new(x, y),
                button: MouseButton::Left,
                modifiers: Modifiers::default(),
            },
            &mut ctx,
        );
    }

    #[test]
    fn keyboard_opened_menu_owns_the_pointer() {
        let (mut col, hit, _focused) = menu_over_sensor();
        open_file_menu(&mut col);
        // A press in the sensor's area — below the bar, where the open menu's
        // popup is drawn on top — must be owned by the menu, not leak through to
        // the widget behind the popup.
        press(&mut col, 40, 100);
        assert!(
            !hit.get(),
            "an open menu must swallow clicks over the popup, not pass them to the widget underneath"
        );
    }

    #[test]
    fn closed_menu_lets_clicks_reach_the_widget_below() {
        // The same setup, but without opening the menu: a press must reach the
        // widget under the cursor as usual.
        let (mut col, hit, _focused) = menu_over_sensor();
        press(&mut col, 40, 100);
        assert!(hit.get(), "with no menu open, the click reaches the sensor");
    }

    #[test]
    fn open_menu_suspends_content_focus_then_restores_it() {
        let (mut col, _hit, focused) = menu_over_sensor();
        assert!(focused.get(), "the sensor starts focused");

        // Opening the menu (reconciled at paint time) suspends the content's
        // focus visual so focus reads as belonging to the menu.
        open_file_menu(&mut col);
        let backend = crate::mock::MockBackend::new(200, 200);
        backend.render(&mut col);
        assert!(
            !focused.get(),
            "content focus is suspended while the menu is open"
        );

        // Closing the menu (Escape) hands the same focus straight back.
        let mut ctx = EventCtx::new();
        col.event(
            &Event::KeyDown {
                key: Key::Named(NamedKey::Escape),
                modifiers: Modifiers::default(),
            },
            &mut ctx,
        );
        backend.render(&mut col);
        assert!(focused.get(), "focus is restored once the menu closes");
    }
}
