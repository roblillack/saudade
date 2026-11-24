//! The input-method bookkeeping the runtimes share: what the platform is
//! composing, and whether the focused widget takes text at all — see
//! [`Event::Ime`] and [`EventCtx::set_ime_area`](crate::EventCtx::set_ime_area).
//!
//! The winit runtime, the Wayland one and `MockBackend` each keep one of these
//! and turn what it says into their own calls: `Window::set_ime_allowed` and
//! `set_ime_cursor_area`, `zwp_text_input_v3`, or a field a test reads back.

use crate::event::{Event, Ime};
use crate::geometry::Rect;

/// What the runtime knows about input-method (IME) text input.
///
/// Whether the input method is on is the focused widget's to say, and it says
/// so anew with [`EventCtx::set_ime_area`](crate::EventCtx::set_ime_area)
/// whenever it handles an event. Nothing tells the runtime when the focus
/// moves, though — the containers keep it among themselves — so after each
/// event that may have moved it, the runtime asks: it dispatches the
/// composition it knows of again (see [`Self::probe`]), which the containers
/// route to whichever widget now has the focus, and keeps the input method on
/// only if that widget answers. The composition being what the platform
/// reported, a widget that hears it again has nothing to change.
#[derive(Debug, Default)]
pub(crate) struct ImeTracker {
    /// The composition in progress as the platform last reported it, and as
    /// the widgets were told of it: empty when there is none.
    preedit: String,
    cursor: Option<(usize, usize)>,
    /// Where the focused widget's caret is, in logical window coordinates,
    /// while it takes text input. `None` turns the input method off.
    wanted: Option<Rect>,
}

impl ImeTracker {
    /// Where the focused widget wants the input method, or `None` for no input
    /// method at all.
    pub(crate) fn wanted(&self) -> Option<Rect> {
        self.wanted
    }

    /// Whether the platform is in the middle of composing something.
    #[cfg_attr(not(target_os = "macos"), allow(dead_code))]
    pub(crate) fn is_composing(&self) -> bool {
        !self.preedit.is_empty()
    }

    /// The event that asks the focused widget whether it takes text: the
    /// composition in progress, again — empty when there is none. A text widget
    /// answers it with `set_ime_area`; anything else leaves it be.
    pub(crate) fn probe(&self) -> Event {
        Event::Ime(Ime::Preedit {
            text: self.preedit.clone(),
            cursor: self.cursor,
        })
    }

    /// Take in a dispatch of `event` during which a widget asked for the input
    /// method at `asserted`, if one did. Returns the probe to dispatch next
    /// when `event` may have moved the keyboard focus; whatever answers that
    /// is handed to [`Self::probed`] and decides.
    ///
    /// - An [`Event::Ime`] went to the focused widget just as the probe would
    ///   have, so it decides on its own.
    /// - A key, a click or a drop may have moved the focus — Tab, a click on
    ///   another field, a dialog opening — or have been taken by a menu before
    ///   it reached the focused widget at all, so its answer counts for
    ///   nothing and the probe asks again.
    /// - Anything else — a pointer move, a tick — leaves the focus where it
    ///   was. It can move the area, or turn the input method on for a focused
    ///   widget that asks, but a widget that didn't hear it can't be taken to
    ///   have said no.
    pub(crate) fn settle(&mut self, event: &Event, asserted: Option<Rect>) -> Option<Event> {
        match event {
            Event::Ime(_) => {
                self.wanted = asserted;
                None
            }
            Event::KeyDown { .. }
            | Event::KeyUp { .. }
            | Event::Char { .. }
            | Event::PointerDown { .. }
            | Event::PointerUp { .. }
            | Event::Drop { .. } => Some(self.probe()),
            _ => {
                if asserted.is_some() {
                    self.wanted = asserted;
                }
                None
            }
        }
    }

    /// The answer to a [`Self::probe`]: the area the focused widget asked for,
    /// or `None` if it takes no text.
    pub(crate) fn probed(&mut self, asserted: Option<Rect>) {
        self.wanted = asserted;
    }

    /// The platform reports `text` as the composition now in progress, with
    /// `cursor` a byte range within it. Returns the [`Event::Ime`] to tell the
    /// widgets, or `None` if that is what they were last told.
    pub(crate) fn preedit(
        &mut self,
        text: String,
        cursor: Option<(usize, usize)>,
    ) -> Option<Event> {
        let cursor = if text.is_empty() {
            None
        } else {
            cursor.map(|range| clamp_range(&text, range))
        };
        if text == self.preedit && cursor == self.cursor {
            return None;
        }
        self.preedit.clone_from(&text);
        self.cursor = cursor;
        Some(Event::Ime(Ime::Preedit { text, cursor }))
    }

    /// The platform committed `text`, which ends the composition. Returns the
    /// [`Event::Ime`] to dispatch, or `None` for an empty commit.
    pub(crate) fn commit(&mut self, text: String) -> Option<Event> {
        self.preedit.clear();
        self.cursor = None;
        (!text.is_empty()).then_some(Event::Ime(Ime::Commit(text)))
    }

    /// The composition ended without a commit: the input method was turned
    /// off, or the window lost the keyboard. Returns the empty preedit to tell
    /// the widgets, if they were shown one.
    pub(crate) fn cancel(&mut self) -> Option<Event> {
        self.preedit(String::new(), None)
    }

    /// Keep up with an [`Event::Ime`] dispatched by someone other than the
    /// platform — a test, through `MockBackend` — as if the platform had sent
    /// it, so the next probe repeats it.
    pub(crate) fn observe(&mut self, event: &Event) {
        match event {
            Event::Ime(Ime::Preedit { text, cursor }) => {
                let _ = self.preedit(text.clone(), *cursor);
            }
            Event::Ime(Ime::Commit(_)) => {
                self.preedit.clear();
                self.cursor = None;
            }
            _ => {}
        }
    }
}

/// `range` made safe to slice `text` with: inside it, in order, and on
/// character boundaries. Input methods have been seen to send ranges past the
/// end of their text.
fn clamp_range(text: &str, (start, end): (usize, usize)) -> (usize, usize) {
    let boundary = |mut i: usize| {
        i = i.min(text.len());
        while !text.is_char_boundary(i) {
            i -= 1;
        }
        i
    };
    let (start, end) = (boundary(start), boundary(end));
    (start.min(end), end.max(start))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::event::Modifiers;
    use crate::geometry::Point;

    const CARET: Rect = Rect::new(10, 20, 1, 14);

    fn key() -> Event {
        Event::Char {
            ch: 'a',
            modifiers: Modifiers::default(),
        }
    }

    #[test]
    fn a_key_is_followed_by_a_probe_that_decides() {
        let mut ime = ImeTracker::default();
        let probe = ime.settle(&key(), Some(CARET));
        assert_eq!(
            probe.map(|e| format!("{e:?}")),
            Some(format!("{:?}", ime.probe()))
        );
        ime.probed(None);
        assert_eq!(ime.wanted(), None);
        ime.probed(Some(CARET));
        assert_eq!(ime.wanted(), Some(CARET));
    }

    #[test]
    fn a_pointer_move_can_turn_it_on_but_not_off() {
        let mut ime = ImeTracker::default();
        let moved = Event::PointerMove {
            pos: Point::new(1, 1),
        };
        assert!(ime.settle(&moved, Some(CARET)).is_none());
        assert_eq!(ime.wanted(), Some(CARET));
        assert!(ime.settle(&moved, None).is_none());
        assert_eq!(ime.wanted(), Some(CARET));
        assert!(ime.settle(&Event::Tick, None).is_none());
        assert_eq!(ime.wanted(), Some(CARET));
    }

    #[test]
    fn an_ime_event_decides_on_its_own() {
        let mut ime = ImeTracker::default();
        ime.probed(Some(CARET));
        let event = ime.commit("é".into()).unwrap();
        assert!(ime.settle(&event, None).is_none());
        assert_eq!(ime.wanted(), None);
    }

    #[test]
    fn the_probe_repeats_the_composition() {
        let mut ime = ImeTracker::default();
        assert!(!ime.is_composing());
        let shown = ime.preedit("´".into(), Some((2, 2))).unwrap();
        assert!(ime.is_composing());
        assert_eq!(format!("{shown:?}"), format!("{:?}", ime.probe()));
        // The same again is nothing new for the widgets.
        assert!(ime.preedit("´".into(), Some((2, 2))).is_none());
        let committed = ime.commit("é".into());
        assert!(matches!(committed, Some(Event::Ime(Ime::Commit(t))) if t == "é"));
        assert!(!ime.is_composing());
        assert!(matches!(
            ime.probe(),
            Event::Ime(Ime::Preedit { text, cursor: None }) if text.is_empty()
        ));
    }

    #[test]
    fn a_cancel_clears_only_a_composition_that_was_shown() {
        let mut ime = ImeTracker::default();
        assert!(ime.cancel().is_none());
        ime.preedit("か".into(), None);
        assert!(matches!(
            ime.cancel(),
            Some(Event::Ime(Ime::Preedit { text, .. })) if text.is_empty()
        ));
        assert!(ime.cancel().is_none());
    }

    #[test]
    fn a_cursor_past_the_text_is_pulled_back_onto_it() {
        let mut ime = ImeTracker::default();
        // "´" is two bytes; a range past its end, or into its middle, is not.
        let shown = ime.preedit("´".into(), Some((1, 9)));
        assert!(matches!(
            shown,
            Some(Event::Ime(Ime::Preedit {
                cursor: Some((0, 2)),
                ..
            }))
        ));
        assert_eq!(clamp_range("abc", (3, 1)), (1, 3));
    }

    #[test]
    fn an_empty_commit_is_no_event() {
        let mut ime = ImeTracker::default();
        assert!(ime.commit(String::new()).is_none());
    }
}
