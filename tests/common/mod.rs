//! Helpers shared by every snapshot test.
//!
//! All tests render against the bundled DejaVu fonts so glyph rasterization
//! is bit-identical regardless of which fonts happen to be installed on
//! the host. Tests run at four scales (1.0x, 1.25x, 1.5x, 2.0x) so we
//! catch regressions in fractional-DPI snapping as well as integer-DPI
//! layout.

use retrogui::mock::MockBackend;
use retrogui::{Font, Widget};

pub fn sans_font() -> Font {
    Font::from_bytes(include_bytes!("../fonts/DejaVuSans.ttf").to_vec())
        .expect("bundled DejaVuSans.ttf failed to load")
}

pub fn mono_font() -> Font {
    Font::from_bytes(include_bytes!("../fonts/DejaVuSansMono.ttf").to_vec())
        .expect("bundled DejaVuSansMono.ttf failed to load")
}

/// The fractional and integer scales every widget should look correct at.
pub const SCALES: &[f32] = &[1.0, 1.25, 1.5, 2.0];

/// Render `build()` at each scale in [`SCALES`] and emit a binary insta
/// snapshot per scale. `name` is the snapshot's base name (typically the
/// caller's test function name); each scale appends its own suffix, so
/// the resulting on-disk artifacts look like `<name>_1_00.snap.png`.
///
/// We pass `name` explicitly because insta derives snapshot names from
/// the function that contains the `assert_binary_snapshot!` call — if we
/// called it from this helper, every test would share one name and
/// insta would disambiguate with a sequence number.
pub fn snapshot_at_all_scales<F>(name: &str, width: i32, height: i32, mut build: F)
where
    F: FnMut() -> Box<dyn Widget>,
{
    for &scale in SCALES {
        snapshot_one(name, width, height, scale, build());
    }
}

fn snapshot_one(
    name: &str,
    width: i32,
    height: i32,
    scale: f32,
    mut widget: Box<dyn Widget>,
) {
    let backend = MockBackend::new(width, height)
        .with_scale(scale)
        .with_font(sans_font())
        .with_mono_font(mono_font());
    let snap = backend.render(widget.as_mut());
    let snap_name = format!("{}_{}.png", name, scale_tag(scale));
    let mut settings = insta::Settings::clone_current();
    settings.set_prepend_module_to_snapshot(false);
    // Pin the snapshot path to a single dedicated directory so every
    // test's artifacts live next to each other regardless of which
    // helper function actually called insta.
    settings.set_snapshot_path("../snapshots");
    settings.bind(|| {
        insta::assert_binary_snapshot!(snap_name.as_str(), snap.to_png());
    });
}

fn scale_tag(scale: f32) -> String {
    // 1.25 → "1_25", 2.0 → "2_00" — filesystem-safe and sortable.
    let scaled = (scale * 100.0).round() as i32;
    format!("{}_{:02}", scaled / 100, scaled % 100)
}
