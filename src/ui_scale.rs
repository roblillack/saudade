//! How big a logical pixel is.
//!
//! saudade lays its widgets out in logical pixels at a reference density of
//! 72 dpi ([`REFERENCE_DPI`]): a list row is 18 of them, a checkbox 13, the
//! padding either side of a menu-bar label 8.
//!
//! The scale the whole UI is drawn at is two factors multiplied together:
//!
//! 1. **Reference alignment** — the density of one of the *platform's* own
//!    logical units, 96 dpi ([`NOMINAL_DPI`]), over [`REFERENCE_DPI`]: 96/72 =
//!    4/3. Windows and X11 report a scale factor that is already a ratio
//!    against 96 dpi: winit divides the reported DPI, or `Xft.dpi`, by 96. A
//!    macOS point counts the same, though `backingScaleFactor` says nothing of
//!    density: on the same glass a point and a Windows unit come out the same
//!    size — a 27" 4K panel is 1.5 panel pixels to either, at the 150% Windows
//!    picks for it and in the 2560x1440 mode macOS does — and each platform
//!    lays its own UI out in about as many of them. Physically both are nearer
//!    1/108 in there; 96 is what they are called, not what either measures,
//!    so measuring one and not the other would draw a Mac an eighth larger.
//! 2. **The system's scaling factor** — what the platform reports for the
//!    display, which carries both its density and the scaling the user set
//!    in the OS: the percentage in Windows' Display Settings, `Xft.dpi` on
//!    X11, `backingScaleFactor` on macOS.
//!
//! The product is snapped to a twelfth ([`STEP`]). Both halves of the
//! derivation are configurable, from the environment or from the code, and the
//! environment wins:
//!
//! * [`SCALE_VAR`], and `App::with_ui_scale`, set the drawn scale *outright*:
//!   the derivation above is skipped entire, the system factor included.
//! * [`DPI_VAR`] moves [`REFERENCE_DPI`] instead, so `SAUDADE_UI_DPI=96` draws
//!   the chrome at the platform's own size, a quarter smaller than the default
//!   does.

/// Name of the scale override, `SAUDADE_UI_SCALE`: the scale to draw at,
/// outright, in place of everything this module works out.
const SCALE_VAR: &str = "SAUDADE_UI_SCALE";

/// Name of the reference-density override, `SAUDADE_UI_DPI`: what a logical
/// pixel is taken to span, in place of [`REFERENCE_DPI`].
const DPI_VAR: &str = "SAUDADE_UI_DPI";

/// What [`SCALE_VAR`] may ask for. The top is past anything either platform
/// produces and the bottom leaves a readable window: a guard against a typo
/// leaving nothing usable.
const SCALE_RANGE: std::ops::RangeInclusive<f32> = 0.25..=8.0;

/// What [`DPI_VAR`] may ask for: half the platforms' own 96 to twice it, which
/// is as far either way as the chrome's proportions bear looking at.
const DPI_RANGE: std::ops::RangeInclusive<f32> = 48.0..=192.0;

/// The density saudade's logical pixel is drawn for unless [`DPI_VAR`] says
/// otherwise.
const REFERENCE_DPI: f32 = 72.0;

/// What one of the platform's logical units stands for at a scale factor of 1:
/// the 96 dpi a Windows or X11 factor is a ratio against, and what a macOS
/// point is taken for too.
const NOMINAL_DPI: f32 = 96.0;

/// The grid the scale is snapped to: a twelfth.
const STEP: f32 = 12.0;

/// The floor under a snapped scale — three steps, a quarter. A guard against a
/// factor small enough to snap the product to nothing.
const MIN_STEPS: f32 = 3.0;

/// The scale to draw at on a display whose system scale factor is `os_scale`,
/// given the density `reference_dpi` a logical pixel is aimed at: the platform
/// unit's [96 dpi](NOMINAL_DPI) over that one, times the factor, snapped to a
/// [twelfth](STEP).
pub(crate) fn effective_for(os_scale: f32, reference_dpi: f32) -> f32 {
    if !os_scale.is_finite() || os_scale <= 0.0 {
        return 1.0;
    }
    let dpi = os_scale * NOMINAL_DPI;
    (dpi / reference_dpi * STEP).round().max(MIN_STEPS) / STEP
}

/// The scale pinned by the environment or by the app, drawn at in place of
/// everything [`effective_for`] works out — the system factor included, since
/// this *is* the logical→physical scale and not a correction over one. Read
/// once at startup; the environment wins over the app's own choice.
pub(crate) fn pinned(app: Option<f32>) -> Option<f32> {
    from_env(SCALE_VAR, SCALE_RANGE).or(app)
}

/// The density a logical pixel is aimed at: [`DPI_VAR`] where it says, else
/// [`REFERENCE_DPI`]. Read once at startup. Moving it moves every scale at
/// once, by the default's 72 over whatever it is set to.
pub(crate) fn reference_dpi() -> f32 {
    from_env(DPI_VAR, DPI_RANGE).unwrap_or(REFERENCE_DPI)
}

/// Read `var`. `None` if it is unset, empty, or `auto` — all of which mean
/// "whatever the app and the platform say" — and also what a value outside
/// `range` falls back to, with a warning.
fn from_env(var: &str, range: std::ops::RangeInclusive<f32>) -> Option<f32> {
    let raw = std::env::var(var).ok()?;
    match parse(&raw, range) {
        Parsed::Value(value) => Some(value),
        Parsed::Auto => None,
        Parsed::Junk => {
            eprintln!("[saudade] ignoring unknown {var}={raw:?}");
            None
        }
    }
}

/// What either variable can say.
#[derive(Debug, PartialEq)]
enum Parsed {
    Value(f32),
    Auto,
    Junk,
}

fn parse(raw: &str, range: std::ops::RangeInclusive<f32>) -> Parsed {
    let value = raw.trim();
    if value.is_empty() || value.eq_ignore_ascii_case("auto") {
        return Parsed::Auto;
    }
    match value.parse::<f32>() {
        Ok(value) if value.is_finite() && range.contains(&value) => Parsed::Value(value),
        _ => Parsed::Junk,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every scale factor Windows' own knob can be set to, as winit reports it:
    /// a ratio against 96 dpi, and always a quarter step. Both of the
    /// `backingScaleFactor`s a Mac reports are among them.
    const FACTORS: &[f32] = &[1.0, 1.25, 1.5, 1.75, 2.0, 2.25, 2.5, 3.0, 3.5];

    /// The scale a display of factor `os_scale` is drawn at, aiming a logical
    /// pixel at the default density.
    fn at(os_scale: f32) -> f32 {
        effective_for(os_scale, REFERENCE_DPI)
    }

    /// Every scale lands on a twelfth, which is what the painter's crisp chrome
    /// is tested against.
    #[test]
    fn every_scale_we_produce_is_a_twelfth_step() {
        for os_scale in [0.5, 0.75, 1.0, 1.1, 1.25, 1.5, 1.75, 2.0, 2.5, 3.0] {
            let eff = at(os_scale);
            assert_eq!((eff * STEP).fract(), 0.0, "{os_scale} {eff}");
        }
    }

    /// The ladder: a third over the factor, a platform's unit being 96 dpi to
    /// our 72 — on a Mac as much as on Windows and X11.
    #[test]
    fn the_ladder() {
        assert_eq!(at(1.0), 4.0 / 3.0);
        assert_eq!(at(1.25), 5.0 / 3.0);
        assert_eq!(at(1.5), 2.0);
        assert_eq!(at(2.0), 8.0 / 3.0);
    }

    /// The snap costs nothing on any rung: a platform unit is sixteen twelfths
    /// of ours, so every factor lands on the grid exactly as it stands.
    #[test]
    fn no_rung_of_the_ladder_is_rounded() {
        assert_eq!(NOMINAL_DPI / REFERENCE_DPI, 16.0 / STEP);
        for &os_scale in FACTORS {
            let unsnapped = os_scale * NOMINAL_DPI / REFERENCE_DPI;
            assert_eq!(at(os_scale), unsnapped, "at {os_scale}");
        }
    }

    /// The one factor that is off the grid entirely: an `Xft.dpi` set by hand,
    /// which winit hands over as a ratio against 96. Half a step at most from
    /// wherever it started.
    #[test]
    fn a_hand_set_dpi_lands_on_the_ladder() {
        // 18⅓ twelfths and 16⅓: the nearest rungs are 150% and 100%'s.
        assert_eq!(at(110.0 / 96.0), 1.5);
        assert_eq!(at(98.0 / 96.0), 4.0 / 3.0);
        // 141 dpi is the worst the snap can do: 23.5 twelfths, exactly half a
        // step from either neighbour — hence the hair of slack, which is the
        // division's own rounding and not the snap's.
        let worst = 0.5 / STEP + f32::EPSILON;
        for dpi in [70.0, 84.0, 96.0, 110.0, 120.0, 141.0, 192.0] {
            let scale = at(dpi / 96.0);
            let wanted = dpi / REFERENCE_DPI;
            assert!((scale - wanted).abs() <= worst, "{dpi} -> {scale}");
        }
    }

    /// The point of the one alignment: a 27" 4K panel comes out the same size
    /// whichever platform is describing it — Windows at the 150% it picks there,
    /// straight onto the panel, and macOS at 2x in the 2560x1440 mode it picks,
    /// rendering 5120 wide and resampled by 0.75 on the way out. Either way a
    /// platform unit is 1.5 panel pixels, so a logical pixel has to be the same
    /// number of them: two.
    #[test]
    fn the_same_glass_comes_out_the_same_size_either_way() {
        let panel_px = 3840.0;
        let inches = 23.5;
        // Panel pixels a logical pixel covers, given the scale drawn at and how
        // many panel pixels one of that scale's device pixels is worth.
        let covers = |scale: f32, panel_per_device: f32| scale * panel_per_device;

        let on_windows = covers(at(1.5), 1.0);
        let on_macos = covers(at(2.0), panel_px / 5120.0);
        assert_eq!(on_windows, 2.0);
        assert_eq!(on_macos, on_windows);

        // Which puts it within a fifth of the 1/72 in target.
        let dpi = panel_px / inches / on_windows;
        let target = 0.8 * REFERENCE_DPI..=1.2 * REFERENCE_DPI;
        assert!(target.contains(&dpi), "{dpi}");
    }

    #[test]
    fn a_nonsense_factor_does_not_produce_a_nonsense_scale() {
        assert_eq!(effective_for(f32::NAN, REFERENCE_DPI), 1.0);
        assert_eq!(effective_for(f32::INFINITY, REFERENCE_DPI), 1.0);
        assert_eq!(effective_for(0.0, REFERENCE_DPI), 1.0);
        assert_eq!(effective_for(-2.0, REFERENCE_DPI), 1.0);
        // A factor no display reports, which would otherwise snap to zero and
        // leave nothing to draw with.
        assert_eq!(effective_for(f32::MIN_POSITIVE, REFERENCE_DPI), 0.25);
    }

    /// A pinned scale is drawn at as it stands — no alignment, no snapping, no
    /// system factor — so a card that wants 2.65 can have it.
    #[test]
    fn a_pinned_scale_is_the_scale() {
        assert_eq!(parse("2.65", SCALE_RANGE), Parsed::Value(2.65));
        assert_eq!(parse("1.5", SCALE_RANGE), Parsed::Value(1.5));
        assert_eq!(parse(" 2 ", SCALE_RANGE), Parsed::Value(2.0));
        assert_eq!(parse("1", SCALE_RANGE), Parsed::Value(1.0));
        assert_eq!(parse("6.67", SCALE_RANGE), Parsed::Value(6.67));
        assert_eq!(parse("12", SCALE_RANGE), Parsed::Junk);
    }

    /// The other knob moves the reference density, so every scale moves with
    /// it: 96 dpi — the platforms' own — draws a quarter under the default at
    /// every rung, which is the factor as it stands.
    #[test]
    fn the_reference_density_moves_the_whole_ladder() {
        for &os_scale in FACTORS {
            assert_eq!(effective_for(os_scale, 96.0), os_scale, "at {os_scale}");
        }
    }

    #[test]
    fn a_density_that_would_leave_no_usable_ui_is_refused() {
        assert_eq!(parse("72", DPI_RANGE), Parsed::Value(72.0));
        assert_eq!(parse("48", DPI_RANGE), Parsed::Value(48.0));
        assert_eq!(parse("192", DPI_RANGE), Parsed::Value(192.0));
        assert_eq!(parse("47", DPI_RANGE), Parsed::Junk);
        assert_eq!(parse("300", DPI_RANGE), Parsed::Junk);
        // The scale knob's units, typed into the density one by mistake.
        assert_eq!(parse("1.5", DPI_RANGE), Parsed::Junk);
    }

    #[test]
    fn nothing_in_particular_means_the_default() {
        for range in [SCALE_RANGE, DPI_RANGE] {
            assert_eq!(parse("", range.clone()), Parsed::Auto);
            assert_eq!(parse("  ", range.clone()), Parsed::Auto);
            assert_eq!(parse("auto", range.clone()), Parsed::Auto);
            assert_eq!(parse("AUTO", range), Parsed::Auto);
        }
    }

    #[test]
    fn a_scale_that_would_leave_no_usable_ui_is_refused() {
        assert_eq!(parse("0", SCALE_RANGE), Parsed::Junk);
        assert_eq!(parse("-2", SCALE_RANGE), Parsed::Junk);
        assert_eq!(parse("0.1", SCALE_RANGE), Parsed::Junk);
        assert_eq!(parse("nan", SCALE_RANGE), Parsed::Junk);
        assert_eq!(parse("inf", SCALE_RANGE), Parsed::Junk);
        assert_eq!(parse("1.5x", SCALE_RANGE), Parsed::Junk);
    }
}
