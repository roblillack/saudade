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
    /// Periodic animation tick, fired by the runtime at roughly 60 Hz while
    /// any widget in the tree returns `wants_ticks() == true`.
    Tick,
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
    /// Set by a widget that has fully handled an event and wants to stop
    /// further routing. Parent containers check it after each dispatch
    /// step (accelerator pass, focused dispatch, …) and bail out so the
    /// event doesn't trigger a second action elsewhere in the tree.
    pub(crate) consumed: bool,
}

impl EventCtx {
    pub(crate) fn new() -> Self {
        Self {
            paint_requested: false,
            close_requested: false,
            focus_requested: false,
            focus_released: false,
            consumed: false,
        }
    }

    /// Returns `true` if a widget has called [`Self::consume_event`] during
    /// this dispatch. Used by parent containers to decide whether to keep
    /// routing the event.
    pub fn is_consumed(&self) -> bool {
        self.consumed
    }

    /// Mark the current event as handled. Parent containers stop routing
    /// once they see this flag, so the same keystroke doesn't fire two
    /// actions (e.g., a default button's Enter accelerator stopping the
    /// focused list from also reacting to the keypress).
    pub fn consume_event(&mut self) {
        self.consumed = true;
    }

    /// Mark the window dirty so the runtime repaints on the next idle tick.
    pub fn request_paint(&mut self) {
        self.paint_requested = true;
    }

    /// Ask the runtime to close the window after this dispatch completes.
    pub fn close(&mut self) {
        self.close_requested = true;
    }

    /// `true` if a widget called [`Self::request_focus`] during this dispatch.
    /// Custom container widgets (outside retrogui) read this after forwarding
    /// an event to a child to learn the child wants focus, then call
    /// [`Self::clear_focus_flags`] and move focus to it — the same protocol the
    /// built-in `Container` / `Column` use internally.
    pub fn is_focus_requested(&self) -> bool {
        self.focus_requested
    }

    /// `true` if a widget called [`Self::release_focus`] during this dispatch.
    pub fn is_focus_released(&self) -> bool {
        self.focus_released
    }

    /// Reset both focus-change flags after a custom container has acted on
    /// them.
    pub fn clear_focus_flags(&mut self) {
        self.focus_requested = false;
        self.focus_released = false;
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
