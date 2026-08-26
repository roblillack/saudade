//! How big a logical pixel is, in the real world.
//!
//! saudade's logical pixel is a *96-dpi* pixel. A list row is 18 of them, a
//! checkbox 13, the padding either side of a menu-bar label 8: Windows 3.1
//! metrics, drawn for a display where 96 of them span an inch.
//!
//! Except that no display ever did. 96 dpi was Windows' *nominal* figure, and
//! the glass these metrics were drawn against was much coarser: a 14" CRT at
//! 640x480 runs 57 ppi, a 15" at 1024x768 runs 85. A 13-pixel checkbox was a
//! sixth of an inch across on the screen it was designed on, and it is that
//! size — the size the chrome was drawn to *feel* — that we are trying to put
//! back. Rendered at a true 96 dpi the whole UI comes out small.
//!
//! So the chrome is drawn a quarter over nominal ([`OVERSIZE`]), which puts a
//! logical pixel at 1/77 in — the coarse end of what 90s screens really ran.
//! The question is a quarter over nominal *of what*, and the platforms disagree
//! about what their own logical unit stands for:
//!
//! * **Windows and X11** hand over a ratio against 96 dpi, so their unit is the
//!   nominal one already. winit's Windows backend divides the reported DPI by a
//!   `BASE_DPI` of 96, so whatever the user picks in Display Settings — 100%,
//!   125%, 150% — a logical pixel is 1/96 in by construction. Its X11 backend
//!   divides `Xft.dpi` by 96, and when that resource is unset it measures the
//!   panel through XRandR and divides *that* by 96 (`calc_dpi_factor`). The
//!   correction is [`OVERSIZE`] as it stands: 1.25.
//! * **macOS in a HiDPI mode** hands over an `NSWindow.backingScaleFactor` of
//!   2 — a count of device pixels per point, not a density — and lays the
//!   desktop out in a point nearer 1/108 in: a 27" 4K in its default mode puts
//!   108 points in an inch, a 14" MacBook Pro 127.5, both reporting exactly
//!   2.0. Reaching the same physical size from a denser unit takes 108/96 more,
//!   so the correction is 1.25 × 108/96 = 1.40625.
//! * **macOS in a 1x mode** is back to the nominal unit — a point is a device
//!   pixel there, and the density is whatever the panel's own is, the same
//!   position Windows and X11 leave us in. So 1.25, like them.
//!
//! The *product* — the OS factor times that correction — is snapped to a quarter,
//! since the number that has to look right is the scale the window is actually
//! drawn at. At a quarter step a logical edge lands on a whole device pixel
//! every four logical pixels; the nearest twelfth only manages every six. It is
//! also the ladder the painter's chrome is tested along.
//!
//! One panel, both platforms, as a check that the two baselines agree — a 27"
//! 4K, which Windows drives at 150% and macOS in its default "looks like
//! 2560x1440" HiDPI mode. Both lay the desktop out at ~109 points per inch:
//!
//! ```text
//! Windows  1.5 x 1.25    = 1.875  -> 2.0     2.0 device px per logical px
//! macOS    2.0 x 1.40625 = 2.8125 -> 2.75    2.75 x 0.75 = 2.06 on the glass
//! ```
//!
//! The macOS row carries the extra 0.75 because that mode renders 5120x2880 and
//! is scaled down to the panel's 3840x2160 — so the two land within 3% of each
//! other on the same glass, at very nearly the same physical size.
//!
//! What we deliberately do *not* do is measure the panel and divide, the way
//! winit does for X11. It defeats itself on macOS, where the *resolution* is
//! the size knob — the "Larger Text ↔ More Space" slider is nothing but a list
//! of display modes — so a correction derived from the measurement cancels out
//! the one control the user has: pick a roomier mode and every point gets
//! smaller by exactly as much as the correction grows, and the UI comes back
//! the size it was. A constant base leaves the knob working. On Windows and X11
//! the knob is the scale factor itself, which we are already multiplying, so it
//! keeps working there for the same reason.

/// Name of the environment override, `SAUDADE_UI_SCALE`.
const ENV_VAR: &str = "SAUDADE_UI_SCALE";

/// Widest scale [`ENV_VAR`] may ask for. Well past any real display; a guard
/// against a typo leaving no usable window.
const MAX_SCALE: f32 = 4.0;

/// The dpi saudade's logical pixel is nominally in — Windows 3.1's figure, and
/// what winit's own scale factors are ratios against.
const NOMINAL_DPI: f32 = 96.0;

/// How much larger than its nominal rendering the chrome wants to be: a
/// quarter, which puts a logical pixel at 1/77 in rather than 1/96.
const OVERSIZE: f32 = 1.25;

/// What the platform's logical unit actually stands for, on a display whose OS
/// scale factor is `os_scale`.
///
/// [`NOMINAL_DPI`] wherever the scale factor is already a ratio against it —
/// Windows, X11, and a macOS 1x mode, where a point is a device pixel. 108 in a
/// macOS HiDPI mode, which lays the desktop out in a point nearer 1/108 in.
fn baseline_dpi(os_scale: f32) -> f32 {
    // A `backingScaleFactor` of 2 is the only value above 1 that macOS reports,
    // and it says "HiDPI" rather than naming any particular density.
    if cfg!(target_os = "macos") && os_scale > 1.0 {
        108.0
    } else {
        NOMINAL_DPI
    }
}

/// The scale to draw at on a display whose OS scale factor is `os_scale`:
/// [`OVERSIZE`] against that display's [`baseline_dpi`], times `os_scale`,
/// snapped to a quarter.
///
/// Nearest quarter rather than the next one up, so the ladder doesn't
/// systematically inflate — though the two agree on the cases that matter, a
/// Windows display at 150% (1.875) and a Retina Mac (2.8125) both landing where
/// they would either way.
///
/// Never below `os_scale` itself, following winit's X11 code, which floors its
/// own factor at 1.0: a display sparse enough to want shrinking is a television
/// being read from the sofa, where the chrome is already as small as it should
/// get.
pub(crate) fn effective_for(os_scale: f32) -> f32 {
    if !os_scale.is_finite() || os_scale <= 0.0 {
        return 1.0;
    }
    let base = OVERSIZE * baseline_dpi(os_scale) / NOMINAL_DPI;
    let snapped = (os_scale * base * 4.0).round() / 4.0;
    snapped.max(os_scale)
}

/// A scale pinned by the environment or by the app, used verbatim in place of
/// [`effective_for`]. Read once at startup.
///
/// The environment wins over the app's own choice, in the spirit of winit's
/// `WINIT_X11_SCALE_FACTOR`: it is there to try a UI at another size without
/// touching the program.
pub(crate) fn pinned(app: Option<f32>) -> Option<f32> {
    from_env().or(app)
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

    /// Every OS factor lands on a quarter, which is what the painter's crisp
    /// chrome is tested against.
    #[test]
    fn every_scale_we_produce_is_a_quarter_step() {
        for os_scale in [0.75, 1.0, 1.25, 1.5, 1.75, 2.0, 2.5, 3.0] {
            let eff = effective_for(os_scale);
            assert_eq!((eff * 4.0).fract(), 0.0, "{os_scale} -> {eff}");
        }
    }

    /// The same 27" 4K panel on both platforms, as the module docs work it
    /// through: Windows drives it at 150%, macOS at 2.0 in its default HiDPI
    /// mode, and the two come out within 3% on the glass.
    #[test]
    #[cfg(not(target_os = "macos"))]
    fn a_4k_27_at_150_percent_lands_on_a_clean_2x() {
        assert_eq!(effective_for(1.5), 2.0);
    }

    #[test]
    #[cfg(target_os = "macos")]
    fn a_retina_mac_lands_on_two_and_three_quarters() {
        // 2.0 x 1.25 x 108/96 = 2.8125, nearer 2.75 than 3.0.
        assert_eq!(effective_for(2.0), 2.75);
    }

    /// A 1x mode is corrected against the nominal unit like everything else —
    /// on a Mac too, a point being a device pixel there.
    #[test]
    fn a_1x_display_is_corrected_against_the_nominal_unit() {
        assert_eq!(baseline_dpi(1.0), NOMINAL_DPI);
        assert_eq!(effective_for(1.0), 1.25);
    }

    /// The denser baseline belongs to the HiDPI *mode*, not to the platform: it
    /// applies exactly where the OS is doubling, and the correction it implies
    /// is the nominal one scaled by the two baselines' ratio.
    #[test]
    #[cfg(target_os = "macos")]
    fn only_a_hidpi_mode_gets_the_denser_baseline() {
        assert_eq!(baseline_dpi(2.0), 108.0);
        assert_eq!(OVERSIZE * baseline_dpi(2.0) / NOMINAL_DPI, 1.40625);
    }

    /// The Windows ladder, in full. 125% is the one that rounds *down*: 1.5625
    /// is nearer 1.5 than 1.75.
    #[test]
    #[cfg(not(target_os = "macos"))]
    fn the_windows_ladder_walks_in_quarters() {
        assert_eq!(effective_for(1.25), 1.5);
        assert_eq!(effective_for(1.75), 2.25);
        assert_eq!(effective_for(2.0), 2.5);
        assert_eq!(effective_for(3.0), 3.75);
    }

    #[test]
    fn a_nonsense_factor_does_not_produce_a_nonsense_scale() {
        assert_eq!(effective_for(f32::NAN), 1.0);
        assert_eq!(effective_for(f32::INFINITY), 1.0);
        assert_eq!(effective_for(0.0), 1.0);
        assert_eq!(effective_for(-2.0), 1.0);
        // Below 0.5 the snapped product could fall under the factor itself.
        assert!(effective_for(0.3) >= 0.3);
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
