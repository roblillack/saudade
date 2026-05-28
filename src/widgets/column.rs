use crate::event::{Event, EventCtx};
use crate::geometry::{Color, Rect};
use crate::painter::Painter;
use crate::theme::Theme;
use crate::widget::{PopupRequest, Widget};

/// Vertical layout container. Each child is given a horizontal slice of the
/// column's bounds: either a *fixed* height it asked for, or it shares the
/// space left after every fixed child has been laid out (a *fill* child).
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
    captured: Option<usize>,
    focused: Option<usize>,
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
            background: None,
            children: Vec::new(),
            captured: None,
            focused: None,
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

    pub fn focus_first(&mut self) {
        for (idx, child) in self.children.iter_mut().enumerate() {
            if child.widget.focusable() {
                child.widget.set_focused(true);
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
            .find(|&i| self.children[i].widget.bounds().contains(pos))
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
            c.widget.set_focused(true);
        }
        self.focused = new_focus;
        ctx.request_paint();
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
    }

    fn paint(&mut self, painter: &mut Painter, theme: &Theme) {
        if let Some(bg) = self.background {
            painter.fill_rect(self.bounds, bg);
        }
        for child in &mut self.children {
            child.widget.paint(painter, theme);
        }
        for child in &mut self.children {
            child.widget.paint_overlay(painter, theme);
        }
    }

    fn paint_overlay(&mut self, painter: &mut Painter, theme: &Theme) {
        for child in &mut self.children {
            child.widget.paint_overlay(painter, theme);
        }
    }

    fn event(&mut self, event: &Event, ctx: &mut EventCtx) {
        if !event.is_keyboard() && event.position().is_none() && self.captured.is_none() {
            for child in &mut self.children {
                child.widget.event(event, ctx);
            }
            return;
        }

        if event.is_keyboard() {
            let mut accelerator_blocking = false;
            for (idx, child) in self.children.iter_mut().enumerate() {
                if child.widget.accepts_accelerators() && Some(idx) != self.focused {
                    child.widget.event(event, ctx);
                    if child.widget.captures_pointer() {
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
        self.captured.is_some()
    }

    fn popup_request(&self) -> Option<PopupRequest> {
        for child in &self.children {
            if let Some(req) = child.widget.popup_request() {
                return Some(req);
            }
        }
        None
    }
}
