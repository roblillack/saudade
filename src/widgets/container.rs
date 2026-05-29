use crate::event::{Event, EventCtx};
use crate::geometry::{Color, Rect, Size};
use crate::painter::Painter;
use crate::theme::Theme;
use crate::widget::{PopupRequest, Widget};
use crate::widgets::{TabAction, tab_action};

/// A flat collection of widgets at absolute positions inside a fixed-size area.
///
/// The container is the only thing retrogui ships with right now — enough for
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
pub struct Container {
    pub size: Size,
    pub background: Option<Color>,
    pub border: Option<Color>,
    children: Vec<Box<dyn Widget>>,
    captured: Option<usize>,
    focused: Option<usize>,
}

impl Container {
    pub fn new(width: i32, height: i32) -> Self {
        Self {
            size: Size::new(width, height),
            background: None,
            border: None,
            children: Vec::new(),
            captured: None,
            focused: None,
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


    fn choose_target(&self, event: &Event) -> Option<usize> {
        if event.is_keyboard() {
            return self.focused;
        }
        if let Some(idx) = self.captured {
            return Some(idx);
        }
        let Some(pos) = event.position() else {
            return None;
        };
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
        Rect::new(0, 0, self.size.w, self.size.h)
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

        // Keyboard events first go to every accelerator-accepting child
        // (e.g. a MenuBar listening for Alt+letter). If any of those is
        // *actively capturing* — typically a menubar with an open menu —
        // the focused widget below is locked out: the menu owns the user's
        // attention until it closes. This is what stops keystrokes from
        // leaking into an editor while a menu is up.
        if event.is_keyboard() {
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
                Some(TabAction::Cycle(dir)) => {
                    if self.cycle_focus(dir, ctx) {
                        return;
                    }
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

    fn wants_ticks(&self) -> bool {
        self.children.iter().any(|c| c.wants_ticks())
    }
}
