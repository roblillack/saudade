use crate::event::{Event, EventCtx};
use crate::geometry::{Color, Point, Rect, Size};
use crate::painter::Painter;
use crate::theme::Theme;
use crate::widget::{PopupRequest, Widget};
use crate::widgets::{TabAction, tab_action};

/// A flat collection of widgets at absolute positions inside a fixed-size area.
///
/// The container is the only thing saudade ships with right now — enough for
/// WINGs-style dialog layouts. It is responsible for three runtime concerns:
///
/// * **hit testing**: pointer events are routed to the top-most child whose
///   bounds contain the cursor;
/// * **pointer capture**: a child that returns `captures_pointer() == true`
///   keeps receiving pointer events until it stops capturing — used by
///   buttons and menus to keep events flowing while a press is in progress;
/// * **keyboard focus**: clicking a focusable child makes it the keyboard
///   target. Keyboard events are routed to the focused child only.
///
/// An overlay paint pass runs after every child has rendered, so widgets like
/// menus can draw popups on top of their siblings.
///
/// Children are added at positions relative to the container's own top-left.
/// [`layout`](Widget::layout) shifts them all by the origin it is given, so a
/// `Container` can be placed anywhere — including as the content of a
/// [`Modal`](crate::widgets::Modal), which centers it at a non-zero origin.
/// Laid out at `(0, 0)` (the common app-root case) the shift is zero, so this
/// is backward compatible with containers used directly as a window root.
pub struct Container {
    pub size: Size,
    pub background: Option<Color>,
    pub border: Option<Color>,
    children: Vec<Box<dyn Widget>>,
    captured: Option<usize>,
    focused: Option<usize>,
    /// Top-left the children are currently positioned against. Children are
    /// authored relative to `(0, 0)`; each `layout` translates them by the
    /// delta to the new origin and records it here.
    origin: Point,
}

impl Container {
    pub fn new(width: i32, height: i32) -> Self {
        Self {
            size: Size::new(width, height),
            // Transparent by default: the runtime already fills the window
            // with `theme.background`, so an opaque white here would just
            // repaint it. Set one explicitly (`with_background`) only when the
            // interior should differ from whatever sits behind the container —
            // e.g. a gray card, or to mask the window's background pattern.
            background: None,
            border: None,
            children: Vec::new(),
            captured: None,
            focused: None,
            origin: Point::new(0, 0),
        }
    }

    pub fn with_background(mut self, color: Color) -> Self {
        self.background = Some(color);
        self
    }

    pub fn with_border(mut self, color: Color) -> Self {
        self.border = Some(color);
        self
    }

    // `add` is an intentional builder-style method name, not an arithmetic op.
    #[allow(clippy::should_implement_trait)]
    pub fn add(mut self, widget: impl Widget + 'static) -> Self {
        self.push(widget);
        self
    }

    pub fn push(&mut self, widget: impl Widget + 'static) {
        self.children.push(Box::new(widget));
    }

    /// Index of the currently-focused child, or `None` if nothing has been
    /// focused yet. Exposed mainly so tests can verify focus cycling.
    pub fn focused_index(&self) -> Option<usize> {
        self.focused
    }

    /// Direct keyboard focus to a specific child by index (its position in add
    /// order), delegating into it via `focus_first` so wrapper widgets pick the
    /// right nested leaf. Returns `true` if the index named a focusable child.
    /// Use it to choose a non-default initial focus target — e.g. a confirm
    /// box opening with its Cancel button focused rather than the first one.
    pub fn focus_child(&mut self, index: usize) -> bool {
        if self.children.get(index).map(|c| c.focusable()) != Some(true) {
            return false;
        }
        if let Some(old) = self.focused
            && old != index
            && let Some(c) = self.children.get_mut(old)
        {
            c.set_focused(false);
        }
        let focused = self.children[index].focus_first();
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
            .position(|c| c.accepts_accelerators() && c.captures_pointer())
        {
            return Some(idx);
        }
        let pos = event.position()?;
        (0..self.children.len())
            .rev()
            .find(|&i| self.children[i].bounds().contains(pos))
    }

    fn change_focus(&mut self, new_focus: Option<usize>, ctx: &mut EventCtx) {
        if new_focus == self.focused {
            return;
        }
        if let Some(old) = self.focused
            && let Some(child) = self.children.get_mut(old)
        {
            child.set_focused(false);
        }
        if let Some(new) = new_focus
            && let Some(child) = self.children.get_mut(new)
        {
            // Use `focus_first` so wrapper widgets that delegate focus to a
            // nested target get a chance to set up the right leaf, rather
            // than just marking themselves focused.
            child.focus_first();
        }
        self.focused = new_focus;
        ctx.request_paint();
    }

    fn focusable_count(&self) -> usize {
        self.children.iter().filter(|c| c.focusable()).count()
    }

    /// Move focus to the next / previous focusable child. Wraps at either end
    /// so Tab/Shift+Tab cycling stays inside the container. Returns `true`
    /// only when focus actually moved — that way an outer container with a
    /// single nested container as its only focusable child *doesn't*
    /// consume Tab, letting the event propagate to the inner container
    /// which can then cycle among its own children.
    fn cycle_focus(&mut self, dir: i32, ctx: &mut EventCtx) -> bool {
        let n = self.children.len();
        if n == 0 {
            return false;
        }
        let candidates: Vec<usize> = (0..n).filter(|&i| self.children[i].focusable()).collect();
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

impl Widget for Container {
    fn bounds(&self) -> Rect {
        Rect::new(self.origin.x, self.origin.y, self.size.w, self.size.h)
    }

    fn layout(&mut self, bounds: Rect) {
        // Shift every child by the delta from where they currently sit to the
        // container's new origin. Authored at `(0, 0)` and first laid out at a
        // non-zero origin, this moves them into place; laid out at `(0, 0)`
        // (a window root) the delta is zero and nothing moves.
        let dx = bounds.x - self.origin.x;
        let dy = bounds.y - self.origin.y;
        if dx != 0 || dy != 0 {
            for child in &mut self.children {
                let b = child.bounds();
                child.layout(Rect::new(b.x + dx, b.y + dy, b.w, b.h));
            }
        }
        self.origin = Point::new(bounds.x, bounds.y);
    }

    fn paint(&mut self, painter: &mut Painter, theme: &Theme) {
        if let Some(bg) = self.background {
            painter.fill_rect(self.bounds(), bg);
        }
        for child in &mut self.children {
            child.paint(painter, theme);
        }
        if let Some(border) = self.border {
            painter.stroke_rect(self.bounds(), border);
        }
        // Overlay pass: floating UI (menus, tooltips) on top of every sibling.
        for child in &mut self.children {
            child.paint_overlay(painter, theme);
        }
    }

    fn paint_overlay(&mut self, painter: &mut Painter, theme: &Theme) {
        for child in &mut self.children {
            child.paint_overlay(painter, theme);
        }
    }

    fn event(&mut self, event: &Event, ctx: &mut EventCtx) {
        // Non-positional, non-keyboard events with no current capture
        // (e.g. PointerLeave on its own): broadcast to all children.
        if !event.is_keyboard() && event.position().is_none() && self.captured.is_none() {
            for child in &mut self.children {
                child.event(event, ctx);
            }
            return;
        }

        // A focused child that's itself capturing the pointer — an open
        // `Dropdown`, for instance — owns the keyboard while it's up. Skip the
        // accelerator pass (so a sibling default button's Enter can't pre-empt
        // it) and Tab cycling; the focused dispatch below still delivers the
        // key. This mirrors the menu case (handled via `accelerator_blocking`
        // below), but for a popup that *holds focus* rather than floating
        // beside it.
        let focused_capturing = self
            .focused
            .and_then(|i| self.children.get(i))
            .is_some_and(|c| c.captures_pointer());

        // Keyboard events first go to every accelerator-accepting child
        // (e.g. a MenuBar listening for Alt+letter). If any of those is
        // *actively capturing* — typically a menubar with an open menu —
        // the focused widget below is locked out: the menu owns the user's
        // attention until it closes. This is what stops keystrokes from
        // leaking into an editor while a menu is up.
        if event.is_keyboard() && !focused_capturing {
            let mut accelerator_blocking = false;
            for (idx, child) in self.children.iter_mut().enumerate() {
                if child.accepts_accelerators() && Some(idx) != self.focused {
                    child.event(event, ctx);
                    if ctx.is_consumed() {
                        return;
                    }
                    if child.captures_pointer() {
                        accelerator_blocking = true;
                    }
                }
            }
            if accelerator_blocking {
                return;
            }

            // Tab / Shift+Tab cycle focus between sibling focusable
            // children before the event reaches whoever currently holds
            // focus. The runtime fires both `KeyDown(Tab)` and the
            // matching `Char('\t')` for a single press, so we cycle on
            // the KeyDown and swallow the paired Char so it doesn't
            // double-step focus or leak into the new widget. When this
            // container has fewer than two focusable children it lets
            // both events fall through — a lone `TextEditor` can then
            // still receive `'\t'` and insert it as indentation.
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
            child.event(event, ctx);

            if !event.is_keyboard() {
                if child.captures_pointer() {
                    self.captured = Some(idx);
                } else if captured_was_set {
                    self.captured = None;
                }
            }
        }

        // Apply focus changes after dispatch so we can mutably borrow
        // a different child to notify it of focus-out.
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
        self.captured.is_some()
    }

    fn focusable(&self) -> bool {
        self.children.iter().any(|c| c.focusable())
    }

    fn focus_first(&mut self) -> bool {
        for (idx, child) in self.children.iter_mut().enumerate() {
            if child.focus_first() {
                self.focused = Some(idx);
                return true;
            }
        }
        false
    }

    fn popup_request(&self) -> Option<PopupRequest> {
        for child in &self.children {
            if let Some(req) = child.popup_request() {
                return Some(req);
            }
        }
        None
    }

    fn collect_popups(&self, out: &mut Vec<PopupRequest>) {
        for child in &self.children {
            child.collect_popups(out);
        }
    }

    fn wants_ticks(&self) -> bool {
        self.children.iter().any(|c| c.wants_ticks())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::event::MouseButton;
    use crate::geometry::Point;
    use std::cell::Cell;
    use std::rc::Rc;

    /// A focusable leaf that records its laid-out rect and whether a pointer
    /// press reached it, and asks for focus when pressed.
    struct Probe {
        rect: Rect,
        hit: Rc<Cell<bool>>,
    }

    impl Widget for Probe {
        fn bounds(&self) -> Rect {
            self.rect
        }
        fn layout(&mut self, bounds: Rect) {
            self.rect = bounds;
        }
        fn paint(&mut self, _: &mut Painter, _: &Theme) {}
        fn event(&mut self, event: &Event, ctx: &mut EventCtx) {
            if let Event::PointerDown { .. } = event {
                self.hit.set(true);
                ctx.request_focus();
            }
        }
        fn focusable(&self) -> bool {
            true
        }
    }

    fn press(c: &mut Container, x: i32, y: i32) {
        let mut ctx = EventCtx::new();
        c.event(
            &Event::PointerDown {
                pos: Point::new(x, y),
                button: MouseButton::Left,
            },
            &mut ctx,
        );
    }

    #[test]
    fn layout_shifts_children_to_the_container_origin() {
        let hit = Rc::new(Cell::new(false));
        let mut c = Container::new(100, 50).add(Probe {
            // Authored at a local (0,0)-relative position.
            rect: Rect::new(10, 8, 20, 12),
            hit: hit.clone(),
        });
        // Placed at a non-zero origin, as a Modal would center it.
        c.layout(Rect::new(200, 300, 100, 50));
        assert_eq!(c.bounds(), Rect::new(200, 300, 100, 50));

        // The child no longer answers at its authored position…
        press(&mut c, 15, 12);
        assert!(
            !hit.get(),
            "child must have moved off its authored position"
        );

        // …but does at the position shifted by the container's origin.
        press(&mut c, 215, 312);
        assert!(
            hit.get(),
            "child should be hit at its origin-shifted position"
        );
        assert_eq!(c.focused_index(), Some(0));
    }

    #[test]
    fn layout_at_origin_is_a_no_op() {
        let hit = Rc::new(Cell::new(false));
        let mut c = Container::new(100, 50).add(Probe {
            rect: Rect::new(10, 8, 20, 12),
            hit: hit.clone(),
        });
        // A window-root container laid out at (0,0): children stay put.
        c.layout(Rect::new(0, 0, 100, 50));
        press(&mut c, 15, 12);
        assert!(hit.get(), "at the origin the children must not move");
    }
}
