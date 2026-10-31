//! Light or dark: which of an app's two themes paints it.
//!
//! An [`App`](crate::App) carries a light theme and a dark one and paints with
//! whichever matches the system's appearance, switching over live when the
//! user flips it. Where that appearance comes from depends on the platform:
//!
//! * **macOS, Windows** — winit reports it, and every change after.
//! * **X11, Wayland** — the `color-scheme` setting of the freedesktop settings
//!   portal, which GNOME, KDE and most other desktops publish (see
//!   [`crate::portal`]). Without a portal to ask, it is light.
//!
//! [`THEME_VAR`] overrides the system either way.

use crate::theme::Theme;

/// Name of the override, `SAUDADE_THEME`: `light` or `dark` paints with that
/// theme whatever the system's appearance; `auto` — like leaving it unset —
/// follows the system.
const THEME_VAR: &str = "SAUDADE_THEME";

/// The system's light or dark mode.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum Appearance {
    #[default]
    Light,
    Dark,
}

impl Appearance {
    /// What [`THEME_VAR`] pins, if anything. An unknown value is reported on
    /// stderr and ignored. Read once at startup.
    pub(crate) fn forced() -> Option<Self> {
        let raw = std::env::var(THEME_VAR).ok()?;
        match parse(&raw) {
            Parsed::Pinned(appearance) => Some(appearance),
            Parsed::Auto => None,
            Parsed::Junk => {
                eprintln!("[saudade] ignoring unknown {THEME_VAR}={raw:?}");
                None
            }
        }
    }
}

/// What [`THEME_VAR`] can say.
#[derive(Debug, PartialEq)]
enum Parsed {
    Pinned(Appearance),
    Auto,
    Junk,
}

fn parse(raw: &str) -> Parsed {
    let value = raw.trim();
    if value.is_empty() || value.eq_ignore_ascii_case("auto") {
        Parsed::Auto
    } else if value.eq_ignore_ascii_case("light") {
        Parsed::Pinned(Appearance::Light)
    } else if value.eq_ignore_ascii_case("dark") {
        Parsed::Pinned(Appearance::Dark)
    } else {
        Parsed::Junk
    }
}

/// An app's two themes, and which of them is showing.
pub(crate) struct Themes {
    light: Theme,
    dark: Theme,
    /// What the environment pinned, if anything: the system is not asked, and
    /// what it reports is ignored.
    forced: Option<Appearance>,
    appearance: Appearance,
}

impl Themes {
    /// Start out light — or as [`THEME_VAR`] says — until the system is heard
    /// from.
    pub(crate) fn new(light: Theme, dark: Theme) -> Self {
        Self::with_forced(light, dark, Appearance::forced())
    }

    fn with_forced(light: Theme, dark: Theme, forced: Option<Appearance>) -> Self {
        Self {
            light,
            dark,
            forced,
            appearance: forced.unwrap_or_default(),
        }
    }

    /// The theme to paint with.
    pub(crate) fn current(&self) -> &Theme {
        match self.appearance {
            Appearance::Light => &self.light,
            Appearance::Dark => &self.dark,
        }
    }

    /// Which theme is showing.
    pub(crate) fn appearance(&self) -> Appearance {
        self.appearance
    }

    /// Whether to listen to the system at all — not while the environment has
    /// pinned the appearance.
    pub(crate) fn follows_system(&self) -> bool {
        self.forced.is_none()
    }

    /// The system's appearance is `system`. Returns `true` if that changed
    /// the theme, so every window wants painting again; `false` if it was
    /// already showing, or the environment has pinned the other one.
    pub(crate) fn follow(&mut self, system: Appearance) -> bool {
        if !self.follows_system() || self.appearance == system {
            return false;
        }
        self.appearance = system;
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn themes(forced: Option<Appearance>) -> Themes {
        Themes::with_forced(Theme::windows_31(), Theme::windows_31_dark(), forced)
    }

    #[test]
    fn nothing_in_particular_follows_the_system() {
        assert_eq!(parse(""), Parsed::Auto);
        assert_eq!(parse("  "), Parsed::Auto);
        assert_eq!(parse("auto"), Parsed::Auto);
        assert_eq!(parse("AUTO"), Parsed::Auto);
    }

    #[test]
    fn light_and_dark_pin_the_appearance() {
        assert_eq!(parse("light"), Parsed::Pinned(Appearance::Light));
        assert_eq!(parse(" Dark "), Parsed::Pinned(Appearance::Dark));
        assert_eq!(parse("DARK"), Parsed::Pinned(Appearance::Dark));
        assert_eq!(parse("night"), Parsed::Junk);
        assert_eq!(parse("1"), Parsed::Junk);
    }

    #[test]
    fn the_light_theme_shows_until_the_system_says_otherwise() {
        let mut themes = themes(None);
        assert_eq!(themes.appearance(), Appearance::Light);
        assert_eq!(themes.current().background, Theme::windows_31().background);

        assert!(themes.follow(Appearance::Dark), "a change wants a repaint");
        assert_eq!(
            themes.current().background,
            Theme::windows_31_dark().background
        );
        assert!(!themes.follow(Appearance::Dark), "the same again does not");

        assert!(themes.follow(Appearance::Light));
        assert_eq!(themes.current().background, Theme::windows_31().background);
    }

    #[test]
    fn a_pinned_appearance_ignores_the_system() {
        let mut themes = themes(Some(Appearance::Dark));
        assert_eq!(themes.appearance(), Appearance::Dark);
        assert!(!themes.follows_system());
        assert!(!themes.follow(Appearance::Light));
        assert_eq!(themes.appearance(), Appearance::Dark);

        let mut themes = self::themes(Some(Appearance::Light));
        assert!(!themes.follow(Appearance::Dark));
        assert_eq!(themes.appearance(), Appearance::Light);
    }
}
