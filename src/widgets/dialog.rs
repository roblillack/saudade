use crate::event::{Event, EventCtx, Key, MouseButton, NamedKey};
use crate::geometry::{Color, Rect, Size};
use crate::painter::Painter;
use crate::theme::Theme;
use crate::widget::{PopupKind, PopupRequest, Widget};

const TITLE_BAR_H: i32 = 18;
const BUTTON_W: i32 = 70;
const BUTTON_H: i32 = 22;
const ICON_SIZE: i32 = 32;
const PADDING: i32 = 16;

type DismissHandler = Box<dyn FnMut(&mut EventCtx)>;

/// What icon — if any — to show on the left of the message.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DialogIcon {
    None,
    Info,
    Warning,
    Error,
}

/// A modal warning / info dialog.
///
/// `Dialog` lives in the widget tree as a normally-invisible overlay. The
/// application owns it (e.g., via `Rc<RefCell<Dialog>>`) and calls
/// `show_warning` / `show_info` to display it; a single OK button (or
/// Enter / Escape) dismisses it.
///
/// When the dialog opens it reports a [`PopupRequest`] of kind
/// [`PopupKind::Dialog`], so the runtime opens a real top-level window
/// (transient to the main window) and the dialog paints itself into that
/// window's surface — `paint_overlay` only draws while the popup-pass
/// painter is active. Inside the widget tree the dialog still asserts
/// `captures_pointer` and `accepts_accelerators`, so even events that
/// somehow reach the main window are swallowed instead of leaking through
/// to the widgets below.
pub struct Dialog {
    size: Size,
    /// Parent bounds last passed to `layout`; we center inside these.
    parent_bounds: Rect,
    open: bool,
    title: String,
    message: String,
    icon: DialogIcon,
    button_pressed: bool,
    button_armed: bool,
    on_dismiss: Option<DismissHandler>,
}

impl Dialog {
    pub fn new() -> Self {
        Self {
            size: Size::new(340, 150),
            parent_bounds: Rect::new(0, 0, 0, 0),
            open: false,
            title: String::new(),
            message: String::new(),
            icon: DialogIcon::None,
            button_pressed: false,
            button_armed: false,
            on_dismiss: None,
        }
    }

    pub fn with_size(mut self, width: i32, height: i32) -> Self {
        self.size = Size::new(width.max(120), height.max(80));
        self
    }

    pub fn on_dismiss(mut self, handler: impl FnMut(&mut EventCtx) + 'static) -> Self {
        self.on_dismiss = Some(Box::new(handler));
        self
    }

    pub fn show(
        &mut self,
        title: impl Into<String>,
        message: impl Into<String>,
        icon: DialogIcon,
    ) {
        self.title = title.into();
        self.message = message.into();
        self.icon = icon;
        self.open = true;
        self.button_pressed = false;
        self.button_armed = false;
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
        self.open = false;
        self.button_pressed = false;
        self.button_armed = false;
    }

    pub fn is_open(&self) -> bool {
        self.open
    }

    fn dialog_rect(&self) -> Rect {
        let px = self.parent_bounds.x + (self.parent_bounds.w - self.size.w) / 2;
        let py = self.parent_bounds.y + (self.parent_bounds.h - self.size.h) / 2;
        Rect::new(px.max(0), py.max(0), self.size.w, self.size.h)
    }

    fn button_rect(&self) -> Rect {
        let dialog = self.dialog_rect();
        let bx = dialog.x + (dialog.w - BUTTON_W) / 2;
        let by = dialog.bottom() - BUTTON_H - PADDING;
        Rect::new(bx, by, BUTTON_W, BUTTON_H)
    }
}

impl Default for Dialog {
    fn default() -> Self {
        Self::new()
    }
}

impl Widget for Dialog {
    fn bounds(&self) -> Rect {
        if self.open {
            self.dialog_rect()
        } else {
            Rect::new(0, 0, 0, 0)
        }
    }

    fn layout(&mut self, bounds: Rect) {
        self.parent_bounds = bounds;
    }

    fn paint(&mut self, _painter: &mut Painter, _theme: &Theme) {
        // Drawn in paint_overlay so we sit on top of normal siblings —
        // and only into the popup-pass surface that the runtime opens
        // for our top-level dialog window.
    }

    fn paint_overlay(&mut self, painter: &mut Painter, theme: &Theme) {
        // The dialog lives in its own top-level window. Skip the main
        // window's overlay pass so the area under the dialog stays
        // untouched — the runtime hosts our content in a separate popup
        // surface that runs through this same routine with
        // `is_popup_pass() == true`.
        if !self.open || !painter.is_popup_pass() {
            return;
        }

        let dialog = self.dialog_rect();
        let inner = Rect::new(dialog.x + 1, dialog.y + 1, dialog.w - 2, dialog.h - 2);

        // Outer 1-px black border + raised bevel + face fill.
        painter.stroke_rect(dialog, theme.border);
        painter.fill_rect(inner, theme.face);
        painter.raised_bevel(inner, theme.highlight, theme.shadow);

        // Title bar: navy block with white text.
        let title_inset = 2;
        let title_bar = Rect::new(
            dialog.x + title_inset,
            dialog.y + title_inset,
            dialog.w - title_inset * 2,
            TITLE_BAR_H,
        );
        painter.fill_rect(title_bar, theme.highlight_bg);
        painter.text(
            title_bar.x + 6,
            title_bar.y + 2,
            &self.title,
            theme.menu_font_size,
            theme.highlight_text,
        );

        // Body content: icon on the left, wrapped message lines on the
        // right.
        let body_y = title_bar.bottom() + PADDING;
        let icon_x = dialog.x + PADDING;
        let icon_y = body_y;
        if self.icon != DialogIcon::None {
            draw_icon(painter, icon_x, icon_y, ICON_SIZE, self.icon);
        }
        let msg_x = if self.icon == DialogIcon::None {
            dialog.x + PADDING
        } else {
            icon_x + ICON_SIZE + PADDING
        };
        let mut msg_y = body_y;
        for line in self.message.split('\n') {
            painter.text(msg_x, msg_y, line, theme.font_size, theme.text);
            msg_y += (theme.font_size as i32) + 3;
        }

        // OK button — default-styled (1-px outer black border) so Enter
        // is the obvious confirm key.
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
        if !self.open {
            return;
        }
        let btn = self.button_rect();
        match event {
            Event::PointerDown {
                pos,
                button: MouseButton::Left,
            } => {
                if btn.contains(*pos) {
                    self.button_pressed = true;
                    self.button_armed = true;
                    ctx.request_paint();
                }
                // Clicks anywhere else on the dialog are swallowed — modal.
            }
            Event::PointerMove { pos } => {
                if self.button_pressed {
                    let in_btn = btn.contains(*pos);
                    if in_btn != self.button_armed {
                        self.button_armed = in_btn;
                        ctx.request_paint();
                    }
                }
            }
            Event::PointerUp {
                pos,
                button: MouseButton::Left,
            } => {
                if self.button_pressed {
                    let fire = self.button_armed && btn.contains(*pos);
                    self.button_pressed = false;
                    self.button_armed = false;
                    ctx.request_paint();
                    if fire {
                        self.fire(ctx);
                    }
                }
            }
            Event::KeyDown {
                key: Key::Named(NamedKey::Enter | NamedKey::Escape | NamedKey::Space),
                ..
            } => {
                self.fire(ctx);
            }
            _ => {
                // Modal — silently consume every other event.
            }
        }
    }

    fn captures_pointer(&self) -> bool {
        self.open
    }

    fn accepts_accelerators(&self) -> bool {
        self.open
    }

    fn popup_request(&self) -> Option<PopupRequest> {
        if !self.open {
            return None;
        }
        Some(PopupRequest {
            rect: self.dialog_rect(),
            kind: PopupKind::Dialog,
        })
    }
}

impl Dialog {
    fn fire(&mut self, ctx: &mut EventCtx) {
        if let Some(handler) = self.on_dismiss.as_mut() {
            handler(ctx);
        }
        self.dismiss();
        ctx.request_paint();
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
