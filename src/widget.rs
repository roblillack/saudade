use crate::event::{Event, EventCtx};
use crate::geometry::Rect;
use crate::painter::Painter;
use crate::theme::Theme;

/// The fundamental UI abstraction.
///
/// A widget owns its state, draws itself, and reacts to typed input events.
/// It does not own peer widgets and never reaches into the runtime directly —
/// repaint / close requests are issued via [`EventCtx`].
pub trait Widget {
    /// Logical bounds relative to the window root, in retrogui pixels.
    fn bounds(&self) -> Rect;

    /// Render into the painter using the current theme.
    fn paint(&mut self, painter: &mut Painter, theme: &Theme);

    /// Handle a typed input event. Default: ignore.
    fn event(&mut self, _event: &Event, _ctx: &mut EventCtx) {}

    /// Internal hook for capture-on-press dispatch. Default: never captured.
    /// Implementations like [`Button`](crate::widgets::Button) override this so
    /// pointer events keep flowing to them while a press is in progress, even
    /// if the cursor leaves the widget's bounds.
    fn captures_pointer(&self) -> bool {
        false
    }
}
