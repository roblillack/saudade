//! How big a logical pixel is, in the real world.
//!
//! saudade's logical pixel is a *96-dpi* pixel. A list row is 18 of them, a
//! checkbox 13, the padding either side of a menu-bar label 8: Windows 3.1
//! metrics, drawn for a display where 96 of them span an inch. Every widget in
//! the library is dimensioned in that unit, so the chrome only comes out the
//! size it was drawn at if the platform keeps a logical pixel near 1/96 in.
//!
//! Windows and X11 both keep it. winit's Windows backend divides the DPI the
//! system reports by a `BASE_DPI` of 96, so whatever the user picks in Display
//! Settings — 100%, 125%, 150% — a logical pixel stays 1/96 in by construction.
//! Its X11 backend divides `Xft.dpi` by 96, and when that resource is unset it
//! measures the panel through XRandR and divides *that* by 96
//! (`calc_dpi_factor`). Wayland's logical unit is normalized the same way.
//!
//! macOS keeps no such promise. `NSWindow.backingScaleFactor` is 1 or 2 — a
//! property of the panel's backing store, not of its density — and there is no
//! "make the UI 125% bigger" setting to move it: the user changes the
//! *resolution* instead, which slides the point size around underneath a scale
//! factor that never budges. A 27" 4K display in its default HiDPI mode (which
//! "looks like" 2560x1440) puts 108 points in an inch and a 14" MacBook Pro
//! puts 127.5, both reporting exactly 2.0, so the same Win 3.1 chrome lands 11%
//! short on one and 25% short on the other. Apple's own metrics absorb this
//! (the HIG's 24-point menu bar is drawn for ~110 ppi, not for 96); ours
//! cannot.
//!
//! The tempting fix is to measure the panel and divide by 96, the way winit
//! does for X11. It defeats itself. On macOS the *resolution* is the size knob
//! — the "Larger Text ↔ More Space" slider is nothing but a list of display
//! modes — so a correction derived from the measurement cancels out the one
//! control the user has: pick a roomier mode and every point gets smaller by
//! exactly as much as the correction grows, and the UI comes back the size it
//! was. (Nothing like that happens on Windows, where the 125% the user picks
//! *is* the scale factor, and on X11, where `Xft.dpi` is set by hand and the
//! measurement is only the fallback.)
//!
//! So we take the one thing every Mac display has in common instead: it is laid
//! out for a point denser than 1/96 in. A fixed [`DEFAULT`] covers that, the
//! resolution keeps working as the knob it is, and an app or a user who wants
//! another size says so ([`crate::App::with_ui_scale`], [`ENV_VAR`]).

/// Name of the environment override, `SAUDADE_UI_SCALE`.
const ENV_VAR: &str = "SAUDADE_UI_SCALE";

/// Widest scale [`ENV_VAR`] may ask for. Well past any real display; a guard
/// against a typo leaving no usable window.
const MAX_SCALE: f32 = 4.0;

/// How much bigger a saudade logical pixel is than one of the platform's own
/// logical units, by default.
///
/// `1.0` wherever the platform's unit is already a 96-dpi one. On macOS, 1.125
/// — a point of 1/108 in, which is both what Apple's default HiDPI mode gives a
/// 27" 4K and close to the density the HIG's own metrics are drawn for. It puts
/// a Retina Mac at an effective 2.25, a quarter step, where a logical edge
/// lands on a whole device pixel every four logical pixels; the painter's
/// chrome is tested along that ladder.
///
/// A deliberately blunt number: it is a statement about the platform, not about
/// the panel. Anyone on a display that wants more than an eighth — a 14" laptop
/// panel asks for a third — reaches for the resolution, and gets the same
/// effect they would from any other Mac app.
pub(crate) const DEFAULT: f32 = if cfg!(target_os = "macos") {
    1.125
} else {
    1.0
};

/// The scale to apply: the environment, else what the app asked for, else
/// [`DEFAULT`]. Read once at startup.
///
/// The environment wins over the app's own choice, in the spirit of winit's
/// `WINIT_X11_SCALE_FACTOR`: it is there to try a UI at another size without
/// touching the program.
pub(crate) fn resolve(app: Option<f32>) -> f32 {
    from_env().or(app).unwrap_or(DEFAULT)
}

/// Read [`ENV_VAR`]. `None` if it is unset, empty, or `auto` — all of which
/// mean "whatever the app and the platform say" — and also what an unusable
/// value falls back to, with a warning.
fn from_env() -> Option<f32> {
    let raw = std::env::var(ENV_VAR).ok()?;
    match parse(&raw) {
        Parsed::Scale(scale) => Some(scale),
        Parsed::Auto => None,
        Parsed::Junk => {
            eprintln!("[saudade] ignoring unknown {ENV_VAR}={raw:?}");
            None
        }
    }
}

/// What [`ENV_VAR`] can say.
#[derive(Debug, PartialEq)]
enum Parsed {
    Scale(f32),
    Auto,
    Junk,
}

fn parse(raw: &str) -> Parsed {
    let value = raw.trim();
    if value.is_empty() || value.eq_ignore_ascii_case("auto") {
        return Parsed::Auto;
    }
    match value.parse::<f32>() {
        Ok(scale) if scale.is_finite() && (0.25..=MAX_SCALE).contains(&scale) => {
            Parsed::Scale(scale)
        }
        _ => Parsed::Junk,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The scale a Mac ends up rendering at is the backing factor times ours,
    /// and 2.0 is the only backing factor left in the field aside from 1.0 on
    /// the last non-Retina machines.
    #[test]
    #[cfg(target_os = "macos")]
    fn the_default_puts_a_retina_mac_on_a_quarter_step() {
        assert_eq!(2.0 * DEFAULT, 2.25);
        // And a non-Retina one on an eighth, still inside the range the
        // painter's crisp-chrome pass covers.
        assert_eq!(1.0 * DEFAULT, 1.125);
    }

    #[test]
    fn an_explicit_scale_is_taken_at_its_word() {
        assert_eq!(parse("1.5"), Parsed::Scale(1.5));
        assert_eq!(parse(" 2 "), Parsed::Scale(2.0));
        assert_eq!(parse("1"), Parsed::Scale(1.0));
    }

    #[test]
    fn nothing_in_particular_means_the_default() {
        assert_eq!(parse(""), Parsed::Auto);
        assert_eq!(parse("  "), Parsed::Auto);
        assert_eq!(parse("auto"), Parsed::Auto);
        assert_eq!(parse("AUTO"), Parsed::Auto);
    }

    #[test]
    fn a_scale_that_would_leave_no_usable_ui_is_refused() {
        assert_eq!(parse("0"), Parsed::Junk);
        assert_eq!(parse("-2"), Parsed::Junk);
        assert_eq!(parse("0.1"), Parsed::Junk);
        assert_eq!(parse("12"), Parsed::Junk);
        assert_eq!(parse("nan"), Parsed::Junk);
        assert_eq!(parse("inf"), Parsed::Junk);
        assert_eq!(parse("1.5x"), Parsed::Junk);
    }
}
