use crate::event::{Event, EventCtx};
use crate::geometry::{Rect, Size};
use crate::include_svg;
use crate::painter::Painter;
use crate::svg::SvgImage;
use crate::theme::Theme;
use crate::widget::{PopupRequest, Widget};
use crate::widgets::modal::Modal;
use crate::widgets::{Button, Container};

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
                size,
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
/// so all positions are taken relative to that rect. The OK button is a real
/// [`Button`] — it owns the press-then-release behaviour for pointer and
/// keyboard, and its `on_click` simply asks the modal to dismiss.
struct MessageBody {
    icon: DialogIcon,
    message: String,
    ok: Button,
    rect: Rect,
}

impl MessageBody {
    fn new(icon: DialogIcon, message: String) -> Self {
        // Default-styled (1-px outer black border) so Enter is the obvious
        // confirm key; dismissing the modal is all OK needs to do.
        let ok = Button::new(Rect::new(0, 0, BUTTON_W, BUTTON_H), "OK")
            .default(true)
            .on_click(|ctx| ctx.request_dismiss());
        Self {
            icon,
            message,
            ok,
            rect: Rect::new(0, 0, 0, 0),
        }
    }

    /// The OK button, centered along the bottom of `rect`.
    fn button_rect(rect: Rect) -> Rect {
        let bx = rect.x + (rect.w - BUTTON_W) / 2;
        let by = rect.bottom() - BUTTON_H - PADDING;
        Rect::new(bx, by, BUTTON_W, BUTTON_H)
    }
}

impl Widget for MessageBody {
    fn bounds(&self) -> Rect {
        self.rect
    }

    fn layout(&mut self, bounds: Rect) {
        self.rect = bounds;
        self.ok.rect = Self::button_rect(bounds);
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

        self.ok.paint(painter, theme);
    }

    fn event(&mut self, event: &Event, ctx: &mut EventCtx) {
        self.ok.event(event, ctx);
    }

    fn focusable(&self) -> bool {
        true
    }

    fn focus_first(&mut self) -> bool {
        self.ok.set_focused(true);
        true
    }

    fn captures_pointer(&self) -> bool {
        self.ok.captures_pointer()
    }
}

type ConfirmHandler = Box<dyn FnMut(&mut EventCtx)>;

/// The body of a [`Dialog::show_confirm`] box: an icon, the message, and a
/// button *row* — the affirmative action plus Cancel. Mirrors [`MessageBody`]'s
/// icon/message layout, but seats two real [`Button`]s in a [`Container`] (which
/// owns their focus / Tab / press handling) instead of a lone OK. Only the
/// affirmative button runs the caller's handler; Cancel — the default, and the
/// initially-focused button — just dismisses, so Enter or a stray Space backs
/// out without confirming.
struct ConfirmBody {
    icon: DialogIcon,
    message: String,
    rect: Rect,
    body: Container,
}

impl ConfirmBody {
    fn new(
        icon: DialogIcon,
        message: String,
        affirm: String,
        mut on_confirm: ConfirmHandler,
        size: Size,
    ) -> Self {
        // A centered button row along the bottom, each sized from an estimate
        // of its label width — the same estimate `confirm_box_size` uses to size
        // the window, so the two always agree. Affirm on the left, Cancel right.
        let aw = confirm_button_w(&affirm);
        let cw = confirm_button_w(CANCEL_LABEL);
        let total = aw + CONFIRM_BTN_GAP + cw;
        let bx = (size.w - total) / 2;
        let by = size.h - BUTTON_H - PADDING;

        let affirm_btn =
            Button::new(Rect::new(bx, by, aw, BUTTON_H), affirm).on_click(move |ctx| {
                on_confirm(ctx);
                ctx.request_dismiss();
            });
        // Cancel is the default (Enter accelerator) and carries the visible
        // default border, so a stray confirm key lands on the safe choice.
        let cancel_btn = Button::new(
            Rect::new(bx + aw + CONFIRM_BTN_GAP, by, cw, BUTTON_H),
            CANCEL_LABEL,
        )
        .default(true)
        .on_click(|ctx| ctx.request_dismiss());

        let mut body = Container::new(size.w, size.h);
        body.push(affirm_btn);
        body.push(cancel_btn);

        Self {
            icon,
            message,
            rect: Rect::new(0, 0, 0, 0),
            body,
        }
    }
}

impl Widget for ConfirmBody {
    fn bounds(&self) -> Rect {
        self.rect
    }

    fn layout(&mut self, bounds: Rect) {
        self.rect = bounds;
        self.body.layout(bounds);
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

        self.body.paint(painter, theme);
    }

    fn event(&mut self, event: &Event, ctx: &mut EventCtx) {
        self.body.event(event, ctx);
    }

    fn focusable(&self) -> bool {
        self.body.focusable()
    }

    fn focus_first(&mut self) -> bool {
        // Open with Cancel (the second button) focused so Enter *and* a stray
        // Space back out safely; confirming takes a deliberate Tab to the
        // affirmative button.
        self.body.focus_child(1)
    }

    fn captures_pointer(&self) -> bool {
        self.body.captures_pointer()
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
