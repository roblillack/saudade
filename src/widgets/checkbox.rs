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
            on_toggle: None,
        }
    }

    pub fn checked(mut self, checked: bool) -> Self {
        self.checked = checked;
        self
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

        // Sunken etched frame: dark top/left, light bottom/right — like a
        // text field. While the user is actively pressing the checkbox we
        // tint the inner face gray to match Win 3.1 mouse feedback.
        painter.fill_rect(box_rect, if pressed_visual { theme.face } else { theme.background });
        painter.sunken_bevel(box_rect, theme.highlight, theme.shadow);

        if self.checked {
            draw_check(painter, box_rect, theme.text);
        }

        // Label sits to the right of the box, vertically centered with the
        // widget's bounds.
        let text_size = theme.font_size;
        let measured = painter.measure_text(&self.label, text_size);
        let text_x = box_rect.right() + LABEL_GAP;
        let text_y = self.rect.y + ((self.rect.h - measured.h).max(0)) / 2;
        painter.text(text_x, text_y, &self.label, text_size, theme.text);

        if self.focused {
            let focus_rect = Rect::new(
                text_x - FOCUS_PAD_X,
                text_y - FOCUS_PAD_Y,
                measured.w + 2 * FOCUS_PAD_X,
                measured.h + 2 * FOCUS_PAD_Y,
            );
            draw_focus_rect(painter, focus_rect, theme.text);
        }
    }

    fn event(&mut self, event: &Event, ctx: &mut EventCtx) {
        match event {
            Event::PointerDown {
                pos,
                button: MouseButton::Left,
            } => {
                if self.rect.contains(*pos) {
                    self.pressed = true;
                    self.armed = true;
                    ctx.request_focus();
                    ctx.request_paint();
                }
            }
            Event::PointerMove { pos } => {
                if self.pressed {
                    let armed_now = self.rect.contains(*pos);
                    if armed_now != self.armed {
                        self.armed = armed_now;
                        ctx.request_paint();
                    }
                }
            }
            Event::PointerUp {
                pos,
                button: MouseButton::Left,
            } => {
                if self.pressed {
                    let fire = self.armed && self.rect.contains(*pos);
                    self.pressed = false;
                    self.armed = false;
                    ctx.request_paint();
                    if fire {
                        self.toggle(ctx);
                    }
                }
            }
            Event::PointerLeave => {
                if self.armed {
                    self.armed = false;
                    ctx.request_paint();
                }
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
        true
    }

    fn set_focused(&mut self, focused: bool) {
        self.focused = focused;
    }
}

/// Draw the classic Win 3.1 check glyph — two strokes that form a "✓" inside
/// the 13×13 box. The pattern is hand-tuned for the box size so it never
/// touches the bevel.
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
    let offset_y = 3;
    for (row, line) in PATTERN.iter().enumerate() {
        for (col, byte) in line.iter().enumerate() {
            if *byte == b'X' {
                painter.pixel(
                    box_rect.x + col as i32,
                    box_rect.y + offset_y + row as i32,
                    color,
                );
            }
        }
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
