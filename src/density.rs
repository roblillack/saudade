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
//! (`calc_dpi_factor`).
//!
//! macOS keeps no such promise. `NSWindow.backingScaleFactor` is 1 or 2 — a
//! property of the panel's backing store, not of its density — and there is no
//! "make the UI 125% bigger" setting to move it: the user changes the
//! *resolution* instead, which slides the point size around underneath a scale
//! factor that never budges. A 27" 4K display in its default HiDPI mode (which
//! "looks like" 2560x1440) puts 108 points in an inch and a 14" MacBook Pro
//! puts 127.5, both reporting exactly
//! 2.0, so the same Win 3.1 chrome lands 11% short on one and 25% short on the
//! other — and moving a window between them changes nothing about the scale
//! factor at all. Apple's own metrics absorb this (the HIG's 24-point menu bar
//! is drawn for ~110 ppi, not for 96); ours cannot.
//!
//! So on macOS we do what X11 already does for us: measure the display and
//! divide by 96. The result is a *second* factor, multiplied onto the OS scale
//! factor rather than replacing it — the OS still owns the number of physical
//! pixels per point, and this only says how many of saudade's logical pixels
//! should fit in one inch of glass.
//!
//! What that product gets rounded to is the one judgement call here. The number
//! that has to look right is the *effective* scale, not the correction, so the
//! rounding happens there: onto a quarter. See [`snap_effective`].

use winit::window::Window;

/// Name of the environment override, `SAUDADE_UI_SCALE`.
const ENV_VAR: &str = "SAUDADE_UI_SCALE";

/// Widest correction we will apply. Well past any real display; a guard against
/// a driver reporting a physical size in the wrong unit.
const MAX_SCALE: f64 = 4.0;

/// Read `SAUDADE_UI_SCALE`.
///
/// An escape hatch, in the spirit of winit's own `WINIT_X11_SCALE_FACTOR`: it
/// overrides both the derived scale and whatever the app asked for, so a UI can
/// be tried at any size without touching its code. `auto` (or an empty value)
/// means "derive it", which is also what an unparsable value falls back to,
/// with a warning. Read once at startup.
pub(crate) fn from_env() -> Option<f32> {
    let raw = std::env::var(ENV_VAR).ok()?;
    let value = raw.trim();
    if value.is_empty() || value.eq_ignore_ascii_case("auto") {
        return None;
    }
    match value.parse::<f32>() {
        Ok(scale) if scale.is_finite() && (0.25..=MAX_SCALE as f32).contains(&scale) => Some(scale),
        _ => {
            eprintln!("[saudade] ignoring unknown {ENV_VAR}={raw:?}");
            None
        }
    }
}

/// The density correction for the display `win` is currently on: how much to
/// stretch a logical pixel so it lands near 1/96 in of glass.
///
/// `1.0` — leave the OS scale factor alone — on every platform whose scale
/// factor is already a 96-dpi ratio, and on macOS whenever the display declines
/// to say how big it physically is.
#[cfg(target_os = "macos")]
pub(crate) fn for_window(win: &Window) -> f32 {
    use objc2_core_graphics::CGDisplayScreenSize;
    use winit::platform::macos::MonitorHandleExtMacOS;

    let Some(monitor) = win.current_monitor() else {
        return 1.0;
    };
    // `CGDisplayScreenSize` reports the panel's physical size in millimetres,
    // straight from its EDID. AirPlay targets, a few KVMs and the odd projector
    // have nothing to report and answer zero; there is nothing to derive from,
    // so stay out of the way.
    let mm = CGDisplayScreenSize(monitor.native_id());
    let monitor_scale = monitor.scale_factor();
    if mm.width <= 0.0 || mm.height <= 0.0 || monitor_scale <= 0.0 {
        return 1.0;
    }
    // winit reports monitor sizes in physical pixels; the *points* the desktop
    // is laid out in — which is what a logical pixel currently costs — are those
    // with the monitor's own factor divided back out.
    let physical = monitor.size();
    let points = (
        physical.width as f64 / monitor_scale,
        physical.height as f64 / monitor_scale,
    );
    let ratio = density_ratio(points, (mm.width as f64, mm.height as f64));
    // Snapped against the *window's* factor, since that is the number the
    // runtime will multiply this correction by. It is the same screen's backing
    // factor, so the two agree — taking it from the window is what guarantees
    // the product lands on the step rather than near it.
    snap_effective(win.scale_factor(), ratio)
}

#[cfg(not(target_os = "macos"))]
pub(crate) fn for_window(_win: &Window) -> f32 {
    1.0
}

/// Logical pixels per inch of glass, over the 96 they are drawn for.
///
/// Both dimensions go in, as a geometric mean, rather than picking one axis:
/// that is what winit's X11 `calc_dpi_factor` does with the same two numbers,
/// and it keeps a display whose reported millimetres disagree slightly with its
/// aspect ratio from skewing the answer.
#[allow(dead_code)] // Only reachable from the macOS `for_window`.
fn density_ratio((points_w, points_h): (f64, f64), (mm_w, mm_h): (f64, f64)) -> f64 {
    let inches_w = mm_w / 25.4;
    let inches_h = mm_h / 25.4;
    let ppi = ((points_w * points_h) / (inches_w * inches_h)).sqrt();
    ppi / 96.0
}

/// The correction that puts the *effective* scale — `backing` times the result
/// — on a quarter step, and never below `backing` itself.
///
/// Quarters, rather than the twelfths winit rounds X11's measured DPI to,
/// because what has to look right is the product and not the correction. At a
/// quarter step a logical pixel is worth 2.25 or 2.5 physical ones, so an edge
/// lands on an exact physical pixel every four logical ones; the nearest
/// twelfth-derived scale, 13/6, only manages every six. The coarser ladder also
/// keeps a display that measures near a step boundary from flip-flopping
/// between two sizes: `CGDisplayScreenSize` re-derives its millimetres from the
/// current display mode, and the 27" 4K this was written on sits a
/// ten-thousandth off a boundary on the twelfth ladder while landing dead
/// centre of a quarter.
///
/// Never shrinking follows winit's X11 code, which floors its own factor at
/// 1.0: a display less dense than 96 dpi is a big television being read from
/// the sofa, where the chrome is already as small as it should get.
#[allow(dead_code)] // Only reachable from the macOS `for_window`.
fn snap_effective(backing: f64, ratio: f64) -> f32 {
    if !backing.is_finite() || !ratio.is_finite() || backing <= 0.0 {
        return 1.0;
    }
    let snapped = (backing * ratio * 4.0).round() / 4.0;
    ((snapped / backing).clamp(1.0, MAX_SCALE)) as f32
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A Retina Mac: the only backing factor these corrections ever multiply on
    /// macOS, aside from 1.0 on the last non-Retina machines.
    const RETINA: f64 = 2.0;

    /// The two displays this was written on, as Core Graphics reports them: a
    /// 27" 4K in its default HiDPI mode, and a 14" MacBook Pro panel.
    ///
    /// The points are what the desktop is laid out in, which is all this
    /// calculation cares about — the 4K's 3840x2160 of glass never enters into
    /// it, and neither does the 5120x2880 framebuffer macOS renders this mode
    /// into before scaling it down to the panel. The millimetres are what
    /// `CGDisplayScreenSize` answers *in that mode*: it re-derives them from the
    /// mode's aspect, so the same panel in another scaled resolution reports
    /// slightly different ones.
    const LG_4K: ((f64, f64), (f64, f64)) = ((2560.0, 1440.0), (599.2995, 340.2419));
    const MACBOOK_14: ((f64, f64), (f64, f64)) = ((1512.0, 982.0), (301.2141, 195.6298));

    /// The effective scale a display ends up rendering at.
    fn effective(backing: f64, display: ((f64, f64), (f64, f64))) -> f64 {
        backing * snap_effective(backing, density_ratio(display.0, display.1)) as f64
    }

    #[test]
    fn a_96_dpi_display_needs_no_correction() {
        // 1920x1080 across a 20" x 11.25" panel: exactly 96 dpi.
        let display = ((1920.0, 1080.0), (508.0, 285.75));
        assert!((density_ratio(display.0, display.1) - 1.0).abs() < 0.01);
        assert_eq!(
            snap_effective(RETINA, density_ratio(display.0, display.1)),
            1.0
        );
        assert_eq!(effective(RETINA, display), 2.0);
    }

    #[test]
    fn a_27_inch_4k_is_an_eighth_short_and_lands_on_a_clean_quarter() {
        let ratio = density_ratio(LG_4K.0, LG_4K.1);
        assert!((ratio - 1.125).abs() < 0.001, "{ratio}");
        assert_eq!(effective(RETINA, LG_4K), 2.25);
        assert_eq!(snap_effective(RETINA, ratio), 1.125);
    }

    /// An eighth over is 2.25 effective — dead centre of a quarter step, a whole
    /// eighth from the boundaries either side. The same panel sits a
    /// ten-thousandth off a boundary when the correction itself is rounded to a
    /// twelfth instead, where a shift in its reported millimetres would resize
    /// the whole UI by 7%. Snapping the product is what buys that margin.
    #[test]
    fn the_4k_is_nowhere_near_a_quarter_boundary() {
        let raw = RETINA * density_ratio(LG_4K.0, LG_4K.1);
        let distance_to_step = (raw * 4.0 - (raw * 4.0).round()).abs() / 4.0;
        assert!(distance_to_step < 0.001, "{distance_to_step}");
    }

    #[test]
    fn a_14_inch_macbook_rounds_up_to_two_and_three_quarters() {
        let ratio = density_ratio(MACBOOK_14.0, MACBOOK_14.1);
        assert!((ratio - 1.328).abs() < 0.001, "{ratio}");
        // 2.65625 raw, a hair past the 2.625 boundary.
        assert_eq!(effective(RETINA, MACBOOK_14), 2.75);
        assert_eq!(snap_effective(RETINA, ratio), 1.375);
    }

    #[test]
    fn a_non_retina_mac_gets_a_fractional_scale_of_its_own() {
        // Backing 1.0 on the MacBook's density would ask for 1.328 and snap to
        // 1.25 — inside the range the painter's crisp-chrome pass covers.
        assert_eq!(effective(1.0, MACBOOK_14), 1.25);
    }

    #[test]
    fn a_sparse_display_is_left_alone_rather_than_shrunk() {
        // A 55" television at 1080p — 40 dpi, and no business being scaled down.
        let tv = ((1920.0, 1080.0), (1210.0, 680.0));
        assert_eq!(snap_effective(RETINA, density_ratio(tv.0, tv.1)), 1.0);
        assert_eq!(effective(1.0, tv), 1.0);
    }

    #[test]
    fn nonsense_measurements_do_not_produce_a_nonsense_scale() {
        // A zero-millimetre display divides to infinity rather than erroring.
        assert_eq!(snap_effective(RETINA, f64::NAN), 1.0);
        assert_eq!(snap_effective(RETINA, f64::INFINITY), 1.0);
        assert_eq!(snap_effective(0.0, 1.5), 1.0);
        // Millimetres mistaken for centimetres would ask for an 11x UI.
        let wrong_unit = density_ratio((2560.0, 1440.0), (60.0, 34.0));
        assert_eq!(snap_effective(RETINA, wrong_unit), MAX_SCALE as f32);
    }
}
