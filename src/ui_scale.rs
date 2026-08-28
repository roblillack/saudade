//! How big a logical pixel is.
//!
//! saudade lays its widgets out in logical pixels at a reference density of
//! 96 dpi ([`REFERENCE_DPI`]): a list row is 18 of them, a checkbox 13, the
//! padding either side of a menu-bar label 8.
//!
//! The scale the whole UI is drawn at is two factors multiplied together:
//!
//! 1. **Reference alignment** — the density of one of the *platform's* own
//!    logical units over [`REFERENCE_DPI`]. Windows and X11 report a scale
//!    factor that is already a ratio against 96 dpi ([`NOMINAL_DPI`]): winit
//!    divides the reported DPI, or `Xft.dpi`, by 96, so their logical unit is
//!    ours and the alignment is 1. A macOS point is nearer 1/108 in
//!    ([`MACOS_DPI`]) — about how Apple lays a desktop out, 218 dpi over a
//!    `backingScaleFactor` of 2 — so there the alignment is 108/96 = 1.125.
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
//! * [`DPI_VAR`] moves [`REFERENCE_DPI`] instead, so `SAUDADE_UI_DPI=72` draws
//!   the chrome a third larger than the default does.

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

/// What [`DPI_VAR`] may ask for: half the default to twice it, which is as far
/// either way as the chrome's proportions bear looking at.
const DPI_RANGE: std::ops::RangeInclusive<f32> = 48.0..=192.0;

/// The density saudade's logical pixel is drawn for unless [`DPI_VAR`] says
/// otherwise.
const REFERENCE_DPI: f32 = 96.0;

/// What a logical unit stands for where the platform's scale factor is already
/// a ratio against 96 dpi — Windows and X11/Wayland.
const NOMINAL_DPI: f32 = 96.0;

/// What a point stands for on macOS, whose `backingScaleFactor` counts device
/// pixels per point and carries no density of its own.
const MACOS_DPI: f32 = 108.0;

/// The density one of this platform's logical units stands for at a scale
/// factor of 1.
fn platform_dpi() -> f32 {
    if cfg!(target_os = "macos") {
        MACOS_DPI
    } else {
        NOMINAL_DPI
    }
}

/// The grid the scale is snapped to: a twelfth.
const STEP: f32 = 12.0;

/// The floor under a snapped scale — three steps, a quarter. A guard against a
/// factor small enough to snap the product to nothing.
const MIN_STEPS: f32 = 3.0;

/// The scale to draw at on a display whose system scale factor is `os_scale`,
/// given the density `reference_dpi` a logical pixel is aimed at: the platform
/// unit's own density over that one, times the factor, snapped to a
/// [twelfth](STEP).
pub(crate) fn effective_for(os_scale: f32, reference_dpi: f32) -> f32 {
    if !os_scale.is_finite() || os_scale <= 0.0 {
        return 1.0;
    }
    scale_for(os_scale, platform_dpi(), reference_dpi)
}

/// [`effective_for`] with the platform's baseline spelled out, so both ladders
/// can be walked from either platform's tests.
fn scale_for(os_scale: f32, platform_dpi: f32, reference_dpi: f32) -> f32 {
    let dpi = os_scale * platform_dpi;
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
/// once, by 96 over whatever it is set to.
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
    /// a ratio against 96 dpi, and always a quarter step.
    const WINDOWS_FACTORS: &[f32] = &[1.0, 1.25, 1.5, 1.75, 2.0, 2.25, 2.5, 3.0, 3.5];

    /// Every `backingScaleFactor` macOS reports.
    const MACOS_FACTORS: &[f32] = &[1.0, 2.0];

    /// The scale a Windows or X11 display of factor `os_scale` is drawn at,
    /// aiming a logical pixel at the default density.
    fn windows(os_scale: f32) -> f32 {
        scale_for(os_scale, NOMINAL_DPI, REFERENCE_DPI)
    }

    /// The same for a Mac, whose logical unit is the denser one.
    fn macos(os_scale: f32) -> f32 {
        scale_for(os_scale, MACOS_DPI, REFERENCE_DPI)
    }

    /// Every scale lands on a twelfth, which is what the painter's crisp chrome
    /// is tested against.
    #[test]
    fn every_scale_we_produce_is_a_twelfth_step() {
        for platform_dpi in [NOMINAL_DPI, MACOS_DPI] {
            for os_scale in [0.5, 0.75, 1.0, 1.1, 1.25, 1.5, 1.75, 2.0, 2.5, 3.0] {
                let eff = scale_for(os_scale, platform_dpi, REFERENCE_DPI);
                assert_eq!((eff * STEP).fract(), 0.0, "{platform_dpi} {os_scale} {eff}");
            }
        }
        assert_eq!((effective_for(1.0, REFERENCE_DPI) * STEP).fract(), 0.0);
    }

    /// The two ladders: the factor itself on Windows and X11, whose logical
    /// unit is ours, and an eighth over it on macOS.
    #[test]
    fn the_ladders() {
        assert_eq!(windows(1.0), 1.0);
        assert_eq!(windows(1.25), 1.25);
        assert_eq!(windows(1.5), 1.5);
        assert_eq!(windows(2.0), 2.0);
        assert_eq!(macos(1.0), 7.0 / 6.0);
        assert_eq!(macos(2.0), 2.25);
    }

    /// The snap costs nothing on either platform's own rungs but a Mac at 1x,
    /// where 9/8 is thirteen and a half twelfths and rounds up by 3%.
    #[test]
    fn only_a_mac_at_1x_is_rounded_at_all() {
        for &os_scale in WINDOWS_FACTORS {
            assert_eq!(windows(os_scale), os_scale, "windows at {os_scale}");
        }
        for &os_scale in MACOS_FACTORS {
            let unsnapped = os_scale * MACOS_DPI / REFERENCE_DPI;
            let rounded = macos(os_scale) != unsnapped;
            assert_eq!(rounded, os_scale == 1.0, "macos at {os_scale}");
            assert!((macos(os_scale) - unsnapped).abs() <= 0.5 / STEP);
        }
        assert_eq!(MACOS_DPI / REFERENCE_DPI, 1.125);
        assert_eq!(macos(1.0), 14.0 / STEP);
    }

    /// The one factor that is off the grid entirely: an `Xft.dpi` set by hand,
    /// which winit hands over as a ratio against 96. Half a step at most from
    /// wherever it started.
    #[test]
    fn a_hand_set_dpi_lands_on_the_ladder() {
        assert_eq!(windows(110.0 / 96.0), 7.0 / 6.0);
        assert_eq!(windows(98.0 / 96.0), 1.0);
        // 84 dpi is the worst the snap can do: 10.5 twelfths, exactly half a
        // step from either neighbour — hence the hair of slack, which is the
        // division's own rounding and not the snap's.
        let worst = 0.5 / STEP + f32::EPSILON;
        for dpi in [70.0, 84.0, 96.0, 110.0, 120.0, 141.0, 192.0] {
            let scale = windows(dpi / 96.0);
            let wanted = dpi / REFERENCE_DPI;
            assert!((scale - wanted).abs() <= worst, "{dpi} -> {scale}");
        }
    }

    /// The point of the two alignments: one 27" 4K panel comes out about the
    /// same size whichever platform is describing it — Windows at 150% straight
    /// onto the panel, macOS at 2x in the scaled mode it picks there.
    #[test]
    fn the_same_glass_comes_out_about_the_same_size_either_way() {
        let panel_px = 3840.0;
        let inches = 23.5;
        // Inches a logical pixel covers, given the scale drawn at and how many
        // panel pixels one of that scale's device pixels is worth.
        let span = |scale: f32, panel_per_device: f32| scale * panel_per_device * inches / panel_px;

        let on_windows = span(windows(1.5), 1.0);
        let on_macos = span(macos(2.0), panel_px / 5120.0);

        // Both within a fifth of the 1/96 in target, and within an eighth of
        // each other.
        for (name, span) in [("windows", on_windows), ("macos", on_macos)] {
            let dpi = 1.0 / span;
            assert!((80.0..=120.0).contains(&dpi), "{name} {dpi}");
        }
        assert!(on_windows / on_macos > 0.8 && on_windows / on_macos < 1.25);
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
    /// it: 72 dpi draws a third over the default at every rung, and which rungs
    /// round moves too, the grid being a twelfth of the reference.
    #[test]
    fn the_reference_density_moves_the_whole_ladder() {
        assert_eq!(scale_for(1.0, NOMINAL_DPI, 72.0), 4.0 / 3.0);
        assert_eq!(scale_for(1.5, NOMINAL_DPI, 72.0), 2.0);
        assert_eq!(scale_for(2.0, MACOS_DPI, 72.0), 3.0);
        // At 72 even a Mac at 1x is exact, where the default has to round it.
        assert_eq!(scale_for(1.0, MACOS_DPI, 72.0), 1.5);
        assert_ne!(macos(1.0), MACOS_DPI / REFERENCE_DPI);
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
