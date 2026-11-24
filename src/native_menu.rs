//! What the system menu bar shows for a [`MenuBar`](crate::MenuBar) handed to
//! [`App::with_menu_bar`](crate::App::with_menu_bar) on macOS, worked out
//! without AppKit: which items move to the application menu, which separators
//! that leaves over, what each title and key equivalent is. `macos_menu` turns
//! it into `NSMenu`s. Kept apart so it is tested on every platform, not only
//! on the one that uses it.

// Off macOS only the tests reach this, and not every piece of it.
#![cfg_attr(not(target_os = "macos"), allow(dead_code))]

use crate::accel::Accel;
use crate::event::{Key, NamedKey};
use crate::widgets::mnemonic::parse_label;
use crate::widgets::{Menu, MenuItem, MenuRole};

/// Where an item is in the [`MenuBar`](crate::MenuBar): the index of its menu,
/// and its index among that menu's items, separators included.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct ItemId {
    pub(crate) menu: usize,
    pub(crate) item: usize,
}

/// One row of a native menu.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Entry {
    Item(ItemId),
    Separator,
}

/// One of the app's own menus, as it goes on the menu bar.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct NativeMenu {
    pub(crate) title: String,
    pub(crate) entries: Vec<Entry>,
}

/// The app's menus as the menu bar shows them, with the items whose
/// [`MenuRole`] puts them in the application menu taken out.
#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct Plan {
    pub(crate) about: Option<ItemId>,
    pub(crate) preferences: Option<ItemId>,
    pub(crate) quit: Option<ItemId>,
    /// The app's menus, in the order declared. A menu with nothing left in it
    /// is left out.
    pub(crate) menus: Vec<NativeMenu>,
}

/// Work out the menu bar for `menus`: the first item of each [`MenuRole`]
/// moves to the application menu, leaving its own menu without it, and the
/// separators that leaves at either end of a menu or next to another go too.
/// A later item with a role already taken stays where it is.
pub(crate) fn plan(menus: &[Menu]) -> Plan {
    let mut plan = Plan::default();
    for (m, menu) in menus.iter().enumerate() {
        let mut entries = Vec::with_capacity(menu.items.len());
        for (i, item) in menu.items.iter().enumerate() {
            let id = ItemId { menu: m, item: i };
            let role = match item {
                MenuItem::Separator => {
                    entries.push(Entry::Separator);
                    continue;
                }
                MenuItem::Action { role, .. } => *role,
            };
            let slot = match role {
                Some(MenuRole::About) => &mut plan.about,
                Some(MenuRole::Preferences) => &mut plan.preferences,
                Some(MenuRole::Quit) => &mut plan.quit,
                None => {
                    entries.push(Entry::Item(id));
                    continue;
                }
            };
            if slot.is_none() {
                *slot = Some(id);
            } else {
                entries.push(Entry::Item(id));
            }
        }
        let entries = tidy_separators(entries);
        if !entries.is_empty() {
            plan.menus.push(NativeMenu {
                title: title(&menu.label),
                entries,
            });
        }
    }
    plan
}

/// Drop the separators that separate nothing: at either end, and all but one
/// of several in a row.
pub(crate) fn tidy_separators(entries: Vec<Entry>) -> Vec<Entry> {
    let mut out: Vec<Entry> = Vec::with_capacity(entries.len());
    for entry in entries {
        if entry == Entry::Separator && matches!(out.last(), None | Some(Entry::Separator)) {
            continue;
        }
        out.push(entry);
    }
    if out.last() == Some(&Entry::Separator) {
        out.pop();
    }
    out
}

/// A row of the application menu.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum AppEntry {
    /// The app's [`MenuRole::About`] item.
    About(ItemId),
    /// The app's [`MenuRole::Preferences`] item.
    Preferences(ItemId),
    /// The Services submenu, which macOS fills.
    Services,
    /// `Hide <App>`, ⌘H.
    Hide,
    /// "Hide Others", ⌥⌘H.
    HideOthers,
    /// "Show All".
    ShowAll,
    /// `Quit <App>`, ⌘Q: the app's [`MenuRole::Quit`] item, or the runtime's
    /// own, which closes the window as its close button does.
    Quit(Option<ItemId>),
    Separator,
}

/// The application menu, top to bottom: About, Preferences, Services, the
/// three Hide items and Quit, in groups of their own between separators. A
/// group the app has no item for is left out, separator and all.
pub(crate) fn app_menu(plan: &Plan) -> Vec<AppEntry> {
    let groups = [
        plan.about.map(AppEntry::About).into_iter().collect(),
        plan.preferences
            .map(AppEntry::Preferences)
            .into_iter()
            .collect(),
        vec![AppEntry::Services],
        vec![AppEntry::Hide, AppEntry::HideOthers, AppEntry::ShowAll],
        vec![AppEntry::Quit(plan.quit)],
    ];
    let mut out = Vec::new();
    for group in groups {
        if group.is_empty() {
            continue;
        }
        if !out.is_empty() {
            out.push(AppEntry::Separator);
        }
        out.extend(group);
    }
    out
}

/// Where the Window menu goes among the app's menus: in front of Help, as on
/// every Mac, or last if the app has no Help menu.
pub(crate) fn window_menu_index(menus: &[NativeMenu]) -> usize {
    menus
        .iter()
        .position(|m| is_help(&m.title))
        .unwrap_or(menus.len())
}

/// Whether a menu titled `title` is the Help menu, which macOS gives a search
/// field for the menus.
pub(crate) fn is_help(title: &str) -> bool {
    title.eq_ignore_ascii_case("help")
}

/// The title a menu or item labelled `label` has on the menu bar: the label
/// without its mnemonic marker (`&&` being a literal `&`), and a trailing
/// `...` as the one character `…` macOS writes it with.
pub(crate) fn title(label: &str) -> String {
    let display = parse_label(label).display;
    match display.strip_suffix("...") {
        Some(stem) => format!("{stem}…"),
        None => display,
    }
}

/// An `NSMenuItem`'s key equivalent: the character AppKit matches the key
/// press against, and the modifiers that must be held with it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct KeyEquivalent {
    pub(crate) key: String,
    pub(crate) command: bool,
    pub(crate) control: bool,
    pub(crate) option: bool,
    pub(crate) shift: bool,
}

impl KeyEquivalent {
    /// `key` with ⌘ alone, as the application menu's own items have it.
    pub(crate) fn command(key: &str) -> Self {
        Self {
            key: key.to_owned(),
            command: true,
            control: false,
            option: false,
            shift: false,
        }
    }
}

/// The key equivalent of `accel`, with each modifier role resolved the way
/// [`ModifierScheme::Mac`](crate::ModifierScheme::Mac) does: `primary` is ⌘,
/// `secondary` ⌃, `alt` ⌥.
///
/// A letter is lowercase unless Shift is part of the chord: AppKit reads an
/// uppercase key equivalent as one that wants Shift, so `Accel::primary('R')`
/// has to become `r` to stay ⌘R. Other characters — `[`, `\`, `-`, digits —
/// go in as they are. A key without a character of its own goes in as the
/// character AppKit reports it with: the control character of Return, Tab,
/// Escape and Backspace (⌫), or one of the private-use code points `NSEvent`
/// gives the arrows, the function keys and the rest of the navigation block.
pub(crate) fn key_equivalent(accel: Accel) -> KeyEquivalent {
    let key = match accel.key {
        Key::Char(c) if accel.mods.shift && c.is_alphabetic() => c.to_uppercase().collect(),
        Key::Char(c) => c.to_lowercase().collect(),
        Key::Named(named) => named_key(named).to_string(),
    };
    KeyEquivalent {
        key,
        command: accel.mods.primary,
        control: accel.mods.secondary,
        option: accel.mods.alt,
        shift: accel.mods.shift,
    }
}

/// `NSUpArrowFunctionKey` and the code points that follow it, in `NSEvent`'s
/// order: ↓ ← →, then F1 to F35, then the navigation block.
const UP_ARROW: u32 = 0xF700;
const F1: u32 = 0xF704;
const FORWARD_DELETE: u32 = 0xF728;
const HOME: u32 = 0xF729;
const END: u32 = 0xF72B;
const PAGE_UP: u32 = 0xF72C;
const PAGE_DOWN: u32 = 0xF72D;
const MENU_KEY: u32 = 0xF735;

/// The character AppKit reports `key` with.
fn named_key(key: NamedKey) -> char {
    let code = match key {
        NamedKey::Enter => '\r' as u32,
        // ⌫, the key a Mac labels "delete": `NSBackspaceCharacter`.
        NamedKey::Backspace => 0x08,
        // ⌦, forward delete: `NSDeleteFunctionKey`.
        NamedKey::Delete => FORWARD_DELETE,
        NamedKey::Tab => '\t' as u32,
        NamedKey::Escape => 0x1B,
        NamedKey::Space => ' ' as u32,
        NamedKey::Up => UP_ARROW,
        NamedKey::Down => UP_ARROW + 1,
        NamedKey::Left => UP_ARROW + 2,
        NamedKey::Right => UP_ARROW + 3,
        NamedKey::Home => HOME,
        NamedKey::End => END,
        NamedKey::PageUp => PAGE_UP,
        NamedKey::PageDown => PAGE_DOWN,
        NamedKey::F1 => F1,
        NamedKey::F2 => F1 + 1,
        NamedKey::F3 => F1 + 2,
        NamedKey::F4 => F1 + 3,
        NamedKey::F5 => F1 + 4,
        NamedKey::F6 => F1 + 5,
        NamedKey::F7 => F1 + 6,
        NamedKey::F8 => F1 + 7,
        NamedKey::F9 => F1 + 8,
        NamedKey::F10 => F1 + 9,
        NamedKey::F11 => F1 + 10,
        NamedKey::F12 => F1 + 11,
        NamedKey::F13 => F1 + 12,
        NamedKey::F14 => F1 + 13,
        NamedKey::F15 => F1 + 14,
        NamedKey::F16 => F1 + 15,
        NamedKey::F17 => F1 + 16,
        NamedKey::F18 => F1 + 17,
        NamedKey::F19 => F1 + 18,
        NamedKey::F20 => F1 + 19,
        NamedKey::F21 => F1 + 20,
        NamedKey::F22 => F1 + 21,
        NamedKey::F23 => F1 + 22,
        NamedKey::F24 => F1 + 23,
        // `NSMenuFunctionKey`: what AppKit calls the PC keyboard's Menu key.
        NamedKey::ContextMenu => MENU_KEY,
    };
    char::from_u32(code).expect("a key equivalent is a valid code point")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::geometry::Rect;
    use crate::widgets::MenuBar;

    fn item(label: &str) -> MenuItem {
        MenuItem::action(label, |_| {})
    }

    fn id(menu: usize, item: usize) -> Entry {
        Entry::Item(ItemId { menu, item })
    }

    /// A bar the way a Windows app has it: About under Help, Options under
    /// Tools, Exit at the bottom of File.
    fn windows_style_bar() -> MenuBar {
        MenuBar::new(Rect::new(0, 0, 400, 20))
            .add_menu(Menu::new(
                "&File",
                vec![
                    item("&New"),
                    item("&Open..."),
                    MenuItem::separator(),
                    item("E&xit").with_role(MenuRole::Quit),
                ],
            ))
            .add_menu(Menu::new(
                "&Tools",
                vec![item("&Options...").with_role(MenuRole::Preferences)],
            ))
            .add_menu(Menu::new(
                "&Help",
                vec![
                    item("&Contents"),
                    MenuItem::separator(),
                    item("&About").with_role(MenuRole::About),
                ],
            ))
    }

    #[test]
    fn role_items_move_to_the_application_menu() {
        let plan = plan(windows_style_bar().menus());
        assert_eq!(plan.about, Some(ItemId { menu: 2, item: 2 }));
        assert_eq!(plan.preferences, Some(ItemId { menu: 1, item: 0 }));
        assert_eq!(plan.quit, Some(ItemId { menu: 0, item: 3 }));
        assert_eq!(
            plan.menus,
            vec![
                // File loses Exit, and with it the separator above it.
                NativeMenu {
                    title: "File".into(),
                    entries: vec![id(0, 0), id(0, 1)],
                },
                // Tools had nothing but Options: it is gone.
                NativeMenu {
                    title: "Help".into(),
                    entries: vec![id(2, 0)],
                },
            ]
        );
    }

    #[test]
    fn the_application_menu_has_the_role_items_in_their_places() {
        let plan = plan(windows_style_bar().menus());
        use AppEntry::*;
        assert_eq!(
            app_menu(&plan),
            vec![
                About(ItemId { menu: 2, item: 2 }),
                Separator,
                Preferences(ItemId { menu: 1, item: 0 }),
                Separator,
                Services,
                Separator,
                Hide,
                HideOthers,
                ShowAll,
                Separator,
                Quit(Some(ItemId { menu: 0, item: 3 })),
            ]
        );
    }

    #[test]
    fn without_role_items_the_application_menu_has_its_own_quit() {
        let bar = MenuBar::new(Rect::new(0, 0, 400, 20))
            .add_menu(Menu::new("&File", vec![item("E&xit")]));
        let plan = plan(bar.menus());
        assert_eq!(
            plan,
            Plan {
                menus: vec![NativeMenu {
                    title: "File".into(),
                    entries: vec![id(0, 0)],
                }],
                ..Plan::default()
            }
        );
        use AppEntry::*;
        assert_eq!(
            app_menu(&plan),
            vec![
                Services,
                Separator,
                Hide,
                HideOthers,
                ShowAll,
                Separator,
                Quit(None)
            ]
        );
    }

    #[test]
    fn only_the_first_item_of_a_role_moves() {
        let bar = MenuBar::new(Rect::new(0, 0, 400, 20)).add_menu(Menu::new(
            "&File",
            vec![
                item("&Quit").with_role(MenuRole::Quit),
                item("Quit &All").with_role(MenuRole::Quit),
            ],
        ));
        let plan = plan(bar.menus());
        assert_eq!(plan.quit, Some(ItemId { menu: 0, item: 0 }));
        assert_eq!(plan.menus[0].entries, vec![id(0, 1)]);
    }

    #[test]
    fn separators_that_separate_nothing_go() {
        use Entry::Separator as S;
        let a = id(0, 0);
        let b = id(0, 1);
        assert_eq!(tidy_separators(vec![S, a, S, S, b, S]), vec![a, S, b]);
        assert_eq!(tidy_separators(vec![S, S]), vec![]);
        assert_eq!(tidy_separators(vec![a, S, b]), vec![a, S, b]);
    }

    #[test]
    fn a_move_out_of_the_middle_leaves_one_separator() {
        let bar = MenuBar::new(Rect::new(0, 0, 400, 20)).add_menu(Menu::new(
            "&Edit",
            vec![
                item("&Undo"),
                MenuItem::separator(),
                item("&Settings").with_role(MenuRole::Preferences),
                MenuItem::separator(),
                item("Select &All"),
            ],
        ));
        let plan = plan(bar.menus());
        assert_eq!(
            plan.menus[0].entries,
            vec![id(0, 0), Entry::Separator, id(0, 4)]
        );
    }

    #[test]
    fn titles_lose_their_mnemonic_and_get_an_ellipsis() {
        assert_eq!(title("&File"), "File");
        assert_eq!(title("Save &As..."), "Save As…");
        assert_eq!(title("Tom && Jerry"), "Tom & Jerry");
        assert_eq!(title("&&Co"), "&Co");
        assert_eq!(title("Wait... what"), "Wait... what");
    }

    #[test]
    fn the_window_menu_goes_before_help() {
        let menus = |titles: &[&str]| -> Vec<NativeMenu> {
            titles
                .iter()
                .map(|t| NativeMenu {
                    title: t.to_string(),
                    entries: Vec::new(),
                })
                .collect()
        };
        assert_eq!(window_menu_index(&menus(&["File", "Edit", "Help"])), 2);
        assert_eq!(window_menu_index(&menus(&["File", "Edit"])), 2);
        assert_eq!(window_menu_index(&menus(&[])), 0);
    }

    fn key(key: &str, command: bool, control: bool, option: bool, shift: bool) -> KeyEquivalent {
        KeyEquivalent {
            key: key.into(),
            command,
            control,
            option,
            shift,
        }
    }

    #[test]
    fn modifier_roles_resolve_as_on_a_mac() {
        assert_eq!(
            key_equivalent(Accel::primary('r')),
            key("r", true, false, false, false)
        );
        assert_eq!(
            key_equivalent(Accel::primary('r').secondary().alt()),
            key("r", true, true, true, false)
        );
        assert_eq!(
            key_equivalent(Accel::from("Ctrl+Shift+Z")),
            key("Z", true, false, false, true)
        );
        assert_eq!(
            key_equivalent(Accel::primary('R')),
            key("r", true, false, false, false),
            "an uppercase letter would mean Shift to AppKit"
        );
    }

    #[test]
    fn punctuation_goes_in_as_it_is() {
        for c in ['[', ']', '\\', '-', '=', ',', '.', '/', '1'] {
            assert_eq!(
                key_equivalent(Accel::primary(c)),
                key(&c.to_string(), true, false, false, false)
            );
        }
        assert_eq!(
            key_equivalent(Accel::primary('[').shift()),
            key("[", true, false, false, true)
        );
    }

    #[test]
    fn named_keys_are_the_characters_appkit_reports() {
        let k = |named: NamedKey| key_equivalent(Accel::new(named)).key;
        assert_eq!(k(NamedKey::Up), "\u{F700}");
        assert_eq!(k(NamedKey::Down), "\u{F701}");
        assert_eq!(k(NamedKey::Left), "\u{F702}");
        assert_eq!(k(NamedKey::Right), "\u{F703}");
        assert_eq!(k(NamedKey::F1), "\u{F704}");
        assert_eq!(k(NamedKey::F12), "\u{F70F}");
        assert_eq!(k(NamedKey::F24), "\u{F71B}");
        assert_eq!(k(NamedKey::Delete), "\u{F728}");
        assert_eq!(k(NamedKey::Home), "\u{F729}");
        assert_eq!(k(NamedKey::End), "\u{F72B}");
        assert_eq!(k(NamedKey::PageUp), "\u{F72C}");
        assert_eq!(k(NamedKey::PageDown), "\u{F72D}");
        assert_eq!(k(NamedKey::Backspace), "\u{8}");
        assert_eq!(k(NamedKey::Enter), "\r");
        assert_eq!(k(NamedKey::Tab), "\t");
        assert_eq!(k(NamedKey::Escape), "\u{1B}");
        assert_eq!(k(NamedKey::Space), " ");
        // A key with no modifier is a key equivalent with none.
        assert_eq!(
            key_equivalent(Accel::new(NamedKey::F5)),
            key("\u{F708}", false, false, false, false)
        );
    }
}
