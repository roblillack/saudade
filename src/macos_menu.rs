//! macOS: an app's [`MenuBar`] as the system menu bar.
//!
//! A Mac has one menu bar, at the top of the screen, and it belongs to
//! whichever app is in front; a window has none. So a bar handed to
//! [`App::with_menu_bar`](crate::App::with_menu_bar) isn't drawn in the window
//! here: [`install`] builds an `NSMenu` from it and makes that the
//! application's main menu, in place of the stand-in winit puts up at launch.
//! What goes where — which items move to the application menu, which
//! separators that leaves over, the titles and key equivalents — is worked out
//! by [`native_menu`], without AppKit, where it can be tested anywhere.
//!
//! Every item of the app's own mirrors its [`MenuItem`] and has one shared
//! [`MenuTarget`] for a target, which tells them apart by their tag. AppKit
//! asks that target whether an item is enabled — `validateMenuItem:` — every
//! time a menu opens and before a key equivalent fires it, which is when the
//! item's [`with_enabled`](MenuItem::with_enabled) and
//! [`with_checked`](MenuItem::with_checked) predicates are read: they stay as
//! live as in a bar drawn in the window.
//!
//! An item can't run its callback from the action AppKit sends: the callback
//! wants an [`EventCtx`], whose requests only the runtime can carry out, and
//! AppKit sends the action from inside its own event handling, where the
//! runtime can't be reached. So the action queues the item and wakes the event
//! loop, and the runtime runs the queue (see [`take_commands`] and [`fire`]) as
//! it would a burst of input: an `EventCtx` for each item, its requests carried
//! out, the popups brought in line. Everything here happens on the main thread
//! — AppKit runs menus nowhere else — so thread-locals hold the bar and the
//! queue.
//!
//! The app's own items are disabled for as long as a popup window of saudade's
//! is open — a dialog, a menu, a dropdown's list — so that a key equivalent
//! like ⌘C reaches the dialog's text field as the key press it is, rather than
//! acting on the window behind it. The application menu's items, Quit among
//! them, stay available.
//!
//! A disabled item has to let its key go, too — to the dialog then, and
//! otherwise to the widget with the keyboard, as a chord whose items are all
//! disabled does in a bar drawn in the window (⌘Z reaching a text field's own
//! undo while Edit ▸ Undo has nothing to take back). AppKit doesn't: it
//! swallows a key press that matches a disabled item. So an item only carries
//! its key equivalent while it is enabled, which [`refresh`] works out each
//! time the widgets have handled an event — which is when the app's state
//! behind the predicates changes. While one of the menus is open every item in
//! it shows its key, disabled or not, as on any Mac.
//!
//! A bar that is the system's never reaches the widget tree, so its
//! accelerators are AppKit's key equivalents alone: AppKit takes a key press
//! that matches one before the window sees it, and there is no second,
//! in-window accelerator table to fire it again.

use std::cell::{Cell, RefCell};
use std::panic::{AssertUnwindSafe, catch_unwind};

use objc2::rc::Retained;
use objc2::runtime::{AnyObject, NSObject, NSObjectProtocol, ProtocolObject, Sel};
use objc2::{MainThreadMarker, MainThreadOnly, Message, define_class, msg_send, sel};
use objc2_app_kit::{
    NSApplication, NSControlStateValueOff, NSControlStateValueOn, NSEventModifierFlags, NSMenu,
    NSMenuDelegate, NSMenuItem, NSMenuItemValidation,
};
use objc2_foundation::{NSOperatingSystemVersion, NSProcessInfo, NSString};

use crate::event::EventCtx;
use crate::native_menu::{self, AppEntry, Entry, ItemId, KeyEquivalent};
use crate::widgets::{MenuBar, MenuItem};

/// The tag of the application menu's own Quit item, there for an app without
/// a [`MenuRole::Quit`](crate::MenuRole::Quit) item. The tags of the app's
/// items count up from 1, and AppKit's default is 0.
const QUIT_TAG: isize = -1;

/// A menu item AppKit has fired, waiting for the runtime to run it.
pub(crate) enum Command {
    /// One of the app's own items, by its tag. See [`fire`].
    Item(isize),
    /// The application menu's own Quit: the runtime closes the window as its
    /// close button does — the root's `on_cancel`, then the end of the app.
    /// AppKit's `terminate:` would end it without the root hearing of it while
    /// the window still stands.
    Quit,
}

/// The installed bar: its items' callbacks and predicates, and what each
/// native item's tag stands for.
struct Installed {
    bar: MenuBar,
    /// Tag `n` is `slots[n - 1]`.
    slots: Vec<Slot>,
}

impl Installed {
    fn slot(&self, tag: isize) -> Option<&Slot> {
        let index = usize::try_from(tag).ok()?.checked_sub(1)?;
        self.slots.get(index)
    }

    fn item(&self, id: ItemId) -> Option<&MenuItem> {
        self.bar.menus().get(id.menu)?.items.get(id.item)
    }

    /// Whether the item `slot` stands for is enabled now.
    fn enabled(&self, slot: &Slot) -> bool {
        if BLOCKED.get() && !slot.in_app_menu {
            return false;
        }
        match self.item(slot.id) {
            Some(MenuItem::Action { enabled, .. }) => enabled
                .as_ref()
                .is_none_or(|enabled| predicate(enabled.as_ref())),
            _ => false,
        }
    }
}

/// A native item of the app's own, and what it stands for.
struct Slot {
    id: ItemId,
    /// Whether the item is in the application menu, where it stays enabled
    /// while a popup window is open.
    in_app_menu: bool,
    item: Retained<NSMenuItem>,
    /// The menu the item is in.
    menu: Retained<NSMenu>,
    key: Option<KeyEquivalent>,
    /// Whether the item carries its key equivalent right now — see the
    /// module's docs.
    armed: Cell<bool>,
    /// Whether the item's menu is open, which shows every key.
    shown: Cell<bool>,
}

impl Slot {
    /// Give the item its key equivalent, or take it away.
    fn arm(&self, armed: bool) {
        let armed = armed && self.key.is_some();
        if armed == self.armed.replace(armed) {
            return;
        }
        let key = match &self.key {
            Some(key) if armed => key.key.as_str(),
            _ => "",
        };
        self.item.setKeyEquivalent(&NSString::from_str(key));
    }
}

thread_local! {
    /// The installed bar. Taken back by [`take_bar`] as the app ends.
    static INSTALLED: RefCell<Option<Installed>> = const { RefCell::new(None) };
    /// Items AppKit has fired since the runtime last looked.
    static COMMANDS: RefCell<Vec<Command>> = const { RefCell::new(Vec::new()) };
    /// Wakes the event loop, so it runs [`COMMANDS`] soon.
    static WAKE: RefCell<Option<Box<dyn Fn()>>> = const { RefCell::new(None) };
    /// Whether a popup window of saudade's is open — see [`refresh`].
    static BLOCKED: Cell<bool> = const { Cell::new(false) };
    /// The target of every item, kept here because a menu item doesn't keep
    /// its target alive.
    static TARGET: RefCell<Option<Retained<MenuTarget>>> = const { RefCell::new(None) };
}

define_class!(
    /// The target of every item of the app's own. It fires an item by queuing
    /// it (see [`Command`]) and says whether one is enabled when AppKit asks.
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    struct MenuTarget;

    impl MenuTarget {
        #[unsafe(method(fireMenuItem:))]
        fn fire_menu_item(&self, sender: &NSMenuItem) {
            let tag = sender.tag();
            let command = if tag == QUIT_TAG {
                Command::Quit
            } else {
                Command::Item(tag)
            };
            COMMANDS.with(|commands| commands.borrow_mut().push(command));
            WAKE.with(|wake| {
                if let Some(wake) = wake.borrow().as_ref() {
                    wake();
                }
            });
        }
    }

    unsafe impl NSObjectProtocol for MenuTarget {}

    unsafe impl NSMenuItemValidation for MenuTarget {
        #[unsafe(method(validateMenuItem:))]
        fn validate_menu_item(&self, item: &NSMenuItem) -> bool {
            validate(item)
        }
    }

    // The delegate of each of the app's menus, which shows every item's key
    // while the menu is open — see the module's docs.
    unsafe impl NSMenuDelegate for MenuTarget {
        #[unsafe(method(menuWillOpen:))]
        fn menu_will_open(&self, menu: &NSMenu) {
            show_keys(menu, true);
        }

        #[unsafe(method(menuDidClose:))]
        fn menu_did_close(&self, menu: &NSMenu) {
            show_keys(menu, false);
        }
    }
);

/// `menu` opened, or closed again: while it is open, each of its items shows
/// its key equivalent, enabled or not; once it is closed, only the enabled ones
/// keep theirs. [`refresh`] leaves an open menu alone.
fn show_keys(menu: &NSMenu, open: bool) {
    INSTALLED.with(|installed| {
        let Ok(installed) = installed.try_borrow() else {
            return;
        };
        let Some(installed) = installed.as_ref() else {
            return;
        };
        for slot in installed
            .slots
            .iter()
            .filter(|s| std::ptr::eq(&*s.menu, menu))
        {
            slot.shown.set(open);
            slot.arm(open || installed.enabled(slot));
        }
    });
}

impl MenuTarget {
    fn new(mtm: MainThreadMarker) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(());
        // SAFETY: `NSObject`'s designated initializer, on a fresh allocation.
        unsafe { msg_send![super(this), init] }
    }
}

/// Whether `item` is enabled now, setting its checkmark on the way. Run by
/// AppKit as a menu opens and before a key equivalent fires an item.
fn validate(item: &NSMenuItem) -> bool {
    let tag = item.tag();
    if tag == QUIT_TAG {
        return true;
    }
    INSTALLED.with(|installed| {
        // Borrowed already: an item's callback is running, and something it
        // did has AppKit look at the menus. There is no reading the
        // predicates now; the item was enabled a moment ago.
        let Ok(installed) = installed.try_borrow() else {
            return true;
        };
        let Some(installed) = installed.as_ref() else {
            return false;
        };
        let Some(slot) = installed.slot(tag) else {
            return false;
        };
        if let Some(MenuItem::Action {
            checked: Some(checked),
            ..
        }) = installed.item(slot.id)
        {
            let state = if predicate(checked.as_ref()) {
                NSControlStateValueOn
            } else {
                NSControlStateValueOff
            };
            item.setState(state);
        }
        installed.enabled(slot)
    })
}

/// Run an app's predicate from inside AppKit, which a panic must not unwind
/// into: a predicate that panics counts as false, after the panic hook has
/// reported it.
fn predicate(f: &dyn Fn() -> bool) -> bool {
    catch_unwind(AssertUnwindSafe(f)).unwrap_or(false)
}

/// Make `bar` the application's menu bar, replacing winit's stand-in. `wake`
/// rouses the event loop when an item fires, which then runs it — see
/// [`take_commands`]. Called once, after the app has finished launching; off
/// the main thread it does nothing.
pub(crate) fn install(bar: MenuBar, wake: impl Fn() + 'static) {
    let Some(mtm) = MainThreadMarker::new() else {
        return;
    };
    let app = NSApplication::sharedApplication(mtm);
    let target = MenuTarget::new(mtm);
    let plan = native_menu::plan(bar.menus());
    let name = bar.app_name().map_or_else(
        || NSProcessInfo::processInfo().processName().to_string(),
        str::to_owned,
    );
    let mut builder = Builder {
        mtm,
        target: &target,
        bar: &bar,
        slots: Vec::new(),
    };

    let main_menu = NSMenu::new(mtm);

    // The application menu: the bold one, named after the program.
    let app_menu = builder.menu(&name);
    for entry in native_menu::app_menu(&plan) {
        match entry {
            AppEntry::About(id) => {
                let key = builder.accel(id).map(native_menu::key_equivalent);
                builder.own_item(&app_menu, id, &format!("About {name}"), key, true);
            }
            AppEntry::Preferences(id) => {
                let key = builder
                    .accel(id)
                    .map_or_else(|| KeyEquivalent::command(","), native_menu::key_equivalent);
                builder.own_item(&app_menu, id, preferences_title(), Some(key), true);
            }
            AppEntry::Services => {
                // Filled by macOS with what other apps offer to do with the
                // selection.
                let services = NSMenu::initWithTitle(mtm.alloc(), &NSString::from_str("Services"));
                app.setServicesMenu(Some(&services));
                app_menu.addItem(&submenu_item(mtm, "Services", &services));
            }
            AppEntry::Hide => app_menu.addItem(&system_item(
                mtm,
                &format!("Hide {name}"),
                sel!(hide:),
                Some(KeyEquivalent::command("h")),
            )),
            AppEntry::HideOthers => app_menu.addItem(&system_item(
                mtm,
                "Hide Others",
                sel!(hideOtherApplications:),
                Some(KeyEquivalent {
                    option: true,
                    ..KeyEquivalent::command("h")
                }),
            )),
            AppEntry::ShowAll => app_menu.addItem(&system_item(
                mtm,
                "Show All",
                sel!(unhideAllApplications:),
                None,
            )),
            AppEntry::Quit(Some(id)) => {
                let key = Some(KeyEquivalent::command("q"));
                builder.own_item(&app_menu, id, &format!("Quit {name}"), key, true);
            }
            AppEntry::Quit(None) => app_menu.addItem(&target_item(
                mtm,
                &target,
                &format!("Quit {name}"),
                Some(KeyEquivalent::command("q")),
                QUIT_TAG,
            )),
            AppEntry::Separator => app_menu.addItem(&NSMenuItem::separatorItem(mtm)),
        }
    }
    main_menu.addItem(&submenu_item(mtm, &name, &app_menu));

    // The app's own menus, with the standard Window menu in front of Help.
    let window_at = native_menu::window_menu_index(&plan.menus);
    for (index, native) in plan.menus.iter().enumerate() {
        if index == window_at {
            main_menu.addItem(&window_menu(mtm, &app));
        }
        let menu = builder.menu(&native.title);
        for entry in &native.entries {
            match *entry {
                Entry::Item(id) => {
                    let key = builder.accel(id).map(native_menu::key_equivalent);
                    let title = builder.title(id);
                    builder.own_item(&menu, id, &title, key, false);
                }
                Entry::Separator => menu.addItem(&NSMenuItem::separatorItem(mtm)),
            }
        }
        if native_menu::is_help(&native.title) {
            app.setHelpMenu(Some(&menu));
        }
        main_menu.addItem(&submenu_item(mtm, &native.title, &menu));
    }
    if window_at == plan.menus.len() {
        main_menu.addItem(&window_menu(mtm, &app));
    }

    let slots = builder.slots;
    app.setMainMenu(Some(&main_menu));
    TARGET.with(|t| t.replace(Some(target)));
    WAKE.with(|w| w.replace(Some(Box::new(wake))));
    INSTALLED.with(|i| i.replace(Some(Installed { bar, slots })));
    refresh(false);
}

/// Builds the native menus of one bar, numbering its items as it goes.
struct Builder<'a> {
    mtm: MainThreadMarker,
    target: &'a MenuTarget,
    bar: &'a MenuBar,
    slots: Vec<Slot>,
}

impl Builder<'_> {
    fn menu(&self, title: &str) -> Retained<NSMenu> {
        let menu = NSMenu::initWithTitle(self.mtm.alloc(), &NSString::from_str(title));
        // AppKit asks each item's target whether it is enabled — the app's
        // predicates for the items of its own, AppKit itself for the rest.
        menu.setAutoenablesItems(true);
        menu.setDelegate(Some(ProtocolObject::from_ref(self.target)));
        menu
    }

    fn action(&self, id: ItemId) -> Option<&MenuItem> {
        self.bar.menus().get(id.menu)?.items.get(id.item)
    }

    fn accel(&self, id: ItemId) -> Option<crate::Accel> {
        match self.action(id)? {
            MenuItem::Action { accel, .. } => *accel,
            MenuItem::Separator => None,
        }
    }

    fn title(&self, id: ItemId) -> String {
        match self.action(id) {
            Some(MenuItem::Action { label, .. }) => native_menu::title(label),
            _ => String::new(),
        }
    }

    /// Add the item for the app's item `id`, titled `title` and fired by
    /// `key`, to `menu`, the application menu if `in_app_menu`.
    fn own_item(
        &mut self,
        menu: &NSMenu,
        id: ItemId,
        title: &str,
        key: Option<KeyEquivalent>,
        in_app_menu: bool,
    ) {
        let tag = self.slots.len() as isize + 1;
        let item = target_item(self.mtm, self.target, title, key.clone(), tag);
        menu.addItem(&item);
        self.slots.push(Slot {
            id,
            in_app_menu,
            item,
            menu: menu.retain(),
            armed: Cell::new(key.is_some()),
            shown: Cell::new(false),
            key,
        });
    }
}

/// An item firing `target`'s `fireMenuItem:`, tagged `tag`.
fn target_item(
    mtm: MainThreadMarker,
    target: &MenuTarget,
    title: &str,
    key: Option<KeyEquivalent>,
    tag: isize,
) -> Retained<NSMenuItem> {
    let item = action_item(mtm, title, sel!(fireMenuItem:), key);
    let target: &AnyObject = target;
    // SAFETY: the target is kept alive in `TARGET` for as long as the app
    // runs, and it answers `fireMenuItem:`.
    unsafe { item.setTarget(Some(target)) };
    item.setTag(tag);
    item
}

/// An item sending `action` to whoever answers it — the key window, or the
/// application — as AppKit's own standard items do.
fn system_item(
    mtm: MainThreadMarker,
    title: &str,
    action: Sel,
    key: Option<KeyEquivalent>,
) -> Retained<NSMenuItem> {
    action_item(mtm, title, action, key)
}

fn action_item(
    mtm: MainThreadMarker,
    title: &str,
    action: Sel,
    key: Option<KeyEquivalent>,
) -> Retained<NSMenuItem> {
    let key_string = key.as_ref().map_or("", |key| key.key.as_str());
    // SAFETY: `action` is a selector AppKit or `MenuTarget` answers.
    let item = unsafe {
        NSMenuItem::initWithTitle_action_keyEquivalent(
            mtm.alloc(),
            &NSString::from_str(title),
            Some(action),
            &NSString::from_str(key_string),
        )
    };
    if let Some(key) = key {
        item.setKeyEquivalentModifierMask(modifier_mask(&key));
    }
    item
}

/// An item opening `menu`, as one of the menu bar's titles or a submenu.
fn submenu_item(mtm: MainThreadMarker, title: &str, menu: &NSMenu) -> Retained<NSMenuItem> {
    let item = NSMenuItem::new(mtm);
    item.setTitle(&NSString::from_str(title));
    item.setSubmenu(Some(menu));
    item
}

/// The modifiers `key` wants held. Set on every item that has a key
/// equivalent, as AppKit's default is ⌘ — which a function key alone, say,
/// doesn't want.
fn modifier_mask(key: &KeyEquivalent) -> NSEventModifierFlags {
    let mut mask = NSEventModifierFlags::empty();
    for (held, flag) in [
        (key.command, NSEventModifierFlags::Command),
        (key.control, NSEventModifierFlags::Control),
        (key.option, NSEventModifierFlags::Option),
        (key.shift, NSEventModifierFlags::Shift),
    ] {
        if held {
            mask |= flag;
        }
    }
    mask
}

/// The standard Window menu: Minimize, Zoom, Bring All to Front, and the list
/// of the app's windows, which AppKit keeps once it knows the menu.
fn window_menu(mtm: MainThreadMarker, app: &NSApplication) -> Retained<NSMenuItem> {
    let menu = NSMenu::initWithTitle(mtm.alloc(), &NSString::from_str("Window"));
    menu.addItem(&system_item(
        mtm,
        "Minimize",
        sel!(performMiniaturize:),
        Some(KeyEquivalent::command("m")),
    ));
    menu.addItem(&system_item(mtm, "Zoom", sel!(performZoom:), None));
    menu.addItem(&NSMenuItem::separatorItem(mtm));
    menu.addItem(&system_item(
        mtm,
        "Bring All to Front",
        sel!(arrangeInFront:),
        None,
    ));
    app.setWindowsMenu(Some(&menu));
    submenu_item(mtm, "Window", &menu)
}

/// What the item for [`MenuRole::Preferences`](crate::MenuRole::Preferences)
/// is called: "Settings…" since macOS 13 (Ventura) renamed it, "Preferences…"
/// before.
fn preferences_title() -> &'static str {
    let ventura = NSOperatingSystemVersion {
        majorVersion: 13,
        minorVersion: 0,
        patchVersion: 0,
    };
    if NSProcessInfo::processInfo().isOperatingSystemAtLeastVersion(ventura) {
        "Settings…"
    } else {
        "Preferences…"
    }
}

/// Take the items AppKit has fired since the last call, oldest first.
pub(crate) fn take_commands() -> Vec<Command> {
    COMMANDS.with(|commands| std::mem::take(&mut *commands.borrow_mut()))
}

/// Run the callback of the item tagged `tag` with `ctx` — unless the item is
/// disabled by now, which the menu showed it wasn't a moment ago.
pub(crate) fn fire(tag: isize, ctx: &mut EventCtx) {
    INSTALLED.with(|installed| {
        let mut installed = installed.borrow_mut();
        let Some(installed) = installed.as_mut() else {
            return;
        };
        if let Some(id) = installed.slot(tag).map(|slot| slot.id) {
            installed.bar.fire_item(id.menu, id.item, ctx);
        }
    });
}

/// Bring the items' key equivalents in line with the app's state: each item
/// of the app's own carries its key while it is enabled, and only then — see
/// the module's docs. `blocked` says whether a popup window of saudade's is
/// open over the main window, which disables the app's own items for as long
/// as it is. The runtime calls this after every event the widgets handle.
pub(crate) fn refresh(blocked: bool) {
    BLOCKED.set(blocked);
    INSTALLED.with(|installed| {
        let Ok(installed) = installed.try_borrow() else {
            return;
        };
        if let Some(installed) = installed.as_ref() {
            for slot in installed.slots.iter().filter(|s| !s.shown.get()) {
                slot.arm(installed.enabled(slot));
            }
        }
    });
}

/// Whether a bar is installed — nothing to [`refresh`] otherwise.
pub(crate) fn installed() -> bool {
    INSTALLED.with(|installed| installed.try_borrow().is_ok_and(|i| i.is_some()))
}

/// Take the bar back, as the app ends, so the app's state its callbacks hold
/// on to goes when the rest of the app does. The menus stay up, with the app's
/// items disabled.
pub(crate) fn take_bar() -> Option<MenuBar> {
    INSTALLED.with(|installed| installed.borrow_mut().take().map(|i| i.bar))
}
