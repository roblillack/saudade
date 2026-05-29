use crate::event::{Event, EventCtx, Key, MouseButton, NamedKey};
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
    focused: bool,
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
            focused: false,
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

    fn fire(&mut self, ctx: &mut EventCtx) {
        if let Some(handler) = self.on_click.as_mut() {
            handler(ctx);
        }
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

        // Dotted focus rectangle inside the bevel — the same chrome Win 3.1
        // drew on its focused buttons. Keep it inset enough that it doesn't
        // collide with the raised bevel highlights.
        if self.focused {
            let inset = if self.default { 4 } else { 3 };
            let r = self.rect.inset(inset);
            draw_focus_rect(painter, r, theme.text);
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
                        self.fire(ctx);
                    }
                }
            }
            Event::PointerLeave => {
                if self.armed {
                    self.armed = false;
                    ctx.request_paint();
                }
            }
            // Keyboard activation when focused: Enter and Space both fire the
            // button's action, matching Win 3.1 / Windows behavior. We only
            // react to KeyDown so a held key doesn't auto-repeat fires.
            Event::KeyDown { key, modifiers } if self.focused && !modifiers.has_command() => {
                let activate = matches!(
                    key,
                    Key::Named(NamedKey::Enter) | Key::Named(NamedKey::Space)
                );
                if activate {
                    self.pressed = true;
                    self.armed = true;
                    ctx.request_paint();
                    self.fire(ctx);
                    self.pressed = false;
                    self.armed = false;
                    ctx.request_paint();
                    ctx.consume_event();
                }
            }
            // Default-button accelerator: Enter pressed anywhere in the
            // surrounding container fires the dialog's default button
            // even when focus is elsewhere (typical Win 3.1 dialog
            // behavior). The parent's accelerator pass routes the event
            // here only when `self` is not the currently-focused child,
            // so we don't double-fire the focused-button case above.
            Event::KeyDown {
                key: Key::Named(NamedKey::Enter),
                modifiers,
            } if self.default && !self.focused && !modifiers.has_command() => {
                self.pressed = true;
                self.armed = true;
                ctx.request_paint();
                self.fire(ctx);
                self.pressed = false;
                self.armed = false;
                ctx.request_paint();
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

    /// A default button doubles as an Enter accelerator for the entire
    /// container — pressing Enter while any non-button widget holds focus
    /// fires the default action. We piggyback on the existing accelerator
    /// routing so the parent container forwards keyboard events here
    /// even when the focus is parked on a sibling.
    fn accepts_accelerators(&self) -> bool {
        self.default
    }
}

fn draw_focus_rect(painter: &mut Painter, rect: Rect, color: crate::geometry::Color) {
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
