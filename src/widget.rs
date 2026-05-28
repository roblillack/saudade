use crate::event::{Event, EventCtx};
use crate::geometry::Rect;
use crate::painter::Painter;
use crate::theme::Theme;

/// What kind of subordinate top-level a widget is asking the runtime to
/// host. The runtime maps this onto different windowing-system objects:
/// menus get override-redirect / xdg_popup chrome that's anchored to the
/// parent surface, dialogs get a real top-level window with transient /
/// modal hints and no fixed position.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PopupKind {
    /// Borderless dropdown-style popup, anchored to a point inside the
    /// parent surface. Used by [`MenuBar`](crate::widgets::MenuBar) and
    /// other "floating chrome" widgets.
    Popup,
    /// Decorationless modal dialog window. The widget paints its own
    /// chrome (title bar, border) and the runtime opens a real top-level
    /// window transient to the parent — without override-redirect on X11
    /// and as a regular `xdg_toplevel` on Wayland.
    Dialog,
}

/// A widget asks the runtime to host its popup in a separate top-level
/// window. The runtime polls `Widget::popup_request` after each event and
/// matches the request against any existing popup window — opening,
/// repositioning, or closing as needed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PopupRequest {
    /// Popup's logical bounds in the *root widget's* coordinate space.
    /// The runtime translates this into screen coordinates by adding the
    /// main window's inner position and scaling by the current DPI for
    /// [`PopupKind::Popup`]. For [`PopupKind::Dialog`] only the size is
    /// significant — the WM / compositor decides the final placement.
    pub rect: Rect,
    /// What kind of host window the widget needs.
    pub kind: PopupKind,
}

/// The fundamental UI abstraction.
///
/// A widget owns its state, draws itself, and reacts to typed input events.
/// It does not own peer widgets and never reaches into the runtime directly —
/// repaint / close / focus requests are issued via [`EventCtx`].
pub trait Widget {
    /// Logical bounds relative to the window root, in retrogui pixels.
    fn bounds(&self) -> Rect;

    /// Render the widget in the normal pass.
    fn paint(&mut self, painter: &mut Painter, theme: &Theme);

    /// Render anything that needs to float on top of *every* sibling — open
    /// menu popups, tooltips, drag previews. Runs after every widget's
    /// regular `paint` is finished. Default: no-op.
    fn paint_overlay(&mut self, _painter: &mut Painter, _theme: &Theme) {}

    /// Handle a typed input event. Default: ignore.
    fn event(&mut self, _event: &Event, _ctx: &mut EventCtx) {}

    /// Internal hook for capture-on-press dispatch. Default: never captured.
    /// Implementations like [`Button`](crate::widgets::Button) override this so
    /// pointer events keep flowing to them while a press is in progress, even
    /// if the cursor leaves the widget's bounds.
    fn captures_pointer(&self) -> bool {
        false
    }

    /// `true` if this widget accepts keyboard focus. The parent container
    /// remembers the last focusable widget the user clicked, and routes
    /// keyboard events only there.
    fn focusable(&self) -> bool {
        false
    }

    /// Inform the widget that it has gained or lost keyboard focus.
    /// Default: ignore. Editing widgets override this to show/hide their
    /// cursor or to commit pending input.
    fn set_focused(&mut self, _focused: bool) {}

    /// `true` if this widget should receive every keyboard event regardless
    /// of focus. Used by menu bars so that Alt+letter accelerators reach
    /// them even while a sibling (e.g., a text editor) holds focus.
    fn accepts_accelerators(&self) -> bool {
        false
    }

    /// Position the widget inside the rectangle the parent has allocated.
    ///
    /// Layout containers (`Column`, etc.) call this to tell each child where
    /// it lives now; the widget should store the rect and propagate to its
    /// own children. The default is a no-op: widgets with absolute, fixed
    /// positions (the ones in retrofetch's about box) ignore the call and
    /// stay where they were placed at construction.
    fn layout(&mut self, _bounds: Rect) {}

    /// Ask the runtime to host a popup window for this widget. Returning
    /// `Some` makes the runtime open (or move) a borderless top-level
    /// window at the indicated logical-coord rect so the popup can extend
    /// past the main window's edges. Container widgets propagate this from
    /// their children. Default: no popup.
    fn popup_request(&self) -> Option<PopupRequest> {
        None
    }
}
