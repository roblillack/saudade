//! The window around an app's root widget that holds its menu bar, where the
//! bar is drawn in the window: what [`App::with_menu_bar`](crate::App::with_menu_bar)
//! builds on Windows, X11 and Wayland, and what [`MenuBar::above`] hands out.

use crate::event::{Event, EventCtx};
use crate::geometry::Rect;
use crate::painter::Painter;
use crate::theme::Theme;
use crate::widget::{PopupRequest, Widget};
use crate::widgets::{Column, MenuBar};

/// Where the bar is in the column, and where the content beneath it is.
const BAR: usize = 0;
const CONTENT: usize = 1;

/// A menu bar across the top of the window and the app's content filling the
/// rest of it.
///
/// It is the `Column` an app would build for that itself —
/// `Column::new().add_fixed(bar, h).add_fill(content)` — so the bar's
/// mnemonics and accelerators, its popups, the keyboard starting out with the
/// content's first focusable widget and the layout following the window all
/// work the way they do there. Two things are added, which the app's own column
/// got from holding its dialogs beside the bar:
///
/// * While the content holds the pointer — a `Modal`, a `Dialog` or a
///   `FileDialog` open in it, or a dropdown — it gets every event, and the bar
///   gets none. The column would hit-test a click on the bar's strip to the
///   bar and open a menu behind the dialog, where an overlay of the column
///   holding the bar takes every event first.
/// * The window's close button reaches the content's `on_cancel`, which the
///   runtime only ever calls on the root widget.
pub(crate) struct MenuBarHost {
    column: Column,
    bar_height: i32,
}

impl MenuBarHost {
    pub(crate) fn new(bar: MenuBar, content: Box<dyn Widget>) -> Self {
        let bar_height = bar.natural_height();
        let mut column = Column::new();
        column.push_fixed(bar, bar_height);
        column.push_fill_boxed(content);
        Self { column, bar_height }
    }

    fn bar(&self) -> &dyn Widget {
        self.column
            .child(BAR)
            .expect("the bar is the column's first child")
    }

    fn content(&self) -> &dyn Widget {
        self.column
            .child(CONTENT)
            .expect("the content is the column's second child")
    }

    fn content_mut(&mut self) -> &mut (dyn Widget + 'static) {
        self.column
            .child_mut(CONTENT)
            .expect("the content is the column's second child")
    }

    /// Whether the content takes every event: it holds the pointer, and no
    /// menu of the bar's is open. The two can't both be true but for an
    /// instant — a click outside an open menu closes it — and the open menu is
    /// what the user sees, so it wins.
    fn content_holds_events(&self) -> bool {
        self.content().captures_pointer() && !self.bar().captures_pointer()
    }
}

impl Widget for MenuBarHost {
    fn bounds(&self) -> Rect {
        self.column.bounds()
    }

    fn paint(&mut self, painter: &mut Painter, theme: &Theme) {
        self.column.paint(painter, theme);
    }

    fn paint_overlay(&mut self, painter: &mut Painter, theme: &Theme) {
        self.column.paint_overlay(painter, theme);
    }

    fn event(&mut self, event: &Event, ctx: &mut EventCtx) {
        if self.content_holds_events() {
            self.content_mut().event(event, ctx);
        } else {
            self.column.event(event, ctx);
        }
    }

    fn on_cancel(&mut self, ctx: &mut EventCtx) {
        self.content_mut().on_cancel(ctx);
    }

    /// The column's own answer misses a content that holds the pointer
    /// without a press through the column — a dialog a menu item opened —
    /// which an overlay of the app's own column would have reported.
    fn captures_pointer(&self) -> bool {
        self.column.captures_pointer() || self.content().captures_pointer()
    }

    fn focusable(&self) -> bool {
        self.column.focusable()
    }

    fn set_focused(&mut self, focused: bool) {
        self.column.set_focused(focused);
    }

    fn layout(&mut self, bounds: Rect) {
        self.column.layout(bounds);
    }

    fn popup_request(&self) -> Option<PopupRequest> {
        self.column.popup_request()
    }

    fn collect_popups(&self, out: &mut Vec<PopupRequest>) {
        self.column.collect_popups(out);
    }

    fn focus_first(&mut self) -> bool {
        self.column.focus_first()
    }

    /// The bar fills its strip; whether the rest is covered is the content's
    /// to say, as long as it fills the slot it was given.
    fn paints_own_background(&self) -> bool {
        let b = self.column.bounds();
        let slot = Rect::new(
            b.x,
            b.y + self.bar_height,
            b.w,
            (b.h - self.bar_height).max(0),
        );
        let c = self.content().bounds();
        self.content().paints_own_background()
            && c.x <= slot.x
            && c.y <= slot.y
            && c.right() >= slot.right()
            && c.bottom() >= slot.bottom()
    }

    fn wants_ticks(&self) -> bool {
        self.column.wants_ticks()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::accel::{Accel, ModifierScheme};
    use crate::event::{Key, Modifiers, MouseButton, NamedKey};
    use crate::geometry::Point;
    use crate::mock::MockBackend;
    use crate::widget::PopupKind;
    use crate::widgets::{Dialog, Menu, MenuItem};
    use std::cell::{Cell, RefCell};
    use std::rc::Rc;

    /// What a [`Sensor`] has been through.
    struct Seen {
        bounds: Cell<Rect>,
        focused: Cell<bool>,
        keys: Cell<usize>,
        presses: Cell<usize>,
        cancelled: Cell<bool>,
    }

    impl Default for Seen {
        fn default() -> Self {
            Self {
                bounds: Cell::new(Rect::new(0, 0, 0, 0)),
                focused: Cell::default(),
                keys: Cell::default(),
                presses: Cell::default(),
                cancelled: Cell::default(),
            }
        }
    }

    /// A focusable leaf standing in for an app's content.
    struct Sensor(Rc<Seen>);

    impl Widget for Sensor {
        fn bounds(&self) -> Rect {
            self.0.bounds.get()
        }
        fn layout(&mut self, bounds: Rect) {
            self.0.bounds.set(bounds);
        }
        fn paint(&mut self, _: &mut Painter, _: &Theme) {}
        fn event(&mut self, event: &Event, _: &mut EventCtx) {
            match event {
                Event::KeyDown { .. } => self.0.keys.set(self.0.keys.get() + 1),
                Event::PointerDown { .. } => self.0.presses.set(self.0.presses.get() + 1),
                _ => {}
            }
        }
        fn focusable(&self) -> bool {
            true
        }
        fn set_focused(&mut self, focused: bool) {
            self.0.focused.set(focused);
        }
        fn on_cancel(&mut self, _: &mut EventCtx) {
            self.0.cancelled.set(true);
        }
    }

    /// A File menu with a Ctrl+N item that counts how often it fired, pinned
    /// to the PC scheme so the tests behave the same on a Mac.
    fn bar(height: i32, fired: Rc<Cell<usize>>) -> MenuBar {
        MenuBar::new(Rect::new(0, 0, 0, height))
            .with_scheme(ModifierScheme::Pc)
            .add_menu(Menu::new(
                "&File",
                vec![
                    MenuItem::action("&New", move |_| fired.set(fired.get() + 1))
                        .with_accel(Accel::primary('n')),
                ],
            ))
    }

    /// The bar above a [`Sensor`], started up as the runtime starts a window:
    /// rendered once, which lays it out, then focused.
    fn window(backend: &MockBackend) -> (MenuBarHost, Rc<Seen>, Rc<Cell<usize>>) {
        let seen = Rc::new(Seen::default());
        let fired = Rc::new(Cell::new(0));
        let mut host = MenuBarHost::new(bar(20, fired.clone()), Box::new(Sensor(seen.clone())));
        backend.render(&mut host);
        host.focus_first();
        (host, seen, fired)
    }

    fn key(key: Key, modifiers: Modifiers) -> Event {
        Event::KeyDown { key, modifiers }
    }

    fn ctrl() -> Modifiers {
        Modifiers {
            control: true,
            ..Modifiers::default()
        }
    }

    fn alt() -> Modifiers {
        Modifiers {
            alt: true,
            ..Modifiers::default()
        }
    }

    fn press(x: i32, y: i32) -> Event {
        Event::PointerDown {
            pos: Point::new(x, y),
            button: MouseButton::Left,
            modifiers: Modifiers::default(),
        }
    }

    #[test]
    fn the_bar_takes_its_height_across_the_top_and_the_content_the_rest() {
        let backend = MockBackend::new(300, 200);
        let (host, seen, _) = window(&backend);
        assert_eq!(host.bar().bounds(), Rect::new(0, 0, 300, 20));
        assert_eq!(seen.bounds.get(), Rect::new(0, 20, 300, 180));

        // And follows the window.
        let mut host = host;
        host.layout(Rect::new(0, 0, 400, 300));
        assert_eq!(host.bar().bounds(), Rect::new(0, 0, 400, 20));
        assert_eq!(seen.bounds.get(), Rect::new(0, 20, 400, 280));
    }

    #[test]
    fn a_bar_built_without_a_height_gets_the_default() {
        let seen = Rc::new(Seen::default());
        let mut host = MenuBarHost::new(
            bar(0, Rc::new(Cell::new(0))),
            Box::new(Sensor(seen.clone())),
        );
        host.layout(Rect::new(0, 0, 300, 200));
        assert_eq!(host.bar().bounds().h, DEFAULT_HEIGHT);
        assert_eq!(seen.bounds.get().y, DEFAULT_HEIGHT);
    }

    const DEFAULT_HEIGHT: i32 = 20;

    #[test]
    fn the_keyboard_starts_out_with_the_content() {
        let backend = MockBackend::new(300, 200);
        let (mut host, seen, fired) = window(&backend);
        assert!(seen.focused.get(), "the content has the focus, not the bar");
        backend.dispatch(&mut host, &key(Key::Char('x'), Modifiers::default()));
        assert_eq!(seen.keys.get(), 1, "a key reaches the content");
        assert_eq!(fired.get(), 0);
    }

    #[test]
    fn accelerators_and_mnemonics_work_while_the_content_has_the_keyboard() {
        let backend = MockBackend::new(300, 200);
        let (mut host, seen, fired) = window(&backend);
        let outcome = backend.dispatch(&mut host, &key(Key::Char('n'), ctrl()));
        assert_eq!(fired.get(), 1, "Ctrl+N fires File ▸ New");
        assert!(outcome.paint_requested);
        assert_eq!(seen.keys.get(), 0, "and goes no further");

        // Alt+F opens File, which then owns the keyboard …
        backend.dispatch(&mut host, &key(Key::Char('f'), alt()));
        assert!(host.bar().captures_pointer(), "Alt+F opens the File menu");
        backend.render(&mut host);
        let mut popups = Vec::new();
        host.collect_popups(&mut popups);
        assert_eq!(popups.len(), 1, "the menu gets a popup window");
        assert_eq!(popups[0].kind, PopupKind::Popup);
        assert!(
            !seen.focused.get(),
            "the content reads as unfocused while the menu is open"
        );
        backend.dispatch(&mut host, &key(Key::Char('x'), Modifiers::default()));
        assert_eq!(seen.keys.get(), 0, "the open menu keeps the keys");

        // … and N there picks New.
        backend.dispatch(&mut host, &key(Key::Char('n'), Modifiers::default()));
        assert_eq!(fired.get(), 2);
        backend.render(&mut host);
        assert!(seen.focused.get(), "the content has the keyboard back");
    }

    #[test]
    fn a_click_on_the_bar_opens_a_menu_and_one_below_reaches_the_content() {
        let backend = MockBackend::new(300, 200);
        let (mut host, seen, _) = window(&backend);
        backend.dispatch(&mut host, &press(100, 100));
        assert_eq!(seen.presses.get(), 1);
        backend.dispatch(&mut host, &press(5, 10));
        assert!(host.bar().captures_pointer(), "File opens");
        assert_eq!(seen.presses.get(), 1);
    }

    /// The runtime calls the root's `on_cancel` when the window's close button
    /// is clicked, and when something else ends the app (`exiting`) — an
    /// editor's last chance to save. With the bar above it, the app's own root
    /// is no longer the runtime's root, and a `Column` keeps `on_cancel` to
    /// itself; the host must pass it on.
    #[test]
    fn the_close_button_reaches_the_content() {
        let backend = MockBackend::new(300, 200);
        let (mut host, seen, _) = window(&backend);
        host.on_cancel(&mut EventCtx::new());
        assert!(seen.cancelled.get());

        // The same through the public way in, as an offscreen test builds it.
        let seen = Rc::new(Seen::default());
        let mut window = bar(20, Rc::new(Cell::new(0))).above(Sensor(seen.clone()));
        backend.render(&mut window);
        assert!(window.focus_first(), "the content's focus_first is asked");
        assert!(seen.focused.get());
        window.on_cancel(&mut EventCtx::new());
        assert!(seen.cancelled.get());
    }

    /// The content of an app with a dialog: a sensor, and a `Dialog` over it
    /// as an overlay of the content's own column, as the notepad has it.
    struct WithDialog {
        column: Column,
        dialog: Rc<RefCell<Dialog>>,
    }

    struct SharedDialog(Rc<RefCell<Dialog>>);

    impl Widget for SharedDialog {
        fn bounds(&self) -> Rect {
            self.0.borrow().bounds()
        }
        fn paint(&mut self, painter: &mut Painter, theme: &Theme) {
            self.0.borrow_mut().paint(painter, theme);
        }
        fn event(&mut self, event: &Event, ctx: &mut EventCtx) {
            self.0.borrow_mut().event(event, ctx);
        }
        fn captures_pointer(&self) -> bool {
            self.0.borrow().captures_pointer()
        }
        fn layout(&mut self, bounds: Rect) {
            self.0.borrow_mut().layout(bounds);
        }
        fn popup_request(&self) -> Option<PopupRequest> {
            self.0.borrow().popup_request()
        }
    }

    fn with_dialog(seen: Rc<Seen>) -> WithDialog {
        let dialog = Rc::new(RefCell::new(Dialog::new()));
        let column = Column::new()
            .add_fill(Sensor(seen))
            .add_overlay(SharedDialog(dialog.clone()));
        WithDialog { column, dialog }
    }

    #[test]
    fn the_bar_stays_shut_while_a_dialog_is_open_in_the_content() {
        let backend = MockBackend::new(300, 200);
        let seen = Rc::new(Seen::default());
        let fired = Rc::new(Cell::new(0));
        let content = with_dialog(seen.clone());
        let dialog = content.dialog.clone();
        let mut host = MenuBarHost::new(bar(20, fired.clone()), Box::new(content.column));
        backend.render(&mut host);
        host.focus_first();

        // A menu item opens the dialog — no click through the bar's column.
        dialog.borrow_mut().show_info("Note", "Hello");
        backend.render(&mut host);
        assert!(host.captures_pointer(), "the open dialog holds the pointer");

        backend.dispatch(&mut host, &press(5, 10));
        assert!(
            !host.bar().captures_pointer(),
            "a click on the bar's strip doesn't open a menu behind the dialog"
        );
        backend.dispatch(&mut host, &key(Key::Char('n'), ctrl()));
        backend.dispatch(&mut host, &key(Key::Char('f'), alt()));
        assert_eq!(fired.get(), 0, "nor do its accelerators fire");
        assert!(!host.bar().captures_pointer());

        // Escape closes the dialog, and the bar is back.
        backend.dispatch(
            &mut host,
            &key(Key::Named(NamedKey::Escape), Modifiers::default()),
        );
        assert!(!host.captures_pointer());
        backend.dispatch(&mut host, &key(Key::Char('n'), ctrl()));
        assert_eq!(fired.get(), 1);
    }

    #[test]
    fn a_column_content_hands_the_focus_to_its_widget() {
        // The notepad's case: the content is a column, and the widget the
        // keyboard goes to is inside it.
        let backend = MockBackend::new(300, 200);
        let seen = Rc::new(Seen::default());
        let content = Column::new().add_fill(Sensor(seen.clone()));
        let mut host = MenuBarHost::new(bar(20, Rc::new(Cell::new(0))), Box::new(content));
        backend.render(&mut host);
        host.focus_first();
        assert!(seen.focused.get());

        backend.dispatch(&mut host, &key(Key::Char('f'), alt()));
        backend.render(&mut host);
        assert!(!seen.focused.get(), "suspended while the menu is open");
        backend.dispatch(
            &mut host,
            &key(Key::Named(NamedKey::Escape), Modifiers::default()),
        );
        backend.render(&mut host);
        assert!(seen.focused.get(), "and back once it closes");
    }

    /// A content that covers every pixel it is given.
    struct Opaque(Rect);

    impl Widget for Opaque {
        fn bounds(&self) -> Rect {
            self.0
        }
        fn layout(&mut self, bounds: Rect) {
            self.0 = bounds;
        }
        fn paint(&mut self, _: &mut Painter, _: &Theme) {}
        fn paints_own_background(&self) -> bool {
            true
        }
    }

    #[test]
    fn it_paints_its_own_background_where_the_content_does() {
        let mut host = MenuBarHost::new(
            bar(20, Rc::new(Cell::new(0))),
            Box::new(Opaque(Rect::new(0, 0, 0, 0))),
        );
        host.layout(Rect::new(0, 0, 300, 200));
        assert!(host.paints_own_background());

        let mut host = MenuBarHost::new(
            bar(20, Rc::new(Cell::new(0))),
            Box::new(Sensor(Rc::new(Seen::default()))),
        );
        host.layout(Rect::new(0, 0, 300, 200));
        assert!(!host.paints_own_background());
    }

    #[test]
    fn above_draws_the_bar_in_an_offscreen_render() {
        // What a screenshot of an app built for `App::with_menu_bar` shows
        // everywhere but on a Mac: the bar's strip along the top.
        let theme = crate::theme::Theme::default();
        let mut window = bar(20, Rc::new(Cell::new(0))).above(Opaque(Rect::new(0, 0, 0, 0)));
        let snap = MockBackend::new(100, 60).render(&mut window);
        let row = |y: i32| &snap.pixels()[(y * snap.width()) as usize..][..snap.width() as usize];
        assert!(
            row(19).iter().all(|&px| px == theme.shadow.0),
            "the bar's bottom edge is its shadow line"
        );
    }
}
