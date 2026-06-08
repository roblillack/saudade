use crate::event::{Event, EventCtx, Key, Modifiers};
use crate::geometry::{Color, Rect};
use crate::painter::Painter;
use crate::theme::Theme;
use crate::widget::Widget;
use crate::widgets::mnemonic::{ParsedLabel, draw_label_with_mnemonic, parse_label};

/// A single-line caption that carries a keyboard *mnemonic* and hands focus to
/// the field beside it when that mnemonic is pressed.
///
/// Mark the mnemonic with `&`, exactly like a [`MenuBar`](crate::widgets::MenuBar)
/// label: `FocusLabel::new(rect, "Last &name:")` renders *Last name:* with the
/// **n** underlined and listens for **Alt+N**. When that combination is
/// pressed anywhere in the surrounding container, focus jumps to the *next
/// focusable widget added to the same parent* — the classic "buddy label"
/// convention. Place the label immediately before the field it describes:
///
/// ```no_run
/// use saudade::*;
///
/// let form = Container::new(220, 60)
///     // Alt+N focuses the field that follows…
///     .add(FocusLabel::new(Rect::new(8, 8, 80, 20), "Last &name:"))
///     .add(TextInput::new(Rect::new(92, 6, 120, 22)));
/// # let _ = form;
/// ```
///
/// A `FocusLabel` is not itself focusable and never takes focus; it only
/// redirects it. Without a `&` marker it behaves like a plain, static
/// [`Label`](crate::widgets::Label) (minus word wrapping).
pub struct FocusLabel {
    pub rect: Rect,
    parsed: ParsedLabel,
    size: Option<f32>,
    color: Option<Color>,
    background: Option<Color>,
}

impl FocusLabel {
    /// Create a label from a rect and a `&`-marked caption. The character after
    /// the first unescaped `&` is shown underlined and bound as the mnemonic;
    /// `&&` prints a literal ampersand.
    pub fn new(rect: Rect, text: impl AsRef<str>) -> Self {
        Self {
            rect,
            parsed: parse_label(text.as_ref()),
            size: None,
            color: None,
            background: None,
        }
    }

    pub fn with_color(mut self, color: Color) -> Self {
        self.color = Some(color);
        self
    }

    pub fn with_size(mut self, size: f32) -> Self {
        self.size = Some(size);
        self
    }

    /// Paint a solid fill across the label's rectangle before the text — handy
    /// to keep it legible over a window background pattern. See
    /// [`Label::with_background`](crate::widgets::Label::with_background).
    pub fn with_background(mut self, color: Color) -> Self {
        self.background = Some(color);
        self
    }

    /// The mnemonic letter this label listens for, if any (ASCII-lowercased).
    pub fn mnemonic(&self) -> Option<char> {
        self.parsed.mnemonic_char
    }

    /// `true` if `ch`+`modifiers` is this label's accelerator: the mnemonic
    /// letter with the (left) Alt held. Alt is required so a plain letter still
    /// reaches a focused text field; AltGr is excluded so composed characters
    /// pass through untouched.
    fn is_accelerator(&self, ch: char, modifiers: Modifiers) -> bool {
        modifiers.mnemonic_alt() && self.parsed.mnemonic_char == Some(ch.to_ascii_lowercase())
    }

    /// Ask the parent container to focus the next field, and swallow the
    /// triggering keystroke so it neither acts twice nor leaks into the field
    /// that just gained focus.
    fn fire(&self, ctx: &mut EventCtx) {
        ctx.request_focus_next();
        // Don't let the Alt+letter that moved focus also reach the newly
        // focused widget (its trailing `Char` / release), mirroring how a menu
        // item fired by mnemonic swallows its keystroke.
        ctx.swallow_key_until_release();
        ctx.consume_event();
        ctx.request_paint();
    }
}

impl Widget for FocusLabel {
    fn bounds(&self) -> Rect {
        self.rect
    }

    fn layout(&mut self, bounds: Rect) {
        self.rect = bounds;
    }

    fn paint(&mut self, painter: &mut Painter, theme: &Theme) {
        if let Some(bg) = self.background {
            painter.fill_rect(self.rect, bg);
        }
        let size = self.size.unwrap_or(theme.font_size);
        let color = self.color.unwrap_or(theme.text);
        // Center the single line vertically in the slot so the caption lines up
        // with the field beside it; clip so an over-long caption never spills
        // past its rectangle.
        let line_height = painter.measure_text("", size).h.max(1);
        let y = self.rect.y + ((self.rect.h - line_height) / 2).max(0);
        let saved = painter.push_clip(self.rect);
        draw_label_with_mnemonic(painter, self.rect.x, y, 0, &self.parsed, size, color);
        painter.restore_clip(saved);
    }

    fn event(&mut self, event: &Event, ctx: &mut EventCtx) {
        // No mnemonic → behave like a static label and ignore the keyboard.
        if self.parsed.mnemonic_char.is_none() {
            return;
        }
        match event {
            // Most backends deliver Alt+letter as a `KeyDown` carrying the
            // character; some route it through `Char` with Alt held. Accept
            // either, exactly as the menu bar does.
            Event::KeyDown {
                key: Key::Char(ch),
                modifiers,
            }
            | Event::Char { ch, modifiers }
                if self.is_accelerator(*ch, *modifiers) =>
            {
                self.fire(ctx);
            }
            _ => {}
        }
    }

    /// Listen for accelerators only when there's a mnemonic to match — so a
    /// marker-less caption doesn't draw keyboard traffic it will never use.
    fn accepts_accelerators(&self) -> bool {
        self.parsed.mnemonic_char.is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::event::Modifiers;

    fn alt() -> Modifiers {
        Modifiers {
            alt: true,
            ..Modifiers::default()
        }
    }

    fn altgr() -> Modifiers {
        Modifiers {
            alt: true,
            alt_graph: true,
            ..Modifiers::default()
        }
    }

    fn key(ch: char, modifiers: Modifiers) -> Event {
        Event::KeyDown {
            key: Key::Char(ch),
            modifiers,
        }
    }

    #[test]
    fn matched_mnemonic_requests_focus_next() {
        let mut label = FocusLabel::new(Rect::new(0, 0, 80, 20), "Last &name:");
        let mut ctx = EventCtx::new();
        label.event(&key('n', alt()), &mut ctx);
        assert!(ctx.is_focus_next_requested());
        assert!(ctx.is_consumed());
        assert!(ctx.swallow_key);
    }

    #[test]
    fn ignores_letter_without_alt() {
        let mut label = FocusLabel::new(Rect::new(0, 0, 80, 20), "&Name:");
        let mut ctx = EventCtx::new();
        label.event(&key('n', Modifiers::default()), &mut ctx);
        assert!(!ctx.is_focus_next_requested());
        assert!(!ctx.is_consumed());
    }

    #[test]
    fn ignores_altgr_so_composed_chars_pass_through() {
        let mut label = FocusLabel::new(Rect::new(0, 0, 80, 20), "&Name:");
        let mut ctx = EventCtx::new();
        label.event(&key('n', altgr()), &mut ctx);
        assert!(!ctx.is_focus_next_requested());
    }

    #[test]
    fn match_is_case_insensitive() {
        let mut label = FocusLabel::new(Rect::new(0, 0, 80, 20), "&Name:");
        let mut ctx = EventCtx::new();
        label.event(&key('N', alt()), &mut ctx);
        assert!(ctx.is_focus_next_requested());
    }

    #[test]
    fn label_without_mnemonic_is_inert() {
        let label = FocusLabel::new(Rect::new(0, 0, 80, 20), "Plain caption");
        assert_eq!(label.mnemonic(), None);
        assert!(!label.accepts_accelerators());
    }

    #[test]
    fn paint_path_does_not_panic() {
        // Exercise the clip + background + underlined-mnemonic paint path
        // through the offscreen backend; the mnemonic branch and the empty
        // case must both survive a font-less render.
        let backend = crate::mock::MockBackend::new(120, 24);
        let mut marked =
            FocusLabel::new(Rect::new(2, 2, 116, 20), "Last &name:").with_background(Color::WHITE);
        backend.render(&mut marked);
        let mut plain = FocusLabel::new(Rect::new(2, 2, 116, 20), "no marker");
        crate::mock::MockBackend::new(120, 24).render(&mut plain);
    }
}
