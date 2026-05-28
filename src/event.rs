use crate::geometry::Point;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MouseButton {
    Left,
    Right,
    Middle,
}

/// Identifies a key press independent of any text it produces. `Char` events
/// carry the *text* the user typed; `KeyDown` / `KeyUp` carry the *key*. Most
/// editing widgets want both — `Char` for insertion, `KeyDown(Named)` for
/// navigation and editing actions like Backspace.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Key {
    Named(NamedKey),
    Char(char),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NamedKey {
    Enter,
    Backspace,
    Delete,
    Tab,
    Escape,
    Space,
    Left,
    Right,
    Up,
    Down,
    Home,
    End,
    PageUp,
    PageDown,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Modifiers {
    pub shift: bool,
    pub control: bool,
    pub alt: bool,
    pub logo: bool,
}

impl Modifiers {
    /// True if any of Ctrl / Alt / Logo is held. Editing widgets use this to
    /// decide whether a `Char` event should be inserted as text or treated as
    /// a hotkey instead.
    pub fn has_command(&self) -> bool {
        self.control || self.alt || self.logo
    }
}

#[derive(Clone, Copy, Debug)]
pub enum Event {
    PointerMove { pos: Point },
    PointerDown { pos: Point, button: MouseButton },
    PointerUp { pos: Point, button: MouseButton },
    PointerLeave,
    KeyDown { key: Key, modifiers: Modifiers },
    KeyUp { key: Key, modifiers: Modifiers },
    /// A character produced by the user's keyboard. Backspace, arrow keys etc.
    /// arrive as `KeyDown` only; this event is for inserting visible text.
    Char { ch: char, modifiers: Modifiers },
}

impl Event {
    pub fn position(&self) -> Option<Point> {
        match self {
            Event::PointerMove { pos }
            | Event::PointerDown { pos, .. }
            | Event::PointerUp { pos, .. } => Some(*pos),
            _ => None,
        }
    }

    pub fn is_keyboard(&self) -> bool {
        matches!(
            self,
            Event::KeyDown { .. } | Event::KeyUp { .. } | Event::Char { .. }
        )
    }
}

/// Capabilities granted to a widget while it handles an event.
///
/// Widgets do not mutate the runtime directly: they set request flags here, and
/// the runtime applies them after dispatch completes.
pub struct EventCtx {
    pub(crate) paint_requested: bool,
    pub(crate) close_requested: bool,
    pub(crate) focus_requested: bool,
    pub(crate) focus_released: bool,
}

impl EventCtx {
    pub(crate) fn new() -> Self {
        Self {
            paint_requested: false,
            close_requested: false,
            focus_requested: false,
            focus_released: false,
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

    /// The widget asks to become the keyboard-focused widget. Parent
    /// containers observe this flag during pointer dispatch and route
    /// subsequent keyboard events here.
    pub fn request_focus(&mut self) {
        self.focus_requested = true;
        self.focus_released = false;
    }

    /// The widget asks to drop keyboard focus. Useful when an editor wants
    /// the window to stop sending it characters.
    pub fn release_focus(&mut self) {
        self.focus_released = true;
        self.focus_requested = false;
    }
}
