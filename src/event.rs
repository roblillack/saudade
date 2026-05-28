use crate::geometry::Point;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MouseButton {
    Left,
    Right,
    Middle,
}

#[derive(Clone, Copy, Debug)]
pub enum Event {
    PointerMove { pos: Point },
    PointerDown { pos: Point, button: MouseButton },
    PointerUp { pos: Point, button: MouseButton },
    PointerLeave,
}

impl Event {
    pub fn position(&self) -> Option<Point> {
        match self {
            Event::PointerMove { pos }
            | Event::PointerDown { pos, .. }
            | Event::PointerUp { pos, .. } => Some(*pos),
            Event::PointerLeave => None,
        }
    }
}

/// Capabilities granted to a widget while it handles an event.
///
/// Widgets do not mutate the runtime directly: they set request flags here, and
/// the runtime applies them after dispatch completes.
pub struct EventCtx {
    pub(crate) paint_requested: bool,
    pub(crate) close_requested: bool,
}

impl EventCtx {
    pub(crate) fn new() -> Self {
        Self {
            paint_requested: false,
            close_requested: false,
        }
    }

    /// Mark the window dirty so the runtime repaints on the next idle tick.
    pub fn request_paint(&mut self) {
        self.paint_requested = true;
    }

    /// Ask the runtime to close the window after this dispatch completes.
    pub fn close(&mut self) {
        self.close_requested = true;
    }
}
