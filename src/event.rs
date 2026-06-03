use crate::geometry::Point;

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

#[derive(Clone, Copy, Debug)]
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
            | Event::Scroll { pos, .. } => Some(*pos),
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
    /// Set when a widget calls [`Self::set_scale_factor`] to override the
    /// runtime's logical→physical scale factor. Applied by the runtime
    /// after dispatch, alongside [`Self::request_paint`] and [`Self::close`].
    pub(crate) scale_request: Option<f32>,
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
            scale_request: None,
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

    /// Override the runtime's logical→physical scale factor. The new value
    /// replaces what the OS reported (and what subsequent `paint` /
    /// `layout` passes see) until the OS itself reports a different scale.
    /// To *read* the current scale, use [`Painter::scale`](crate::Painter::scale)
    /// inside [`Widget::paint`](crate::Widget::paint) — that's the canonical
    /// source and works equally well in both event and paint paths.
    ///
    /// Values are clamped to a small positive minimum to keep buffers
    /// non-degenerate. The change is applied after the current event
    /// finishes dispatching and the runtime triggers a repaint, so
    /// callers do not need to call [`Self::request_paint`] separately.
    pub fn set_scale_factor(&mut self, factor: f32) {
        self.scale_request = Some(factor.max(0.1));
    }
}
