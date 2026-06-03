use crate::event::{Event, EventCtx, Key, NamedKey};
use crate::geometry::{Point, Rect, Size};
use crate::painter::Painter;
use crate::theme::Theme;
use crate::widget::{PopupKind, PopupRequest, Widget};

type DismissHandler = Box<dyn FnMut(&mut EventCtx)>;

/// A modal dialog hosted in its own top-level window.
///
/// `Modal` is the general-purpose modal facility: it owns an arbitrary content
/// [`Widget`] and presents it centered over the parent in a real OS window
/// (the runtime opens a [`PopupKind::Dialog`] toplevel for it). While it is up
/// it is *modal* — it captures every pointer and keyboard event in the widget
/// tree, so the widgets behind it can't be interacted with. Escape (and the
/// window's close button, which the runtime maps to Escape) dismisses it.
///
/// Content closes the dialog by calling
/// [`EventCtx::request_dismiss`](crate::EventCtx::request_dismiss) — typically
/// from an OK / Close button's `on_click`. When the modal dismisses (via
/// Escape, the close button, content's request, or [`Modal::dismiss`]) it runs
/// its optional [`on_dismiss`](Modal::on_dismiss) handler, which is where the
/// caller commits whatever the dialog was editing.
///
/// The message-box [`Dialog`](crate::Dialog) is built on top of `Modal`: it
/// supplies a small body that draws the icon, message, and OK button. Custom
/// dialogs supply their own content — for example the 7GUIs circle-drawer hosts
/// a diameter [`Slider`](crate::Slider) here.
///
/// Like the message box, `Modal` lives in the widget tree as a normally-empty
/// overlay (`Column::add_overlay`). It paints nothing in the main pass; its
/// body is drawn during the popup pass into the toplevel the runtime opens.
///
/// # Hosting content
///
/// The content is laid out into the dialog's client rectangle (in the root
/// widget's coordinate space), so it must be a layout-aware widget that honors
/// the bounds passed to [`Widget::layout`] — a `Column`/`Row`, or a custom
/// widget that positions itself relative to that rect. A plain
/// [`Container`](crate::Container) (which ignores `layout` and keeps absolute
/// positions) is **not** suitable as modal content.
pub struct Modal {
    open: bool,
    title: String,
    size: Size,
    /// Parent bounds last seen in `layout`; used to center at show time.
    parent_bounds: Rect,
    /// Top-left of the dialog in root coordinates, frozen at show time so a
    /// parent resize doesn't yank the dialog around.
    frozen_origin: Option<Point>,
    content: Option<Box<dyn Widget>>,
    on_dismiss: Option<DismissHandler>,
}

impl Modal {
    pub fn new() -> Self {
        Self {
            open: false,
            title: String::new(),
            size: Size::new(320, 160),
            parent_bounds: Rect::new(0, 0, 0, 0),
            frozen_origin: None,
            content: None,
            on_dismiss: None,
        }
    }

    /// Install a handler run whenever the dialog is dismissed (by any route).
    /// This is where the caller commits the result of the dialog.
    pub fn on_dismiss(mut self, handler: impl FnMut(&mut EventCtx) + 'static) -> Self {
        self.on_dismiss = Some(Box::new(handler));
        self
    }

    /// Install (or replace) the dismiss handler after construction.
    pub fn set_on_dismiss(&mut self, handler: impl FnMut(&mut EventCtx) + 'static) {
        self.on_dismiss = Some(Box::new(handler));
    }

    /// Open the dialog, centered over the parent, hosting `content` at the
    /// given logical size. `content` is laid out into the dialog's client area
    /// and given initial focus.
    pub fn show(&mut self, title: impl Into<String>, size: Size, content: Box<dyn Widget>) {
        self.title = title.into();
        self.size = Size::new(size.w.max(80), size.h.max(48));
        self.open = true;
        self.frozen_origin = self.centered_origin();
        let mut content = content;
        if let Some(rect) = self.frozen_origin.map(|o| self.rect_at(o)) {
            content.layout(rect);
        }
        content.focus_first();
        self.content = Some(content);
    }

    /// Close the dialog without running `on_dismiss`. Use this for a
    /// programmatic close; the usual route is content calling
    /// [`EventCtx::request_dismiss`] (which *does* fire `on_dismiss`).
    pub fn dismiss(&mut self) {
        self.open = false;
        self.frozen_origin = None;
        self.content = None;
    }

    pub fn is_open(&self) -> bool {
        self.open
    }

    fn centered_origin(&self) -> Option<Point> {
        if self.parent_bounds.w <= 0 || self.parent_bounds.h <= 0 {
            return None;
        }
        let px = self.parent_bounds.x + (self.parent_bounds.w - self.size.w) / 2;
        let py = self.parent_bounds.y + (self.parent_bounds.h - self.size.h) / 2;
        Some(Point::new(px.max(0), py.max(0)))
    }

    fn rect_at(&self, origin: Point) -> Rect {
        Rect::new(origin.x, origin.y, self.size.w, self.size.h)
    }

    /// The dialog's rectangle in root coordinates, falling back to a freshly
    /// computed centered position when no origin has been frozen yet.
    fn rect(&self) -> Rect {
        let origin = self.frozen_origin.unwrap_or_else(|| {
            let px = self.parent_bounds.x + (self.parent_bounds.w - self.size.w) / 2;
            let py = self.parent_bounds.y + (self.parent_bounds.h - self.size.h) / 2;
            Point::new(px.max(0), py.max(0))
        });
        self.rect_at(origin)
    }

    /// Run `on_dismiss` then tear the dialog down. Clears the dismiss request
    /// flag so it doesn't leak past this dispatch.
    fn fire_dismiss(&mut self, ctx: &mut EventCtx) {
        ctx.dismiss_requested = false;
        if let Some(handler) = self.on_dismiss.as_mut() {
            handler(ctx);
        }
        self.dismiss();
        ctx.request_paint();
    }
}

impl Default for Modal {
    fn default() -> Self {
        Self::new()
    }
}

impl Widget for Modal {
    fn bounds(&self) -> Rect {
        if self.open {
            self.rect()
        } else {
            Rect::new(0, 0, 0, 0)
        }
    }

    fn layout(&mut self, bounds: Rect) {
        self.parent_bounds = bounds;
        if self.open {
            if self.frozen_origin.is_none() {
                self.frozen_origin = self.centered_origin();
            }
            let rect = self.rect();
            if let Some(content) = self.content.as_mut() {
                content.layout(rect);
            }
        }
    }

    fn paint(&mut self, _painter: &mut Painter, _theme: &Theme) {
        // The dialog body lives in its own top-level window; nothing is drawn
        // in the main window's normal pass.
    }

    fn paint_overlay(&mut self, painter: &mut Painter, theme: &Theme) {
        // Skip the main pass entirely — the dialog lives in its own top-level
        // window opened by the runtime.
        if !self.open || !painter.is_popup_pass() {
            return;
        }
        let rect = self.rect();
        // Each popup window in a nested stack (e.g. our dialog and a Dropdown
        // opened inside it) runs the same paint pass against the root, so
        // every paint_overlay sees every popup pass. Only stamp the dialog
        // body when this pass is *ours*; for nested-popup passes we just
        // forward to the content so the nested widget can find its own pass.
        let is_our_pass = painter.popup_anchor() == Some(rect);
        if is_our_pass {
            // Plain background fill across the client area; the WM /
            // compositor draws the surrounding chrome (title bar, close
            // button).
            painter.fill_rect(rect, theme.background);
        }
        if let Some(content) = self.content.as_mut() {
            if is_our_pass {
                content.paint(painter, theme);
            }
            content.paint_overlay(painter, theme);
        }
    }

    fn event(&mut self, event: &Event, ctx: &mut EventCtx) {
        if !self.open {
            return;
        }
        // Escape (and the window-manager close button, routed to Escape by the
        // runtime) dismisses the dialog.
        if let Event::KeyDown {
            key: Key::Named(NamedKey::Escape),
            ..
        } = event
        {
            self.fire_dismiss(ctx);
            return;
        }
        if let Some(content) = self.content.as_mut() {
            content.event(event, ctx);
        }
        // Content asked to close (e.g. an OK button) — honor it.
        if ctx.is_dismiss_requested() {
            self.fire_dismiss(ctx);
        }
    }

    fn captures_pointer(&self) -> bool {
        // Modal: swallow every event in the tree while open.
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
            rect: self.rect(),
            kind: PopupKind::Dialog,
            title: Some(self.title.clone()),
        })
    }

    fn collect_popups(&self, out: &mut Vec<PopupRequest>) {
        // The dialog's own window first, then any popup the hosted content
        // wants (e.g. a `Dropdown` list opened inside the dialog) — that nested
        // request would otherwise be lost, since the dialog is the only popup
        // the tree surfaced before.
        if let Some(req) = self.popup_request() {
            out.push(req);
            if let Some(content) = self.content.as_ref() {
                content.collect_popups(out);
            }
        }
    }

    fn wants_ticks(&self) -> bool {
        self.open && self.content.as_ref().is_some_and(|c| c.wants_ticks())
    }
}
