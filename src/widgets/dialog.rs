use crate::event::{Event, EventCtx, Key, MouseButton, NamedKey};
use crate::geometry::{Color, Rect, Size};
use crate::painter::Painter;
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
    button_pressed: bool,
    button_armed: bool,
}

impl MessageBody {
    fn new(icon: DialogIcon, message: String) -> Self {
        Self {
            icon,
            message,
            rect: Rect::new(0, 0, 0, 0),
            button_pressed: false,
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
        let pressed = self.button_pressed && self.button_armed;
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
            } if btn.contains(*pos) => {
                self.button_pressed = true;
                self.button_armed = true;
                ctx.request_paint();
            }
            Event::PointerMove { pos } if self.button_pressed => {
                let in_btn = btn.contains(*pos);
                if in_btn != self.button_armed {
                    self.button_armed = in_btn;
                    ctx.request_paint();
                }
            }
            Event::PointerUp {
                pos,
                button: MouseButton::Left,
            } if self.button_pressed => {
                let fire = self.button_armed && btn.contains(*pos);
                self.button_pressed = false;
                self.button_armed = false;
                ctx.request_paint();
                if fire {
                    ctx.request_dismiss();
                }
            }
            // Enter / Space confirm; Escape is handled by the hosting `Modal`.
            Event::KeyDown {
                key: Key::Named(NamedKey::Enter | NamedKey::Space),
                ..
            } => {
                ctx.request_dismiss();
            }
            _ => {}
        }
    }
}

/// Draw a Win 3.1-style icon at `(x, y)` with the given pixel size.
fn draw_icon(painter: &mut Painter, x: i32, y: i32, size: i32, icon: DialogIcon) {
    match icon {
        DialogIcon::None => {}
        DialogIcon::Warning => {
            // Yellow filled triangle with a black "!".
            let yellow = Color::rgb(0xFF, 0xCC, 0x00);
            let black = Color::BLACK;
            let apex_x = x + size / 2;
            let bottom_y = y + size - 1;
            // Fill the triangle row by row, widening linearly from the apex.
            for row in 0..size {
                let half = (row as f32 * (size as f32 / 2.0) / size as f32).round() as i32;
                let line_x = apex_x - half;
                let line_w = (half * 2 + 1).max(1);
                painter.h_line(line_x, y + row, line_w, yellow);
            }
            // Black border along the two slopes + bottom edge.
            for row in 0..size {
                let half = (row as f32 * (size as f32 / 2.0) / size as f32).round() as i32;
                painter.pixel(apex_x - half, y + row, black);
                painter.pixel(apex_x + half, y + row, black);
            }
            painter.h_line(x, bottom_y, size, black);
            // Exclamation mark — vertical bar + dot.
            let bar_x = apex_x - 1;
            painter.fill_rect(Rect::new(bar_x, y + 10, 2, 12), black);
            painter.fill_rect(Rect::new(bar_x, y + 24, 2, 2), black);
        }
        DialogIcon::Info => {
            // Blue circle with a white "i". Approximated as a filled
            // rectangle with rounded-feeling corners.
            let blue = Color::NAVY;
            let white = Color::WHITE;
            painter.fill_rect(Rect::new(x + 2, y, size - 4, size), blue);
            painter.fill_rect(Rect::new(x, y + 2, size, size - 4), blue);
            painter.fill_rect(Rect::new(x + 1, y + 1, size - 2, size - 2), blue);
            // Dot above + bar below for the "i".
            let mid = x + size / 2 - 1;
            painter.fill_rect(Rect::new(mid, y + 6, 2, 2), white);
            painter.fill_rect(Rect::new(mid, y + 11, 2, 14), white);
        }
        DialogIcon::Error => {
            // Red square with white "X".
            let red = Color::RED;
            let white = Color::WHITE;
            painter.fill_rect(Rect::new(x + 2, y, size - 4, size), red);
            painter.fill_rect(Rect::new(x, y + 2, size, size - 4), red);
            painter.fill_rect(Rect::new(x + 1, y + 1, size - 2, size - 2), red);
            // Diagonal lines for the X.
            for i in 0..size - 12 {
                painter.pixel(x + 6 + i, y + 6 + i, white);
                painter.pixel(x + 6 + i + 1, y + 6 + i, white);
                painter.pixel(x + size - 7 - i, y + 6 + i, white);
                painter.pixel(x + size - 7 - i - 1, y + 6 + i, white);
            }
        }
    }
}
