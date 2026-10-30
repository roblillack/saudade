use crate::event::{Event, EventCtx};
use crate::font::FontFamily;
use crate::geometry::{Rect, Size};
use crate::include_svg;
use crate::painter::Painter;
use crate::svg::SvgImage;
use crate::theme::Theme;
use crate::widget::{PopupRequest, Widget};
use crate::widgets::modal::Modal;
use crate::widgets::{Button, Container, Label};

const BUTTON_W: i32 = 70;
// Tall enough to give the 13pt label comfortable breathing room above and
// below — matches the push-button height the CRUD example uses.
const BUTTON_H: i32 = 26;
const ICON_SIZE: i32 = 32;
const PADDING: i32 = 16;
/// Vertical breathing room between the message block and the OK button.
const BUTTON_GAP: i32 = 16;
/// Classic message-box width. The message is word-wrapped to fit it, and the
/// height is derived from how many lines that takes (see [`fit`]), so a long
/// message grows the box downwards and never crowds the buttons.
const DEFAULT_WIDTH: i32 = 340;
/// Horizontal padding around a confirm button's label, each side.
const CONFIRM_BTN_PAD: i32 = 14;
/// Gap between the two buttons of a confirm box.
const CONFIRM_BTN_GAP: i32 = 10;
/// The cancel button's fixed label.
const CANCEL_LABEL: &str = "Cancel";

/// Size a box to its content: the classic width — or wider, if that is what it
/// takes to seat a button row `buttons_w` wide — and a height that stacks the
/// icon / wrapped message, a gap, and the buttons, each framed by `PADDING`.
/// Because the message is top-anchored and the buttons bottom-anchored,
/// fitting the content here is what keeps the two from colliding.
fn fit(
    painter: &Painter,
    theme: &Theme,
    message: &Label,
    icon: DialogIcon,
    buttons_w: i32,
) -> Size {
    let width = DEFAULT_WIDTH.max(PADDING + buttons_w + PADDING);
    let text_h = message.text_height(painter, theme, width - message_x(icon) - PADDING);
    let icon_h = if icon == DialogIcon::None {
        0
    } else {
        ICON_SIZE
    };
    let height = PADDING + text_h.max(icon_h) + BUTTON_GAP + BUTTON_H + PADDING;
    Size::new(width, height)
}

/// Width of a confirm button sized to hold `label` in the face buttons are set
/// in, never narrower than the classic [`BUTTON_W`].
fn confirm_button_w(painter: &Painter, theme: &Theme, label: &str) -> i32 {
    let text_w = painter
        .measure_text_styled(label, theme.font_size, FontFamily::Sans, theme.button_style)
        .w;
    (text_w + 2 * CONFIRM_BTN_PAD).max(BUTTON_W)
}

/// How far in from a body's left edge the message starts: past the icon, if
/// there is one.
fn message_x(icon: DialogIcon) -> i32 {
    if icon == DialogIcon::None {
        PADDING
    } else {
        PADDING + ICON_SIZE + PADDING
    }
}

/// Where the message goes in a body laid out at `rect`: right of the icon,
/// from the top padding down to the gap above the buttons. The message is a
/// [`Label`], which clips to its rect, so a message too long for a box pinned
/// with [`Dialog::with_size`] is cut off there rather than running under the
/// buttons.
fn message_rect(rect: Rect, icon: DialogIcon) -> Rect {
    let x = rect.x + message_x(icon);
    let y = rect.y + PADDING;
    let bottom = rect.bottom() - PADDING - BUTTON_H - BUTTON_GAP;
    Rect::new(x, y, rect.right() - PADDING - x, bottom - y)
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
/// [`Modal`]: it hosts a body that draws an icon, the message, and a single OK
/// button. The message is word-wrapped to the box's classic width (see
/// [`Painter::wrap_text`]), and the box grows in height to fit it, so it takes
/// no line breaks of its own — a `\n` is for starting a new paragraph. The
/// application owns it (e.g., via `Rc<RefCell<Dialog>>`) as an overlay and
/// calls `show_warning` / `show_info` / `show_error` to display it; OK, Enter,
/// Space, Escape, or the window's close button all dismiss it.
///
/// As with any [`Modal`], the dialog opens in a real top-level window
/// (transient to the main window, with server-side decorations), so no
/// client-side title bar is drawn — the title rides along on the
/// [`PopupRequest`] and becomes the OS window title.
///
/// How big that window has to be depends on how the message wraps, which
/// takes a font to work out, so the box measures itself on the first paint
/// after it is shown and opens its window then — like a
/// [`ContextMenu`](crate::ContextMenu) does. It counts as open from the moment
/// it is shown.
///
/// To run code when the dialog closes (the classic "OK closes the window"),
/// install an [`on_dismiss`](Dialog::on_dismiss) handler.
pub struct Dialog {
    modal: Modal,
    /// Explicit size set via [`with_size`]; `None` lets the dialog size itself
    /// to its message each time it's shown.
    size: Option<Size>,
    /// The box last asked for, until a paint has measured and opened it.
    pending: Option<Pending>,
}

/// A box [`Dialog::show`] or [`Dialog::show_confirm`] asked for, waiting for
/// a painter to measure its message with.
struct Pending {
    title: String,
    icon: DialogIcon,
    message: String,
    /// A confirm box's affirmative button: its label and what it runs. `None`
    /// for a message box, which has just OK.
    confirm: Option<(String, ConfirmHandler)>,
}

impl Dialog {
    pub fn new() -> Self {
        Self {
            modal: Modal::new(),
            size: None,
            pending: None,
        }
    }

    /// Pin the dialog to a fixed size, opting out of the content-based
    /// auto-sizing that [`show`](Self::show) otherwise applies. The message
    /// still wraps to the width this leaves it, and is cut off above the
    /// buttons if it doesn't fit.
    pub fn with_size(mut self, width: i32, height: i32) -> Self {
        self.size = Some(Size::new(width.max(120), height.max(60)));
        self
    }

    pub fn on_dismiss(mut self, handler: impl FnMut(&mut EventCtx) + 'static) -> Self {
        self.modal.set_on_dismiss(handler);
        self
    }

    pub fn show(&mut self, title: impl Into<String>, message: impl Into<String>, icon: DialogIcon) {
        self.pending = Some(Pending {
            title: title.into(),
            icon,
            message: message.into(),
            confirm: None,
        });
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
        self.pending = Some(Pending {
            title: title.into(),
            icon: DialogIcon::Warning,
            message: message.into(),
            confirm: Some((affirm.into(), Box::new(on_confirm))),
        });
    }

    pub fn dismiss(&mut self) {
        self.pending = None;
        self.modal.dismiss();
    }

    pub fn is_open(&self) -> bool {
        self.pending.is_some() || self.modal.is_open()
    }

    /// Open the box last asked for, if any, now that there is a painter to
    /// measure its message with.
    fn open_pending(&mut self, painter: &Painter, theme: &Theme) {
        let Some(Pending {
            title,
            icon,
            message,
            confirm,
        }) = self.pending.take()
        else {
            return;
        };
        let message = Label::new(Rect::new(0, 0, 0, 0), message);
        let (size, body): (Size, Box<dyn Widget>) = match confirm {
            None => {
                let size = self
                    .size
                    .unwrap_or_else(|| fit(painter, theme, &message, icon, BUTTON_W));
                (size, Box::new(MessageBody::new(icon, message)))
            }
            Some((affirm, on_confirm)) => {
                let widths = (
                    confirm_button_w(painter, theme, &affirm),
                    confirm_button_w(painter, theme, CANCEL_LABEL),
                );
                let size = self.size.unwrap_or_else(|| {
                    let buttons_w = widths.0 + CONFIRM_BTN_GAP + widths.1;
                    fit(painter, theme, &message, icon, buttons_w)
                });
                let body = ConfirmBody::new(icon, message, affirm, widths, on_confirm, size);
                (size, Box::new(body))
            }
        };
        self.modal.show(title, size, body);
    }
}

impl Default for Dialog {
    fn default() -> Self {
        Self::new()
    }
}

// `Dialog` is a `Modal` with a fixed body, so every `Widget` method delegates
// straight through — once the box is open. Until then it measures itself in
// whichever paint comes first, and holds on to the input meanwhile.
impl Widget for Dialog {
    fn bounds(&self) -> Rect {
        self.modal.bounds()
    }
    fn layout(&mut self, bounds: Rect) {
        self.modal.layout(bounds);
    }
    fn paint(&mut self, painter: &mut Painter, theme: &Theme) {
        self.open_pending(painter, theme);
        self.modal.paint(painter, theme);
    }
    fn paint_overlay(&mut self, painter: &mut Painter, theme: &Theme) {
        // A dialog shown from inside another one is only painted in that
        // one's pass, which comes through here.
        self.open_pending(painter, theme);
        self.modal.paint_overlay(painter, theme);
    }
    fn event(&mut self, event: &Event, ctx: &mut EventCtx) {
        if self.pending.is_some() {
            // Shown, but not measured yet: make sure a paint comes to measure
            // it in, even if whatever showed it didn't ask for one, and keep
            // the input from the widgets behind it as the open box will.
            ctx.request_paint();
            ctx.consume_event();
            return;
        }
        self.modal.event(event, ctx);
    }
    fn captures_pointer(&self) -> bool {
        self.pending.is_some() || self.modal.captures_pointer()
    }
    fn accepts_accelerators(&self) -> bool {
        self.pending.is_some() || self.modal.accepts_accelerators()
    }
    fn popup_request(&self) -> Option<PopupRequest> {
        self.modal.popup_request()
    }
    fn collect_popups(&self, out: &mut Vec<PopupRequest>) {
        self.modal.collect_popups(out);
    }
    fn wants_ticks(&self) -> bool {
        // A tick is an event, and an event asks for the paint that opens the
        // box: see `event`.
        self.pending.is_some() || self.modal.wants_ticks()
    }
}

/// The message box's body: the icon, the message, and the OK button. Hosted
/// as a [`Modal`]'s content and laid out into the dialog's client rect, so all
/// positions are taken relative to that rect. The OK button is a real
/// [`Button`] — it owns the press-then-release behaviour for pointer and
/// keyboard, and its `on_click` simply asks the modal to dismiss.
struct MessageBody {
    icon: DialogIcon,
    message: Label,
    ok: Button,
    rect: Rect,
}

impl MessageBody {
    fn new(icon: DialogIcon, message: Label) -> Self {
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
        self.message.layout(message_rect(bounds, self.icon));
        self.ok.rect = Self::button_rect(bounds);
    }

    fn paint(&mut self, painter: &mut Painter, theme: &Theme) {
        draw_icon(painter, self.rect, self.icon);
        self.message.paint(painter, theme);
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
    message: Label,
    rect: Rect,
    body: Container,
}

impl ConfirmBody {
    /// `widths` are the affirmative and Cancel buttons' — measured to their
    /// labels, as the box was sized to seat them.
    fn new(
        icon: DialogIcon,
        message: Label,
        affirm: String,
        (aw, cw): (i32, i32),
        mut on_confirm: ConfirmHandler,
        size: Size,
    ) -> Self {
        // A centered button row along the bottom. Affirm on the left, Cancel
        // right.
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
        self.message.layout(message_rect(bounds, self.icon));
        self.body.layout(bounds);
    }

    fn paint(&mut self, painter: &mut Painter, theme: &Theme) {
        draw_icon(painter, self.rect, self.icon);
        self.message.paint(painter, theme);
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

/// Draw a Win 3.1-style alert icon in the top-left corner of a body laid out
/// at `body`.
fn draw_icon(painter: &mut Painter, body: Rect, icon: DialogIcon) {
    let svg = match icon {
        DialogIcon::None => return,
        DialogIcon::Info => &INFO_ICON,
        DialogIcon::Warning => &WARNING_ICON,
        DialogIcon::Error => &ERROR_ICON,
    };
    let rect = Rect::new(body.x + PADDING, body.y + PADDING, ICON_SIZE, ICON_SIZE);
    svg.draw(painter, rect);
}
