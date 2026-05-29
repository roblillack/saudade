use crate::event::{Event, EventCtx};
use crate::geometry::{Color, Rect, Size};
use crate::painter::Painter;
use crate::theme::Theme;
use crate::widget::{PopupRequest, Widget};

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

    /// Focus the first focusable child, if any. Call this at startup if you
    /// want a window to begin with a particular widget keyboard-ready (e.g.
    /// a Notepad window that should accept typing immediately).
    pub fn focus_first(&mut self) {
        for (idx, child) in self.children.iter_mut().enumerate() {
            if child.focusable() {
                child.set_focused(true);
                self.focused = Some(idx);
                return;
            }
        }
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
            child.set_focused(true);
        }
        self.focused = new_focus;
        ctx.request_paint();
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
                    if child.captures_pointer() {
                        accelerator_blocking = true;
                    }
                }
            }
            if accelerator_blocking {
                return;
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
