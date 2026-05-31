//! Window-background patterns.
//!
//! A *regular* top-level window can paint a 1-bit texture behind the widget
//! tree — the System 6 / Win 3.1 desktop look. Dialogs and modals (which the
//! runtime renders in a separate "popup pass") deliberately stay plain white;
//! only the main window honors the selected [`BackgroundPattern`].
//!
//! The pattern + color are a property of the *window surface*, not of any
//! widget, so the live state lives on the backend ([`BackgroundState`]) rather
//! than in the tree. While developing, the `p` key rotates the pattern and the
//! `c` key rotates the color (see [`BackgroundState::handle_key`]); each change
//! is logged to the console.

use crate::geometry::Color;

/// A fill pattern for the background of a regular top-level window.
///
/// Patterns are 1-bit and snapped to a small physical-pixel grid so they stay
/// crisp at any DPI — retro desktop texture, not anti-aliased wallpaper. The
/// foreground (the dots / lines) is drawn in a [`PatternColor`]; the gaps stay
/// the window's base background color.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BackgroundPattern {
    /// Plain background — no texture. The toolkit default.
    None,
    /// Flood the whole window with the pattern color.
    Solid,
    /// Sparse single dots on a 4px grid (the classic Finder desktop).
    Dots,
    /// Like [`BackgroundPattern::Dots`] but on a wider 8px grid.
    Dots2,
    /// Horizontal hairlines every 4px.
    Lines,
    /// Forward diagonal hatching (`///`).
    DiagonalForward,
    /// Backward diagonal hatching (`\\\`).
    DiagonalBack,
    /// Both diagonals together — a cross-stitch / `XXXX` weave.
    CrossStitch,
}

impl BackgroundPattern {
    /// Every pattern, in the order the `p` key rotates through them.
    pub const ALL: [BackgroundPattern; 8] = [
        Self::None,
        Self::Solid,
        Self::Dots,
        Self::Dots2,
        Self::Lines,
        Self::DiagonalForward,
        Self::DiagonalBack,
        Self::CrossStitch,
    ];

    /// Human-readable name, used in the debug console log.
    pub fn name(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Solid => "solid",
            Self::Dots => "dots",
            Self::Dots2 => "dots2",
            Self::Lines => "lines",
            Self::DiagonalForward => "diagonal ///",
            Self::DiagonalBack => "diagonal \\\\\\",
            Self::CrossStitch => "cross-stitch",
        }
    }

    /// The next pattern in [`BackgroundPattern::ALL`], wrapping at the end.
    pub fn next(self) -> Self {
        let i = Self::ALL.iter().position(|&p| p == self).unwrap_or(0);
        Self::ALL[(i + 1) % Self::ALL.len()]
    }
}

/// A named foreground color the `c` key rotates through.
#[derive(Clone, Copy, Debug)]
pub struct PatternColor {
    pub name: &'static str,
    pub color: Color,
}

/// The palette the `c` key cycles, lightest → darkest.
pub const PATTERN_COLORS: [PatternColor; 4] = [
    PatternColor {
        name: "superlight",
        color: Color::rgb(0xEE, 0xEE, 0xEE),
    },
    PatternColor {
        name: "light",
        color: Color::rgb(0xC0, 0xC0, 0xC0),
    },
    PatternColor {
        name: "dark",
        color: Color::rgb(0x40, 0x40, 0x40),
    },
    PatternColor {
        name: "black",
        color: Color::rgb(0x00, 0x00, 0x00),
    },
];

/// Live background-pattern state owned by a backend (winit or Wayland).
///
/// Holds the selected pattern and an index into [`PATTERN_COLORS`], and turns
/// the `p` / `c` debug keystrokes into rotations. The pattern is deliberately
/// kept out of the widget tree: it belongs to the window, and both backends
/// share this one small state object.
pub(crate) struct BackgroundState {
    pub pattern: BackgroundPattern,
    color_idx: usize,
}

impl BackgroundState {
    /// Start out plain (no pattern) so existing apps render unchanged until
    /// the user opts in by pressing `p`.
    pub fn new() -> Self {
        Self {
            pattern: BackgroundPattern::None,
            color_idx: 0,
        }
    }

    /// The currently selected foreground color.
    pub fn color(&self) -> Color {
        PATTERN_COLORS[self.color_idx % PATTERN_COLORS.len()].color
    }

    fn color_name(&self) -> &'static str {
        PATTERN_COLORS[self.color_idx % PATTERN_COLORS.len()].name
    }

    /// Apply a debug hotkey: `p` rotates the pattern, `c` rotates the color.
    /// Returns `true` if `ch` was one of those keys (so the caller can consume
    /// the keystroke and repaint). Any change is echoed to the console.
    pub fn handle_key(&mut self, ch: char) -> bool {
        match ch.to_ascii_lowercase() {
            'p' => {
                self.pattern = self.pattern.next();
                self.log();
                true
            }
            'c' => {
                self.color_idx = (self.color_idx + 1) % PATTERN_COLORS.len();
                self.log();
                true
            }
            _ => false,
        }
    }

    /// Print the current pattern + color so the change is observable even
    /// when the pattern is hidden behind an opaque widget tree.
    fn log(&self) {
        let c = self.color();
        println!(
            "[saudade] background pattern: {} | color: {} (#{:02X}{:02X}{:02X})",
            self.pattern.name(),
            self.color_name(),
            c.red(),
            c.green(),
            c.blue(),
        );
    }
}
