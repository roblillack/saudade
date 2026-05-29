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
            background: Some(Color::WHITE),
            children: Vec::new(),
            overlays: Vec::new(),
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

    /// Index of the first overlay that's currently asserting pre-emptive
    /// capture (typically: a dialog that's just been shown).
    fn active_overlay(&self) -> Option<usize> {
        self.overlays
            .iter()
            .position(|o| o.captures_pointer())
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

            // Tab / Shift+Tab cycle focus between sibling focusable
            // children before the event reaches the focused widget. The
            // matching `Char('\t')` is swallowed so a single Tab press
            // doesn't move focus twice; when this column has fewer than
            // two focusable children we let both events fall through so
            // a sole `TextEditor` can still receive `'\t'`.
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

    fn wants_ticks(&self) -> bool {
        self.children.iter().any(|c| c.widget.wants_ticks())
            || self.overlays.iter().any(|o| o.wants_ticks())
    }
}
