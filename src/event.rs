use std::path::PathBuf;

use crate::geometry::{Point, Size};

/// How many document lines one mouse-wheel detent (notch) scrolls. Matches the
/// modern desktop default of three lines per notch. The platform backends
/// multiply a raw wheel step by this when building an [`Event::Scroll`], so the
/// "3 lines per notch" policy lives in one place rather than in every widget.
pub(crate) const WHEEL_LINES_PER_DETENT: f32 = 3.0;

/// Nominal logical-pixel height of one line. The backends divide a continuous
/// (trackpad / high-resolution) pixel scroll delta by this to express it in the
/// same line units a wheel detent uses, so both kinds of scroll feed widgets a
/// single currency.
pub(crate) const SCROLL_PIXELS_PER_LINE: f32 = 16.0;

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
    /// Any Alt / Option key, either side.
    pub alt: bool,
    /// The right Alt key specifically — `AltGr` on PC keyboards, the right
    /// `Option` on macOS. It is reserved for *composing* characters (umlauts,
    /// currency symbols, …), so it deliberately does not open menu mnemonics
    /// and does not count as a command modifier. Whenever `alt_graph` is set,
    /// `alt` is set too.
    pub alt_graph: bool,
    pub logo: bool,
}

impl Modifiers {
    /// True if a command modifier (Ctrl / left Alt / Logo) is held. Editing
    /// widgets use this to decide whether a `Char` event should be inserted as
    /// text or treated as a hotkey instead.
    ///
    /// `AltGr` (right Alt) is intentionally *not* a command modifier: it
    /// composes characters and must let the resulting text through. Windows
    /// reports `AltGr` as `Ctrl+Alt`, so when it is held we ignore those bits.
    pub fn has_command(&self) -> bool {
        if self.alt_graph {
            return false;
        }
        self.control || self.alt || self.logo
    }

    /// True when an Alt key that should activate menu mnemonics is held — the
    /// left Alt only. The right Alt (`AltGr`) is reserved for composing
    /// characters such as umlauts and must not steal those keystrokes.
    pub fn mnemonic_alt(&self) -> bool {
        self.alt && !self.alt_graph
    }
}

/// The payload of a drag-and-drop operation — what the user is dragging into
/// the window.
///
/// Today this is the list of file-system paths a drag carries (parsed from the
/// platform's `text/uri-list` on Wayland, or winit's `HoveredFile` /
/// `DroppedFile` paths). It's a struct rather than a bare `Vec<PathBuf>` so that
/// future payload kinds (dragged text, an image) can be added without breaking
/// the [`Event`] variants that carry it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DragData {
    /// File-system paths being dragged. Empty when the drag carries no files
    /// the backend could resolve to local paths.
    pub paths: Vec<PathBuf>,
}

impl DragData {
    /// A payload carrying the given file paths.
    pub fn from_paths(paths: impl IntoIterator<Item = PathBuf>) -> Self {
        Self {
            paths: paths.into_iter().collect(),
        }
    }

    /// `true` when the drag resolved to at least one file path.
    pub fn has_paths(&self) -> bool {
        !self.paths.is_empty()
    }
}

/// Note: [`Event`] is `Clone` but **not** `Copy` — the drag variants carry a
/// [`DragData`] (which owns a `Vec`). Dispatch always passes `&Event`, so this
/// costs nothing on the hot path; only a widget that wants to *keep* a dropped
/// payload pays for the clone.
#[derive(Clone, Debug)]
pub enum Event {
    PointerMove {
        pos: Point,
    },
    PointerDown {
        pos: Point,
        button: MouseButton,
    },
    PointerUp {
        pos: Point,
        button: MouseButton,
    },
    PointerLeave,
    /// The mouse wheel turned or a trackpad scroll gesture moved. `delta` is
    /// measured in document *lines*, and positive values scroll toward the end
    /// of the content — down for `delta_y`, right for `delta_x` — the same
    /// direction a [`ScrollBar`](crate::widgets::ScrollBar)'s value grows. One
    /// wheel notch is [`WHEEL_LINES_PER_DETENT`] lines; trackpad pixel deltas
    /// are converted to a fractional line count, which is why the fields are
    /// `f32`. `pos` is the cursor's position when the wheel turned, so a
    /// container can route the gesture to the widget under the pointer.
    Scroll {
        pos: Point,
        delta_x: f32,
        delta_y: f32,
    },
    /// A drag carrying droppable content entered the window over `pos`. A drop
    /// target highlights itself here. The payload itself arrives with
    /// [`Event::Drop`] — not here — because the platforms only let us read it
    /// reliably once the user actually drops (reading mid-hover can block on a
    /// source that withholds the data until then). `pos` is the pointer
    /// location in logical pixels; on Wayland it is exact, on the winit backends
    /// (macOS / Windows / X11) it is best-effort — winit reports no cursor
    /// coordinates during a file drag, so it reflects the last in-window
    /// pointer position.
    DragEnter {
        pos: Point,
    },
    /// The drag moved to `pos` while still inside the window. Routed to the
    /// widget under the cursor, so dragging across widgets hands the highlight
    /// from one drop target to the next. Only the Wayland backend tracks the
    /// pointer during a drag and emits this; the winit backends go straight
    /// from [`Event::DragEnter`] to [`Event::Drop`].
    DragMove {
        pos: Point,
    },
    /// The drag left the window, or was cancelled, without dropping. Like
    /// [`Event::PointerLeave`] it carries no position and is broadcast to every
    /// widget so any drop target can clear its highlight.
    DragLeave,
    /// The content was released over `pos`. This is where the payload (see
    /// [`DragData`]) is consumed — e.g. open the dropped file. Routed to the
    /// widget under the cursor exactly like [`Event::PointerUp`].
    Drop {
        pos: Point,
        data: DragData,
    },
    KeyDown {
        key: Key,
        modifiers: Modifiers,
    },
    KeyUp {
        key: Key,
        modifiers: Modifiers,
    },
    /// A character produced by the user's keyboard. Backspace, arrow keys etc.
    /// arrive as `KeyDown` only; this event is for inserting visible text.
    Char {
        ch: char,
        modifiers: Modifiers,
    },
    /// Periodic animation tick, fired by the runtime at roughly 60 Hz while
    /// any widget in the tree returns `wants_ticks() == true`.
    Tick,
}

impl Event {
    pub fn position(&self) -> Option<Point> {
        match self {
            Event::PointerMove { pos }
            | Event::PointerDown { pos, .. }
            | Event::PointerUp { pos, .. }
            | Event::Scroll { pos, .. }
            | Event::DragEnter { pos, .. }
            | Event::DragMove { pos, .. }
            | Event::Drop { pos, .. } => Some(*pos),
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
    /// Set by content inside a modal dialog to ask its host to close. The
    /// hosting [`Modal`](crate::widgets::Modal) observes this after forwarding
    /// an event to its content, runs its `on_dismiss` handler, and tears the
    /// dialog down — the same protocol a message box's OK button uses.
    pub(crate) dismiss_requested: bool,
    /// Set when a widget calls [`Self::request_window_size`] to ask the runtime
    /// to resize the window. Applied after dispatch, alongside the other
    /// requests. The window's size is the app's to choose (unlike the scale
    /// factor, which only the OS sets).
    pub(crate) resize_request: Option<Size>,
    /// Set when a widget calls [`Self::start_drag`] to begin dragging content
    /// out of the window. Consumed by the backend after dispatch, which turns
    /// it into a real OS drag. Only the Wayland backend acts on it (see the
    /// method docs); the winit backends drop it.
    pub(crate) drag_request: Option<DragData>,
    /// Set when a widget calls [`Self::accept_drop`] while handling an incoming
    /// [`Event::DragEnter`] / [`Event::DragMove`] to say it will take a drop at
    /// this position. The Wayland backend reads it after dispatch to accept or
    /// reject the drag offer, so the source app learns whether this is a valid
    /// drop target.
    pub(crate) accepts_drop: bool,
}

impl EventCtx {
    pub(crate) fn new() -> Self {
        Self {
            paint_requested: false,
            close_requested: false,
            focus_requested: false,
            focus_released: false,
            consumed: false,
            dismiss_requested: false,
            resize_request: None,
            drag_request: None,
            accepts_drop: false,
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
    /// Custom container widgets (outside saudade) read this after forwarding
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

    /// Content inside a modal dialog calls this to ask the hosting
    /// [`Modal`](crate::widgets::Modal) to close — e.g. from an OK / Close
    /// button's `on_click`. The modal runs its `on_dismiss` handler and
    /// dismisses after the current event finishes dispatching.
    pub fn request_dismiss(&mut self) {
        self.dismiss_requested = true;
    }

    /// `true` if a widget called [`Self::request_dismiss`] during this
    /// dispatch. Read by [`Modal`](crate::widgets::Modal) after forwarding an
    /// event to its content.
    pub fn is_dismiss_requested(&self) -> bool {
        self.dismiss_requested
    }

    /// Ask the runtime to resize the window to `width × height` *logical*
    /// pixels, applied after the current event finishes dispatching (and
    /// followed by a repaint, so callers needn't also call
    /// [`Self::request_paint`]). Handy for a window that should grow or shrink
    /// to fit a mode the user just toggled. The window's size is the app's to
    /// set — unlike the logical→physical scale factor, which only the OS
    /// controls.
    pub fn request_window_size(&mut self, width: i32, height: i32) {
        self.resize_request = Some(Size::new(width.max(1), height.max(1)));
    }

    /// Begin dragging `data` out of this window as an OS drag-and-drop
    /// operation — the mirror of receiving a [`Event::Drop`]. Call this from a
    /// widget's `event` handler once a press-then-move gesture is recognized
    /// (typically on [`Event::PointerMove`] after the pointer has travelled a
    /// few pixels from a [`Event::PointerDown`]), so a plain click still reads
    /// as a click rather than starting a drag.
    ///
    /// **Wayland only.** The winit backends (macOS, Windows, X11) expose no API
    /// to *initiate* a drag, so this is a no-op there; only the Wayland backend
    /// turns it into a real drag whose `text/uri-list` payload other
    /// applications can drop. Receiving drops, by contrast, works on every
    /// backend. The drag copies (never moves) the referenced files.
    pub fn start_drag(&mut self, data: DragData) {
        self.drag_request = Some(data);
    }

    /// Signal, while handling an incoming [`Event::DragEnter`] or
    /// [`Event::DragMove`], that this widget will accept a drop at the current
    /// position. A drop target **must** call this to receive drops: the runtime
    /// treats a widget that doesn't as not interested, so the drag falls through
    /// it. Re-evaluated on every move, so a target can accept only over its hot
    /// region and decline elsewhere in the same window.
    ///
    /// On Wayland this is what tells the source app the spot is a valid target
    /// (its drag cursor / feedback reflects it) and is the prerequisite for a
    /// later [`Event::Drop`] landing here. On the winit backends the OS has
    /// already committed to offering the drop, so this is advisory there —
    /// [`Event::Drop`] arrives regardless — but calling it keeps drop targets
    /// portable.
    pub fn accept_drop(&mut self) {
        self.accepts_drop = true;
    }
}
