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
    pub font_size: f32,
    /// Font size used by `MenuBar` — kept separate from `font_size` so the
    /// menu chrome can carry slightly larger, more legible glyphs without
    /// inflating dialog labels everywhere else.
    pub menu_font_size: f32,
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
            menu_font_size: 13.0,
        }
    }
}

impl Default for Theme {
    fn default() -> Self {
        Self::windows_31()
    }
}
