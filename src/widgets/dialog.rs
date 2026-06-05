use crate::event::{Event, EventCtx, Key, MouseButton, NamedKey};
use crate::geometry::{Rect, Size};
use crate::include_svg;
use crate::painter::Painter;
use crate::svg::SvgImage;
use crate::theme::Theme;
use crate::widget::{PopupRequest, Widget};
use crate::widgets::modal::Modal;

const BUTTON_W: i32 = 70;
// Tall enough to give the 13pt label comfortable breathing room above and
// below — matches the push-button height the CRUD example uses.
const BUTTON_H: i32 = 26;
const ICON_SIZE: i32 = 32;
const PADDING: i32 = 16;
/// Vertical breathing room between the message block and the OK button.
const BUTTON_GAP: i32 = 16;
/// Height of one message line: the default chrome font (13pt) plus a few px of
/// leading. Kept in sync with the per-line advance in `MessageBody::paint` so
/// the auto-computed window height matches what actually gets drawn.
const MSG_LINE_HEIGHT: i32 = 16;
/// Classic message-box width. The height is derived from the message instead
/// (see [`message_box_size`]), so the box grows to fit multi-line text and
/// never crowds the OK button.
const DEFAULT_WIDTH: i32 = 340;
/// Horizontal padding around a confirm button's label, each side.
const CONFIRM_BTN_PAD: i32 = 14;
/// Gap between the two buttons of a confirm box.
const CONFIRM_BTN_GAP: i32 = 10;
/// The cancel button's fixed label.
const CANCEL_LABEL: &str = "Cancel";
/// Rough advance of one chrome-font character at the default size, used only to
/// auto-size a confirm box without a font in hand (the actual glyph widths are
/// measured at paint time). Deliberately generous so text never clips.
const APPROX_CHAR_W: i32 = 8;

/// Size a message box to its content: a fixed classic width, and a height that
/// stacks the icon / message lines, a gap, and the OK button, each framed by
/// `PADDING`. Because the message is top-anchored and the button bottom-
/// anchored, fitting the content here is what keeps the two from colliding.
fn message_box_size(message: &str, icon: DialogIcon) -> Size {
    let lines = message.split('\n').count() as i32;
    let text_h = lines * MSG_LINE_HEIGHT;
    let icon_h = if icon == DialogIcon::None {
        0
    } else {
        ICON_SIZE
    };
    let content_h = text_h.max(icon_h);
    let height = PADDING + content_h + BUTTON_GAP + BUTTON_H + PADDING;
    Size::new(DEFAULT_WIDTH, height)
}

/// Width of a confirm button sized to hold `label`, never narrower than the
/// classic [`BUTTON_W`]. Estimated from [`APPROX_CHAR_W`] so it can be computed
/// without a font; [`ConfirmBody`] re-measures the real glyphs at paint time,
/// and the box is sized generously enough to seat either.
fn confirm_button_w(label: &str) -> i32 {
    (label.chars().count() as i32 * APPROX_CHAR_W + 2 * CONFIRM_BTN_PAD).max(BUTTON_W)
}

/// Size a confirm box: the icon/message stack is measured exactly as a message
/// box, but the width is also widened to seat the affirmative + Cancel button
/// row, and to fit the (un-wrapped) message lines.
fn confirm_box_size(message: &str, icon: DialogIcon, affirm: &str) -> Size {
    let base = message_box_size(message, icon);
    let longest = message
        .split('\n')
        .map(|l| l.chars().count() as i32)
        .max()
        .unwrap_or(0);
    let icon_w = if icon == DialogIcon::None {
        0
    } else {
        ICON_SIZE + PADDING
    };
    let text_w = PADDING + icon_w + longest * APPROX_CHAR_W + PADDING;
    let buttons_w = PADDING
        + confirm_button_w(affirm)
        + CONFIRM_BTN_GAP
        + confirm_button_w(CANCEL_LABEL)
        + PADDING;
    Size::new(base.w.max(text_w).max(buttons_w), base.h)
}

/// What icon — if any — to show on the left of the message.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DialogIcon {
    None,
    Info,
    Warning,
    Error,
}

/// A modal message / alert box.
///
/// `Dialog` is the ready-made message box built on the general-purpose
/// [`Modal`] facility: it hosts a body that draws an icon, a wrapped message,
/// and a single OK button. The application owns it (e.g., via
/// `Rc<RefCell<Dialog>>`) as an overlay and calls `show_warning` / `show_info`
/// / `show_error` to display it; OK, Enter, Space, Escape, or the window's
/// close button all dismiss it.
///
/// As with any [`Modal`], the dialog opens in a real top-level window
/// (transient to the main window, with server-side decorations), so no
/// client-side title bar is drawn — the title rides along on the
/// [`PopupRequest`] and becomes the OS window title.
///
/// To run code when the dialog closes (the classic "OK closes the window"),
/// install an [`on_dismiss`](Dialog::on_dismiss) handler.
pub struct Dialog {
    modal: Modal,
    /// Explicit size set via [`with_size`]; `None` lets the dialog size itself
    /// to its message each time it's shown.
    size: Option<Size>,
}

impl Dialog {
    pub fn new() -> Self {
        Self {
            modal: Modal::new(),
            size: None,
        }
    }

    /// Pin the dialog to a fixed size, opting out of the content-based
    /// auto-sizing that [`show`](Self::show) otherwise applies.
    pub fn with_size(mut self, width: i32, height: i32) -> Self {
        self.size = Some(Size::new(width.max(120), height.max(60)));
        self
    }

    pub fn on_dismiss(mut self, handler: impl FnMut(&mut EventCtx) + 'static) -> Self {
        self.modal.set_on_dismiss(handler);
        self
    }

    pub fn show(&mut self, title: impl Into<String>, message: impl Into<String>, icon: DialogIcon) {
        let message = message.into();
        let size = self
            .size
            .unwrap_or_else(|| message_box_size(&message, icon));
        self.modal
            .show(title, size, Box::new(MessageBody::new(icon, message)));
    }

    pub fn show_warning(&mut self, title: impl Into<String>, message: impl Into<String>) {
        self.show(title, message, DialogIcon::Warning);
    }

    pub fn show_info(&mut self, title: impl Into<String>, message: impl Into<String>) {
        self.show(title, message, DialogIcon::Info);
    }

    pub fn show_error(&mut self, title: impl Into<String>, message: impl Into<String>) {
        self.show(title, message, DialogIcon::Error);
    }

    /// Show a two-button confirmation box. The affirmative button is labeled
    /// `affirm` and runs `on_confirm` before closing; the Cancel button, the
    /// window's close button, Escape, and Enter all close it *without* running
    /// the handler — so the safe choice is the one a stray keypress lands on. A
    /// warning icon flags that the affirmative action is the consequential one.
    pub fn show_confirm(
        &mut self,
        title: impl Into<String>,
        message: impl Into<String>,
        affirm: impl Into<String>,
        on_confirm: impl FnMut(&mut EventCtx) + 'static,
    ) {
        let message = message.into();
        let affirm = affirm.into();
        let icon = DialogIcon::Warning;
        let size = self
            .size
            .unwrap_or_else(|| confirm_box_size(&message, icon, &affirm));
        self.modal.show(
            title,
            size,
            Box::new(ConfirmBody::new(
                icon,
                message,
                affirm,
                Box::new(on_confirm),
            )),
        );
    }

    pub fn dismiss(&mut self) {
        self.modal.dismiss();
    }

    pub fn is_open(&self) -> bool {
        self.modal.is_open()
    }
}

impl Default for Dialog {
    fn default() -> Self {
        Self::new()
    }
}

// `Dialog` is just a `Modal` with a fixed body, so every `Widget` method
// delegates straight through.
impl Widget for Dialog {
    fn bounds(&self) -> Rect {
        self.modal.bounds()
    }
    fn layout(&mut self, bounds: Rect) {
        self.modal.layout(bounds);
    }
    fn paint(&mut self, painter: &mut Painter, theme: &Theme) {
        self.modal.paint(painter, theme);
    }
    fn paint_overlay(&mut self, painter: &mut Painter, theme: &Theme) {
        self.modal.paint_overlay(painter, theme);
    }
    fn event(&mut self, event: &Event, ctx: &mut EventCtx) {
        self.modal.event(event, ctx);
    }
    fn captures_pointer(&self) -> bool {
        self.modal.captures_pointer()
    }
    fn accepts_accelerators(&self) -> bool {
        self.modal.accepts_accelerators()
    }
    fn popup_request(&self) -> Option<PopupRequest> {
        self.modal.popup_request()
    }
    fn collect_popups(&self, out: &mut Vec<PopupRequest>) {
        self.modal.collect_popups(out);
    }
    fn wants_ticks(&self) -> bool {
        self.modal.wants_ticks()
    }
}

/// The message box's body: the icon, the wrapped message, and the OK button.
/// Hosted as a [`Modal`]'s content and laid out into the dialog's client rect,
/// so all positions are taken relative to that rect.
struct MessageBody {
    icon: DialogIcon,
    message: String,
    rect: Rect,
    /// Active press source on the OK button. `None` is the resting state;
    /// `Mouse` is a pointer press in progress, `Keyboard` is Enter / Space
    /// held while the dialog has focus.
    button_press: BodyPress,
    /// Whether the active press is currently "armed" — pointer still over the
    /// button, or Escape hasn't canceled the keyboard arm yet. A release
    /// (PointerUp / KeyUp) fires the action only when this is `true`.
    button_armed: bool,
}

/// Source of an in-progress button press inside a dialog body. Pointer and
/// keyboard presses are tracked separately so a keyboard release doesn't
/// fire a button the mouse is still holding, and vice versa.
#[derive(Clone, Copy, PartialEq, Eq)]
enum BodyPress {
    None,
    Mouse,
    Keyboard,
}

impl MessageBody {
    fn new(icon: DialogIcon, message: String) -> Self {
        Self {
            icon,
            message,
            rect: Rect::new(0, 0, 0, 0),
            button_press: BodyPress::None,
            button_armed: false,
        }
    }

    fn button_rect(&self) -> Rect {
        let bx = self.rect.x + (self.rect.w - BUTTON_W) / 2;
        let by = self.rect.bottom() - BUTTON_H - PADDING;
        Rect::new(bx, by, BUTTON_W, BUTTON_H)
    }
}

impl Widget for MessageBody {
    fn bounds(&self) -> Rect {
        self.rect
    }

    fn layout(&mut self, bounds: Rect) {
        self.rect = bounds;
    }

    fn paint(&mut self, painter: &mut Painter, theme: &Theme) {
        let body = self.rect;

        // Icon on the left, wrapped message lines on the right.
        let body_y = body.y + PADDING;
        let icon_x = body.x + PADDING;
        if self.icon != DialogIcon::None {
            draw_icon(painter, icon_x, body_y, ICON_SIZE, self.icon);
        }
        let msg_x = if self.icon == DialogIcon::None {
            body.x + PADDING
        } else {
            icon_x + ICON_SIZE + PADDING
        };
        let mut msg_y = body_y;
        for line in self.message.split('\n') {
            painter.text(msg_x, msg_y, line, theme.font_size, theme.text);
            msg_y += (theme.font_size as i32) + 3;
        }

        // OK button — default-styled (1-px outer black border) so Enter is the
        // obvious confirm key.
        let btn = self.button_rect();
        let pressed = self.button_press != BodyPress::None && self.button_armed;
        painter.button(btn, theme, pressed, true);
        let inset = if pressed { 1 } else { 0 };
        painter.text_centered(
            Rect::new(btn.x + inset, btn.y + inset, btn.w, btn.h),
            "OK",
            theme.font_size,
            theme.text,
        );
    }

    fn event(&mut self, event: &Event, ctx: &mut EventCtx) {
        let btn = self.button_rect();
        match event {
            Event::PointerDown {
                pos,
                button: MouseButton::Left,
            } if btn.contains(*pos) && self.button_press == BodyPress::None => {
                self.button_press = BodyPress::Mouse;
                self.button_armed = true;
                ctx.request_paint();
            }
            Event::PointerMove { pos } if self.button_press == BodyPress::Mouse => {
                let in_btn = btn.contains(*pos);
                if in_btn != self.button_armed {
                    self.button_armed = in_btn;
                    ctx.request_paint();
                }
            }
            Event::PointerUp {
                pos,
                button: MouseButton::Left,
            } if self.button_press == BodyPress::Mouse => {
                let fire = self.button_armed && btn.contains(*pos);
                self.button_press = BodyPress::None;
                self.button_armed = false;
                ctx.request_paint();
                if fire {
                    ctx.request_dismiss();
                }
            }
            // Enter / Space arm the OK button on KeyDown; the matching KeyUp
            // fires the dismiss. Autorepeat just re-arms and never fires,
            // mirroring how a mouse press waits for the release. Escape is
            // handled by the hosting `Modal`, but if a keyboard press is
            // in progress we clear it here first so the matching KeyUp is
            // a no-op (and the Modal still gets the Escape to dismiss).
            Event::KeyDown {
                key: Key::Named(NamedKey::Enter | NamedKey::Space),
                modifiers,
            } if !modifiers.has_command() && self.button_press == BodyPress::None => {
                self.button_press = BodyPress::Keyboard;
                self.button_armed = true;
                ctx.request_paint();
            }
            Event::KeyUp {
                key: Key::Named(NamedKey::Enter | NamedKey::Space),
                modifiers,
            } if !modifiers.has_command() && self.button_press == BodyPress::Keyboard => {
                let fire = self.button_armed;
                self.button_press = BodyPress::None;
                self.button_armed = false;
                ctx.request_paint();
                if fire {
                    ctx.request_dismiss();
                }
            }
            Event::KeyDown {
                key: Key::Named(NamedKey::Escape),
                ..
            } if self.button_press == BodyPress::Keyboard => {
                self.button_press = BodyPress::None;
                self.button_armed = false;
                ctx.request_paint();
                ctx.consume_event();
            }
            _ => {}
        }
    }
}

type ConfirmHandler = Box<dyn FnMut(&mut EventCtx)>;

/// The body of a [`Dialog::show_confirm`] box: an icon, the message, and a
/// button *row* — the affirmative action plus Cancel. Mirrors [`MessageBody`]'s
/// icon/message layout, but seats two label-sized buttons instead of a lone OK,
/// and only the affirmative one runs the caller's handler.
struct ConfirmBody {
    icon: DialogIcon,
    message: String,
    affirm: String,
    on_confirm: ConfirmHandler,
    rect: Rect,
    /// Button rects, recomputed each paint (where the font is available to
    /// measure the labels) and read back when hit-testing pointer events.
    affirm_rect: Rect,
    cancel_rect: Rect,
    /// Which button a press is held on — `Some(true)` = affirmative,
    /// `Some(false)` = Cancel — and whether the pointer is still over it.
    /// The cancel button binds to Enter (default) so Enter / Space dismiss
    /// without confirming.
    pressed: Option<bool>,
    /// Source of the active press. Keeps a keyboard release from firing a
    /// mouse-held button and vice versa, and lets Escape cancel an in-flight
    /// keyboard press without disturbing a parallel mouse press.
    press_source: BodyPress,
    armed: bool,
}

impl ConfirmBody {
    fn new(icon: DialogIcon, message: String, affirm: String, on_confirm: ConfirmHandler) -> Self {
        Self {
            icon,
            message,
            affirm,
            on_confirm,
            rect: Rect::new(0, 0, 0, 0),
            affirm_rect: Rect::new(0, 0, 0, 0),
            cancel_rect: Rect::new(0, 0, 0, 0),
            pressed: None,
            press_source: BodyPress::None,
            armed: false,
        }
    }

    /// Lay out the affirmative + Cancel buttons as a centered row along the
    /// bottom, each sized to its measured label. Called from `paint` (the only
    /// place a font is in hand); the rects are cached for the next event's hit
    /// test, matching the warm-up-render-then-dispatch order the runtime uses.
    fn layout_buttons(&mut self, painter: &Painter, theme: &Theme) {
        let button_w = |label: &str| {
            (painter.measure_text(label, theme.font_size).w + 2 * CONFIRM_BTN_PAD).max(BUTTON_W)
        };
        let aw = button_w(&self.affirm);
        let cw = button_w(CANCEL_LABEL);
        let total = aw + CONFIRM_BTN_GAP + cw;
        let bx = self.rect.x + (self.rect.w - total) / 2;
        let by = self.rect.bottom() - BUTTON_H - PADDING;
        self.affirm_rect = Rect::new(bx, by, aw, BUTTON_H);
        self.cancel_rect = Rect::new(bx + aw + CONFIRM_BTN_GAP, by, cw, BUTTON_H);
    }

    fn draw_button(painter: &mut Painter, theme: &Theme, rect: Rect, label: &str, default: bool) {
        painter.button(rect, theme, false, default);
        painter.text_centered(rect, label, theme.font_size, theme.text);
    }
}

impl Widget for ConfirmBody {
    fn bounds(&self) -> Rect {
        self.rect
    }

    fn layout(&mut self, bounds: Rect) {
        self.rect = bounds;
    }

    fn paint(&mut self, painter: &mut Painter, theme: &Theme) {
        let body = self.rect;
        let body_y = body.y + PADDING;
        let icon_x = body.x + PADDING;
        if self.icon != DialogIcon::None {
            draw_icon(painter, icon_x, body_y, ICON_SIZE, self.icon);
        }
        let msg_x = if self.icon == DialogIcon::None {
            body.x + PADDING
        } else {
            icon_x + ICON_SIZE + PADDING
        };
        let mut msg_y = body_y;
        for line in self.message.split('\n') {
            painter.text(msg_x, msg_y, line, theme.font_size, theme.text);
            msg_y += (theme.font_size as i32) + 3;
        }

        self.layout_buttons(painter, theme);
        // The pressed button sinks 1px (drawn directly); Cancel carries the
        // default border, so the safe choice is the visually default one.
        let affirm = self.affirm_rect;
        let cancel = self.cancel_rect;
        match (self.pressed, self.armed) {
            (Some(true), true) => {
                painter.button(affirm, theme, true, false);
                painter.text_centered(
                    Rect::new(affirm.x + 1, affirm.y + 1, affirm.w, affirm.h),
                    &self.affirm,
                    theme.font_size,
                    theme.text,
                );
                Self::draw_button(painter, theme, cancel, CANCEL_LABEL, true);
            }
            (Some(false), true) => {
                Self::draw_button(painter, theme, affirm, &self.affirm, false);
                painter.button(cancel, theme, true, true);
                painter.text_centered(
                    Rect::new(cancel.x + 1, cancel.y + 1, cancel.w, cancel.h),
                    CANCEL_LABEL,
                    theme.font_size,
                    theme.text,
                );
            }
            _ => {
                Self::draw_button(painter, theme, affirm, &self.affirm, false);
                Self::draw_button(painter, theme, cancel, CANCEL_LABEL, true);
            }
        }
    }

    fn event(&mut self, event: &Event, ctx: &mut EventCtx) {
        match event {
            Event::PointerDown {
                pos,
                button: MouseButton::Left,
            } if self.press_source == BodyPress::None => {
                if self.affirm_rect.contains(*pos) {
                    self.pressed = Some(true);
                    self.press_source = BodyPress::Mouse;
                    self.armed = true;
                    ctx.request_paint();
                } else if self.cancel_rect.contains(*pos) {
                    self.pressed = Some(false);
                    self.press_source = BodyPress::Mouse;
                    self.armed = true;
                    ctx.request_paint();
                }
            }
            Event::PointerMove { pos } if self.press_source == BodyPress::Mouse => {
                let rect = if self.pressed == Some(true) {
                    self.affirm_rect
                } else {
                    self.cancel_rect
                };
                let over = rect.contains(*pos);
                if over != self.armed {
                    self.armed = over;
                    ctx.request_paint();
                }
            }
            Event::PointerUp {
                pos,
                button: MouseButton::Left,
            } if self.press_source == BodyPress::Mouse => {
                let affirm = self.pressed == Some(true);
                let rect = if affirm {
                    self.affirm_rect
                } else {
                    self.cancel_rect
                };
                let fire = self.armed && rect.contains(*pos);
                self.pressed = None;
                self.press_source = BodyPress::None;
                self.armed = false;
                ctx.request_paint();
                if fire {
                    if affirm {
                        (self.on_confirm)(ctx);
                    }
                    ctx.request_dismiss();
                }
            }
            // Enter / Space arm the Cancel button (the default) on KeyDown; the
            // matching KeyUp dismisses without confirming. Autorepeated KeyDowns
            // re-arm but never fire — fire is reserved for the release, like a
            // mouse click. Escape during the keyboard press cancels the arm and
            // is left for the hosting `Modal` to also dismiss the dialog.
            Event::KeyDown {
                key: Key::Named(NamedKey::Enter | NamedKey::Space),
                modifiers,
            } if !modifiers.has_command() && self.press_source == BodyPress::None => {
                self.pressed = Some(false);
                self.press_source = BodyPress::Keyboard;
                self.armed = true;
                ctx.request_paint();
            }
            Event::KeyUp {
                key: Key::Named(NamedKey::Enter | NamedKey::Space),
                modifiers,
            } if !modifiers.has_command() && self.press_source == BodyPress::Keyboard => {
                let fire = self.armed;
                self.pressed = None;
                self.press_source = BodyPress::None;
                self.armed = false;
                ctx.request_paint();
                if fire {
                    ctx.request_dismiss();
                }
            }
            Event::KeyDown {
                key: Key::Named(NamedKey::Escape),
                ..
            } if self.press_source == BodyPress::Keyboard => {
                self.pressed = None;
                self.press_source = BodyPress::None;
                self.armed = false;
                ctx.request_paint();
                ctx.consume_event();
            }
            _ => {}
        }
    }
}

// The three message-box marks, baked from SVG at compile time. They render the
// same Win 3.1 alert glyphs the dialog used to draw by hand, but as crisp,
// DPI-independent vector art (and with no SVG machinery in the binary). The
// paths resolve relative to the crate root — see `include_svg!`.
const INFO_ICON: SvgImage = include_svg!("assets/dialog/info.svg");
const WARNING_ICON: SvgImage = include_svg!("assets/dialog/warning.svg");
const ERROR_ICON: SvgImage = include_svg!("assets/dialog/error.svg");

/// Draw a Win 3.1-style alert icon into the `size × size` box at `(x, y)`.
fn draw_icon(painter: &mut Painter, x: i32, y: i32, size: i32, icon: DialogIcon) {
    let svg = match icon {
        DialogIcon::None => return,
        DialogIcon::Info => &INFO_ICON,
        DialogIcon::Warning => &WARNING_ICON,
        DialogIcon::Error => &ERROR_ICON,
    };
    svg.draw(painter, Rect::new(x, y, size, size));
}
