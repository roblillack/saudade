mod bevel;
mod button;
mod checkbox;
mod column;
mod container;
mod dialog;
mod image;
mod label;
mod list;
mod menu;
mod scrollbar;
mod text_editor;

use crate::event::{Event, Key, NamedKey};

/// What focus cycling should do for a Tab-shaped event. Shared by
/// `Container` and `Column`, both of which intercept Tab/Shift+Tab the same
/// way before forwarding events to focused children.
pub(crate) enum TabAction {
    /// `KeyDown(Tab)` — move focus by the given direction (1 = forward,
    /// -1 = backward).
    Cycle(i32),
    /// `Char('\t')` paired with the matching `KeyDown(Tab)` — already
    /// handled, swallow so it doesn't leak into the focused widget.
    Swallow,
}

/// Map an event to the Tab handling it should trigger, or `None` if it's
/// not a tab keystroke at all. We treat the modifier-less `KeyDown(Tab)`
/// as the canonical "move focus" trigger and the trailing `Char('\t')`
/// as a no-op to consume. Modified taps (Ctrl/Alt/Logo+Tab) pass through
/// untouched so the OS / WM can still handle them.
pub(crate) fn tab_action(event: &Event) -> Option<TabAction> {
    match event {
        Event::KeyDown {
            key: Key::Named(NamedKey::Tab),
            modifiers,
        } if !modifiers.control && !modifiers.alt && !modifiers.logo => {
            Some(TabAction::Cycle(if modifiers.shift { -1 } else { 1 }))
        }
        Event::Char { ch: '\t', modifiers }
            if !modifiers.control && !modifiers.alt && !modifiers.logo =>
        {
            Some(TabAction::Swallow)
        }
        _ => None,
    }
}

pub use bevel::Bevel;
pub use button::Button;
pub use checkbox::Checkbox;
pub use column::Column;
pub use container::Container;
pub use dialog::{Dialog, DialogIcon};
pub use image::Image;
pub use label::Label;
pub use list::{List, ListIcon, ListItem};
pub use menu::{Menu, MenuBar, MenuItem};
pub use scrollbar::{Orientation, ScrollBar, SCROLLBAR_THICKNESS};
pub use text_editor::TextEditor;
