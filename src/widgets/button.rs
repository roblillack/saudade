use crate::event::{Event, EventCtx, MouseButton};
use crate::geometry::Rect;
use crate::painter::Painter;
use crate::theme::Theme;
use crate::widget::Widget;

type ClickHandler = Box<dyn FnMut(&mut EventCtx)>;

/// Classic Win 3.1 push button: raised face by default, sunken while pressed,
/// optional 1px outer black border for the dialog's default action.
pub struct Button {
    pub rect: Rect,
    pub label: String,
    pub default: bool,
    pressed: bool,
    armed: bool,
    on_click: Option<ClickHandler>,
}

impl Button {
    pub fn new(rect: Rect, label: impl Into<String>) -> Self {
        Self {
            rect,
            label: label.into(),
            default: false,
            pressed: false,
            armed: false,
            on_click: None,
        }
    }

    pub fn default(mut self, default: bool) -> Self {
        self.default = default;
        self
    }

    pub fn on_click<F>(mut self, handler: F) -> Self
    where
        F: FnMut(&mut EventCtx) + 'static,
    {
        self.on_click = Some(Box::new(handler));
        self
    }
}

impl Widget for Button {
    fn bounds(&self) -> Rect {
        self.rect
    }

    fn paint(&mut self, painter: &mut Painter, theme: &Theme) {
        let pressed_visual = self.pressed && self.armed;
        painter.button(self.rect, theme, pressed_visual, self.default);

        // When pressed, push the label one pixel down/right for tactile feel.
        let mut label_rect = self.rect;
        if pressed_visual {
            label_rect.x += 1;
            label_rect.y += 1;
        }
        painter.text_centered(label_rect, &self.label, theme.font_size, theme.text);
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
                    if fire
                        && let Some(handler) = self.on_click.as_mut()
                    {
                        handler(ctx);
                    }
                }
            }
            Event::PointerLeave => {
                if self.armed {
                    self.armed = false;
                    ctx.request_paint();
                }
            }
            _ => {}
        }
    }

    fn captures_pointer(&self) -> bool {
        self.pressed
    }
}
