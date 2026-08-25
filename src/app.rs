use std::num::NonZeroU32;
use std::path::PathBuf;
use std::rc::Rc;
use std::time::{Duration, Instant};

use winit::application::ApplicationHandler;
use winit::dpi::{LogicalSize, PhysicalPosition, PhysicalSize};
use winit::event::{ElementState, MouseButton as WinitMouseButton, MouseScrollDelta, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::keyboard::{Key as WKey, ModifiersKeyState, NamedKey as WNamedKey};
use winit::window::{CursorIcon, Window, WindowAttributes, WindowButtons, WindowId, WindowLevel};

// X11 platform extensions. winit 0.30's generic `with_parent_window` is
// not enough on X11 (it reparents into the main window, which then clips
// the popup to its bounds) and has no effect on Wayland (the backend
// still creates an `xdg_toplevel` instead of an `xdg_popup`). We use
// override-redirect + the DropdownMenu window type hint to get proper
// instant-popup behavior on X11. Wayland keeps the top-level fallback
// until winit adds real popup support.
#[cfg(all(unix, not(target_os = "macos")))]
use winit::platform::x11::{WindowAttributesExtX11, WindowType as XWindowType};
// macOS: `with_has_shadow` lives on the same kind of platform extension trait.
#[cfg(target_os = "macos")]
use winit::platform::macos::WindowAttributesExtMacOS;

use crate::background::BackgroundState;
use crate::event::{
    Cursor, DragData, Event, EventCtx, Key, Modifiers, MouseButton, NamedKey,
    SCROLL_PIXELS_PER_LINE, WHEEL_LINES_PER_DETENT,
};
use crate::font::{Font, FontSet};
use crate::geometry::{Point, Rect, Size};
use crate::painter::Painter;
use crate::theme::Theme;
use crate::widget::{PopupKind, PopupRequest, Widget};

pub struct WindowConfig {
    pub title: String,
    pub size: Size,
    pub resizable: bool,
    /// Smallest inner size the window may be resized to, in logical pixels.
    /// `None` leaves the lower bound to the window manager. Only meaningful for
    /// [`resizable`](Self::resizable) windows.
    pub min_size: Option<Size>,
}

impl WindowConfig {
    pub fn new(title: impl Into<String>, width: i32, height: i32) -> Self {
        Self {
            title: title.into(),
            size: Size::new(width, height),
            resizable: false,
            min_size: None,
        }
    }

    pub fn resizable(mut self, resizable: bool) -> Self {
        self.resizable = resizable;
        self
    }

    /// Constrain how small the user may drag the window. The window manager
    /// enforces the bound, so the layout never has to cope with sizes below it.
    pub fn min_size(mut self, width: i32, height: i32) -> Self {
        self.min_size = Some(Size::new(width, height));
        self
    }
}

/// Top-level entry point. Owns the window configuration, the theme, and the
/// root widget tree, and drives the winit event loop until the user closes the
/// window or a widget calls [`EventCtx::close`](crate::event::EventCtx::close).
pub struct App {
    window: WindowConfig,
    theme: Theme,
    root: Box<dyn Widget>,
}

impl App {
    pub fn new(window: WindowConfig, root: impl Widget + 'static) -> Self {
        Self {
            window,
            theme: Theme::default(),
            root: Box::new(root),
        }
    }

    pub fn with_theme(mut self, theme: Theme) -> Self {
        self.theme = theme;
        self
    }

    pub fn run(self) {
        #[cfg(all(unix, not(target_os = "macos")))]
        {
            if is_wayland_session() {
                crate::wayland::run(self);
                return;
            }
        }
        self.run_winit();
    }

    fn run_winit(self) {
        let event_loop = EventLoop::new().expect("saudade: failed to create event loop");
        event_loop.set_control_flow(ControlFlow::Wait);

        let mut handler = AppHandler::new(self);
        event_loop
            .run_app(&mut handler)
            .expect("saudade: event loop error");
    }

    #[cfg(all(unix, not(target_os = "macos")))]
    pub(crate) fn into_parts(self) -> (WindowConfig, Theme, Box<dyn Widget>) {
        (self.window, self.theme, self.root)
    }
}

#[cfg(all(unix, not(target_os = "macos")))]
fn is_wayland_session() -> bool {
    std::env::var_os("WAYLAND_DISPLAY")
        .map(|v| !v.is_empty())
        .unwrap_or(false)
}

/// All persistent runtime state. Constructed at startup; resources that
/// require an `ActiveEventLoop` (the main window, softbuffer context, the
/// optional popup window) are filled in on `resumed`.
struct AppHandler {
    // Static configuration:
    window_config: WindowConfig,
    design_size: Size,
    theme: Theme,
    root: Box<dyn Widget>,
    font: Option<Font>,
    serif_font: Option<Font>,
    mono_font: Option<Font>,

    // Resources created in `resumed`:
    main_win: Option<Rc<Window>>,
    main_id: Option<WindowId>,
    context: Option<softbuffer::Context<Rc<Window>>>,
    main_surface: Option<softbuffer::Surface<Rc<Window>, Rc<Window>>>,
    physical: PhysicalSize<u32>,
    /// Logical→physical scale factor, owned by the OS: adopted from
    /// [`Window::scale_factor`] at startup and refreshed only when the
    /// compositor reports a
    /// [`ScaleFactorChanged`](winit::event::WindowEvent::ScaleFactorChanged)
    /// event. Widget code cannot override it — a widget that wants to render
    /// at a different scale paints into a sub-region with
    /// [`Painter::with_scale`](crate::Painter::with_scale) instead.
    scale: f32,

    // Per-frame state:
    cursor: Option<Point>,
    /// Pointer shape currently applied to the main window. Tracked so we only
    /// call into winit (whose cursor is a sticky window attribute) when a move
    /// actually changes the shape. Reset toward [`Cursor::Default`] when no
    /// widget under the pointer asks for one.
    cursor_icon: Cursor,
    /// Mouse buttons currently held. While any is down an OS pointer grab is in
    /// effect, which on X11 keeps motion flowing past the window edge — see
    /// [`PointerButtons`] and the `CursorLeft` handling.
    buttons: PointerButtons,
    modifiers: Modifiers,
    needs_redraw: bool,
    /// Background pattern + color for the main window, toggled with the
    /// `p` / `c` debug keys. Popups/dialogs ignore it and stay white.
    bg: BackgroundState,
    /// Where the display is, in the root widget's logical coordinates —
    /// [`Painter::screen_area`], handed to every paint pass. Recomputed only
    /// when the window moves, resizes or changes DPI, since asking the platform
    /// costs a round trip and the answer is stable in between.
    screen: Option<Rect>,
    /// Stack of popup windows, outermost first. Usually empty or one entry (a
    /// menu / dropdown / dialog); a dropdown opened *inside* a dialog nests a
    /// second entry on top.
    popups: Vec<PopupWindow>,
    /// Last `Event::Tick` we dispatched. `None` until the first tick is
    /// fired. The runtime uses this to pace ticks while a widget
    /// reports `wants_ticks()`.
    last_tick: Option<Instant>,
    /// Latch set whenever a dispatch called [`EventCtx::request_tick`] — the
    /// *push* path to keep ticks flowing without any `wants_ticks()` forwarding.
    /// Cleared just before each tick we fire, so the tick handler re-arms it
    /// only while it still needs more (see [`Self::pump_ticks`]).
    tick_requested: bool,

    // File drag-and-drop coalescing. winit reports `HoveredFile` / `DroppedFile`
    // one path at a time with no "that's all the files" terminal event, so we
    // accumulate the paths arriving in an event burst and flush a single
    // `DragEnter` / `Drop` from `about_to_wait`, once the burst has settled.
    /// Paths gathered from `HoveredFile` events for the in-flight hover.
    drag_hovered: Vec<PathBuf>,
    /// Paths gathered from `DroppedFile` events, flushed as one `Drop`.
    drag_dropped: Vec<PathBuf>,
    /// `true` once we've emitted `DragEnter` for the current hover, so we don't
    /// re-announce it every idle iteration while the drag lingers.
    drag_active: bool,
    /// Tracks a key press a widget asked to swallow until release (via
    /// [`EventCtx::swallow_key_until_release`]).
    swallow: KeySwallow,
}

/// Bookkeeping for a key press a widget asked to swallow until its release.
///
/// A single physical press surfaces as separate `KeyDown` / `Char` / `KeyUp`
/// events (and OS autorepeat re-fires the first two). When a handler signals
/// [`EventCtx::swallow_key_until_release`], the runtime must drop every one of
/// those for the same key until the release — otherwise the trailing text leaks
/// past the handler (e.g. into a dialog the handler just opened).
///
/// Shared by both runtime backends — the winit `AppHandler` here and the
/// Wayland `State` — which run identical `KeyDown` / `Char` / `KeyUp` paths.
#[derive(Default)]
pub(crate) struct KeySwallow {
    /// The key currently being swallowed, or `None` when nothing is.
    key: Option<Key>,
}

impl KeySwallow {
    /// True if a fresh press of `mapped` should be dropped wholesale — it's an
    /// autorepeat of the key being swallowed.
    pub(crate) fn drops_press(&self, mapped: Option<Key>) -> bool {
        mapped.is_some() && mapped == self.key
    }

    /// Record that the press of `mapped` is now swallowed until release.
    pub(crate) fn begin(&mut self, mapped: Option<Key>) {
        self.key = mapped;
    }

    /// True if the release of `mapped` ends the swallowed press (also clearing
    /// it, so the key is handled normally again afterwards).
    pub(crate) fn ends_on_release(&mut self, mapped: Option<Key>) -> bool {
        if mapped.is_some() && mapped == self.key {
            self.key = None;
            true
        } else {
            false
        }
    }
}

/// Count of mouse buttons currently held down on the winit backend.
///
/// X11 keeps an implicit pointer grab from the first button press until the
/// last release; during it, motion events keep flowing to the grabbing window
/// even after the cursor leaves it. The runtime consults [`Self::any_held`] to
/// recognize that grab and ignore the `CursorLeft` winit reports across the
/// window edge mid-drag (see [`AppHandler::drag_grab_active`]).
///
/// A plain count tolerates a missing event (e.g. a release delivered to another
/// window) without going negative; pairing it with an active-capture check at
/// the call site means a drifted count can never wrongly swallow a real leave.
#[derive(Default)]
struct PointerButtons {
    held: u32,
}

impl PointerButtons {
    /// Record a button press.
    fn press(&mut self) {
        self.held += 1;
    }

    /// Record a button release.
    fn release(&mut self) {
        self.held = self.held.saturating_sub(1);
    }

    /// True while at least one button is down — i.e. an OS pointer grab is
    /// keeping motion events flowing regardless of the cursor's position.
    fn any_held(&self) -> bool {
        self.held > 0
    }
}

/// Target interval between [`Event::Tick`](crate::event::Event::Tick)
/// dispatches when a widget is animating — roughly 60 Hz.
const TICK_INTERVAL: Duration = Duration::from_millis(16);

impl AppHandler {
    fn new(app: App) -> Self {
        let design_size = app.window.size;
        Self {
            window_config: app.window,
            design_size,
            theme: app.theme,
            root: app.root,
            font: Font::load_sans(),
            serif_font: Font::load_serif(),
            mono_font: Font::load_monospace(),
            main_win: None,
            main_id: None,
            context: None,
            main_surface: None,
            physical: PhysicalSize::new(0, 0),
            scale: 1.0,
            cursor: None,
            cursor_icon: Cursor::Default,
            buttons: PointerButtons::default(),
            modifiers: Modifiers::default(),
            needs_redraw: true,
            bg: BackgroundState::from_env(),
            screen: None,
            popups: Vec::new(),
            last_tick: None,
            tick_requested: false,
            drag_hovered: Vec::new(),
            drag_dropped: Vec::new(),
            drag_active: false,
            swallow: KeySwallow::default(),
        }
    }
}

impl ApplicationHandler for AppHandler {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.main_win.is_some() {
            return; // already initialized; ignore redundant resumes
        }
        let mut attrs = WindowAttributes::default()
            .with_title(&self.window_config.title)
            .with_inner_size(LogicalSize::new(
                self.window_config.size.w as f64,
                self.window_config.size.h as f64,
            ))
            .with_resizable(self.window_config.resizable);
        if let Some(min) = self.window_config.min_size {
            attrs = attrs.with_min_inner_size(LogicalSize::new(min.w as f64, min.h as f64));
        }
        let win = event_loop
            .create_window(attrs)
            .expect("saudade: failed to create window");
        let win = Rc::new(win);
        let id = win.id();

        let context = softbuffer::Context::new(win.clone())
            .expect("saudade: failed to create softbuffer context");
        let mut surface = softbuffer::Surface::new(&context, win.clone())
            .expect("saudade: failed to create softbuffer surface");

        self.physical = win.inner_size();
        self.scale = win.scale_factor() as f32;
        resize_surface(&mut surface, self.physical);
        relayout(&mut self.root, self.physical, self.scale, self.design_size);
        // Give the first focusable widget in the tree keyboard focus by
        // default so apps with a clear "primary" target (a text editor, a
        // list) accept typing / arrow keys without manual setup.
        self.root.focus_first();

        self.main_win = Some(win);
        self.main_id = Some(id);
        self.context = Some(context);
        self.main_surface = Some(surface);
        self.refresh_screen();
        self.needs_redraw = true;
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        window_id: WindowId,
        event: WindowEvent,
    ) {
        if Some(window_id) == self.main_id {
            self.handle_main_event(event, event_loop);
        } else if let Some(idx) = self.popups.iter().position(|p| p.win_id == window_id) {
            self.handle_popup_event(idx, event, event_loop);
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        self.flush_file_drag(event_loop);
        self.sync_popup(event_loop);
        self.pump_ticks(event_loop);

        if self.needs_redraw
            && let Some(win) = self.main_win.as_ref()
        {
            win.request_redraw();
            self.needs_redraw = false;
        }
        for p in &mut self.popups {
            if p.needs_redraw {
                p.win.request_redraw();
                p.needs_redraw = false;
            }
        }
    }

    fn exiting(&mut self, _event_loop: &ActiveEventLoop) {
        // Tear the windows (and the softbuffer surfaces holding `Rc<Window>`
        // clones) down here, while the event loop is still alive. On macOS a
        // winit `Window` dropped *after* `run_app` returns calls into a
        // by-then-deconfigured `NSApplication` delegate and panics inside
        // `Window::drop` — and a panic in `drop` can't unwind, so the process
        // aborts. Releasing every `Rc<Window>` now drops the windows while the
        // delegate is still configured. Order: surfaces/context before the
        // window handles, since they hold the other `Rc` clones.
        self.popups.clear();
        self.main_surface = None;
        self.context = None;
        self.main_win = None;
    }
}

impl AppHandler {
    fn handle_main_event(&mut self, event: WindowEvent, event_loop: &ActiveEventLoop) {
        match event {
            WindowEvent::CloseRequested => {
                // Give the root a chance to react before we quit: a widget used
                // directly as the window root (rather than hosted in a `Modal`)
                // gets its `on_cancel` here, so a dialog-as-window can revert
                // pending edits on close, just as Escape or a `Modal`'s close
                // would. Most roots leave this a no-op.
                let mut ctx = EventCtx::new();
                self.root.on_cancel(&mut ctx);
                event_loop.exit();
            }
            WindowEvent::Moved(_) => {
                // The main window changed screen position. Override-redirect
                // popups are unmanaged top-level windows that don't follow
                // their "parent", so we have to reposition them manually
                // each time the main window moves.
                self.reposition_popup();
                // The screen rect is relative to the window, and the window may
                // even have crossed onto another display.
                self.refresh_screen();
            }
            // The main window took the keyboard back while a menu was up, so
            // something outside the menu was clicked — the title bar, most
            // often, on the way to dragging the window. Native menus don't
            // survive that.
            WindowEvent::Focused(true) => {
                self.dismiss_transient_popup(event_loop);
            }
            WindowEvent::Resized(new_size) => {
                self.physical = new_size;
                if let Some(s) = self.main_surface.as_mut() {
                    resize_surface(s, self.physical);
                }
                relayout(&mut self.root, self.physical, self.scale, self.design_size);
                // Dragging the top or left edge resizes *and* moves the window.
                self.refresh_screen();
                self.needs_redraw = true;
            }
            WindowEvent::ScaleFactorChanged { scale_factor, .. } => {
                self.scale = scale_factor as f32;
                if let Some(win) = self.main_win.as_ref() {
                    self.physical = win.inner_size();
                }
                if let Some(s) = self.main_surface.as_mut() {
                    resize_surface(s, self.physical);
                }
                relayout(&mut self.root, self.physical, self.scale, self.design_size);
                // The screen rect is in logical units, so a new scale rescales
                // it even when the display is the same one.
                self.refresh_screen();
                self.needs_redraw = true;
            }
            WindowEvent::CursorMoved { position, .. } => {
                let content = self.root.bounds().into();
                let (origin_x, origin_y) = origin(content, self.scale, self.physical);
                let pos = physical_to_logical(position, self.scale, origin_x, origin_y);
                self.cursor = Some(pos);
                let ctx = self.dispatch(&Event::PointerMove { pos }, event_loop);
                // The widget the pointer now rests over picks the shape; an
                // unanswered move falls back to the arrow.
                if let Some(win) = self.main_win.clone() {
                    reconcile_cursor(&win, &mut self.cursor_icon, ctx.cursor_request);
                }
            }
            WindowEvent::CursorLeft { .. } => {
                // Ignore the leave while a button-held drag owns the pointer:
                // the OS grab keeps motion flowing past the window edge, so the
                // drag genuinely continues even though winit reports a leave
                // here (see `drag_grab_active`). Honoring it would cut a
                // scrollbar / slider thumb drag short.
                if self.drag_grab_active() {
                    return;
                }
                self.cursor = None;
                self.dispatch(&Event::PointerLeave, event_loop);
                // Back to the arrow so a stale hand / I-beam doesn't linger if
                // the pointer re-enters over empty space.
                if let Some(win) = self.main_win.clone() {
                    reconcile_cursor(&win, &mut self.cursor_icon, Some(Cursor::Default));
                }
            }
            WindowEvent::MouseInput {
                state,
                button: winit_button,
                ..
            } => {
                let Some(button) = map_button(winit_button) else {
                    return;
                };
                // Track the OS pointer grab before anything can bail out, so the
                // held-button count stays balanced regardless of cursor state.
                match state {
                    ElementState::Pressed => self.buttons.press(),
                    ElementState::Released => self.buttons.release(),
                }
                let Some(pos) = self.cursor else { return };
                let modifiers = self.modifiers;
                let event = match state {
                    ElementState::Pressed => Event::PointerDown {
                        pos,
                        button,
                        modifiers,
                    },
                    ElementState::Released => Event::PointerUp {
                        pos,
                        button,
                        modifiers,
                    },
                };
                self.dispatch(&event, event_loop);
            }
            WindowEvent::MouseWheel { delta, .. } => {
                // The wheel carries no position; use the last cursor location.
                let Some(pos) = self.cursor else { return };
                let (delta_x, delta_y) = scroll_delta_lines(delta, self.scale);
                self.dispatch(
                    &Event::Scroll {
                        pos,
                        delta_x,
                        delta_y,
                    },
                    event_loop,
                );
            }
            // File drag-and-drop. winit delivers these one path at a time and
            // without a cursor position, so we only buffer here; the actual
            // `DragEnter` / `DragLeave` / `Drop` events are synthesized and
            // coalesced in `flush_file_drag`, called from `about_to_wait`.
            WindowEvent::HoveredFile(path) => {
                self.drag_hovered.push(path);
            }
            WindowEvent::HoveredFileCancelled => {
                if self.drag_active {
                    self.dispatch(&Event::DragLeave, event_loop);
                }
                self.drag_hovered.clear();
                self.drag_active = false;
            }
            WindowEvent::DroppedFile(path) => {
                self.drag_dropped.push(path);
            }
            WindowEvent::ModifiersChanged(new_mods) => {
                let s = new_mods.state();
                self.modifiers = Modifiers {
                    shift: s.shift_key(),
                    control: s.control_key(),
                    alt: s.alt_key(),
                    // Right Alt / Option = AltGr: reserved for composing
                    // characters, so it must not trigger menu mnemonics.
                    alt_graph: new_mods.ralt_state() == ModifiersKeyState::Pressed,
                    logo: s.super_key(),
                };
            }
            WindowEvent::KeyboardInput { event: key, .. } => {
                self.dispatch_key(&key, event_loop);
            }
            WindowEvent::RedrawRequested => {
                self.paint_main();
                self.mark_popups_dirty();
            }
            _ => {}
        }
    }

    fn handle_popup_event(&mut self, idx: usize, event: WindowEvent, event_loop: &ActiveEventLoop) {
        match event {
            WindowEvent::CloseRequested => {
                self.dismiss_via_escape(event_loop);
            }
            // The menu itself lost the keyboard: the user switched to another
            // app, or to another window of this one. Only the popup on top of
            // the stack speaks for the stack — a *parent* popup loses focus for
            // an innocent reason, namely a nested popup of its own opening
            // above it (a dropdown inside a dialog).
            WindowEvent::Focused(focused) => {
                let Some(p) = self.popups.get_mut(idx) else {
                    return;
                };
                if focused {
                    p.was_focused = true;
                    return;
                }
                // A brand-new popup window reports one unfocused *before* it is
                // given the keyboard, so a loss only means anything once it has
                // actually held it.
                if p.was_focused && idx + 1 == self.popups.len() {
                    self.dismiss_transient_popup(event_loop);
                }
            }
            WindowEvent::Moved(_) => {
                // A managed popup (typically a dialog) was dragged by the
                // user / WM. Override-redirect popups nested inside it
                // (e.g. an open dropdown) need to follow.
                self.reposition_popup();
            }
            WindowEvent::Resized(new_size) => {
                if let Some(p) = self.popups.get_mut(idx) {
                    p.physical = new_size;
                    resize_surface(&mut p.surface, new_size);
                    p.needs_redraw = true;
                }
            }
            WindowEvent::CursorMoved { position, .. } => {
                let Some(p) = self.popups.get_mut(idx) else {
                    return;
                };
                let pos = popup_position_to_widget(position, p);
                p.cursor = Some(pos);
                let ctx = self.dispatch(&Event::PointerMove { pos }, event_loop);
                if let Some(p) = self.popups.get_mut(idx) {
                    let win = p.win.clone();
                    reconcile_cursor(&win, &mut p.cursor_icon, ctx.cursor_request);
                }
                self.mark_popups_dirty();
            }
            WindowEvent::CursorLeft { .. } => {
                // As in the main window: keep a button-held drag alive across
                // the popup edge instead of cutting it short on the spurious
                // winit leave.
                if self.drag_grab_active() {
                    return;
                }
                if let Some(p) = self.popups.get_mut(idx) {
                    p.cursor = None;
                }
                self.dispatch(&Event::PointerLeave, event_loop);
                if let Some(p) = self.popups.get_mut(idx) {
                    let win = p.win.clone();
                    reconcile_cursor(&win, &mut p.cursor_icon, Some(Cursor::Default));
                }
                self.mark_popups_dirty();
            }
            WindowEvent::MouseInput {
                state,
                button: winit_button,
                ..
            } => {
                let Some(button) = map_button(winit_button) else {
                    return;
                };
                match state {
                    ElementState::Pressed => self.buttons.press(),
                    ElementState::Released => self.buttons.release(),
                }
                let Some(pos) = self.popups.get(idx).and_then(|p| p.cursor) else {
                    return;
                };
                let modifiers = self.modifiers;
                let event = match state {
                    ElementState::Pressed => Event::PointerDown {
                        pos,
                        button,
                        modifiers,
                    },
                    ElementState::Released => Event::PointerUp {
                        pos,
                        button,
                        modifiers,
                    },
                };
                self.dispatch(&event, event_loop);
                self.mark_popups_dirty();
            }
            WindowEvent::ModifiersChanged(new_mods) => {
                let s = new_mods.state();
                self.modifiers = Modifiers {
                    shift: s.shift_key(),
                    control: s.control_key(),
                    alt: s.alt_key(),
                    // Right Alt / Option = AltGr: reserved for composing
                    // characters, so it must not trigger menu mnemonics.
                    alt_graph: new_mods.ralt_state() == ModifiersKeyState::Pressed,
                    logo: s.super_key(),
                };
            }
            WindowEvent::KeyboardInput { event: key, .. } => {
                self.dispatch_key(&key, event_loop);
                self.mark_popups_dirty();
            }
            WindowEvent::RedrawRequested => {
                self.paint_popup(idx);
            }
            _ => {}
        }
    }

    /// Mark every popup window dirty. Cheap, and the right thing after any
    /// dispatch: a single event can change widgets shown in several popups at
    /// once (e.g. closing a nested dropdown repaints the dialog beneath it).
    fn mark_popups_dirty(&mut self) {
        for p in &mut self.popups {
            p.needs_redraw = true;
        }
    }

    /// Whether an OS pointer grab is currently keeping a widget's drag alive
    /// past the window edge — a mouse button is held *and* a widget is
    /// capturing the pointer from that press.
    ///
    /// This is the signal to ignore winit's `CursorLeft`: on X11 the pointer is
    /// implicitly grabbed for the lifetime of a button press, during which
    /// motion keeps being delivered even after the cursor crosses the window
    /// edge — but winit still reports that crossing as a leave (an `XI_Leave`
    /// with mode `WhileGrabbed`, which it does not filter out). Treating it as a
    /// real leave would end a scrollbar / slider thumb drag the moment the
    /// cursor stepped outside. Wayland's compositor sends no leave at all during
    /// its own implicit grab, so suppressing the winit one makes the backends
    /// agree. Requiring an active capture keeps a popup-held capture (an open
    /// menu / dropdown the user navigates without holding a button) still
    /// receiving its leave.
    fn drag_grab_active(&self) -> bool {
        self.buttons.any_held() && self.root.captures_pointer()
    }

    fn dispatch(&mut self, event: &Event, event_loop: &ActiveEventLoop) -> EventCtx {
        let mut ctx = EventCtx::new();
        self.root.event(event, &mut ctx);
        if ctx.paint_requested {
            self.needs_redraw = true;
            self.mark_popups_dirty();
        }
        // Push-based tick keep-alive: any widget that asked for another tick
        // (e.g. a scrollbar auto-repeating while held) latches it here, no
        // matter how deeply it's nested — no `wants_ticks()` forwarding needed.
        if ctx.tick_requested {
            self.tick_requested = true;
        }
        if ctx.close_requested {
            event_loop.exit();
        }
        if let Some(size) = ctx.resize_request {
            self.apply_resize(size);
        }
        if ctx.drag_request.is_some() {
            // A widget asked to start an outbound drag. winit exposes no API to
            // initiate a drag-and-drop operation on any of its platforms
            // (macOS, Windows, X11), so we can only drop the request here —
            // drag *sources* are a Wayland-only capability (see
            // `EventCtx::start_drag`). Receiving drops still works everywhere.
        }
        // A drop target may have called `accept_drop`, but the OS has already
        // committed to offering the file drag on the winit backends — we can't
        // refuse it the way Wayland can — so the flag is advisory here and the
        // `Drop` is delivered regardless.
        let _ = ctx.accepts_drop;
        ctx
    }

    /// Turn the file paths buffered from `HoveredFile` / `DroppedFile` events
    /// into coalesced [`Event::DragEnter`] / [`Event::Drop`] events. Called once
    /// per loop iteration from `about_to_wait`, after the event burst that
    /// delivered the individual paths has settled, so all files from one drag
    /// arrive in a single event.
    ///
    /// winit carries no cursor position with these events, so the drop point is
    /// the last in-window pointer location (`self.cursor`) — exact on platforms
    /// that move the cursor during a drag, stale otherwise. Defaults to the
    /// origin when the pointer has never entered the window.
    fn flush_file_drag(&mut self, event_loop: &ActiveEventLoop) {
        if self.drag_hovered.is_empty() && self.drag_dropped.is_empty() {
            return;
        }
        let pos = self.cursor.unwrap_or(Point::new(0, 0));

        if !self.drag_active && !self.drag_hovered.is_empty() {
            self.drag_active = true;
            self.drag_hovered.clear();
            self.dispatch(&Event::DragEnter { pos }, event_loop);
        }

        if !self.drag_dropped.is_empty() {
            let data = DragData::from_paths(std::mem::take(&mut self.drag_dropped));
            self.dispatch(&Event::Drop { pos, data }, event_loop);
            self.drag_hovered.clear();
            self.drag_active = false;
        }
    }

    /// Resize the window to `size` logical pixels, at the widget's request.
    /// winit applies the new inner size either synchronously (returning the new
    /// physical size, which we adopt right away) or by emitting a later
    /// `Resized` event — which runs the same surface-resize + relayout path.
    fn apply_resize(&mut self, size: Size) {
        let Some(win) = self.main_win.clone() else {
            return;
        };
        let requested = LogicalSize::new(size.w as f64, size.h as f64);
        if let Some(new_phys) = win.request_inner_size(requested) {
            self.physical = new_phys;
            if let Some(s) = self.main_surface.as_mut() {
                resize_surface(s, new_phys);
            }
            relayout(&mut self.root, self.physical, self.scale, self.design_size);
            self.needs_redraw = true;
        }
    }

    /// Drive animation ticks: if any widget in the tree wants ticks,
    /// dispatch [`Event::Tick`] when the interval has elapsed and ask
    /// the event loop to wake us up again at the next deadline.
    /// Otherwise revert to a plain `Wait` so the process stays idle
    /// when nothing is animating.
    fn pump_ticks(&mut self, event_loop: &ActiveEventLoop) {
        // Two independent reasons to keep the clock running: a widget that
        // *pulls* ticks via `wants_ticks()` (steady-state, e.g. a blinking
        // caret), or one that *pushed* a one-shot request via
        // `EventCtx::request_tick` (transient, e.g. a held scrollbar arrow).
        if !self.root.wants_ticks() && !self.tick_requested {
            self.last_tick = None;
            event_loop.set_control_flow(ControlFlow::Wait);
            return;
        }
        let now = Instant::now();
        let due = match self.last_tick {
            None => true,
            Some(prev) => now.duration_since(prev) >= TICK_INTERVAL,
        };
        if due {
            self.last_tick = Some(now);
            // Clear the push latch *before* dispatching: the tick handler
            // re-arms it (via `request_tick`) only if it still wants more, so a
            // widget that's done lets the ticks stop on their own.
            self.tick_requested = false;
            self.dispatch(&Event::Tick, event_loop);
        }
        let next = self.last_tick.unwrap_or(now) + TICK_INTERVAL;
        event_loop.set_control_flow(ControlFlow::WaitUntil(next));
    }

    fn dispatch_key(&mut self, key: &winit::event::KeyEvent, event_loop: &ActiveEventLoop) {
        let mapped = map_base_key(key);
        match key.state {
            ElementState::Pressed => {
                // A key whose press a widget asked to swallow (e.g. the letter
                // that fired a menu item) is dropped wholesale — including OS
                // autorepeat — until its release, so neither the `KeyDown` nor
                // the text it produces reaches the tree.
                if self.swallow.drops_press(mapped) {
                    return;
                }
                if let Some(m) = mapped {
                    let ctx = self.dispatch(
                        &Event::KeyDown {
                            key: m,
                            modifiers: self.modifiers,
                        },
                        event_loop,
                    );
                    if ctx.swallow_key {
                        // The handler fully owns this press; drop its trailing
                        // text now and everything up to the release.
                        self.swallow.begin(mapped);
                        return;
                    }
                }
                if !self.modifiers.has_command()
                    && let Some(text) = key.text.as_deref()
                {
                    for ch in text.chars() {
                        if (ch.is_control() && ch != '\t' && ch != '\n') || ch == '\r' {
                            continue;
                        }
                        let ctx = self.dispatch(
                            &Event::Char {
                                ch,
                                modifiers: self.modifiers,
                            },
                            event_loop,
                        );
                        if ctx.swallow_key {
                            // A handler fired on the text itself (a mnemonic
                            // routed through `Char` on this platform); swallow
                            // the rest of the press too.
                            self.swallow.begin(mapped);
                            return;
                        }
                    }
                }
            }
            ElementState::Released => {
                // The release that ends a swallowed press: consume it and arm
                // the key for normal handling again.
                if self.swallow.ends_on_release(mapped) {
                    return;
                }
                if let Some(m) = mapped {
                    self.dispatch(
                        &Event::KeyUp {
                            key: m,
                            modifiers: self.modifiers,
                        },
                        event_loop,
                    );
                }
            }
        }
    }

    /// Fold away a transient popup — a menu, a dropdown list — because
    /// something outside it took over the keyboard. A menu is a modal gesture
    /// that belongs to the click that opened it; clicking a title bar or
    /// switching apps ends the gesture, and every desktop's own menus close on
    /// it.
    ///
    /// A [`PopupKind::Dialog`] is *not* transient — it is a window in its own
    /// right and stays put — so this only acts while a [`PopupKind::Popup`] is
    /// on top. It dismisses by way of Escape, which the widget owning that
    /// popup consumes: that is what lets a dropdown open inside a dialog close
    /// without taking the dialog with it.
    fn dismiss_transient_popup(&mut self, event_loop: &ActiveEventLoop) {
        if self
            .popups
            .last()
            .is_some_and(|p| p.kind == PopupKind::Popup)
        {
            self.dismiss_via_escape(event_loop);
        }
    }

    fn dismiss_via_escape(&mut self, event_loop: &ActiveEventLoop) {
        let mods = self.modifiers;
        self.dispatch(
            &Event::KeyDown {
                key: Key::Named(NamedKey::Escape),
                modifiers: mods,
            },
            event_loop,
        );
    }

    fn paint_main(&mut self) {
        let content = self.root.bounds().into();
        let (origin_x, origin_y) = origin(content, self.scale, self.physical);
        let Some(surface) = self.main_surface.as_mut() else {
            return;
        };
        let mut surface_buf = surface
            .buffer_mut()
            .expect("saudade: failed to acquire surface buffer");
        let mut painter = Painter::with_popup_anchor(
            &mut surface_buf,
            self.physical.width as i32,
            self.physical.height as i32,
            self.scale,
            origin_x,
            origin_y,
            FontSet {
                sans: self.font.as_ref(),
                serif: self.serif_font.as_ref(),
                mono: self.mono_font.as_ref(),
            },
            None,
        )
        .with_screen(self.screen);
        // A root that paints its own background covers the pattern anyway, so
        // only the letterbox around it is worth laying down — usually nothing.
        if self.root.paints_own_background() {
            painter.fill_pattern_around(
                self.theme.background,
                self.bg.pattern,
                self.bg.color,
                self.root.bounds(),
            );
        } else {
            painter.fill_pattern(self.theme.background, self.bg.pattern, self.bg.color);
        }
        self.root.paint(&mut painter, &self.theme);
        surface_buf
            .present()
            .expect("saudade: failed to present buffer");
    }

    fn paint_popup(&mut self, idx: usize) {
        let Some(p) = self.popups.get_mut(idx) else {
            return;
        };
        let origin_x = -((p.anchor.x as f32 * p.scale).round() as i32);
        let origin_y = -((p.anchor.y as f32 * p.scale).round() as i32);
        let popup_phys_w = (p.anchor.w as f32 * p.scale).round() as i32;
        let popup_phys_h = (p.anchor.h as f32 * p.scale).round() as i32;
        let anchor = p.anchor;
        let screen = self.screen;
        let mut surface_buf = p
            .surface
            .buffer_mut()
            .expect("saudade: failed to acquire popup buffer");
        let mut painter = Painter::with_popup_anchor(
            &mut surface_buf,
            p.physical.width as i32,
            p.physical.height as i32,
            p.scale,
            origin_x,
            origin_y,
            FontSet {
                sans: self.font.as_ref(),
                serif: self.serif_font.as_ref(),
                mono: self.mono_font.as_ref(),
            },
            Some(anchor),
        )
        .with_screen(screen);
        painter.fill(self.theme.background);
        painter.set_clip_phys(0, 0, popup_phys_w, popup_phys_h);
        self.root.paint(&mut painter, &self.theme);
        painter.clear_clip();
        surface_buf
            .present()
            .expect("saudade: failed to present popup buffer");
    }

    /// Recompute [`Self::screen`]: the usable area of the display the window is
    /// on, expressed in the root widget's logical coordinates — the same space
    /// a [`PopupRequest`] is in, i.e. measured from the window's inner top-left
    /// corner. Widgets that open a top-level window of their own read it back
    /// through [`Painter::screen_area`] to keep that window on screen.
    fn refresh_screen(&mut self) {
        self.screen = self.compute_screen();
    }

    fn compute_screen(&self) -> Option<Rect> {
        let win = self.main_win.as_ref()?;
        let monitor = win.current_monitor()?;
        let inner = win.inner_position().ok()?;
        let origin = monitor.position();
        let size = monitor.size();
        let s = self.scale.max(0.01);
        let logical = |v: i32| (v as f32 / s).round() as i32;
        // The desktop reserves some of the display for its own furniture — the
        // macOS menu bar and Dock. A menu clamped over the Dock would hide its
        // own last row, and with it the arrow saying there is more.
        let (left, top, right, bottom) = desktop_insets(win);
        Some(Rect::new(
            logical(origin.x - inner.x) + left,
            logical(origin.y - inner.y) + top,
            logical(size.width as i32) - left - right,
            logical(size.height as i32) - top - bottom,
        ))
    }

    /// Re-anchor every override-redirect popup to follow the window above
    /// it in the stack (the main window for the outermost popup, the
    /// dialog above for a nested dropdown). Called when the main window —
    /// or a popup window — moves, since override-redirect popups are
    /// unmanaged top-levels that the WM doesn't drag along. Managed
    /// dialogs (`PopupKind::Dialog`) keep their own screen position.
    fn reposition_popup(&mut self) {
        let Some(main_win) = self.main_win.as_ref() else {
            return;
        };
        for idx in 0..self.popups.len() {
            let popup = &self.popups[idx];
            if popup.kind != PopupKind::Popup {
                continue;
            }
            // Outermost popup anchors to the main window; nested popups
            // anchor to the popup directly above them.
            let (parent_inner, parent_origin) = if idx == 0 {
                (main_win.inner_position(), Rect::new(0, 0, 0, 0))
            } else {
                let parent = &self.popups[idx - 1];
                (parent.win.inner_position(), parent.anchor)
            };
            let Ok(inner) = parent_inner else { continue };
            let dx = (popup.anchor.x - parent_origin.x) as f32;
            let dy = (popup.anchor.y - parent_origin.y) as f32;
            let px = inner.x + (dx * self.scale).round() as i32;
            let py = inner.y + (dy * self.scale).round() as i32;
            popup.win.set_outer_position(PhysicalPosition::new(px, py));
        }
    }

    fn sync_popup(&mut self, event_loop: &ActiveEventLoop) {
        let mut requests = Vec::new();
        self.root.collect_popups(&mut requests);

        // Keep the longest prefix of existing popup windows that still matches
        // the requested stack (same anchor + kind). The first mismatch — and
        // everything above it — is torn down and rebuilt, so opening a nested
        // dropdown adds a window without disturbing the dialog beneath it, and a
        // menu sliding to a new anchor rebuilds just that one.
        let keep = self
            .popups
            .iter()
            .zip(requests.iter())
            .take_while(|(p, req)| p.anchor == req.rect && p.kind == req.kind)
            .count();
        self.popups.truncate(keep);
        for req in requests.into_iter().skip(keep) {
            // Nested popups anchor against the popup *above* them in the
            // stack (e.g. a dropdown opened inside a dialog tracks the
            // dialog's actual screen position, not the main window's —
            // otherwise the WM moving the dialog leaves the dropdown
            // floating over the parent window). The outermost popup keeps
            // anchoring to the main window.
            let parent = self.popups.last();
            match self.open_popup(req, parent, event_loop) {
                Some(p) => self.popups.push(p),
                // If a popup can't be created, stop — a child popup must not
                // outlive a missing parent.
                None => break,
            }
        }
    }

    fn open_popup(
        &self,
        request: PopupRequest,
        parent: Option<&PopupWindow>,
        event_loop: &ActiveEventLoop,
    ) -> Option<PopupWindow> {
        let main_win = self.main_win.as_ref()?;
        let context = self.context.as_ref()?;

        let rect = request.rect;
        let phys_w = ((rect.w as f32) * self.scale).round().max(1.0) as u32;
        let phys_h = ((rect.h as f32) * self.scale).round().max(1.0) as u32;
        let size = PhysicalSize::new(phys_w, phys_h);

        let mut attrs = WindowAttributes::default()
            .with_resizable(false)
            .with_inner_size(size)
            .with_visible(false);

        match request.kind {
            PopupKind::Popup => {
                attrs = attrs
                    .with_title("saudade popup")
                    .with_decorations(false)
                    .with_transparent(true)
                    .with_window_level(WindowLevel::AlwaysOnTop);

                // macOS: no system drop shadow. A menu panel draws its own
                // sharp L-shape shadow, and the compositor's soft halo around
                // the window reads as a second, blurry frame outside the
                // popup's black border.
                #[cfg(target_os = "macos")]
                {
                    attrs = attrs.with_has_shadow(false)
                }

                // X11: take the WM completely out of the loop.
                // override-redirect makes this an unmanaged window — it
                // sits at the exact screen position requested, at the
                // exact requested size, and may extend past the main
                // window's bounds. The DropdownMenu type hint helps WMs
                // route it (e.g., place above the main window in
                // stacking order). These attributes are silently ignored
                // on other backends.
                #[cfg(all(unix, not(target_os = "macos")))]
                {
                    attrs = attrs
                        .with_override_redirect(true)
                        .with_x11_window_type(vec![XWindowType::DropdownMenu]);
                }

                // Absolute screen position = anchor window inner position
                // + popup offset, where the anchor is the parent popup
                // (for a nested popup) or the main window otherwise.
                // Popup rects are in *root* coords as they would be if the
                // parent were at its own anchor — translate by the parent
                // anchor's origin so the offset is parent-local. On X11
                // with override-redirect this is honored exactly. On
                // Wayland (no popup support yet) winit creates a top-level
                // window and the compositor places it on its own — the
                // position request is ignored.
                let anchor_inner = parent
                    .map(|p| p.win.inner_position())
                    .unwrap_or_else(|| main_win.inner_position());
                let parent_origin = parent.map(|p| p.anchor).unwrap_or(Rect::new(0, 0, 0, 0));
                if let Ok(inner) = anchor_inner {
                    let dx = (rect.x - parent_origin.x) as f32;
                    let dy = (rect.y - parent_origin.y) as f32;
                    let px = inner.x + (dx * self.scale).round() as i32;
                    let py = inner.y + (dy * self.scale).round() as i32;
                    attrs = attrs.with_position(PhysicalPosition::new(px, py));
                }
            }
            PopupKind::Dialog => {
                // A real managed top-level dialog: server-side
                // decorations (title bar + close button only), fixed
                // size (already set via `with_resizable(false)`), no
                // minimize / maximize controls. The WM places the
                // window — we don't pass a position — and the Dialog
                // window-type hint keeps it visually grouped with the
                // main window. The dialog's caption rides along on the
                // PopupRequest so it ends up as the OS window title.
                attrs = attrs
                    .with_title(request.title.as_deref().unwrap_or("Dialog"))
                    .with_decorations(true)
                    .with_enabled_buttons(WindowButtons::CLOSE)
                    // Pin min == max so the WM advertises a fixed size.
                    // `with_resizable(false)` alone is unreliable on many
                    // X11 WMs; equal min/max inner-size hints
                    // (WM_NORMAL_HINTS) are honored far more consistently
                    // — mirroring the Wayland backend's set_min/max_size.
                    .with_min_inner_size(size)
                    .with_max_inner_size(size);

                #[cfg(all(unix, not(target_os = "macos")))]
                {
                    attrs = attrs.with_x11_window_type(vec![XWindowType::Dialog]);
                }
            }
        }

        let win = event_loop.create_window(attrs).ok()?;
        // Before the first `set_visible` below, while the window is still
        // off-screen: the behavior applies to the animation that showing it
        // would otherwise play.
        #[cfg(target_os = "macos")]
        if request.kind == PopupKind::Popup {
            suppress_show_animation(&win);
        }
        let win = Rc::new(win);
        let id = win.id();
        let mut surface = softbuffer::Surface::new(context, win.clone()).ok()?;
        let actual = win.inner_size();
        resize_surface(&mut surface, actual);
        win.set_visible(true);

        Some(PopupWindow {
            win,
            win_id: id,
            surface,
            anchor: rect,
            kind: request.kind,
            physical: actual,
            scale: self.scale,
            cursor: None,
            cursor_icon: Cursor::Default,
            was_focused: false,
            needs_redraw: true,
        })
    }
}

/// A separate top-level / popup window dedicated to drawing a widget's
/// popup. Lives only while the requesting widget keeps reporting a
/// `PopupRequest`.
struct PopupWindow {
    win: Rc<Window>,
    win_id: WindowId,
    surface: softbuffer::Surface<Rc<Window>, Rc<Window>>,
    anchor: Rect,
    kind: PopupKind,
    physical: PhysicalSize<u32>,
    scale: f32,
    cursor: Option<Point>,
    /// Pointer shape currently applied to this popup window — see the same
    /// field on [`AppHandler`].
    cursor_icon: Cursor,
    /// Whether this window has ever held the keyboard. winit reports a fresh
    /// popup as unfocused before focusing it, and that first loss must not be
    /// read as the user clicking away — see the `Focused` arm in
    /// [`AppHandler::handle_popup_event`].
    was_focused: bool,
    needs_redraw: bool,
}

/// The logical-pixel margins the desktop keeps for itself along each edge of
/// the display a window is on — `(left, top, right, bottom)`. A window may be
/// dragged over them, but nothing the app *places* should land there.
///
/// macOS reports them (the menu bar, and the Dock along whichever edge it
/// lives on) as the difference between an `NSScreen`'s full frame and its
/// visible frame. X11 has `_NET_WORKAREA` and Windows `SPI_GETWORKAREA`, but
/// winit surfaces neither, so those return zeroes and a popup near the bottom
/// of the screen may sit under a panel or the taskbar.
#[cfg(target_os = "macos")]
fn desktop_insets(win: &Window) -> (i32, i32, i32, i32) {
    let Some(screen) = ns_window(win).and_then(|window| window.screen()) else {
        return (0, 0, 0, 0);
    };
    let full = screen.frame();
    let visible = screen.visibleFrame();
    // `NSRect`s are in points — logical pixels — with y growing upwards, so the
    // *top* inset is the gap above the visible frame's far edge. Both frames
    // describe the same screen, so the differences are the four margins.
    let round = |v: f64| v.round().max(0.0) as i32;
    (
        round(visible.origin.x - full.origin.x),
        round((full.origin.y + full.size.height) - (visible.origin.y + visible.size.height)),
        round((full.origin.x + full.size.width) - (visible.origin.x + visible.size.width)),
        round(visible.origin.y - full.origin.y),
    )
}

#[cfg(not(target_os = "macos"))]
fn desktop_insets(_win: &Window) -> (i32, i32, i32, i32) {
    (0, 0, 0, 0)
}

/// The `NSWindow` behind a winit window, or `None` if its view can't be
/// reached.
#[cfg(target_os = "macos")]
fn ns_window(win: &Window) -> Option<objc2::rc::Retained<objc2_app_kit::NSWindow>> {
    use objc2_app_kit::NSView;
    use winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};

    let RawWindowHandle::AppKit(handle) = win.window_handle().ok()?.as_raw() else {
        return None;
    };
    // SAFETY: winit owns this `NSView` for as long as the window lives, which
    // is past the end of this borrow — `win` is still holding it.
    let view: &NSView = unsafe { handle.ns_view.cast().as_ref() };
    view.window()
}

/// Take a macOS popup window out of the system's window choreography: a menu
/// must be on screen the instant it is asked for, not faded in over a few
/// frames. winit has no API for `NSWindow.animationBehavior`, so it is set on
/// the window itself — before it is first shown, since that is the appearance
/// being suppressed. A window whose view we can't reach is left alone.
#[cfg(target_os = "macos")]
fn suppress_show_animation(win: &Window) {
    if let Some(window) = ns_window(win) {
        window.setAnimationBehavior(objc2_app_kit::NSWindowAnimationBehavior::None);
    }
}

fn popup_position_to_widget(pos: PhysicalPosition<f64>, popup: &PopupWindow) -> Point {
    let s = popup.scale.max(0.01) as f64;
    let lx = pos.x / s;
    let ly = pos.y / s;
    Point::new((lx as i32) + popup.anchor.x, (ly as i32) + popup.anchor.y)
}

/// The key to report for `KeyDown` / `KeyUp` — the layout-resolved key with
/// *modifiers* stripped (Shift, Ctrl, and, crucially on macOS, Alt/Option).
///
/// We deliberately do not use `key.logical_key` here: on macOS winit folds the
/// Option modifier into the character, so `Option+F` arrives as `'ƒ'` and
/// `Option+O` as `'ø'`, which never match an ASCII menu mnemonic — only dead
/// keys like `Option+E` happen to fall back to their base letter. winit's
/// `key_without_modifiers()` resolves the key through the *active* keyboard
/// layout (so German QWERTZ still reports `z`/`y` correctly) while discarding
/// the modifier-composition, which is exactly what mnemonics and hotkeys want.
///
/// Text insertion is unaffected: that path uses `key.text` via the `Char`
/// event, which still carries the composed character. The trait is only
/// available on the desktop backends, so non-desktop targets fall back to
/// `logical_key` (their previous behaviour).
#[cfg(any(
    target_os = "windows",
    target_os = "macos",
    target_os = "linux",
    target_os = "freebsd",
    target_os = "dragonfly",
    target_os = "netbsd",
    target_os = "openbsd",
    target_os = "redox",
))]
fn map_base_key(key: &winit::event::KeyEvent) -> Option<Key> {
    use winit::platform::modifier_supplement::KeyEventExtModifierSupplement;
    map_key(&key.key_without_modifiers())
}

#[cfg(not(any(
    target_os = "windows",
    target_os = "macos",
    target_os = "linux",
    target_os = "freebsd",
    target_os = "dragonfly",
    target_os = "netbsd",
    target_os = "openbsd",
    target_os = "redox",
)))]
fn map_base_key(key: &winit::event::KeyEvent) -> Option<Key> {
    map_key(&key.logical_key)
}

fn map_key(key: &WKey) -> Option<Key> {
    match key {
        WKey::Named(named) => map_named(*named).map(Key::Named),
        WKey::Character(s) => s.chars().next().map(Key::Char),
        _ => None,
    }
}

fn map_named(named: WNamedKey) -> Option<NamedKey> {
    Some(match named {
        WNamedKey::Enter => NamedKey::Enter,
        WNamedKey::Backspace => NamedKey::Backspace,
        WNamedKey::Delete => NamedKey::Delete,
        WNamedKey::Tab => NamedKey::Tab,
        WNamedKey::Escape => NamedKey::Escape,
        WNamedKey::Space => NamedKey::Space,
        WNamedKey::ArrowLeft => NamedKey::Left,
        WNamedKey::ArrowRight => NamedKey::Right,
        WNamedKey::ArrowUp => NamedKey::Up,
        WNamedKey::ArrowDown => NamedKey::Down,
        WNamedKey::Home => NamedKey::Home,
        WNamedKey::End => NamedKey::End,
        WNamedKey::PageUp => NamedKey::PageUp,
        WNamedKey::PageDown => NamedKey::PageDown,
        _ => return None,
    })
}

fn map_button(button: WinitMouseButton) -> Option<MouseButton> {
    match button {
        WinitMouseButton::Left => Some(MouseButton::Left),
        WinitMouseButton::Right => Some(MouseButton::Right),
        WinitMouseButton::Middle => Some(MouseButton::Middle),
        _ => None,
    }
}

/// Map a saudade [`Cursor`] onto winit's [`CursorIcon`]. Both vocabularies are
/// CSS-derived, so every saudade variant has a direct winit equivalent (the one
/// rename is [`Cursor::Hand`] → [`CursorIcon::Pointer`]).
fn winit_cursor(cursor: Cursor) -> CursorIcon {
    match cursor {
        Cursor::Default => CursorIcon::Default,
        Cursor::Hand => CursorIcon::Pointer,
        Cursor::Text => CursorIcon::Text,
        Cursor::VerticalText => CursorIcon::VerticalText,
        Cursor::Crosshair => CursorIcon::Crosshair,
        Cursor::Move => CursorIcon::Move,
        Cursor::Grab => CursorIcon::Grab,
        Cursor::Grabbing => CursorIcon::Grabbing,
        Cursor::NotAllowed => CursorIcon::NotAllowed,
        Cursor::NoDrop => CursorIcon::NoDrop,
        Cursor::Copy => CursorIcon::Copy,
        Cursor::Alias => CursorIcon::Alias,
        Cursor::ContextMenu => CursorIcon::ContextMenu,
        Cursor::Help => CursorIcon::Help,
        Cursor::Progress => CursorIcon::Progress,
        Cursor::Wait => CursorIcon::Wait,
        Cursor::Cell => CursorIcon::Cell,
        Cursor::ZoomIn => CursorIcon::ZoomIn,
        Cursor::ZoomOut => CursorIcon::ZoomOut,
        Cursor::AllScroll => CursorIcon::AllScroll,
        Cursor::ColResize => CursorIcon::ColResize,
        Cursor::RowResize => CursorIcon::RowResize,
        Cursor::EwResize => CursorIcon::EwResize,
        Cursor::NsResize => CursorIcon::NsResize,
        Cursor::NeswResize => CursorIcon::NeswResize,
        Cursor::NwseResize => CursorIcon::NwseResize,
    }
}

/// Apply the pointer shape a [`PointerMove`](Event::PointerMove) dispatch asked
/// for to `win`, tracking the last shape in `current` so an unchanged hover
/// doesn't re-issue the request (winit's cursor is a sticky window attribute).
/// `requested` is `None` when no widget under the pointer asked for one, which
/// resolves to the arrow.
fn reconcile_cursor(win: &Window, current: &mut Cursor, requested: Option<Cursor>) {
    let want = requested.unwrap_or(Cursor::Default);
    if *current != want {
        *current = want;
        win.set_cursor(winit_cursor(want));
    }
}

/// Translate a winit scroll delta into saudade's `(delta_x, delta_y)` in
/// document lines, positive toward the content's end. winit reports positive
/// values as scrolling *up/left* (revealing earlier content), the opposite of
/// our convention, so we negate. Discrete wheel notches scale by
/// [`WHEEL_LINES_PER_DETENT`]; trackpad pixel deltas (physical pixels) are taken
/// back to logical pixels by `scale` and divided into lines.
fn scroll_delta_lines(delta: MouseScrollDelta, scale: f32) -> (f32, f32) {
    match delta {
        MouseScrollDelta::LineDelta(x, y) => {
            (-x * WHEEL_LINES_PER_DETENT, -y * WHEEL_LINES_PER_DETENT)
        }
        MouseScrollDelta::PixelDelta(p) => {
            let per_line = scale.max(0.01) * SCROLL_PIXELS_PER_LINE;
            (-(p.x as f32) / per_line, -(p.y as f32) / per_line)
        }
    }
}

fn resize_surface(
    surface: &mut softbuffer::Surface<Rc<Window>, Rc<Window>>,
    size: PhysicalSize<u32>,
) {
    let w = NonZeroU32::new(size.width.max(1)).unwrap();
    let h = NonZeroU32::new(size.height.max(1)).unwrap();
    surface
        .resize(w, h)
        .expect("saudade: failed to resize surface");
}

fn origin(logical: Size, scale: f32, physical: PhysicalSize<u32>) -> (i32, i32) {
    let content_w = (logical.w as f32 * scale).round() as i32;
    let content_h = (logical.h as f32 * scale).round() as i32;
    let ox = ((physical.width as i32 - content_w) / 2).max(0);
    let oy = ((physical.height as i32 - content_h) / 2).max(0);
    (ox, oy)
}

fn relayout(
    root: &mut Box<dyn Widget>,
    physical: PhysicalSize<u32>,
    scale: f32,
    _design_size: Size,
) {
    let s = scale.max(0.01);
    let logical_w = (physical.width as f32 / s).round() as i32;
    let logical_h = (physical.height as f32 / s).round() as i32;
    root.layout(Rect::new(0, 0, logical_w.max(1), logical_h.max(1)));
}

fn physical_to_logical(
    pos: PhysicalPosition<f64>,
    scale: f32,
    origin_x: i32,
    origin_y: i32,
) -> Point {
    let s = scale.max(0.01) as f64;
    let x = ((pos.x - origin_x as f64) / s).floor() as i32;
    let y = ((pos.y - origin_y as f64) / s).floor() as i32;
    Point::new(x, y)
}

impl From<Rect> for Size {
    fn from(r: Rect) -> Size {
        Size::new(r.w, r.h)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const O: Option<Key> = Some(Key::Char('o'));
    const F: Option<Key> = Some(Key::Char('f'));

    #[test]
    fn nothing_is_swallowed_by_default() {
        let s = KeySwallow::default();
        assert!(!s.drops_press(O));
        assert!(!s.drops_press(None));
    }

    #[test]
    fn a_swallowed_press_drops_text_autorepeat_and_release() {
        let mut s = KeySwallow::default();
        // A handler fired on the press of 'o' and asked to swallow it.
        s.begin(O);
        // Its trailing text and any autorepeat of the same key are dropped …
        assert!(
            s.drops_press(O),
            "autorepeat of the swallowed key is dropped"
        );
        // … and so is the release, which then re-arms the key.
        assert!(s.ends_on_release(O));
        assert!(!s.drops_press(O), "after release the key is live again");
        assert!(!s.ends_on_release(O));
    }

    #[test]
    fn other_keys_pass_through_a_swallow() {
        let mut s = KeySwallow::default();
        s.begin(O);
        // A different key pressed/released meanwhile is untouched …
        assert!(!s.drops_press(F));
        assert!(!s.ends_on_release(F), "an unrelated release isn't consumed");
        // … and doesn't disturb the key still being swallowed.
        assert!(s.ends_on_release(O));
    }

    #[test]
    fn an_unmapped_key_never_matches() {
        let mut s = KeySwallow::default();
        s.begin(None);
        // `None` (a key that didn't map) must not collapse into a match — that
        // would swallow every other unmapped event.
        assert!(!s.drops_press(None));
        assert!(!s.ends_on_release(None));
    }

    #[test]
    fn no_buttons_held_by_default() {
        assert!(!PointerButtons::default().any_held());
    }

    #[test]
    fn a_held_button_marks_the_grab_active_until_released() {
        let mut b = PointerButtons::default();
        b.press();
        assert!(b.any_held(), "a held button means an OS grab is in effect");
        b.release();
        assert!(!b.any_held(), "the grab ends once the button is released");
    }

    #[test]
    fn the_grab_persists_until_the_last_of_several_buttons_is_up() {
        // X11's implicit grab lives until *all* buttons are released, so a chord
        // (e.g. left held while right taps) must keep the grab active.
        let mut b = PointerButtons::default();
        b.press();
        b.press();
        b.release();
        assert!(b.any_held(), "one button still down keeps the grab alive");
        b.release();
        assert!(!b.any_held());
    }

    #[test]
    fn an_extra_release_cannot_drive_the_count_negative() {
        // A release whose press was delivered elsewhere (e.g. to a popup window
        // that was torn down) must not underflow the count and wedge the grab
        // on forever.
        let mut b = PointerButtons::default();
        b.release();
        assert!(!b.any_held());
        b.press();
        assert!(
            b.any_held(),
            "the count still tracks a real press afterward"
        );
    }
}
