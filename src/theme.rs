use crate::font::FontStyle;
use crate::geometry::Color;

/// Visual style palette. Widgets read from this rather than hard-coding colors,
/// so the same widget code can render in different retro themes later.
#[derive(Clone)]
pub struct Theme {
    pub background: Color,
    pub face: Color,
    pub highlight: Color,
    pub shadow: Color,
    pub border: Color,
    pub text: Color,
    pub disabled_text: Color,
    /// Selected-item background — Win 3.1 dark navy blue.
    pub highlight_bg: Color,
    /// Selected-item foreground text color — white on Win 3.1.
    pub highlight_text: Color,
    /// Point size for all chrome text — labels, buttons, fields, list rows,
    /// dropdowns, dialogs, and the menu bar. One value so the whole UI stays
    /// visually consistent; content widgets that want a different size (e.g.
    /// `TextEditor`) carry their own override.
    pub font_size: f32,
    /// Face a push button sets its label in. Bold by default: a button is the
    /// one thing on a panel you press, and the weight is what says so at a
    /// glance. Menus, captions, fields and list rows stay regular — this is a
    /// button knob, not a chrome-wide weight. A theme after a lighter look sets
    /// it to `FontStyle::Regular`; a family the host ships no real bold for
    /// falls back to its regular face on its own (see [`Font`](crate::Font)),
    /// never to a synthesized smear.
    pub button_style: FontStyle,
}

impl Theme {
    /// Default Windows 3.1 palette: white workspace, light-gray button face,
    /// white highlight, mid-gray shadow, black outer border.
    pub const fn windows_31() -> Self {
        Self {
            background: Color::WHITE,
            face: Color::LIGHT_GRAY,
            highlight: Color::WHITE,
            shadow: Color::MID_GRAY,
            border: Color::BLACK,
            text: Color::BLACK,
            disabled_text: Color::MID_GRAY,
            highlight_bg: Color::NAVY,
            highlight_text: Color::WHITE,
            font_size: 11.0,
            button_style: FontStyle::Bold,
        }
    }
}

impl Default for Theme {
    fn default() -> Self {
        Self::windows_31()
    }
}
