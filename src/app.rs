use std::num::NonZeroU32;
use std::rc::Rc;
use std::time::{Duration, Instant};

use winit::application::ApplicationHandler;
use winit::dpi::{LogicalSize, PhysicalPosition, PhysicalSize};
use winit::event::{ElementState, MouseButton as WinitMouseButton, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::keyboard::{Key as WKey, NamedKey as WNamedKey};
use winit::window::{Window, WindowAttributes, WindowButtons, WindowId};

// X11 platform extensions. winit 0.30's generic `with_parent_window` is
// not enough on X11 (it reparents into the main window, which then clips
// the popup to its bounds) and has no effect on Wayland (the backend
// still creates an `xdg_toplevel` instead of an `xdg_popup`). We use
// override-redirect + the DropdownMenu window type hint to get proper
// instant-popup behavior on X11. Wayland keeps the top-level fallback
// until winit adds real popup support.
#[cfg(any(
    target_os = "linux",
    target_os = "dragonfly",
    target_os = "freebsd",
    target_os = "netbsd",
    target_os = "openbsd",
))]
use winit::platform::x11::{WindowAttributesExtX11, WindowType as XWindowType};

use crate::event::{Event, EventCtx, Key, Modifiers, MouseButton, NamedKey};
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
        if is_wayland_session() {
            crate::wayland::run(self);
            return;
        }
        self.run_winit();
    }

    fn run_winit(self) {
        let event_loop = EventLoop::new().expect("retrogui: failed to create event loop");
        event_loop.set_control_flow(ControlFlow::Wait);

        let mut handler = AppHandler::new(self);
        event_loop
            .run_app(&mut handler)
            .expect("retrogui: event loop error");
    }

    pub(crate) fn into_parts(self) -> (WindowConfig, Theme, Box<dyn Widget>) {
        (self.window, self.theme, self.root)
    }
}

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
    scale: f32,

    // Per-frame state:
    cursor: Option<Point>,
    modifiers: Modifiers,
    needs_redraw: bool,
    popup: Option<PopupWindow>,
    /// Last `Event::Tick` we dispatched. `None` until the first tick is
    /// fired. The runtime uses this to pace ticks while a widget
    /// reports `wants_ticks()`.
    last_tick: Option<Instant>,
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
            popup: None,
            last_tick: None,
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
            .expect("retrogui: failed to create window");
        let win = Rc::new(win);
        let id = win.id();

        let context = softbuffer::Context::new(win.clone())
            .expect("retrogui: failed to create softbuffer context");
        let mut surface = softbuffer::Surface::new(&context, win.clone())
            .expect("retrogui: failed to create softbuffer surface");

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
        } else if let Some(p) = self.popup.as_ref()
            && p.win_id == window_id
        {
            self.handle_popup_event(event, event_loop);
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        self.sync_popup(event_loop);
        self.pump_ticks(event_loop);

        if self.needs_redraw
            && let Some(win) = self.main_win.as_ref()
        {
            win.request_redraw();
            self.needs_redraw = false;
        }
        if let Some(p) = self.popup.as_mut()
            && p.needs_redraw
        {
            p.win.request_redraw();
            p.needs_redraw = false;
        }
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
            WindowEvent::ModifiersChanged(new_mods) => {
                let s = new_mods.state();
                self.modifiers = Modifiers {
                    shift: s.shift_key(),
                    control: s.control_key(),
                    alt: s.alt_key(),
                    logo: s.super_key(),
                };
            }
            WindowEvent::KeyboardInput { event: key, .. } => {
                self.dispatch_key(&key, event_loop);
            }
            WindowEvent::RedrawRequested => {
                self.paint_main();
                if let Some(p) = self.popup.as_mut() {
                    p.needs_redraw = true;
                }
            }
            _ => {}
        }
    }

    fn handle_popup_event(&mut self, event: WindowEvent, event_loop: &ActiveEventLoop) {
        match event {
            WindowEvent::CloseRequested => {
                self.dismiss_via_escape(event_loop);
            }
            WindowEvent::Resized(new_size) => {
                if let Some(p) = self.popup.as_mut() {
                    p.physical = new_size;
                    resize_surface(&mut p.surface, new_size);
                    p.needs_redraw = true;
                }
            }
            WindowEvent::CursorMoved { position, .. } => {
                let Some(p) = self.popup.as_mut() else { return };
                let pos = popup_position_to_widget(position, p);
                p.cursor = Some(pos);
                self.dispatch(&Event::PointerMove { pos }, event_loop);
                if let Some(p) = self.popup.as_mut() {
                    p.needs_redraw = true;
                }
            }
            WindowEvent::CursorLeft { .. } => {
                if let Some(p) = self.popup.as_mut() {
                    p.cursor = None;
                }
                self.dispatch(&Event::PointerLeave, event_loop);
                if let Some(p) = self.popup.as_mut() {
                    p.needs_redraw = true;
                }
            }
            WindowEvent::MouseInput {
                state,
                button: winit_button,
                ..
            } => {
                let Some(pos) = self.popup.as_ref().and_then(|p| p.cursor) else {
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
                if let Some(p) = self.popup.as_mut() {
                    p.needs_redraw = true;
                }
            }
            WindowEvent::ModifiersChanged(new_mods) => {
                let s = new_mods.state();
                self.modifiers = Modifiers {
                    shift: s.shift_key(),
                    control: s.control_key(),
                    alt: s.alt_key(),
                    logo: s.super_key(),
                };
            }
            WindowEvent::KeyboardInput { event: key, .. } => {
                self.dispatch_key(&key, event_loop);
                if let Some(p) = self.popup.as_mut() {
                    p.needs_redraw = true;
                }
            }
            WindowEvent::RedrawRequested => {
                self.paint_popup();
            }
            _ => {}
        }
    }

    fn dispatch(&mut self, event: &Event, event_loop: &ActiveEventLoop) {
        let mut ctx = EventCtx::new();
        self.root.event(event, &mut ctx);
        if ctx.paint_requested {
            self.needs_redraw = true;
        }
        if ctx.close_requested {
            event_loop.exit();
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
        let mapped = map_key(&key.logical_key);
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
            .expect("retrogui: failed to acquire surface buffer");
        let mut painter = Painter::with_popup_pass(
            &mut surface_buf,
            self.physical.width as i32,
            self.physical.height as i32,
            self.scale,
            origin_x,
            origin_y,
            self.font.as_ref(),
            self.mono_font.as_ref(),
            false,
        );
        painter.fill(self.theme.background);
        self.root.paint(&mut painter, &self.theme);
        surface_buf
            .present()
            .expect("retrogui: failed to present buffer");
    }

    fn paint_popup(&mut self) {
        let Some(p) = self.popup.as_mut() else { return };
        let origin_x = -((p.anchor.x as f32 * p.scale).round() as i32);
        let origin_y = -((p.anchor.y as f32 * p.scale).round() as i32);
        let popup_phys_w = (p.anchor.w as f32 * p.scale).round() as i32;
        let popup_phys_h = (p.anchor.h as f32 * p.scale).round() as i32;
        let mut surface_buf = p
            .surface
            .buffer_mut()
            .expect("retrogui: failed to acquire popup buffer");
        let mut painter = Painter::with_popup_pass(
            &mut surface_buf,
            p.physical.width as i32,
            p.physical.height as i32,
            p.scale,
            origin_x,
            origin_y,
            self.font.as_ref(),
            self.mono_font.as_ref(),
            true,
        );
        painter.fill(self.theme.background);
        painter.set_clip_phys(0, 0, popup_phys_w, popup_phys_h);
        self.root.paint(&mut painter, &self.theme);
        painter.clear_clip();
        surface_buf
            .present()
            .expect("retrogui: failed to present popup buffer");
    }

    /// Re-anchor the popup window to the main window's current screen
    /// position. Called when the main window emits `Moved`. No-op when
    /// there's no popup or when the platform doesn't support querying
    /// the main window's inner position (e.g., Wayland). Dialog windows
    /// are managed top-levels so the WM moves them on its own — we only
    /// reposition popup-kind children.
    fn reposition_popup(&mut self) {
        let Some(popup) = self.popup.as_ref() else { return };
        if popup.kind != PopupKind::Popup {
            return;
        }
        let Some(main_win) = self.main_win.as_ref() else { return };
        let Ok(inner) = main_win.inner_position() else { return };
        let px = inner.x + ((popup.anchor.x as f32) * self.scale).round() as i32;
        let py = inner.y + ((popup.anchor.y as f32) * self.scale).round() as i32;
        popup.win.set_outer_position(PhysicalPosition::new(px, py));
    }

    fn sync_popup(&mut self, event_loop: &ActiveEventLoop) {
        let request = self.root.popup_request();
        match (request, self.popup.as_mut()) {
            (None, Some(_)) => {
                self.popup = None;
            }
            (Some(req), None) => {
                if let Some(p) = self.open_popup(req, event_loop) {
                    self.popup = Some(p);
                }
            }
            (Some(req), Some(existing))
                if existing.anchor != req.rect || existing.kind != req.kind =>
            {
                // Anchor moved (slide-over between top-level menus), or
                // the widget switched between Popup and Dialog hosting.
                // Tear the child window down and rebuild — fastest
                // reliable path that works the same on every backend.
                self.popup = None;
                if let Some(p) = self.open_popup(req, event_loop) {
                    self.popup = Some(p);
                }
            }
            _ => {}
        }
    }

    fn open_popup(
        &self,
        request: PopupRequest,
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
                attrs = attrs.with_title("retrogui popup").with_decorations(false);

                // X11: take the WM completely out of the loop.
                // override-redirect makes this an unmanaged window — it
                // sits at the exact screen position requested, at the
                // exact requested size, and may extend past the main
                // window's bounds. The DropdownMenu type hint helps WMs
                // route it (e.g., place above the main window in
                // stacking order). These attributes are silently ignored
                // on other backends.
                #[cfg(any(
                    target_os = "linux",
                    target_os = "dragonfly",
                    target_os = "freebsd",
                    target_os = "netbsd",
                    target_os = "openbsd",
                ))]
                {
                    attrs = attrs
                        .with_override_redirect(true)
                        .with_x11_window_type(vec![XWindowType::DropdownMenu]);
                }

                // Absolute screen position = main window inner position +
                // popup offset in physical pixels. On X11 with
                // override-redirect this is honored exactly. On Wayland
                // (no popup support yet) winit creates a top-level window
                // and the compositor places it on its own — the position
                // request is ignored.
                if let Ok(inner) = main_win.inner_position() {
                    let px = inner.x + ((rect.x as f32) * self.scale).round() as i32;
                    let py = inner.y + ((rect.y as f32) * self.scale).round() as i32;
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

                #[cfg(any(
                    target_os = "linux",
                    target_os = "dragonfly",
                    target_os = "freebsd",
                    target_os = "netbsd",
                    target_os = "openbsd",
                ))]
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
    Point::new(
        (lx as i32) + popup.anchor.x,
        (ly as i32) + popup.anchor.y,
    )
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

fn resize_surface(
    surface: &mut softbuffer::Surface<Rc<Window>, Rc<Window>>,
    size: PhysicalSize<u32>,
) {
    let w = NonZeroU32::new(size.width.max(1)).unwrap();
    let h = NonZeroU32::new(size.height.max(1)).unwrap();
    surface
        .resize(w, h)
        .expect("retrogui: failed to resize surface");
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
