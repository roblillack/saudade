use crate::event::{Event, EventCtx, Key, MouseButton, NamedKey};
use crate::geometry::Rect;
use crate::include_svg;
use crate::painter::Painter;
use crate::svg::SvgImage;
use crate::theme::Theme;
use crate::widget::Widget;

type ToggleHandler = Box<dyn FnMut(&mut EventCtx, bool)>;

const BOX_SIZE: i32 = 13;
const LABEL_GAP: i32 = 4;
const FOCUS_PAD_X: i32 = 2;
const FOCUS_PAD_Y: i32 = 1;
/// Lift the label a hair above the box's geometric center so it reads as
/// aligned with the check glyph rather than sitting a touch low.
const LABEL_NUDGE_Y: i32 = 1;

/// The check glyph, baked from SVG at compile time. Its 13-unit viewBox maps
/// 1:1 onto the 13×13 box, and `SvgImage::draw_tinted` re-snaps it crisply at
/// every scale while tinting the placeholder black with the theme's (possibly
/// disabled) text color.
const CHECK: SvgImage = include_svg!("assets/checkbox/check.svg");

/// Win 3.1 checkbox: a 13×13 sunken white box with a check glyph when set,
/// followed by a text label. Click or Space toggles the state; the optional
/// `on_toggle` handler fires with the new value.
pub struct Checkbox {
    rect: Rect,
    label: String,
    checked: bool,
    pressed: bool,
    armed: bool,
    focused: bool,
    enabled: bool,
    on_toggle: Option<ToggleHandler>,
}

impl Checkbox {
    pub fn new(rect: Rect, label: impl Into<String>) -> Self {
        Self {
            rect,
            label: label.into(),
            checked: false,
            pressed: false,
            armed: false,
            focused: false,
            enabled: true,
            on_toggle: None,
        }
    }

    pub fn checked(mut self, checked: bool) -> Self {
        self.checked = checked;
        self
    }

    pub fn with_enabled(mut self, enabled: bool) -> Self {
        self.set_enabled(enabled);
        self
    }

    pub fn is_enabled(&self) -> bool {
        self.enabled
    }

    /// Enable or disable the checkbox. A disabled checkbox paints greyed, can't
    /// take focus, and ignores clicks and Space.
    pub fn set_enabled(&mut self, enabled: bool) {
        self.enabled = enabled;
        if !enabled {
            self.pressed = false;
            self.armed = false;
        }
    }

    pub fn on_toggle<F>(mut self, handler: F) -> Self
    where
        F: FnMut(&mut EventCtx, bool) + 'static,
    {
        self.on_toggle = Some(Box::new(handler));
        self
    }

    pub fn is_checked(&self) -> bool {
        self.checked
    }

    pub fn set_checked(&mut self, checked: bool) {
        self.checked = checked;
    }

    fn toggle(&mut self, ctx: &mut EventCtx) {
        self.checked = !self.checked;
        ctx.request_paint();
        if let Some(handler) = self.on_toggle.as_mut() {
            handler(ctx, self.checked);
        }
    }

    fn box_rect(&self) -> Rect {
        let y = self.rect.y + (self.rect.h - BOX_SIZE).max(0) / 2;
        Rect::new(self.rect.x, y, BOX_SIZE, BOX_SIZE)
    }
}

impl Widget for Checkbox {
    fn bounds(&self) -> Rect {
        self.rect
    }

    fn paint(&mut self, painter: &mut Painter, theme: &Theme) {
        let box_rect = self.box_rect();
        let pressed_visual = self.pressed && self.armed;

        let fg = if self.enabled {
            theme.text
        } else {
            theme.disabled_text
        };
        let box_fill = if pressed_visual {
            theme.face
        } else {
            theme.background
        };

        // 1px black outline around a flat field — enough to stay visible on a
        // white window background without leaning on a sunken bevel. The fill +
        // outline are kept together in one physical-pixel pass in the crisp
        // range: the inset face fill must snap against the *same* physical box
        // as the outline, so it can't be pulled out to a plain logical call
        // without shifting an edge pixel.
        if painter.wants_1x_crispness() {
            painter.physical(box_rect, |p, r| {
                p.fill_rect(r.inset(1), box_fill);
                p.stroke_rect(r, theme.border);
            });
        } else {
            painter.fill_rect(box_rect.inset(1), box_fill);
            painter.stroke_rect(box_rect, theme.border);
        }
        // The check glyph rides on top as a baked SVG; `draw_tinted` runs its
        // own crisp physical-pixel pass, so it stays out of the box's.
        if self.checked {
            CHECK.draw_tinted(painter, box_rect, fg);
        }

        // Label sits to the right of the box, vertically centered with the
        // widget's bounds and lifted a hair (see `LABEL_NUDGE_Y`).
        let text_size = theme.font_size;
        let measured = painter.measure_text(&self.label, text_size);
        let text_x = box_rect.right() + LABEL_GAP;
        let text_y = self.rect.y + ((self.rect.h - measured.h).max(0)) / 2 - LABEL_NUDGE_Y;
        painter.text(text_x, text_y, &self.label, text_size, fg);

        if self.focused && self.enabled {
            let focus_rect = Rect::new(
                text_x - FOCUS_PAD_X,
                text_y - FOCUS_PAD_Y,
                measured.w + 2 * FOCUS_PAD_X,
                measured.h + 2 * FOCUS_PAD_Y,
            );
            painter.focus_rect(focus_rect, theme.text);
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
                self.pressed = true;
                self.armed = true;
                ctx.request_focus();
                ctx.request_paint();
            }
            Event::PointerMove { pos } if self.pressed => {
                let armed_now = self.rect.contains(*pos);
                if armed_now != self.armed {
                    self.armed = armed_now;
                    ctx.request_paint();
                }
            }
            Event::PointerUp {
                pos,
                button: MouseButton::Left,
            } if self.pressed => {
                let fire = self.armed && self.rect.contains(*pos);
                self.pressed = false;
                self.armed = false;
                ctx.request_paint();
                if fire {
                    self.toggle(ctx);
                }
            }
            Event::PointerLeave if self.armed => {
                self.armed = false;
                ctx.request_paint();
            }
            Event::KeyDown { key, modifiers }
                if self.focused
                    && !modifiers.has_command()
                    && matches!(key, Key::Named(NamedKey::Space)) =>
            {
                self.toggle(ctx);
                ctx.consume_event();
            }
            _ => {}
        }
    }

    fn captures_pointer(&self) -> bool {
        self.pressed
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
