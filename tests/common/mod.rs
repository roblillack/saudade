//! Helpers shared by every snapshot test.
//!
//! All tests render against the bundled DejaVu fonts so glyph rasterization
//! is stable regardless of which fonts happen to be installed on the host.
//! Tests run at four scales (1.0x, 1.25x, 1.5x, 2.0x) so we catch regressions
//! in fractional-DPI snapping as well as integer-DPI layout.
//!
//! Each rendered frame is compared to a checked-in baseline PNG. In practice
//! fontdue rasterizes identically across the dev and CI machines — measured
//! across the whole suite, the macOS/aarch64 and Linux/x86_64 renders differ
//! by at most a single channel level on a single pixel. So rather than a
//! pixel-count budget we use a per-pixel *amount* tolerance ([`MAX_CHANNEL_DELTA`]):
//! a channel may drift by a few levels, but if even one pixel exceeds that, the
//! snapshot is a regression. The threshold leaves generous headroom over the
//! observed drift while still catching real rendering changes, which move
//! pixels far past it.
//!
//! After an intentional rendering change, regenerate the baselines with:
//!
//! ```sh
//! UPDATE_SNAPSHOTS=1 cargo test -p retrogui
//! ```

use std::path::{Path, PathBuf};

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

/// Per-pixel amount of difference we tolerate: a channel may be off by up to
/// this many levels (0–255) without counting as a change. Measured cross-machine
/// drift is at most 1 level, so 16 is comfortable headroom (covering the dev
/// arch, the Linux/Windows CI runners, and minor future toolchain drift) while
/// staying far below any real rendering change. A single pixel exceeding this
/// fails the snapshot — there is no count budget.
const MAX_CHANNEL_DELTA: u8 = 16;

/// Render `build()` at each scale in [`SCALES`] and compare each frame to its
/// checked-in baseline PNG. `name` is the snapshot's base name (typically the
/// caller's test function name); each scale appends its own suffix, so the
/// on-disk artifacts look like `<name>_1_00.snap.png`.
pub fn snapshot_at_all_scales<F>(name: &str, width: i32, height: i32, mut build: F)
where
    F: FnMut() -> Box<dyn Widget>,
{
    for &scale in SCALES {
        snapshot_one(name, width, height, scale, build());
    }
}

fn snapshot_one(name: &str, width: i32, height: i32, scale: f32, mut widget: Box<dyn Widget>) {
    let backend = MockBackend::new(width, height)
        .with_scale(scale)
        .with_font(sans_font())
        .with_mono_font(mono_font());
    let snap = backend.render(widget.as_mut());

    let path = snapshot_path(&format!("{}_{}.snap.png", name, scale_tag(scale)));

    // Regeneration mode: overwrite the baseline with the freshly rendered
    // frame. Use after an intentional rendering change.
    if std::env::var_os("UPDATE_SNAPSHOTS").is_some() {
        std::fs::write(&path, snap.to_png())
            .unwrap_or_else(|e| panic!("failed to write baseline {}: {e}", path.display()));
        return;
    }

    let baseline = std::fs::read(&path).unwrap_or_else(|_| {
        panic!(
            "missing baseline {} — create it with `UPDATE_SNAPSHOTS=1 cargo test -p retrogui`",
            path.display()
        )
    });
    let (bw, bh, base) = decode_rgba(&baseline, &path);

    if bw as i32 != snap.width() || bh as i32 != snap.height() {
        let actual = write_actual_render(&snap, name, scale);
        panic!(
            "snapshot `{name}` @ {scale}x: size changed (baseline {bw}x{bh}, rendered {}x{}). \
             Rendered frame written to {}. Run `UPDATE_SNAPSHOTS=1 cargo test -p retrogui` if \
             this is intended.",
            snap.width(),
            snap.height(),
            actual.display(),
        );
    }

    // Compare the rendered ARGB32 framebuffer against the decoded RGBA
    // baseline. For each pixel we take the largest per-channel delta; the
    // snapshot is a regression as soon as one pixel drifts past
    // MAX_CHANNEL_DELTA. (`offenders` keeps counting so the message reports the
    // full extent of a real change, not just the first pixel.)
    let mut offenders = 0usize;
    let mut max_delta = 0u32;
    let mut first_offender = None;
    for (i, &px) in snap.pixels().iter().enumerate() {
        let actual = [
            ((px >> 16) & 0xFF) as u8, // r
            ((px >> 8) & 0xFF) as u8,  // g
            (px & 0xFF) as u8,         // b
            ((px >> 24) & 0xFF) as u8, // a
        ];
        let expected = &base[i * 4..i * 4 + 4];
        let mut pixel_delta = 0u32;
        for c in 0..4 {
            pixel_delta = pixel_delta.max(actual[c].abs_diff(expected[c]) as u32);
        }
        max_delta = max_delta.max(pixel_delta);
        if pixel_delta > MAX_CHANNEL_DELTA as u32 {
            offenders += 1;
            first_offender.get_or_insert_with(|| {
                let x = i as i32 % snap.width();
                let y = i as i32 / snap.width();
                (x, y, pixel_delta)
            });
        }
    }

    if offenders > 0 {
        let actual = write_actual_render(&snap, name, scale);
        panic!(
            "snapshot `{name}` @ {scale}x differs from baseline: {offenders} pixel(s) off by more \
             than {MAX_CHANNEL_DELTA}/255 (largest channel delta {max_delta}, first at \
             {first_offender:?}). Rendered frame written to {} (uploaded as a CI artifact on \
             failure). If this is an intended rendering change, regenerate with \
             `UPDATE_SNAPSHOTS=1 cargo test -p retrogui`; otherwise it is a regression. \
             Baseline: {}",
            actual.display(),
            path.display(),
        );
    }
}

/// On a mismatch, dump the freshly rendered frame next to its baseline as
/// `<name>_<scale>.snap.actual.png` so the failure can be eyeballed locally and
/// uploaded as a CI artifact (these files are git-ignored). Best-effort: a
/// write error must not mask the assertion failure that triggered it.
fn write_actual_render(snap: &retrogui::mock::Snapshot, name: &str, scale: f32) -> PathBuf {
    let path = snapshot_path(&format!("{}_{}.snap.actual.png", name, scale_tag(scale)));
    let _ = std::fs::write(&path, snap.to_png());
    path
}

/// Absolute path to a file in the checked-in snapshot directory.
fn snapshot_path(file: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/snapshots")
        .join(file)
}

/// Decode a baseline PNG into `(width, height, rgba8 bytes)`. Baselines are
/// written by [`retrogui::mock::Snapshot::to_png`], which always emits 8-bit
/// RGBA, so we assert that format rather than handle every PNG variant.
fn decode_rgba(bytes: &[u8], path: &Path) -> (u32, u32, Vec<u8>) {
    let decoder = png::Decoder::new(bytes);
    let mut reader = decoder
        .read_info()
        .unwrap_or_else(|e| panic!("failed to read baseline {}: {e}", path.display()));
    let mut buf = vec![0u8; reader.output_buffer_size()];
    let info = reader
        .next_frame(&mut buf)
        .unwrap_or_else(|e| panic!("failed to decode baseline {}: {e}", path.display()));
    assert!(
        info.color_type == png::ColorType::Rgba && info.bit_depth == png::BitDepth::Eight,
        "baseline {} is not 8-bit RGBA (got {:?} / {:?})",
        path.display(),
        info.color_type,
        info.bit_depth,
    );
    buf.truncate(info.buffer_size());
    (info.width, info.height, buf)
}

fn scale_tag(scale: f32) -> String {
    // 1.25 → "1_25", 2.0 → "2_00" — filesystem-safe and sortable.
    let scaled = (scale * 100.0).round() as i32;
    format!("{}_{:02}", scaled / 100, scaled % 100)
}
