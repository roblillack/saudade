//! Word wrapping: breaking text into the lines it takes to set it in a column
//! of a given width.

/// Break `text` into lines no wider than `max_width`, given the advance of each
/// character in the same units — logical pixels, for the painter.
///
/// Every `\n` (or `\r\n`) starts a new line, and a blank line stays a blank
/// line. Within a paragraph lines are filled greedily, breaking where a line
/// break is allowed:
///
/// - after a run of spaces — which is dropped at the break, so the next line
///   doesn't start with it, and never counts towards the width of the line it
///   ends. Spaces inside a line, and the indentation a paragraph starts with,
///   are kept as they are. A no-break space (U+00A0, U+2007, U+202F) doesn't
///   allow a break.
/// - after a hyphen between two words ("well-|known", but not "10-20"), after a
///   real hyphen, en dash or em dash (U+2010, U+2013, U+2014), and after a
///   zero-width space (U+200B), which is how to mark where a long identifier
///   may break without showing anything.
///
/// A word too wide for a line on its own is broken wherever it has to be,
/// preferably after a `/` or `\` (so a path breaks between its components),
/// and otherwise at the last character that still fits. Each line holds at
/// least one character, so a column narrower than a single character gets one
/// per line.
///
/// The lines borrow from `text`, without the spaces that ended them. A
/// non-positive `max_width` doesn't wrap at all: the paragraphs are returned as
/// they are.
pub(crate) fn wrap(text: &str, max_width: i32, advance: impl Fn(char) -> f32) -> Vec<&str> {
    let mut lines = Vec::new();
    for paragraph in text.split('\n') {
        let paragraph = paragraph.strip_suffix('\r').unwrap_or(paragraph);
        if max_width > 0 {
            wrap_paragraph(paragraph, max_width as f32, &advance, &mut lines);
        } else {
            lines.push(paragraph.trim_end_matches(is_space));
        }
    }
    lines
}

/// Fill the lines of one paragraph, which holds no `\n`, into `out`.
///
/// A line's width is summed from its first character in the order the
/// characters come, the way [`Font::measure`](crate::Font::measure) sums it, so
/// a line found to fit here measures as fitting there too.
fn wrap_paragraph<'t>(
    paragraph: &'t str,
    max: f32,
    advance: &impl Fn(char) -> f32,
    out: &mut Vec<&'t str>,
) {
    // The line being filled: where it starts, how wide it is up to the current
    // character, and whether it has anything but spaces in it yet.
    let mut start = 0;
    let mut width = 0.0;
    let mut ink = false;
    // Where the line can end if the next character doesn't fit: the start of
    // the next line, at the last place a break is allowed, and the last place
    // an overlong word may be broken. Either is only of use while it is past
    // `start`, i.e. inside the line being filled.
    let mut soft = 0;
    let mut hard = 0;
    let mut before = None;
    let mut prev = None;

    for (i, ch) in paragraph.char_indices() {
        if let Some(prev) = prev
            && !is_space(ch)
        {
            if is_space(prev) {
                // Breaking after the indentation of a line that has nothing
                // else in it would only leave an empty line behind.
                if ink {
                    soft = i;
                }
            } else if breaks_after(before, prev, ch) {
                soft = i;
            } else if matches!(prev, '/' | '\\') {
                hard = i;
            }
        }

        let w = advance(ch);
        // Spaces hang past the end of a line, so only something visible can
        // overflow it. Break until it fits: once at the last break allowed,
        // then again inside the word that leaves if that is still too wide.
        if !is_space(ch) {
            while i > start && width + w > max {
                let at = if soft > start {
                    soft
                } else if hard > start {
                    hard
                } else {
                    i
                };
                out.push(paragraph[start..at].trim_end_matches(is_space));
                start = at;
                width = paragraph[start..i].chars().map(advance).sum();
            }
            ink = true;
        }
        width += w;
        before = prev;
        prev = Some(ch);
    }
    out.push(paragraph[start..].trim_end_matches(is_space));
}

/// Whether `ch` is a space a line may break after — any whitespace but the
/// no-break ones.
fn is_space(ch: char) -> bool {
    ch.is_whitespace() && !matches!(ch, '\u{A0}' | '\u{2007}' | '\u{202F}')
}

/// Whether a line may break between `prev` and `ch` (which is no space), with
/// `before` the character ahead of `prev`.
fn breaks_after(before: Option<char>, prev: char, ch: char) -> bool {
    match prev {
        // A hyphen-minus is a hyphen only between two words: not in "10-20",
        // a negative number, or an option like "--force".
        '-' => before.is_some_and(char::is_alphanumeric) && ch.is_alphabetic(),
        '\u{2010}' | '\u{2013}' | '\u{2014}' | '\u{200B}' => true,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Wrap in a font whose every character is one unit wide, so `max_width`
    /// is a count of characters.
    fn mono(text: &str, max_width: i32) -> Vec<&str> {
        wrap(text, max_width, |_| 1.0)
    }

    #[test]
    fn a_line_that_fits_is_left_alone() {
        assert_eq!(mono("The quick brown fox", 40), ["The quick brown fox"]);
    }

    #[test]
    fn lines_fill_up_to_the_last_word_that_fits() {
        assert_eq!(
            mono("The quick brown fox jumps over the lazy dog", 15),
            ["The quick brown", "fox jumps over", "the lazy dog"],
        );
    }

    #[test]
    fn newlines_start_a_new_line_and_blank_lines_stay() {
        assert_eq!(mono("one\n\ntwo\r\nthree", 40), ["one", "", "two", "three"]);
        assert_eq!(mono("trailing\n", 40), ["trailing", ""]);
    }

    #[test]
    fn spaces_inside_a_line_are_kept() {
        assert_eq!(mono("  indented,  twice", 40), ["  indented,  twice"]);
    }

    #[test]
    fn spaces_at_a_break_are_dropped() {
        assert_eq!(mono("aaa     bbb", 4), ["aaa", "bbb"]);
    }

    #[test]
    fn trailing_spaces_hang_past_the_end_of_the_line() {
        assert_eq!(mono("abcd      ", 4), ["abcd"]);
        assert_eq!(mono("abcd      efgh", 4), ["abcd", "efgh"]);
    }

    #[test]
    fn a_word_too_wide_for_a_line_is_broken() {
        assert_eq!(mono("abcdefghij", 4), ["abcd", "efgh", "ij"]);
    }

    #[test]
    fn a_word_too_wide_for_a_line_starts_on_a_line_of_its_own() {
        assert_eq!(mono("a abcdefgh", 4), ["a", "abcd", "efgh"]);
    }

    #[test]
    fn indentation_doesnt_leave_an_empty_line_behind() {
        assert_eq!(mono("  abcdef", 4), ["  ab", "cdef"]);
    }

    #[test]
    fn a_line_may_break_after_a_hyphen_between_words() {
        assert_eq!(mono("a well-known fact", 8), ["a well-", "known", "fact"]);
    }

    #[test]
    fn a_line_never_breaks_inside_a_number_or_an_option() {
        assert_eq!(mono("pages 10-20", 8), ["pages", "10-20"]);
        assert_eq!(mono("use --force", 8), ["use", "--force"]);
    }

    #[test]
    fn a_line_may_break_after_a_dash_or_a_zero_width_space() {
        assert_eq!(mono("yes\u{2014}no", 4), ["yes\u{2014}", "no"]);
        assert_eq!(mono("very\u{200B}long", 6), ["very\u{200B}", "long"]);
    }

    #[test]
    fn a_no_break_space_holds_its_words_together() {
        assert_eq!(mono("a b\u{A0}c", 4), ["a", "b\u{A0}c"]);
    }

    #[test]
    fn a_path_too_wide_for_a_line_breaks_between_its_components() {
        assert_eq!(
            mono("/usr/local/share/fonts", 12),
            ["/usr/local/", "share/fonts"],
        );
        assert_eq!(
            mono(r"C:\Users\rob\Documents", 13),
            [r"C:\Users\rob\", "Documents"]
        );
    }

    #[test]
    fn a_slash_doesnt_break_a_word_that_fits_on_a_line_of_its_own() {
        assert_eq!(mono("this and/or that", 8), ["this", "and/or", "that"]);
    }

    #[test]
    fn a_column_narrower_than_a_character_gets_one_per_line() {
        assert_eq!(mono("abc", 1), ["a", "b", "c"]);
        assert_eq!(wrap("ab", 1, |_| 3.0), ["a", "b"]);
    }

    #[test]
    fn no_width_means_no_wrapping() {
        assert_eq!(mono("one two\nthree  ", 0), ["one two", "three"]);
    }

    #[test]
    fn every_line_fits() {
        // Proportional advances, so lines come out of different lengths.
        let advance = |ch: char| match ch {
            'i' | 'l' | '.' | ',' | ' ' => 2.5,
            'm' | 'w' => 9.0,
            _ => 6.0,
        };
        let text = "Lorem ipsum dolor sit amet, consectetur adipiscing elit, sed \
                    do eiusmod tempor incididunt ut labore et dolore magna aliqua.";
        // The widest word, "consectetur", is 66 wide: narrower columns break
        // words too.
        for max in [20, 40, 57, 66, 80, 123, 200] {
            let lines = wrap(text, max, advance);
            for line in &lines {
                let w: f32 = line.chars().map(advance).sum();
                assert!(w <= max as f32, "{line:?} is {w} wide, over {max}");
            }
            // Nothing is lost but spaces — where every word fits on a line,
            // just the ones the lines broke at.
            if max >= 66 {
                assert_eq!(lines.join(" "), text, "at {max}");
            } else {
                let ink = |s: &str| s.chars().filter(|ch| !is_space(*ch)).collect::<String>();
                assert_eq!(ink(&lines.concat()), ink(text), "at {max}");
            }
        }
    }
}
