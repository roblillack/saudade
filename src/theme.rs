use crate::font::FontStyle;
use crate::geometry::Color;

/// Visual style palette. Widgets read from this rather than hard-coding colors,
/// so the same widget code can render in different retro themes — the light
/// [`windows_31`](Self::windows_31) one and its dark counterpart,
/// [`windows_31_dark`](Self::windows_31_dark), to begin with.
///
/// An [`App`](crate::App) carries one of each and paints with whichever
/// matches the system's light or dark appearance — see
/// [`App::with_theme`](crate::App::with_theme).
#[derive(Clone)]
pub struct Theme {
    /// Window, menu and dialog background, and the inside of a field — a text
    /// input, list or check box.
    pub background: Color,
    /// Face of a button, scrollbar thumb or other raised control.
    pub face: Color,
    /// The lit top/left edge of a raised bevel, and the bottom/right of a
    /// sunken one.
    pub highlight: Color,
    /// The opposite edge of a bevel: bottom/right raised, top/left sunken.
    pub shadow: Color,
    /// The 1-px outline around buttons, fields and popups.
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

    /// The Windows 3.1 palette after dark: the same chrome — black outlines,
    /// bevels lit from the top left — on a charcoal workspace with dark-gray
    /// button faces and light text. What an [`App`](crate::App) paints with
    /// while the system is in dark mode, unless it brings a dark theme of its
    /// own.
    ///
    /// The outline stays the darkest line of a raised control, below its
    /// shadow, as on the light theme — so the shadow sits a step above black,
    /// and the workspace a step above that, where a black outline still marks
    /// the edge of a field, a popup or a check box.
    pub const fn windows_31_dark() -> Self {
        Self {
            background: Color::rgb(0x2C, 0x2C, 0x2C),
            face: Color::rgb(0x50, 0x50, 0x50),
            highlight: Color::rgb(0x80, 0x80, 0x80),
            shadow: Color::rgb(0x20, 0x20, 0x20),
            border: Color::BLACK,
            text: Color::rgb(0xF0, 0xF0, 0xF0),
            disabled_text: Color::rgb(0x88, 0x88, 0x88),
            highlight_bg: Color::rgb(0x00, 0x00, 0xA8),
            highlight_text: Color::WHITE,
            font_size: 11.0,
            button_style: FontStyle::Bold,
        }
    }

    /// The L-shaped drop shadow a menu or dropdown list casts: what is behind
    /// it, darkened to a quarter — dark gray on the light theme, all but black
    /// on the dark one.
    pub(crate) fn drop_shadow(&self) -> Color {
        self.background.mix(Color::BLACK, 0.75)
    }

    /// Ink for the window background pattern unless the environment names
    /// one: a faint tint of the text color over the background, so the hatch
    /// stays a whisper on either theme — the `superlight` gray on white.
    pub(crate) fn pattern_ink(&self) -> Color {
        self.background.mix(self.text, 1.0 / 15.0)
    }
}

impl Default for Theme {
    fn default() -> Self {
        Self::windows_31()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::background::PATTERN_COLORS;

    #[test]
    fn the_light_theme_derives_the_colors_it_always_had() {
        let theme = Theme::windows_31();
        assert_eq!(theme.drop_shadow(), Color::DARK_GRAY);
        assert_eq!(theme.pattern_ink(), PATTERN_COLORS[0].1, "superlight");
    }

    #[test]
    fn the_dark_theme_is_dark_and_keeps_the_bevel_lit_from_the_top_left() {
        let theme = Theme::windows_31_dark();
        let luma = |c: Color| c.red() as u32 + c.green() as u32 + c.blue() as u32;
        assert!(luma(theme.background) < luma(theme.text));
        // A raised control darkens outward: the outline below the shadow,
        // below the face, below the highlight.
        assert!(luma(theme.border) < luma(theme.shadow));
        assert!(luma(theme.shadow) < luma(theme.face));
        assert!(luma(theme.face) < luma(theme.highlight));
        assert!(luma(theme.drop_shadow()) < luma(theme.background));
        assert!(luma(theme.pattern_ink()) > luma(theme.background));
    }
}
