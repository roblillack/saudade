//! What a widget measures has to be what the painter draws, on every display.
//!
//! Both halves matter and they are easy to get separately right and jointly
//! wrong: a platform text stack may hand back different metrics for the same
//! type at a different pixel size (macOS re-picks San Francisco's optical
//! variant), which measures fine at 1x and drifts on a Retina screen.

use saudade::{Color, Font, FontFamily, FontSet, FontStyle, Painter, Rect};

const TEXT: &str = "The quick brown fox jumps over the lazy dog, twice over.";
const SIZE: f32 = 13.0;
const W: i32 = 700;
const H: i32 = 40;

/// Measure and then draw `TEXT` at `scale`, returning the measured logical width
/// and the physical x of the right-most inked pixel.
fn measure_and_draw(font: &Font, scale: f32) -> (i32, i32) {
    let pw = (W as f32 * scale) as i32;
    let ph = (H as f32 * scale) as i32;
    let mut pixels = vec![Color::WHITE.0; (pw * ph) as usize];
    let measured = {
        let mut painter = Painter::new(
            &mut pixels,
            pw,
            ph,
            scale,
            0,
            0,
            FontSet {
                sans: Some(font),
                serif: None,
                mono: None,
            },
        );
        painter.fill_rect(Rect::new(0, 0, W, H), Color::WHITE);
        let measured = painter
            .measure_text_styled(TEXT, SIZE, FontFamily::Sans, FontStyle::Regular)
            .w;
        painter.text_styled(
            0,
            5,
            TEXT,
            SIZE,
            Color::BLACK,
            FontFamily::Sans,
            FontStyle::Regular,
        );
        measured
    };
    let mut right = 0;
    for y in 0..ph {
        for x in 0..pw {
            if pixels[(y * pw + x) as usize] != Color::WHITE.0 && x > right {
                right = x;
            }
        }
    }
    (measured, right)
}

#[test]
fn a_measured_string_is_the_width_it_draws_at_every_dpi() {
    let Some(font) = Font::load_sans() else {
        return; // A host with no fonts draws nothing to measure.
    };
    let mut widths = Vec::new();
    for scale in [1.0f32, 1.25, 1.5, 2.0] {
        let (measured, drawn_right) = measure_and_draw(&font, scale);
        assert!(
            measured > 0 && drawn_right > 0,
            "nothing was drawn at {scale}x"
        );
        // The ink stops a hair short of the pen, by the last glyph's right side
        // bearing — a few physical pixels, not a percentage of the line.
        let expected = measured as f32 * scale;
        let slack = expected - drawn_right as f32;
        assert!(
            (0.0..=6.0).contains(&slack),
            "at {scale}x the text measures {measured} logical px ({expected} physical) \
             but its ink ends at {drawn_right}: off by {slack}"
        );
        widths.push((scale, measured));
    }

    // And the same type measures the same at every DPI: a layout must not
    // reflow because the window moved to a sharper screen.
    let (_, first) = widths[0];
    for (scale, measured) in &widths {
        assert_eq!(
            *measured, first,
            "{scale}x measured {measured} where 1x measured {first}"
        );
    }
}
