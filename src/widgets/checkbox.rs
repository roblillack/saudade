use crate::event::{Event, EventCtx, Key, MouseButton, NamedKey};
use crate::geometry::{Color, Rect};
use crate::painter::Painter;
use crate::theme::Theme;
use crate::widget::Widget;

type ToggleHandler = Box<dyn FnMut(&mut EventCtx, bool)>;

const BOX_SIZE: i32 = 13;
const LABEL_GAP: i32 = 4;
const FOCUS_PAD_X: i32 = 2;
const FOCUS_PAD_Y: i32 = 1;

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
        let crisp = painter.wants_crisp_chrome();

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
        // white window background without leaning on a sunken bevel. In the
        // crisp range the outline + check glyph are drawn at exact
        // physical pixels so the thin chrome doesn't alias.
        if crisp {
            let phys_box = painter.rect_to_physical(box_rect);
            let saved = painter.push_physical_pixels();
            painter.fill_rect(phys_box.inset(1), box_fill);
            painter.stroke_rect(phys_box, theme.border);
            if self.checked {
                draw_check(painter, phys_box, fg);
            }
            painter.restore_scale(saved);
        } else {
            painter.fill_rect(box_rect.inset(1), box_fill);
            painter.stroke_rect(box_rect, theme.border);
            if self.checked {
                draw_check(painter, box_rect, fg);
            }
        }

        // Label sits to the right of the box, vertically centered with the
        // widget's bounds.
        let text_size = theme.font_size;
        let measured = painter.measure_text(&self.label, text_size);
        let text_x = box_rect.right() + LABEL_GAP;
        let text_y = self.rect.y + ((self.rect.h - measured.h).max(0)) / 2;
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

/// Draw the classic Win 3.1 check glyph — two strokes that form a "✓" inside
/// the box. The pattern was hand-tuned for a 13×13 cell so it never touches
/// the bevel; the pattern is centered within `box_rect` so it stays roughly
/// in place when the box happens to be larger (e.g. when the chrome was
/// drawn in physical-pixel mode at a fractional scale, leaving the box a
/// couple of pixels wider than the design size).
fn draw_check(painter: &mut Painter, box_rect: Rect, color: Color) {
    const PATTERN: &[&[u8]] = &[
        b"          X  ",
        b"         XX  ",
        b"        XXX  ",
        b"  X    XXX   ",
        b"  XX  XXX    ",
        b"   XXXXX     ",
        b"    XXX      ",
        b"     X       ",
    ];
    const CELL: i32 = 13;
    // Center the 13-wide pattern in the box horizontally; vertically, the
    // pattern hugs the bottom of its 13-row cell (rows 3..11), so we
    // center the *cell* rather than the pattern itself to keep the check
    // floating in roughly the same spot.
    let dx = box_rect.x + (box_rect.w - CELL) / 2;
    let dy = box_rect.y + (box_rect.h - CELL) / 2 + 3;
    for (row, line) in PATTERN.iter().enumerate() {
        for (col, byte) in line.iter().enumerate() {
            if *byte == b'X' {
                painter.pixel(dx + col as i32, dy + row as i32, color);
            }
        }
    }
}
