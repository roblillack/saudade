use crate::event::{Event, EventCtx};
use crate::geometry::{Color, Rect};
use crate::painter::Painter;
use crate::theme::Theme;
use crate::widget::{PopupRequest, Widget};
use crate::widgets::{TabAction, tab_action};

/// Horizontal layout container — the sibling of [`Column`](crate::widgets::Column).
///
/// Each child is given a vertical-full-height slice of the row's bounds:
/// either a *fixed* width it asked for, or it shares the space left after
/// every fixed child has been laid out (a *fill* child). `Row` propagates
/// `layout` to its children whenever its own bounds change, and handles
/// pointer capture, keyboard focus, accelerator routing and Tab cycling
/// exactly like `Column`.
///
/// Unlike `Column`, `Row` has no overlay layer: floating chrome (modal
/// dialogs, menus) belongs to the top-level container, so nesting a `Row`
/// inside a `Column` keeps a single overlay owner.
pub struct Row {
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

impl Row {
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

    /// Add a child with a *fixed* logical-pixel width. Height is always the
    /// full row height.
    pub fn add_fixed(mut self, widget: impl Widget + 'static, width: i32) -> Self {
        self.push_fixed(widget, width);
        self
    }

    pub fn push_fixed(&mut self, widget: impl Widget + 'static, width: i32) {
        self.children.push(Child {
            widget: Box::new(widget),
            mode: SizeMode::Fixed(width),
        });
    }

    /// Add a child that fills the leftover width. Multiple fill children split
    /// the remaining space equally.
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

    /// Direct keyboard focus to a specific child by index. See
    /// [`Column::focus_child`](crate::widgets::Column::focus_child).
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
        let pos = event.position()?;
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
            c.widget.focus_first();
        }
        self.focused = new_focus;
        ctx.request_paint();
    }

    fn focusable_count(&self) -> usize {
        self.children.iter().filter(|c| c.widget.focusable()).count()
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

impl Default for Row {
    fn default() -> Self {
        Self::new()
    }
}

impl Widget for Row {
    fn bounds(&self) -> Rect {
        self.bounds
    }

    fn layout(&mut self, bounds: Rect) {
        self.bounds = bounds;

        let total_fixed: i32 = self
            .children
            .iter()
            .filter_map(|c| match c.mode {
                SizeMode::Fixed(w) => Some(w),
                SizeMode::Fill => None,
            })
            .sum();
        let fill_count = self
            .children
            .iter()
            .filter(|c| matches!(c.mode, SizeMode::Fill))
            .count() as i32;

        let leftover = (bounds.w - total_fixed).max(0);
        let fill_each = if fill_count > 0 {
            leftover / fill_count
        } else {
            0
        };
        let fill_last_extra = if fill_count > 0 {
            leftover - fill_each * fill_count
        } else {
            0
        };

        let mut x = bounds.x;
        let mut fill_seen = 0;
        for child in &mut self.children {
            let w = match child.mode {
                SizeMode::Fixed(w) => w,
                SizeMode::Fill => {
                    fill_seen += 1;
                    if fill_seen == fill_count {
                        fill_each + fill_last_extra
                    } else {
                        fill_each
                    }
                }
            };
            child.widget.layout(Rect::new(x, bounds.y, w, bounds.h));
            x += w;
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

    fn focusable(&self) -> bool {
        self.children.iter().any(|c| c.widget.focusable())
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
        for child in &self.children {
            if let Some(req) = child.widget.popup_request() {
                return Some(req);
            }
        }
        None
    }

    fn wants_ticks(&self) -> bool {
        self.children.iter().any(|c| c.widget.wants_ticks())
    }
}
