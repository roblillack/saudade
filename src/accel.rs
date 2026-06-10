//! Keyboard accelerators for menu items.
//!
//! An [`Accel`] names a chord in *platform-independent* terms: its modifiers
//! are roles ([`AccelMods`]) rather than physical keys, resolved through a
//! [`ModifierScheme`] both when the chord is matched against keyboard input
//! and when its label is rendered. An application declares
//! `Accel::primary('r')` once; on a Mac that means ⌘R (displayed "Cmd+R"),
//! everywhere else Ctrl+R (displayed "Ctrl+R").
//!
//! The role → key mapping is chosen so that translation between platforms is
//! *injective* — no two distinct chords on one platform collapse into the same
//! chord on another:
//!
//! | role        | macOS         | elsewhere       |
//! |-------------|---------------|-----------------|
//! | `primary`   | ⌘ Command     | Ctrl            |
//! | `secondary` | ⌃ Control     | Super (Win key) |
//! | `alt`       | ⌥ Option      | Alt             |
//! | `shift`     | Shift         | Shift           |
//!
//! `secondary` is best-effort outside macOS: the OS or compositor claims many
//! Super chords for itself (Win+X on Windows, window management on Wayland),
//! so reserve it for rare power chords that also carry `primary` — the way
//! macOS uses ⌃⌘.

use crate::event::{Key, Modifiers, NamedKey};

/// How the abstract modifier roles of an [`Accel`] map onto the physical
/// modifier keys of [`Modifiers`]. See the [module docs](self) for the table.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ModifierScheme {
    /// `primary` is the Command (logo) key, `secondary` the Control key.
    Mac,
    /// `primary` is the Control key, `secondary` the Super / Windows (logo)
    /// key. Used everywhere but macOS.
    Pc,
}

impl ModifierScheme {
    /// The scheme matching the platform this binary was built for.
    pub fn native() -> Self {
        if cfg!(target_os = "macos") {
            ModifierScheme::Mac
        } else {
            ModifierScheme::Pc
        }
    }
}

/// The platform-independent modifier roles of an accelerator chord. Resolved
/// to physical modifiers through a [`ModifierScheme`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct AccelMods {
    /// The platform's command modifier: ⌘ on macOS, Ctrl elsewhere.
    pub primary: bool,
    /// The platform's "other" system modifier: ⌃ on macOS, Super elsewhere.
    pub secondary: bool,
    /// The Alt / ⌥ Option key — the same key everywhere.
    pub alt: bool,
    pub shift: bool,
}

/// A keyboard accelerator: a chord of modifier *roles* plus a key.
///
/// Build one with [`Accel::primary`] and the chainable [`shift`](Self::shift) /
/// [`alt`](Self::alt) / [`secondary`](Self::secondary) builders, or parse the
/// conventional string form (`"Ctrl+Shift+R"`, `"Ctrl+Enter"`) via `From<&str>`
/// — in strings, `Ctrl` and `Cmd` both mean the `primary` role. Exotic
/// combinations without `primary` can be built from the public fields.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Accel {
    pub mods: AccelMods,
    pub key: Key,
}

impl Accel {
    /// A chord of the primary command modifier plus `key` — the everyday
    /// accelerator: `Accel::primary('r')`, `Accel::primary(NamedKey::Enter)`.
    pub fn primary(key: impl Into<Key>) -> Self {
        Self {
            mods: AccelMods {
                primary: true,
                ..AccelMods::default()
            },
            key: key.into(),
        }
    }

    /// Add Shift to the chord.
    pub fn shift(mut self) -> Self {
        self.mods.shift = true;
        self
    }

    /// Add Alt (⌥ Option) to the chord.
    pub fn alt(mut self) -> Self {
        self.mods.alt = true;
        self
    }

    /// Add the secondary system modifier (⌃ on macOS, Super elsewhere) to the
    /// chord. Best-effort outside macOS — see the [module docs](self).
    pub fn secondary(mut self) -> Self {
        self.mods.secondary = true;
        self
    }

    /// Whether the pressed `key` + `modifiers` are exactly this chord under
    /// `scheme`. Modifiers must match exactly (a `Ctrl+R` accel does not fire
    /// on Ctrl+Shift+R); character keys compare case-insensitively. AltGr
    /// chords never match — the right Alt composes characters (and Windows
    /// reports it as Ctrl+Alt), so those keystrokes are text entry, not
    /// commands.
    pub fn matches(&self, key: Key, modifiers: Modifiers, scheme: ModifierScheme) -> bool {
        if modifiers.alt_graph {
            return false;
        }
        let (control, logo) = match scheme {
            ModifierScheme::Mac => (self.mods.secondary, self.mods.primary),
            ModifierScheme::Pc => (self.mods.primary, self.mods.secondary),
        };
        if modifiers.control != control
            || modifiers.logo != logo
            || modifiers.alt != self.mods.alt
            || modifiers.shift != self.mods.shift
        {
            return false;
        }
        match (self.key, key) {
            (Key::Char(a), Key::Char(b)) => a.eq_ignore_ascii_case(&b),
            (a, b) => a == b,
        }
    }

    /// The chord's display label under `scheme`, e.g. `"Ctrl+Shift+R"` on PC
    /// and `"Shift+Cmd+R"` on a Mac. Modifier order follows each platform's
    /// convention: Ctrl, Super, Alt, Shift on PC; Ctrl, Opt, Shift, Cmd
    /// (Apple's ⌃⌥⇧⌘) on macOS.
    pub fn label(&self, scheme: ModifierScheme) -> String {
        let mut parts: Vec<&str> = Vec::new();
        match scheme {
            ModifierScheme::Pc => {
                if self.mods.primary {
                    parts.push("Ctrl");
                }
                if self.mods.secondary {
                    parts.push("Super");
                }
                if self.mods.alt {
                    parts.push("Alt");
                }
                if self.mods.shift {
                    parts.push("Shift");
                }
            }
            ModifierScheme::Mac => {
                if self.mods.secondary {
                    parts.push("Ctrl");
                }
                if self.mods.alt {
                    parts.push("Opt");
                }
                if self.mods.shift {
                    parts.push("Shift");
                }
                if self.mods.primary {
                    parts.push("Cmd");
                }
            }
        }
        let key = key_label(self.key);
        parts.push(&key);
        parts.join("+")
    }
}

fn key_label(key: Key) -> String {
    match key {
        Key::Char(c) => c.to_ascii_uppercase().to_string(),
        Key::Named(named) => match named {
            NamedKey::Enter => "Enter",
            NamedKey::Backspace => "Backspace",
            NamedKey::Delete => "Delete",
            NamedKey::Tab => "Tab",
            NamedKey::Escape => "Esc",
            NamedKey::Space => "Space",
            NamedKey::Left => "Left",
            NamedKey::Right => "Right",
            NamedKey::Up => "Up",
            NamedKey::Down => "Down",
            NamedKey::Home => "Home",
            NamedKey::End => "End",
            NamedKey::PageUp => "PageUp",
            NamedKey::PageDown => "PageDown",
        }
        .to_string(),
    }
}

impl From<&str> for Accel {
    /// Parse the conventional `"Ctrl+Shift+R"` string form. Modifier tokens
    /// (case-insensitive): `Ctrl` / `Cmd` / `Command` / `Primary` → the
    /// primary role; `Super` / `Win` / `Secondary` → secondary; `Alt` / `Opt`
    /// / `Option` → alt; `Shift`. The final token is the key: a single
    /// character, or a named key (`Enter`, `Esc`, `Left`, `PageUp`, …).
    ///
    /// # Panics
    ///
    /// On malformed input. Accelerator strings are in practice compile-time
    /// constants, so a typo should fail loudly at first use, not bind nothing.
    fn from(s: &str) -> Self {
        let mut mods = AccelMods::default();
        let mut tokens = s.split('+').peekable();
        loop {
            let token = tokens
                .next()
                .unwrap_or_else(|| panic!("empty accelerator string {s:?}"));
            // The last token is the key; everything before it is a modifier.
            if tokens.peek().is_none() {
                return Accel {
                    mods,
                    key: parse_key(token, s),
                };
            }
            match token.to_ascii_lowercase().as_str() {
                "ctrl" | "control" | "cmd" | "command" | "primary" => mods.primary = true,
                "super" | "win" | "secondary" => mods.secondary = true,
                "alt" | "opt" | "option" => mods.alt = true,
                "shift" => mods.shift = true,
                _ => panic!("unknown modifier {token:?} in accelerator {s:?}"),
            }
        }
    }
}

fn parse_key(token: &str, accel: &str) -> Key {
    let mut chars = token.chars();
    if let (Some(c), None) = (chars.next(), chars.next()) {
        return Key::Char(c.to_ascii_lowercase());
    }
    let named = match token.to_ascii_lowercase().as_str() {
        "enter" | "return" => NamedKey::Enter,
        "backspace" => NamedKey::Backspace,
        "delete" | "del" => NamedKey::Delete,
        "tab" => NamedKey::Tab,
        "escape" | "esc" => NamedKey::Escape,
        "space" => NamedKey::Space,
        "left" => NamedKey::Left,
        "right" => NamedKey::Right,
        "up" => NamedKey::Up,
        "down" => NamedKey::Down,
        "home" => NamedKey::Home,
        "end" => NamedKey::End,
        "pageup" | "pgup" => NamedKey::PageUp,
        "pagedown" | "pgdn" => NamedKey::PageDown,
        _ => panic!("unknown key {token:?} in accelerator {accel:?}"),
    };
    Key::Named(named)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mods(control: bool, logo: bool, alt: bool, shift: bool) -> Modifiers {
        Modifiers {
            control,
            logo,
            alt,
            shift,
            alt_graph: false,
        }
    }

    #[test]
    fn primary_resolves_to_ctrl_on_pc_and_cmd_on_mac() {
        let accel = Accel::primary('r');
        let ctrl_r = mods(true, false, false, false);
        let cmd_r = mods(false, true, false, false);
        assert!(accel.matches(Key::Char('r'), ctrl_r, ModifierScheme::Pc));
        assert!(!accel.matches(Key::Char('r'), cmd_r, ModifierScheme::Pc));
        assert!(accel.matches(Key::Char('r'), cmd_r, ModifierScheme::Mac));
        assert!(!accel.matches(Key::Char('r'), ctrl_r, ModifierScheme::Mac));
    }

    #[test]
    fn secondary_resolves_to_super_on_pc_and_ctrl_on_mac() {
        let accel = Accel::primary('r').secondary();
        // PC: Ctrl+Super+R; Mac: Ctrl+Cmd+R.
        assert!(accel.matches(Key::Char('r'), mods(true, true, false, false), ModifierScheme::Pc));
        assert!(accel.matches(Key::Char('r'), mods(true, true, false, false), ModifierScheme::Mac));
        // Plain primary input must not satisfy the bigger chord.
        assert!(!accel.matches(Key::Char('r'), mods(true, false, false, false), ModifierScheme::Pc));
    }

    #[test]
    fn modifiers_must_match_exactly() {
        let accel = Accel::primary('r');
        // Ctrl+Shift+R is a different chord than Ctrl+R.
        assert!(!accel.matches(Key::Char('r'), mods(true, false, false, true), ModifierScheme::Pc));
        // …and so is Ctrl+Alt+R.
        assert!(!accel.matches(Key::Char('r'), mods(true, false, true, false), ModifierScheme::Pc));
        // The declared chord requires every declared modifier.
        let shifted = Accel::primary('r').shift();
        assert!(shifted.matches(Key::Char('r'), mods(true, false, false, true), ModifierScheme::Pc));
        assert!(!shifted.matches(Key::Char('r'), mods(true, false, false, false), ModifierScheme::Pc));
    }

    #[test]
    fn char_keys_match_case_insensitively() {
        let accel = Accel::primary('r');
        assert!(accel.matches(Key::Char('R'), mods(true, false, false, false), ModifierScheme::Pc));
        assert!(Accel::from("Ctrl+R").matches(
            Key::Char('r'),
            mods(true, false, false, false),
            ModifierScheme::Pc
        ));
    }

    #[test]
    fn altgr_chords_never_match() {
        // Windows reports AltGr as Ctrl+Alt: composing "@" on a German layout
        // arrives as Ctrl+Alt+Q with alt_graph set. That must not fire a
        // Ctrl+Alt+Q accelerator — it's text entry.
        let accel = Accel::primary('q').alt();
        let altgr = Modifiers {
            control: true,
            alt: true,
            alt_graph: true,
            ..Modifiers::default()
        };
        assert!(!accel.matches(Key::Char('q'), altgr, ModifierScheme::Pc));
    }

    #[test]
    fn parses_the_conventional_string_forms() {
        assert_eq!(Accel::from("Ctrl+R"), Accel::primary('r'));
        assert_eq!(Accel::from("Cmd+R"), Accel::primary('r'));
        assert_eq!(Accel::from("Primary+R"), Accel::primary('r'));
        assert_eq!(Accel::from("Ctrl+Shift+T"), Accel::primary('t').shift());
        assert_eq!(
            Accel::from("Ctrl+Enter"),
            Accel::primary(NamedKey::Enter)
        );
        assert_eq!(Accel::from("Ctrl+Left"), Accel::primary(NamedKey::Left));
        assert_eq!(
            Accel::from("Ctrl+Super+Alt+R"),
            Accel::primary('r').secondary().alt()
        );
    }

    #[test]
    #[should_panic(expected = "unknown modifier")]
    fn parsing_an_unknown_modifier_panics() {
        let _ = Accel::from("Hyper+R");
    }

    #[test]
    #[should_panic(expected = "unknown key")]
    fn parsing_an_unknown_key_panics() {
        let _ = Accel::from("Ctrl+Klick");
    }

    #[test]
    fn labels_follow_each_platforms_convention() {
        let accel = Accel::primary('r').shift();
        assert_eq!(accel.label(ModifierScheme::Pc), "Ctrl+Shift+R");
        assert_eq!(accel.label(ModifierScheme::Mac), "Shift+Cmd+R");
        // The full chord: Ctrl+Super+Alt+Shift on PC, Apple's ⌃⌥⇧⌘ order
        // (Ctrl, Opt, Shift, Cmd) on a Mac.
        let full = Accel::primary('r').secondary().alt().shift();
        assert_eq!(full.label(ModifierScheme::Pc), "Ctrl+Super+Alt+Shift+R");
        assert_eq!(full.label(ModifierScheme::Mac), "Ctrl+Opt+Shift+Cmd+R");
    }

    #[test]
    fn pc_labels_round_trip_through_parsing() {
        // The strings journey bakes into its menu snapshots must come back out
        // exactly as they went in.
        for s in ["Ctrl+R", "Ctrl+Enter", "Ctrl+Left", "Ctrl+Right", "Ctrl+Q"] {
            assert_eq!(Accel::from(s).label(ModifierScheme::Pc), s);
        }
    }
}
