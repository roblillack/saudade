//! Window-background patterns.
//!
//! A *regular* top-level window paints a 1-bit texture behind the widget tree —
//! the System 6 / Win 3.1 desktop look. Dialogs and modals (which the runtime
//! renders in a separate "popup pass") deliberately stay plain white; only the
//! main window honors the [`BackgroundPattern`].
//!
//! The pattern + color are a property of the *window surface*, fixed for the
//! life of the process. The default is a `superlight` forward-diagonal hatch;
//! set `SAUDADE_WINDOW_PATTERN` and `SAUDADE_WINDOW_PATTERN_COLOR` to override
//! (see [`BackgroundState::from_env`]). The `patterns` example cycles them live
//! for previewing.

use crate::geometry::Color;

/// A fill pattern for the background of a regular top-level window.
///
/// Patterns are 1-bit and snapped to a small physical-pixel grid so they stay
/// crisp at any DPI — retro desktop texture, not anti-aliased wallpaper. The
/// foreground (the dots / lines) is drawn in one of the named [`PATTERN_COLORS`];
/// the gaps stay the window's base background color.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BackgroundPattern {
    /// Plain background — no texture.
    None,
    /// Flood the whole window with the pattern color.
    Solid,
    /// A staggered field of dots (every other row offset half a step), like
    /// the classic Mac desktop.
    Dots,
    /// Horizontal hairlines every 4px.
    Lines,
    /// Forward diagonal hatching (`///`).
    DiagonalForward,
    /// Two crossing diagonals — a cross-stitch / `XXXX` weave.
    CrossStitch,
}

impl BackgroundPattern {
    /// Every pattern, in the order the `patterns` demo rotates through them.
    pub const ALL: [BackgroundPattern; 6] = [
        Self::None,
        Self::Solid,
        Self::Dots,
        Self::Lines,
        Self::DiagonalForward,
        Self::CrossStitch,
    ];

    /// Canonical lower-case name — both the display label and the token
    /// accepted by `SAUDADE_WINDOW_PATTERN`.
    pub fn name(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Solid => "solid",
            Self::Dots => "dots",
            Self::Lines => "lines",
            Self::DiagonalForward => "diagonal",
            Self::CrossStitch => "cross-stitch",
        }
    }

    /// Parse a [`name`](Self::name) token (case-insensitive, `_` treated as
    /// `-`). Returns `None` for anything unrecognized.
    pub fn from_name(s: &str) -> Option<Self> {
        let key = s.trim().to_ascii_lowercase().replace('_', "-");
        Self::ALL.into_iter().find(|p| p.name() == key)
    }

    /// The next pattern in [`BackgroundPattern::ALL`], wrapping at the end.
    pub fn next(self) -> Self {
        let i = Self::ALL.iter().position(|&p| p == self).unwrap_or(0);
        Self::ALL[(i + 1) % Self::ALL.len()]
    }
}

/// The named foreground colors, lightest → darkest. These are the tokens
/// `SAUDADE_WINDOW_PATTERN_COLOR` accepts, and what the `patterns` demo cycles.
pub const PATTERN_COLORS: [(&str, Color); 4] = [
    ("superlight", Color::rgb(0xEE, 0xEE, 0xEE)),
    ("light", Color::rgb(0xC0, 0xC0, 0xC0)),
    ("dark", Color::rgb(0x40, 0x40, 0x40)),
    ("black", Color::rgb(0x00, 0x00, 0x00)),
];

/// Look up a named pattern color (case-insensitive). Returns `None` if the
/// name isn't one of [`PATTERN_COLORS`].
pub fn pattern_color(name: &str) -> Option<Color> {
    let key = name.trim().to_ascii_lowercase();
    PATTERN_COLORS
        .iter()
        .find(|(n, _)| *n == key)
        .map(|(_, c)| *c)
}

/// The window background a backend (winit or Wayland) paints behind the widget
/// tree. Fixed for the process: read once from the environment at startup.
pub(crate) struct BackgroundState {
    pub pattern: BackgroundPattern,
    pub color: Color,
}

impl BackgroundState {
    /// Resolve the pattern from `SAUDADE_WINDOW_PATTERN` and the color from
    /// `SAUDADE_WINDOW_PATTERN_COLOR`, falling back to a `superlight` forward
    /// diagonal. An unrecognized value is reported on stderr and ignored.
    pub fn from_env() -> Self {
        let pattern = match std::env::var("SAUDADE_WINDOW_PATTERN") {
            Ok(v) => BackgroundPattern::from_name(&v).unwrap_or_else(|| {
                eprintln!("[saudade] ignoring unknown SAUDADE_WINDOW_PATTERN={v:?}");
                BackgroundPattern::DiagonalForward
            }),
            Err(_) => BackgroundPattern::DiagonalForward,
        };
        let color = match std::env::var("SAUDADE_WINDOW_PATTERN_COLOR") {
            Ok(v) => pattern_color(&v).unwrap_or_else(|| {
                eprintln!("[saudade] ignoring unknown SAUDADE_WINDOW_PATTERN_COLOR={v:?}");
                PATTERN_COLORS[0].1
            }),
            Err(_) => PATTERN_COLORS[0].1,
        };
        Self { pattern, color }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pattern_names_round_trip() {
        for p in BackgroundPattern::ALL {
            assert_eq!(BackgroundPattern::from_name(p.name()), Some(p));
        }
    }

    #[test]
    fn pattern_from_name_is_lenient_but_strict() {
        assert_eq!(
            BackgroundPattern::from_name("  DIAGONAL "),
            Some(BackgroundPattern::DiagonalForward)
        );
        assert_eq!(
            BackgroundPattern::from_name("cross_stitch"),
            Some(BackgroundPattern::CrossStitch)
        );
        // Removed / unknown tokens don't resolve.
        assert_eq!(BackgroundPattern::from_name("dots2"), None);
        assert_eq!(BackgroundPattern::from_name(""), None);
    }

    #[test]
    fn color_lookup_by_name() {
        assert_eq!(pattern_color("superlight"), Some(PATTERN_COLORS[0].1));
        assert_eq!(pattern_color("BLACK"), Some(Color::rgb(0, 0, 0)));
        assert_eq!(pattern_color("chartreuse"), None);
    }
}
