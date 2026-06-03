use crate::event::{Event, EventCtx, Key, MouseButton, NamedKey};
use crate::geometry::{Color, Rect};
use crate::painter::Painter;
use crate::theme::Theme;
use crate::widget::Widget;

type ChangeHandler = Box<dyn FnMut(&mut EventCtx, i32)>;

/// Logical width of the draggable thumb and the thickness of the groove it
/// rides in. The thumb's left edge travels across `rect.w - THUMB_W` pixels,
/// which keeps the whole thumb inside the widget's bounds at both ends.
const THUMB_W: i32 = 10;
const GROOVE_H: i32 = 4;

/// A classic Win 3.1 trackbar: a thin sunken groove with a raised, draggable
/// thumb that selects an integer value in an inclusive `[min, max]` range.
///
/// The slider owns its `value`. Drag the thumb (or click anywhere on the
/// widget) to set it; while focused, the arrow keys nudge it by `step`,
/// Page Up / Page Down jump by a tenth of the range, and Home / End snap to
/// the ends. An optional [`on_change`](Self::on_change) handler fires on every
/// value change — including continuously *during* a drag, not just on release
/// — so callers can react live (which is exactly what the 7GUIs timer needs to
/// re-fill its gauge as the duration slider moves).
pub struct Slider {
    rect: Rect,
    min: i32,
    max: i32,
    value: i32,
    step: i32,
    focused: bool,
    dragging: bool,
    enabled: bool,
    on_change: Option<ChangeHandler>,
}

impl Slider {
    pub fn new(rect: Rect, min: i32, max: i32) -> Self {
        let (min, max) = if min <= max { (min, max) } else { (max, min) };
        Self {
            rect,
            min,
            max,
            value: min,
            step: 1,
            focused: false,
            dragging: false,
            enabled: true,
            on_change: None,
        }
    }

    pub fn with_value(mut self, value: i32) -> Self {
        self.set_value(value);
        self
    }

    pub fn with_enabled(mut self, enabled: bool) -> Self {
        self.set_enabled(enabled);
        self
    }

    pub fn is_enabled(&self) -> bool {
        self.enabled
    }

    /// Enable or disable the slider. A disabled slider can't take focus and
    /// ignores clicks, drags, and arrow keys.
    pub fn set_enabled(&mut self, enabled: bool) {
        self.enabled = enabled;
        if !enabled {
            self.dragging = false;
        }
    }

    /// Set the per-keystroke increment used by the arrow keys (clamped to at
    /// least 1). Defaults to 1.
    pub fn with_step(mut self, step: i32) -> Self {
        self.step = step.max(1);
        self
    }

    pub fn on_change<F>(mut self, handler: F) -> Self
    where
        F: FnMut(&mut EventCtx, i32) + 'static,
    {
        self.on_change = Some(Box::new(handler));
        self
    }

    /// Install (or replace) the change handler after construction. Mirrors
    /// [`on_change`](Self::on_change) for callers that hold the slider behind
    /// an `Rc<RefCell<…>>` and need to wire up the callback once the other
    /// widgets it talks to exist.
    pub fn set_on_change<F>(&mut self, handler: F)
    where
        F: FnMut(&mut EventCtx, i32) + 'static,
    {
        self.on_change = Some(Box::new(handler));
    }

    pub fn value(&self) -> i32 {
        self.value
    }

    pub fn set_value(&mut self, value: i32) {
        self.value = value.clamp(self.min, self.max);
    }

    pub fn min(&self) -> i32 {
        self.min
    }

    pub fn max(&self) -> i32 {
        self.max
    }

    pub fn set_range(&mut self, min: i32, max: i32) {
        let (min, max) = if min <= max { (min, max) } else { (max, min) };
        self.min = min;
        self.max = max;
        self.value = self.value.clamp(min, max);
    }

    pub fn rect(&self) -> Rect {
        self.rect
    }

    pub fn set_rect(&mut self, rect: Rect) {
        self.rect = rect;
    }

    /// Pixels the thumb's leading edge can travel along the track.
    fn travel(&self) -> i32 {
        (self.rect.w - THUMB_W).max(0)
    }

    fn value_span(&self) -> i32 {
        (self.max - self.min).max(0)
    }

    fn thumb_rect(&self) -> Rect {
        let travel = self.travel();
        let span = self.value_span();
        let off = if span == 0 {
            0
        } else {
            ((travel as i64 * (self.value - self.min) as i64) / span as i64) as i32
        };
        Rect::new(self.rect.x + off, self.rect.y, THUMB_W, self.rect.h)
    }

    /// Map a pointer x (logical px) to the value whose thumb center sits under
    /// the cursor, clamped to the range.
    fn value_at_x(&self, x: i32) -> i32 {
        let travel = self.travel();
        if travel <= 0 {
            return self.min;
        }
        let left = (x - self.rect.x - THUMB_W / 2).clamp(0, travel);
        let span = self.value_span();
        // Round to the nearest value rather than truncating.
        self.min + (((left as i64 * span as i64) + travel as i64 / 2) / travel as i64) as i32
    }

    fn set_value_notify(&mut self, value: i32, ctx: &mut EventCtx) {
        let nv = value.clamp(self.min, self.max);
        if nv != self.value {
            self.value = nv;
            if let Some(handler) = self.on_change.as_mut() {
                handler(ctx, nv);
            }
        }
        ctx.request_paint();
    }
}

impl Widget for Slider {
    fn bounds(&self) -> Rect {
        self.rect
    }

    fn paint(&mut self, painter: &mut Painter, theme: &Theme) {
        // Sunken groove running through the vertical center, inset by half a
        // thumb at each end so the thumb's full travel stays over the track.
        let gy = self.rect.y + (self.rect.h - GROOVE_H) / 2;
        let gx = self.rect.x + THUMB_W / 2;
        let gw = (self.rect.w - THUMB_W).max(0);
        let groove = Rect::new(gx, gy, gw, GROOVE_H);
        let thumb = self.thumb_rect();
        let focus = thumb.inset(3);

        // In the `[0.9, 1.5)` crisp range, every chrome edge (groove
        // bevel, thumb bevel, focus dots) is drawn at exact physical
        // pixels — otherwise 1-logical-pixel chrome rounds to either 1
        // or 2 physical pixels and the thumb's bevel looks lopsided.
        if painter.wants_crisp_chrome() {
            let phys_groove = painter.rect_to_physical(groove);
            let phys_thumb = painter.rect_to_physical(thumb);
            let phys_focus = painter.rect_to_physical(focus);
            let saved = painter.push_physical_pixels();
            painter.fill_rect(phys_groove, theme.face);
            painter.sunken_bevel(phys_groove, theme.highlight, theme.shadow);
            painter.button(phys_thumb, theme, false, false);
            if self.focused && self.enabled {
                draw_focus_rect(painter, phys_focus, theme.text);
            }
            painter.restore_scale(saved);
        } else {
            painter.fill_rect(groove, theme.face);
            painter.sunken_bevel(groove, theme.highlight, theme.shadow);
            painter.button(thumb, theme, false, false);
            if self.focused && self.enabled {
                draw_focus_rect(painter, focus, theme.text);
            }
        }
    }

    fn event(&mut self, event: &Event, ctx: &mut EventCtx) {
        if !self.enabled {
            return;
        }
        match event {
            Event::PointerDown {
                pos,
                button: MouseButton::Left,
            } if self.rect.contains(*pos) => {
                ctx.request_focus();
                self.dragging = true;
                let v = self.value_at_x(pos.x);
                self.set_value_notify(v, ctx);
            }
            Event::PointerMove { pos } if self.dragging => {
                let v = self.value_at_x(pos.x);
                self.set_value_notify(v, ctx);
            }
            Event::PointerUp {
                button: MouseButton::Left,
                ..
            } if self.dragging => {
                self.dragging = false;
                ctx.request_paint();
            }
            Event::KeyDown { key, modifiers } if self.focused && !modifiers.has_command() => {
                let page = (self.value_span() / 10).max(self.step);
                let target = match key {
                    Key::Named(NamedKey::Left) | Key::Named(NamedKey::Down) => {
                        Some(self.value - self.step)
                    }
                    Key::Named(NamedKey::Right) | Key::Named(NamedKey::Up) => {
                        Some(self.value + self.step)
                    }
                    Key::Named(NamedKey::PageDown) => Some(self.value - page),
                    Key::Named(NamedKey::PageUp) => Some(self.value + page),
                    Key::Named(NamedKey::Home) => Some(self.min),
                    Key::Named(NamedKey::End) => Some(self.max),
                    _ => None,
                };
                if let Some(target) = target {
                    self.set_value_notify(target, ctx);
                    ctx.consume_event();
                }
            }
            _ => {}
        }
    }

    fn captures_pointer(&self) -> bool {
        self.dragging
    }

    fn focusable(&self) -> bool {
        self.enabled
    }

    fn set_focused(&mut self, focused: bool) {
        self.focused = focused;
    }

    fn layout(&mut self, bounds: Rect) {
        self.rect = bounds;
    }
}

fn draw_focus_rect(painter: &mut Painter, rect: Rect, color: Color) {
    if rect.w <= 0 || rect.h <= 0 {
        return;
    }
    let right = rect.right() - 1;
    let bottom = rect.bottom() - 1;
    let mut x = rect.x;
    while x <= right {
        painter.pixel(x, rect.y, color);
        painter.pixel(x, bottom, color);
        x += 2;
    }
    let mut y = rect.y;
    while y <= bottom {
        painter.pixel(rect.x, y, color);
        painter.pixel(right, y, color);
        y += 2;
    }
}
