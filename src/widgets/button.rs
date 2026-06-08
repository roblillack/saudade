use crate::event::{Event, EventCtx, Key, MouseButton, NamedKey};
use crate::geometry::Rect;
use crate::painter::Painter;
use crate::theme::Theme;
use crate::widget::Widget;

type ClickHandler = Box<dyn FnMut(&mut EventCtx)>;

/// Source of an in-progress press. We track it explicitly so a pointer press
/// and a keyboard press don't accidentally fire each other's release.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Press {
    None,
    Mouse,
    Keyboard,
}

/// Classic Win 3.1 push button: raised face by default, sunken while pressed,
/// optional 1px outer black border for the dialog's default action.
///
/// Pointer and keyboard activation both follow the same press-then-release
/// model: a primary mouse press (or Enter/Space while focused) sinks the
/// button, the release fires the action — and only if the cursor is still
/// over the button (mouse) or the user hasn't backed out with Escape
/// (keyboard). This mirrors how Windows / Win 3.1 buttons work, lets the
/// pressed-state hint reach the screen before the action runs, and crucially
/// makes keyboard autorepeat a no-op: holding Enter only repeats KeyDown, so
/// the button stays armed but never fires until the user actually lifts the
/// key.
pub struct Button {
    pub rect: Rect,
    pub label: String,
    pub default: bool,
    press: Press,
    armed: bool,
    focused: bool,
    enabled: bool,
    on_click: Option<ClickHandler>,
}

impl Button {
    pub fn new(rect: Rect, label: impl Into<String>) -> Self {
        Self {
            rect,
            label: label.into(),
            default: false,
            press: Press::None,
            armed: false,
            focused: false,
            enabled: true,
            on_click: None,
        }
    }

    pub fn default(mut self, default: bool) -> Self {
        self.default = default;
        self
    }

    pub fn with_enabled(mut self, enabled: bool) -> Self {
        self.set_enabled(enabled);
        self
    }

    pub fn is_enabled(&self) -> bool {
        self.enabled
    }

    /// Enable or disable the button. A disabled button paints its label
    /// engraved (greyed), can't take focus, and ignores clicks and Enter / Space
    /// — including the default-button accelerator.
    pub fn set_enabled(&mut self, enabled: bool) {
        self.enabled = enabled;
        if !enabled {
            self.press = Press::None;
            self.armed = false;
        }
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

    /// Which keys arm the button on KeyDown. Enter / Space when focused;
    /// Enter alone when we're a non-focused default button (the dialog
    /// accelerator).
    fn keyboard_arms(&self, key: &Key) -> bool {
        if self.focused {
            matches!(
                key,
                Key::Named(NamedKey::Enter) | Key::Named(NamedKey::Space)
            )
        } else {
            self.default && matches!(key, Key::Named(NamedKey::Enter))
        }
    }
}

impl Widget for Button {
    fn bounds(&self) -> Rect {
        self.rect
    }

    fn paint(&mut self, painter: &mut Painter, theme: &Theme) {
        let pressed_visual = self.press != Press::None && self.armed;
        // The painter decides per-scale whether the frame needs a crisp
        // physical-pixel pass — this widget just hands it logical bounds.
        painter.button(self.rect, theme, pressed_visual, self.default);

        // When pressed, push the label one pixel down/right for tactile feel.
        let mut label_rect = self.rect;
        if pressed_visual {
            label_rect.x += 1;
            label_rect.y += 1;
        }
        if self.enabled {
            painter.text_centered(label_rect, &self.label, theme.font_size, theme.text);
        } else {
            // Engraved disabled label: a white copy nudged down-right, with the
            // grey text laid over it — the classic Win 3.1 greyed look.
            let mut emboss = label_rect;
            emboss.x += 1;
            emboss.y += 1;
            painter.text_centered(emboss, &self.label, theme.font_size, theme.highlight);
            painter.text_centered(
                label_rect,
                &self.label,
                theme.font_size,
                theme.disabled_text,
            );
        }

        // Dotted focus rectangle inside the bevel — the same chrome Win 3.1
        // drew on its focused buttons. Keep it inset enough that it doesn't
        // collide with the raised bevel highlights. `focus_rect` applies the
        // same per-scale crisp treatment as the frame.
        if self.focused && self.enabled {
            let inset = if self.default { 4 } else { 3 };
            painter.focus_rect(self.rect.inset(inset), theme.text);
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
                ..
            } if self.rect.contains(*pos) && self.press == Press::None => {
                self.press = Press::Mouse;
                self.armed = true;
                ctx.request_focus();
                ctx.request_paint();
            }
            Event::PointerMove { pos } if self.press == Press::Mouse => {
                let armed_now = self.rect.contains(*pos);
                if armed_now != self.armed {
                    self.armed = armed_now;
                    ctx.request_paint();
                }
            }
            Event::PointerUp {
                pos,
                button: MouseButton::Left,
                ..
            } if self.press == Press::Mouse => {
                let fire = self.armed && self.rect.contains(*pos);
                self.press = Press::None;
                self.armed = false;
                ctx.request_paint();
                if fire {
                    self.fire(ctx);
                }
            }
            Event::PointerLeave if self.press == Press::Mouse && self.armed => {
                self.armed = false;
                ctx.request_paint();
            }
            // Keyboard arm: Enter / Space while focused (or Enter anywhere when
            // we're the default-button accelerator) sinks the button; the action
            // waits for the matching KeyUp, so a held key fires once, on release,
            // just like a mouse press. The *first* KeyDown arms; an autorepeated
            // one is a no-op — but we still consume it, so a held default-button
            // accelerator can't leak past us to a focused sibling (e.g. in a
            // `Container`) and arm that one too.
            Event::KeyDown { key, modifiers }
                if self.press != Press::Mouse
                    && !modifiers.has_command()
                    && self.keyboard_arms(key) =>
            {
                if self.press == Press::None {
                    self.press = Press::Keyboard;
                    self.armed = true;
                    ctx.request_paint();
                }
                ctx.consume_event();
            }
            // Keyboard fire: release of the arming key activates if still armed.
            // If the user pressed Escape mid-press (handled below) we land here
            // with `armed == false` and just clean up without firing.
            Event::KeyUp { key, modifiers }
                if self.press == Press::Keyboard
                    && !modifiers.has_command()
                    && self.keyboard_arms(key) =>
            {
                let fire = self.armed;
                self.press = Press::None;
                self.armed = false;
                ctx.request_paint();
                if fire {
                    self.fire(ctx);
                }
                ctx.consume_event();
            }
            // Escape during a keyboard press cancels the arm — the matching
            // KeyUp won't fire. We consume the event so a hosting modal /
            // dialog *doesn't* also treat the same Escape as a dismiss
            // request: the first Esc backs out of the in-flight press, the
            // next one closes the dialog.
            Event::KeyDown {
                key: Key::Named(NamedKey::Escape),
                ..
            } if self.press == Press::Keyboard => {
                self.press = Press::None;
                self.armed = false;
                ctx.request_paint();
                ctx.consume_event();
            }
            _ => {}
        }
    }

    fn captures_pointer(&self) -> bool {
        self.press == Press::Mouse
    }

    fn focusable(&self) -> bool {
        self.enabled
    }

    fn set_focused(&mut self, focused: bool) {
        self.focused = focused;
        // Losing focus mid keyboard-press (the user tabbed away while holding
        // Enter) cancels the arm — there's no way to receive the matching
        // KeyUp here, and we don't want to fire when focus returns later.
        if !focused && self.press == Press::Keyboard {
            self.press = Press::None;
            self.armed = false;
        }
    }

    fn layout(&mut self, bounds: Rect) {
        self.rect = bounds;
    }

    /// A default button doubles as an Enter accelerator for the entire
    /// container — pressing Enter while any non-button widget holds focus
    /// fires the default action. We piggyback on the existing accelerator
    /// routing so the parent container forwards keyboard events here
    /// even when the focus is parked on a sibling. A disabled default
    /// button gives up the accelerator so Enter falls through.
    fn accepts_accelerators(&self) -> bool {
        self.default && self.enabled
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::event::Modifiers;

    fn enter() -> Event {
        Event::KeyDown {
            key: Key::Named(NamedKey::Enter),
            modifiers: Modifiers::default(),
        }
    }

    #[test]
    fn held_default_button_consumes_its_autorepeat() {
        // A default button arms on Enter even unfocused (the dialog
        // accelerator). While it stays armed, every autorepeated Enter must
        // also be consumed — otherwise, hosted in a `Container` next to a
        // focused sibling, the held key would leak through and arm that one too
        // (the "both buttons depressed" bug).
        let mut button = Button::new(Rect::new(0, 0, 70, 26), "OK").default(true);

        let mut ctx = EventCtx::new();
        button.event(&enter(), &mut ctx);
        assert!(ctx.is_consumed(), "the first Enter arms and is consumed");

        let mut ctx = EventCtx::new();
        button.event(&enter(), &mut ctx);
        assert!(
            ctx.is_consumed(),
            "an autorepeated Enter is still consumed while armed"
        );
    }
}
