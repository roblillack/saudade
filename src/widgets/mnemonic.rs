//! Mnemonic parsing and rendering shared by the widgets that advertise an
//! `&`-marked accelerator letter — [`MenuBar`](crate::widgets::MenuBar) and
//! [`FocusLabel`](crate::widgets::FocusLabel).
//!
//! A label may embed a single mnemonic by prefixing a character with `&`:
//! `"Last &name:"` displays *Last name:* with the **n** underlined and binds
//! that letter as the widget's accelerator. A literal ampersand is written
//! `&&`.

use crate::geometry::{Color, Rect};
use crate::painter::Painter;

/// Parsed view of a label like `"&File"`: the visible text with the `&`
/// stripped, and the (logical) character index of the mnemonic glyph plus the
/// lowercased mnemonic letter itself.
#[derive(Clone)]
pub(crate) struct ParsedLabel {
    pub display: String,
    pub mnemonic_index: Option<usize>,
    pub mnemonic_char: Option<char>,
}

/// Split a raw label into its visible text and mnemonic. `&` marks the next
/// character as the mnemonic; `&&` is an escaped literal ampersand. Only the
/// first unescaped `&` declares a mnemonic — any later ones are treated as
/// ordinary text.
pub(crate) fn parse_label(raw: &str) -> ParsedLabel {
    let mut display = String::with_capacity(raw.len());
    let mut mnemonic_index = None;
    let mut mnemonic_char = None;
    let mut chars = raw.chars().peekable();
    let mut idx = 0;
    while let Some(c) = chars.next() {
        if c == '&' {
            if chars.peek() == Some(&'&') {
                chars.next();
                display.push('&');
                idx += 1;
            } else if let Some(&next) = chars.peek()
                && mnemonic_char.is_none()
            {
                mnemonic_index = Some(idx);
                mnemonic_char = Some(next.to_ascii_lowercase());
                // do not push the '&' itself; the next loop iteration pushes
                // the actual character at idx.
            }
        } else {
            display.push(c);
            idx += 1;
        }
    }
    ParsedLabel {
        display,
        mnemonic_index,
        mnemonic_char,
    }
}

/// Draw text with the mnemonic glyph underlined. `dy_phys` lets the caller
/// nudge both the text and the underline by a physical-pixel amount
/// independent of any logical-pixel inset (the menu bar uses this to drop its
/// labels exactly one physical pixel without growing the bar by a whole
/// logical pixel); pass `0` when no such nudge is needed.
pub(crate) fn draw_label_with_mnemonic(
    painter: &mut Painter,
    x: i32,
    y: i32,
    dy_phys: i32,
    parsed: &ParsedLabel,
    size: f32,
    color: Color,
) {
    painter.text_with_phys_offset(x, y, 0, dy_phys, &parsed.display, size, color);
    if let Some(idx) = parsed.mnemonic_index {
        let prefix: String = parsed.display.chars().take(idx).collect();
        let mnemonic_ch: String = parsed.display.chars().skip(idx).take(1).collect();
        if mnemonic_ch.is_empty() {
            return;
        }
        let prefix_w = painter.measure_text(&prefix, size).w;
        let glyph_w = painter.measure_text(&mnemonic_ch, size).w;
        // Drop the underline 1 logical pixel below the baseline so it doesn't
        // kiss the bottom of the letter (and doesn't fight any descender on
        // the rare lowercase mnemonic).
        let underline_y = y + (size as i32) + 1;
        painter.fill_rect_with_phys_offset(
            Rect::new(x + prefix_w, underline_y, glyph_w, 1),
            0,
            dy_phys,
            color,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_strips_marker_and_records_letter() {
        let p = parse_label("Last &name:");
        assert_eq!(p.display, "Last name:");
        assert_eq!(p.mnemonic_index, Some(5));
        assert_eq!(p.mnemonic_char, Some('n'));
    }

    #[test]
    fn escaped_ampersand_is_literal() {
        let p = parse_label("Tom && Jerry");
        assert_eq!(p.display, "Tom & Jerry");
        assert_eq!(p.mnemonic_char, None);
    }

    #[test]
    fn only_the_first_marker_declares_a_mnemonic() {
        let p = parse_label("&One &Two");
        assert_eq!(p.display, "One Two");
        assert_eq!(p.mnemonic_char, Some('o'));
    }
}
