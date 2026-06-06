use std::num::NonZeroU32;
use std::path::PathBuf;
use std::rc::Rc;
use std::time::{Duration, Instant};

use winit::application::ApplicationHandler;
use winit::dpi::{LogicalSize, PhysicalPosition, PhysicalSize};
use winit::event::{ElementState, MouseButton as WinitMouseButton, MouseScrollDelta, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::keyboard::{Key as WKey, ModifiersKeyState, NamedKey as WNamedKey};
use winit::window::{Window, WindowAttributes, WindowButtons, WindowId};

// X11 platform extensions. winit 0.30's generic `with_parent_window` is
// not enough on X11 (it reparents into the main window, which then clips
// the popup to its bounds) and has no effect on Wayland (the backend
// still creates an `xdg_toplevel` instead of an `xdg_popup`). We use
// override-redirect + the DropdownMenu window type hint to get proper
// instant-popup behavior on X11. Wayland keeps the top-level fallback
// until winit adds real popup support.
#[cfg(all(unix, not(target_os = "macos")))]
use winit::platform::x11::{WindowAttributesExtX11, WindowType as XWindowType};

use crate::background::BackgroundState;
use crate::event::{
    DragData, Event, EventCtx, Key, Modifiers, MouseButton, NamedKey, SCROLL_PIXELS_PER_LINE,
    WHEEL_LINES_PER_DETENT,
};
use crate::font::Font;
use crate::geometry::{Point, Rect, Size};
use crate::painter::Painter;
use crate::theme::Theme;
use crate::widget::{PopupKind, PopupRequest, Widget};

pub struct WindowConfig {
    pub title: String,
    pub size: Size,
    pub resizable: bool,
}

impl WindowConfig {
    pub fn new(title: impl Into<String>, width: i32, height: i32) -> Self {
        Self {
            title: title.into(),
            size: Size::new(width, height),
            resizable: false,
        }
    }

    pub fn resizable(mut self, resizable: bool) -> Self {
        self.resizable = resizable;
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
    modifiers: Modifiers,
    needs_redraw: bool,
    /// Background pattern + color for the main window, toggled with the
    /// `p` / `c` debug keys. Popups/dialogs ignore it and stay white.
    bg: BackgroundState,
    /// Stack of popup windows, outermost first. Usually empty or one entry (a
    /// menu / dropdown / dialog); a dropdown opened *inside* a dialog nests a
    /// second entry on top.
    popups: Vec<PopupWindow>,
    /// Last `Event::Tick` we dispatched. `None` until the first tick is
    /// fired. The runtime uses this to pace ticks while a widget
    /// reports `wants_ticks()`.
    last_tick: Option<Instant>,

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
            font: Font::load_system(),
            mono_font: Font::load_monospace(),
            main_win: None,
            main_id: None,
            context: None,
            main_surface: None,
            physical: PhysicalSize::new(0, 0),
            scale: 1.0,
            cursor: None,
            modifiers: Modifiers::default(),
            needs_redraw: true,
            bg: BackgroundState::from_env(),
            popups: Vec::new(),
            last_tick: None,
            drag_hovered: Vec::new(),
            drag_dropped: Vec::new(),
            drag_active: false,
        }
    }
}

impl ApplicationHandler for AppHandler {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.main_win.is_some() {
            return; // already initialized; ignore redundant resumes
        }
        let attrs = WindowAttributes::default()
            .with_title(&self.window_config.title)
            .with_inner_size(LogicalSize::new(
                self.window_config.size.w as f64,
                self.window_config.size.h as f64,
            ))
            .with_resizable(self.window_config.resizable);
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
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::Moved(_) => {
                // The main window changed screen position. Override-redirect
                // popups are unmanaged top-level windows that don't follow
                // their "parent", so we have to reposition them manually
                // each time the main window moves.
                self.reposition_popup();
            }
            WindowEvent::Resized(new_size) => {
                self.physical = new_size;
                if let Some(s) = self.main_surface.as_mut() {
                    resize_surface(s, self.physical);
                }
                relayout(&mut self.root, self.physical, self.scale, self.design_size);
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
                self.needs_redraw = true;
            }
            WindowEvent::CursorMoved { position, .. } => {
                let content = self.root.bounds().into();
                let (origin_x, origin_y) = origin(content, self.scale, self.physical);
                let pos = physical_to_logical(position, self.scale, origin_x, origin_y);
                self.cursor = Some(pos);
                self.dispatch(&Event::PointerMove { pos }, event_loop);
            }
            WindowEvent::CursorLeft { .. } => {
                self.cursor = None;
                self.dispatch(&Event::PointerLeave, event_loop);
            }
            WindowEvent::MouseInput {
                state,
                button: winit_button,
                ..
            } => {
                let Some(pos) = self.cursor else { return };
                let Some(button) = map_button(winit_button) else {
                    return;
                };
                let event = match state {
                    ElementState::Pressed => Event::PointerDown { pos, button },
                    ElementState::Released => Event::PointerUp { pos, button },
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
                self.dispatch(&Event::PointerMove { pos }, event_loop);
                self.mark_popups_dirty();
            }
            WindowEvent::CursorLeft { .. } => {
                if let Some(p) = self.popups.get_mut(idx) {
                    p.cursor = None;
                }
                self.dispatch(&Event::PointerLeave, event_loop);
                self.mark_popups_dirty();
            }
            WindowEvent::MouseInput {
                state,
                button: winit_button,
                ..
            } => {
                let Some(pos) = self.popups.get(idx).and_then(|p| p.cursor) else {
                    return;
                };
                let Some(button) = map_button(winit_button) else {
                    return;
                };
                let event = match state {
                    ElementState::Pressed => Event::PointerDown { pos, button },
                    ElementState::Released => Event::PointerUp { pos, button },
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

    fn dispatch(&mut self, event: &Event, event_loop: &ActiveEventLoop) {
        let mut ctx = EventCtx::new();
        self.root.event(event, &mut ctx);
        if ctx.paint_requested {
            self.needs_redraw = true;
            self.mark_popups_dirty();
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
        if !self.root.wants_ticks() {
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
            self.dispatch(&Event::Tick, event_loop);
        }
        let next = self.last_tick.unwrap_or(now) + TICK_INTERVAL;
        event_loop.set_control_flow(ControlFlow::WaitUntil(next));
    }

    fn dispatch_key(&mut self, key: &winit::event::KeyEvent, event_loop: &ActiveEventLoop) {
        let mapped = map_base_key(key);
        match key.state {
            ElementState::Pressed => {
                if let Some(mapped) = mapped {
                    self.dispatch(
                        &Event::KeyDown {
                            key: mapped,
                            modifiers: self.modifiers,
                        },
                        event_loop,
                    );
                }
                if !self.modifiers.has_command()
                    && let Some(text) = key.text.as_deref()
                {
                    for ch in text.chars() {
                        if (ch.is_control() && ch != '\t' && ch != '\n') || ch == '\r' {
                            continue;
                        }
                        self.dispatch(
                            &Event::Char {
                                ch,
                                modifiers: self.modifiers,
                            },
                            event_loop,
                        );
                    }
                }
            }
            ElementState::Released => {
                if let Some(mapped) = mapped {
                    self.dispatch(
                        &Event::KeyUp {
                            key: mapped,
                            modifiers: self.modifiers,
                        },
                        event_loop,
                    );
                }
            }
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
            self.font.as_ref(),
            self.mono_font.as_ref(),
            None,
        );
        painter.fill_pattern(self.theme.background, self.bg.pattern, self.bg.color);
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
            self.font.as_ref(),
            self.mono_font.as_ref(),
            Some(anchor),
        );
        painter.fill(self.theme.background);
        painter.set_clip_phys(0, 0, popup_phys_w, popup_phys_h);
        self.root.paint(&mut painter, &self.theme);
        painter.clear_clip();
        surface_buf
            .present()
            .expect("saudade: failed to present popup buffer");
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
                attrs = attrs.with_title("saudade popup").with_decorations(false);

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
    needs_redraw: bool,
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
