//! Pure SCTK (smithay-client-toolkit) Wayland backend.
//!
//! Used in place of winit when the process is started on a Wayland session
//! (`WAYLAND_DISPLAY` is set). The widget tree, the painter, and every other
//! piece of retrogui stay the same — only the windowing + event loop differ.
//!
//! Why SCTK rather than winit's Wayland support: winit 0.30 still doesn't
//! implement `xdg_popup`, so popups would fall back to plain `xdg_toplevel`s
//! that the compositor places wherever it likes. Going through SCTK gives us
//! real popups with a positioner anchored to the parent — the same behavior
//! Chrome/Firefox have on Wayland.

use std::sync::Arc;
use std::time::Duration;

use smithay_client_toolkit::compositor::{CompositorHandler, CompositorState};
use smithay_client_toolkit::output::{OutputHandler, OutputState};
use smithay_client_toolkit::reexports::calloop::EventLoop as CalloopLoop;
use smithay_client_toolkit::reexports::calloop_wayland_source::WaylandSource;
use smithay_client_toolkit::registry::{ProvidesRegistryState, RegistryState};
use smithay_client_toolkit::seat::keyboard::{
    KeyEvent as WlKeyEvent, KeyboardHandler, Keysym, Modifiers as WlModifiers,
};
use smithay_client_toolkit::seat::pointer::{PointerEvent, PointerEventKind, PointerHandler};
use smithay_client_toolkit::seat::{Capability, SeatHandler, SeatState};
use smithay_client_toolkit::shell::WaylandSurface;
use smithay_client_toolkit::shell::xdg::{XdgShell, XdgSurface};
use smithay_client_toolkit::shell::xdg::popup::{Popup, PopupConfigure, PopupHandler};
use smithay_client_toolkit::shell::xdg::window::{
    Window as XdgWindow, WindowConfigure, WindowDecorations, WindowHandler,
};
use smithay_client_toolkit::shm::slot::{Buffer, SlotPool};
use smithay_client_toolkit::shm::{Shm, ShmHandler};
use smithay_client_toolkit::{
    delegate_compositor, delegate_keyboard, delegate_output, delegate_pointer, delegate_registry,
    delegate_seat, delegate_shm, delegate_xdg_popup, delegate_xdg_shell, delegate_xdg_window,
    registry_handlers,
};
use wayland_client::globals::registry_queue_init;
use wayland_client::protocol::{wl_keyboard, wl_output, wl_pointer, wl_seat, wl_shm, wl_surface};
use wayland_client::{Connection, Dispatch, Proxy, QueueHandle};
use wayland_protocols::xdg::dialog::v1::client::xdg_dialog_v1::XdgDialogV1;
use wayland_protocols::xdg::dialog::v1::client::xdg_wm_dialog_v1::XdgWmDialogV1;
use wayland_protocols::xdg::shell::client::xdg_positioner::{Anchor, Gravity, XdgPositioner};

use crate::app::App;
use crate::event::{Event, EventCtx, Key, Modifiers, MouseButton, NamedKey};
use crate::font::Font;
use crate::geometry::{Point, Rect};
use crate::painter::Painter;
use crate::theme::Theme;
use crate::widget::{PopupKind, PopupRequest, Widget};

pub(crate) fn run(app: App) {
    let (window_cfg, theme, root) = app.into_parts();

    let conn = Connection::connect_to_env().expect("retrogui: Wayland connect failed");
    let (globals, event_queue) =
        registry_queue_init::<State>(&conn).expect("retrogui: registry init failed");
    let qh: QueueHandle<State> = event_queue.handle();

    let mut event_loop: CalloopLoop<State> =
        CalloopLoop::try_new().expect("retrogui: calloop init failed");
    let loop_handle = event_loop.handle();
    WaylandSource::new(conn.clone(), event_queue)
        .insert(loop_handle)
        .expect("retrogui: WaylandSource insert failed");

    let compositor =
        CompositorState::bind(&globals, &qh).expect("retrogui: wl_compositor not available");
    let xdg_shell = XdgShell::bind(&globals, &qh).expect("retrogui: xdg_shell not available");
    let shm = Shm::bind(&globals, &qh).expect("retrogui: wl_shm not available");
    // Optional: the dialog protocol is a "staging" extension. Compositors
    // that don't advertise it fall back to plain xdg_toplevel with
    // set_parent — still a real top-level, just without the explicit
    // "this is a dialog, hide min/max" hint.
    let xdg_dialog_mgr: Option<XdgWmDialogV1> =
        globals.bind::<XdgWmDialogV1, _, _>(&qh, 1..=1, ()).ok();

    let surface = compositor.create_surface(&qh);
    let window =
        xdg_shell.create_window(surface, WindowDecorations::RequestServer, &qh);
    window.set_title(&window_cfg.title);
    window.set_app_id(format!("retrogui.{}", sanitize(&window_cfg.title)));
    window.set_min_size(Some((100, 60)));
    window.commit();

    let initial_w = window_cfg.size.w.max(1) as u32;
    let initial_h = window_cfg.size.h.max(1) as u32;
    let pool = SlotPool::new((initial_w * initial_h * 4) as usize * 4, &shm)
        .expect("retrogui: slot pool init failed");

    let mut state = State {
        registry_state: RegistryState::new(&globals),
        seat_state: SeatState::new(&globals, &qh),
        output_state: OutputState::new(&globals, &qh),
        compositor,
        shm,
        xdg_shell,
        xdg_dialog_mgr,

        window,
        root,
        theme,
        font: Font::load_system(),
        mono_font: Font::load_monospace(),

        pool,
        surface_w: initial_w,
        surface_h: initial_h,
        scale: 1,
        configured: false,
        needs_redraw: true,
        exit: false,

        keyboard: None,
        pointer: None,
        modifiers: Modifiers::default(),
        cursor: None,

        popup: None,
        qh: qh.clone(),
    };
    drop(conn);

    while !state.exit {
        event_loop
            .dispatch(Duration::from_millis(16), &mut state)
            .expect("retrogui: dispatch failed");
        state.tick();
    }
}

/// SCTK app state. Holds protocol objects, widget tree, and per-frame data.
struct State {
    registry_state: RegistryState,
    seat_state: SeatState,
    output_state: OutputState,
    compositor: CompositorState,
    shm: Shm,
    xdg_shell: XdgShell,
    /// Optional `xdg_wm_dialog_v1` global. Compositors that advertise
    /// the protocol (e.g. labwc) let us mark dialog toplevels so the
    /// SSD chrome hides minimize/maximize and the parent gets dimmed.
    xdg_dialog_mgr: Option<XdgWmDialogV1>,

    window: XdgWindow,
    root: Box<dyn Widget>,
    theme: Theme,
    font: Option<Font>,
    mono_font: Option<Font>,

    pool: SlotPool,
    /// Surface (logical) dimensions reported by the compositor. The buffer
    /// we attach is `surface_w * scale` × `surface_h * scale` physical
    /// pixels, and the widget tree lays out into `surface_w × surface_h`.
    surface_w: u32,
    surface_h: u32,
    scale: i32,
    configured: bool,
    /// Set whenever something happened that needs a fresh frame on the
    /// main window. Drawing clears it; the next state change re-sets it.
    /// Without this flag we'd hammer the compositor with one buffer per
    /// loop iteration (~60Hz) and eventually get a BrokenPipe.
    needs_redraw: bool,
    exit: bool,

    keyboard: Option<wl_keyboard::WlKeyboard>,
    pointer: Option<wl_pointer::WlPointer>,
    modifiers: Modifiers,
    /// Cursor position in *widget-tree logical coordinates* — i.e., the
    /// coordinates the widget tree expects (already converted from
    /// pointer pixels, and translated by the popup anchor when the
    /// cursor is over a popup).
    cursor: Option<Point>,

    popup: Option<PopupState>,
    qh: QueueHandle<State>,
}

/// Wayland-side state for the subordinate window that hosts a widget
/// `PopupRequest`. The variant carries the actual xdg object — a
/// dropdown-style popup for menus, or a real top-level dialog window.
enum ChildSurface {
    Popup(Popup),
    Dialog {
        window: XdgWindow,
        /// `xdg_dialog_v1` ancillary object that flags the toplevel as
        /// a (modal) dialog when the compositor advertises the
        /// protocol. `None` when the global is unavailable.
        dialog_v1: Option<XdgDialogV1>,
    },
}

impl ChildSurface {
    fn wl_surface(&self) -> &wl_surface::WlSurface {
        match self {
            ChildSurface::Popup(p) => p.wl_surface(),
            ChildSurface::Dialog { window, .. } => window.wl_surface(),
        }
    }

    fn kind(&self) -> PopupKind {
        match self {
            ChildSurface::Popup(_) => PopupKind::Popup,
            ChildSurface::Dialog { .. } => PopupKind::Dialog,
        }
    }
}

impl Drop for ChildSurface {
    fn drop(&mut self) {
        if let ChildSurface::Dialog { dialog_v1: Some(d), .. } = self {
            d.destroy();
        }
    }
}

struct PopupState {
    surface: ChildSurface,
    pool: SlotPool,
    anchor: Rect,
    /// Popup surface (logical) dimensions. Buffer is `surface_w * scale`
    /// × `surface_h * scale` physical pixels.
    surface_w: u32,
    surface_h: u32,
    configured: bool,
    needs_redraw: bool,
    /// Cursor inside the popup, in widget-tree logical coords.
    cursor: Option<Point>,
}

impl State {
    /// Per-loop housekeeping: sync popup window state with the widget
    /// tree, then redraw any surface that asked for it. Idle iterations
    /// (no state changes since the last frame) do nothing — without
    /// gating on these flags we'd attach one buffer per loop tick and
    /// drown the compositor.
    fn tick(&mut self) {
        self.sync_popup();

        // Animation: while any widget asks for ticks, fan one out each
        // loop iteration (~60 Hz). Idle widgets ignore the event, so
        // the cost is a single function call per widget per frame.
        if self.root.wants_ticks() {
            self.dispatch(Event::Tick);
        }

        if self.configured && self.needs_redraw {
            self.draw_main();
            self.needs_redraw = false;
        }
        let popup_should_draw = matches!(
            self.popup.as_ref(),
            Some(p) if p.configured && p.needs_redraw
        );
        if popup_should_draw && self.draw_popup() {
            if let Some(p) = self.popup.as_mut() {
                p.needs_redraw = false;
            }
        }
    }

    fn relayout(&mut self) {
        // Widget tree's logical coordinates equal Wayland's surface
        // coordinates — both are DPI-independent. Buffer scaling is
        // applied later, when we hand the painter to the widget tree.
        self.root.layout(Rect::new(
            0,
            0,
            self.surface_w.max(1) as i32,
            self.surface_h.max(1) as i32,
        ));
    }

    fn dispatch(&mut self, event: Event) {
        let mut ctx = EventCtx::new();
        self.root.event(&event, &mut ctx);
        if ctx.paint_requested {
            self.needs_redraw = true;
            if let Some(p) = self.popup.as_mut() {
                p.needs_redraw = true;
            }
        }
        if ctx.close_requested {
            self.exit = true;
        }
    }

    fn draw_main(&mut self) {
        let scale = self.scale.max(1);
        let buf_w = (self.surface_w.max(1) * scale as u32) as i32;
        let buf_h = (self.surface_h.max(1) * scale as u32) as i32;
        let stride = buf_w * 4;
        let buffer = match self.pool.create_buffer(
            buf_w,
            buf_h,
            stride,
            wl_shm::Format::Argb8888,
        ) {
            Ok((b, _)) => b,
            Err(_) => return,
        };
        let canvas = match self.pool.canvas(&buffer) {
            Some(c) => c,
            None => return,
        };
        let pixels = bytes_as_u32_mut(canvas);

        // Buffer holds physical pixels; the painter multiplies the
        // widget tree's logical coords by `scale` to land on them.
        let mut painter = Painter::with_popup_pass(
            pixels,
            buf_w,
            buf_h,
            scale as f32,
            0,
            0,
            self.font.as_ref(),
            self.mono_font.as_ref(),
            false,
        );
        painter.fill(self.theme.background);
        self.root.paint(&mut painter, &self.theme);

        let surface = self.window.wl_surface();
        let _ = buffer.attach_to(surface);
        // damage_buffer takes buffer-pixel coordinates.
        surface.damage_buffer(0, 0, buf_w, buf_h);
        // Tell the compositor our buffer is `scale`× the surface size,
        // so it doesn't upscale on HiDPI — we already drew at native
        // resolution.
        surface.set_buffer_scale(scale);
        surface.frame(&self.qh, surface.clone());
        surface.commit();
    }

    /// Draw the popup window. Returns true if anything was drawn.
    fn draw_popup(&mut self) -> bool {
        let scale = self.scale.max(1);
        let Some(p) = self.popup.as_mut() else { return false };
        let buf_w = (p.surface_w.max(1) * scale as u32) as i32;
        let buf_h = (p.surface_h.max(1) * scale as u32) as i32;
        let stride = buf_w * 4;
        let buffer = match p.pool.create_buffer(
            buf_w,
            buf_h,
            stride,
            wl_shm::Format::Argb8888,
        ) {
            Ok((b, _)) => b,
            Err(_) => return false,
        };
        let canvas = match p.pool.canvas(&buffer) {
            Some(c) => c,
            None => return false,
        };
        let pixels = bytes_as_u32_mut(canvas);
        let scale_f = scale as f32;
        let anchor = p.anchor;
        let origin_x = -((anchor.x as f32 * scale_f).round() as i32);
        let origin_y = -((anchor.y as f32 * scale_f).round() as i32);
        let clip_w = (anchor.w as f32 * scale_f).round() as i32;
        let clip_h = (anchor.h as f32 * scale_f).round() as i32;

        let mut painter = Painter::with_popup_pass(
            pixels,
            buf_w,
            buf_h,
            scale_f,
            origin_x,
            origin_y,
            self.font.as_ref(),
            self.mono_font.as_ref(),
            true,
        );
        painter.fill(self.theme.background);
        painter.set_clip_phys(0, 0, clip_w, clip_h);
        self.root.paint(&mut painter, &self.theme);
        painter.clear_clip();

        let surface = p.surface.wl_surface();
        let _ = buffer.attach_to(surface);
        surface.damage_buffer(0, 0, buf_w, buf_h);
        surface.set_buffer_scale(scale);
        surface.frame(&self.qh, surface.clone());
        surface.commit();
        true
    }

    /// Sync the popup window state with the widget tree's
    /// `popup_request`. Opens, destroys, or repositions as needed.
    fn sync_popup(&mut self) {
        let request = self.root.popup_request();
        let existing = self
            .popup
            .as_ref()
            .map(|p| (p.anchor, p.surface.kind()));
        match (request, existing) {
            (None, Some(_)) => {
                self.popup = None;
            }
            (Some(req), None) => {
                self.open_popup(req);
            }
            (Some(req), Some((existing_anchor, existing_kind)))
                if existing_anchor != req.rect || existing_kind != req.kind =>
            {
                // Anchor changed (slide-over), or the widget asked for a
                // different host-window kind. Easiest correct path: close
                // + reopen with the new positioner / toplevel.
                self.popup = None;
                self.open_popup(req);
            }
            _ => {}
        }
    }

    fn open_popup(&mut self, request: PopupRequest) {
        let anchor = request.rect;
        // Buffer dimensions == surface dimensions (we don't set
        // buffer_scale). Anchor is already in surface coords.
        let phys_w = anchor.w.max(1) as u32;
        let phys_h = anchor.h.max(1) as u32;

        let surface = match request.kind {
            PopupKind::Popup => {
                // Build a positioner anchored to a 1×1 rect at the
                // popup's top-left in the parent surface. Gravity goes
                // BottomRight so the popup extends down/right from the
                // anchor — same shape as a classic dropdown menu.
                let positioner: XdgPositioner = self
                    .xdg_shell
                    .xdg_wm_base()
                    .create_positioner(&self.qh, ());
                positioner.set_size(anchor.w.max(1), anchor.h.max(1));
                positioner.set_anchor_rect(anchor.x, anchor.y, 1, 1);
                positioner.set_anchor(Anchor::BottomLeft);
                positioner.set_gravity(Gravity::BottomRight);

                let popup = match Popup::new(
                    self.window.xdg_surface(),
                    &positioner,
                    &self.qh,
                    &self.compositor,
                    &self.xdg_shell,
                ) {
                    Ok(p) => p,
                    Err(_) => return,
                };
                positioner.destroy();
                ChildSurface::Popup(popup)
            }
            PopupKind::Dialog => {
                // A modal dialog is a real top-level with server-side
                // decorations: the compositor draws the title bar +
                // close button, and we ask it to enforce a fixed size
                // (set_min_size == set_max_size disables the resize
                // affordances). `set_parent` makes the dialog transient
                // to the main window. If the compositor advertises
                // `xdg_wm_dialog_v1` we additionally register the
                // toplevel as a dialog and ask for modal semantics —
                // that's what tells wlroots-based compositors (river,
                // labwc, …) to drop the minimize / maximize controls
                // from the SSD chrome.
                let dialog_surface = self.compositor.create_surface(&self.qh);
                let dialog = self.xdg_shell.create_window(
                    dialog_surface,
                    WindowDecorations::RequestServer,
                    &self.qh,
                );
                let title = request.title.as_deref().unwrap_or("Dialog");
                dialog.set_title(title);
                dialog.set_parent(Some(&self.window));
                dialog.set_min_size(Some((phys_w, phys_h)));
                dialog.set_max_size(Some((phys_w, phys_h)));

                let dialog_v1 = self.xdg_dialog_mgr.as_ref().map(|mgr| {
                    let d = mgr.get_xdg_dialog(dialog.xdg_toplevel(), &self.qh, ());
                    d.set_modal();
                    d
                });

                dialog.commit();
                ChildSurface::Dialog { window: dialog, dialog_v1 }
            }
        };

        // Pool sized for two buffers at the maximum DPI we might see
        // (popup might be rendered at scale 1 or 2). Doubling avoids
        // exhausting the pool when SCTK is double-buffering and the
        // previous buffer isn't yet released.
        let max_scale = self.scale.max(1) as u32;
        let pool_bytes = (phys_w * phys_h * max_scale * max_scale * 4) as usize * 2;
        let pool = match SlotPool::new(pool_bytes, &self.shm) {
            Ok(p) => p,
            Err(_) => return,
        };

        self.popup = Some(PopupState {
            surface,
            pool,
            anchor,
            surface_w: phys_w,
            surface_h: phys_h,
            configured: false,
            needs_redraw: true,
            cursor: None,
        });
    }

    fn physical_to_logical(&self, surface_x: f64, surface_y: f64) -> Point {
        let s = self.scale.max(1) as f64;
        // Surface coords are already in logical pixels when wl_pointer
        // reports them (they're "in surface coordinates"). The conversion
        // to scaled pixels happens when we render. So no scale factor
        // applied here — surface coordinates already match the widget
        // tree's logical units.
        let _ = s;
        Point::new(surface_x.floor() as i32, surface_y.floor() as i32)
    }
}

// ---------------------------------------------------------------- Handlers

impl CompositorHandler for State {
    fn scale_factor_changed(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _surface: &wl_surface::WlSurface,
        new_factor: i32,
    ) {
        let new = new_factor.max(1);
        // Some compositors emit this event after every commit even when
        // the factor hasn't changed. Ignore no-op transitions —
        // relayout invalidates MenuBar's cached popup geometry, and
        // doing it every frame causes the popup to flicker
        // open/close in a loop.
        if new == self.scale {
            return;
        }
        self.scale = new;
        self.needs_redraw = true;
        if let Some(p) = self.popup.as_mut() {
            p.needs_redraw = true;
        }
        self.relayout();
    }

    fn transform_changed(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _surface: &wl_surface::WlSurface,
        _new_transform: wl_output::Transform,
    ) {
    }

    fn frame(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _surface: &wl_surface::WlSurface,
        _time: u32,
    ) {
        // Compositor invites another frame; we'll draw inside the next
        // `tick`. No-op here.
    }

    fn surface_enter(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _surface: &wl_surface::WlSurface,
        _output: &wl_output::WlOutput,
    ) {
    }

    fn surface_leave(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _surface: &wl_surface::WlSurface,
        _output: &wl_output::WlOutput,
    ) {
    }
}

impl OutputHandler for State {
    fn output_state(&mut self) -> &mut OutputState {
        &mut self.output_state
    }
    fn new_output(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {}
    fn update_output(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {}
    fn output_destroyed(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {}
}

impl WindowHandler for State {
    fn request_close(&mut self, _: &Connection, _: &QueueHandle<Self>, window: &XdgWindow) {
        if window.xdg_toplevel() == self.window.xdg_toplevel() {
            self.exit = true;
            return;
        }
        // Dialog window close-request: synthesize Escape so the dialog
        // widget's dismiss path runs (which clears `open`, the next
        // sync_popup tear-down then destroys the toplevel).
        if let Some(p) = self.popup.as_ref()
            && let ChildSurface::Dialog { window: dialog, .. } = &p.surface
            && dialog.xdg_toplevel() == window.xdg_toplevel()
        {
            let mods = self.modifiers;
            self.dispatch(Event::KeyDown {
                key: Key::Named(NamedKey::Escape),
                modifiers: mods,
            });
        }
    }

    fn configure(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        window: &XdgWindow,
        configure: WindowConfigure,
        _serial: u32,
    ) {
        if window.xdg_toplevel() == self.window.xdg_toplevel() {
            let w = configure
                .new_size
                .0
                .map(|v| v.get())
                .unwrap_or(self.surface_w.max(1));
            let h = configure
                .new_size
                .1
                .map(|v| v.get())
                .unwrap_or(self.surface_h.max(1));
            self.surface_w = w;
            self.surface_h = h;
            let first_configure = !self.configured;
            self.configured = true;
            self.needs_redraw = true;
            self.relayout();
            if first_configure {
                // Match the winit backend: auto-focus the first focusable
                // widget on initial configure so single-widget roots react
                // to keyboard input out of the box.
                self.root.focus_first();
            }
            return;
        }
        // Dialog toplevel configure. We sized the window at open time
        // and don't allow resizing (set_max_size == set_min_size), so
        // the compositor normally echoes our requested size back. If it
        // proposes something else, accept it but keep at least 1px on
        // each axis.
        if let Some(p) = self.popup.as_mut()
            && let ChildSurface::Dialog { window: dialog, .. } = &p.surface
            && dialog.xdg_toplevel() == window.xdg_toplevel()
        {
            let w = configure
                .new_size
                .0
                .map(|v| v.get())
                .unwrap_or(p.surface_w.max(1));
            let h = configure
                .new_size
                .1
                .map(|v| v.get())
                .unwrap_or(p.surface_h.max(1));
            p.surface_w = w;
            p.surface_h = h;
            p.configured = true;
            p.needs_redraw = true;
        }
    }
}

impl PopupHandler for State {
    fn configure(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        popup: &Popup,
        configure: PopupConfigure,
    ) {
        if let Some(p) = self.popup.as_mut()
            && let ChildSurface::Popup(existing) = &p.surface
            && existing.xdg_popup() == popup.xdg_popup()
        {
            p.surface_w = configure.width.max(1) as u32;
            p.surface_h = configure.height.max(1) as u32;
            p.configured = true;
            p.needs_redraw = true;
        }
    }

    fn done(&mut self, _: &Connection, _: &QueueHandle<Self>, _popup: &Popup) {
        // Compositor dismissed our popup (clicked outside, etc.).
        // Synthesize an Escape so the menu cleans up cleanly.
        let mods = self.modifiers;
        self.dispatch(Event::KeyDown {
            key: Key::Named(NamedKey::Escape),
            modifiers: mods,
        });
    }
}

impl SeatHandler for State {
    fn seat_state(&mut self) -> &mut SeatState {
        &mut self.seat_state
    }
    fn new_seat(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_seat::WlSeat) {}
    fn new_capability(
        &mut self,
        _conn: &Connection,
        qh: &QueueHandle<Self>,
        seat: wl_seat::WlSeat,
        capability: Capability,
    ) {
        if capability == Capability::Keyboard && self.keyboard.is_none() {
            self.keyboard = self
                .seat_state
                .get_keyboard(qh, &seat, None)
                .ok();
        }
        if capability == Capability::Pointer && self.pointer.is_none() {
            self.pointer = self.seat_state.get_pointer(qh, &seat).ok();
        }
    }
    fn remove_capability(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: wl_seat::WlSeat,
        capability: Capability,
    ) {
        if capability == Capability::Keyboard
            && let Some(k) = self.keyboard.take()
        {
            k.release();
        }
        if capability == Capability::Pointer
            && let Some(p) = self.pointer.take()
        {
            p.release();
        }
    }
    fn remove_seat(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_seat::WlSeat) {}
}

impl KeyboardHandler for State {
    fn enter(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _keyboard: &wl_keyboard::WlKeyboard,
        _surface: &wl_surface::WlSurface,
        _serial: u32,
        _raw: &[u32],
        _keysyms: &[Keysym],
    ) {
    }
    fn leave(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_keyboard::WlKeyboard,
        _: &wl_surface::WlSurface,
        _: u32,
    ) {
    }
    fn press_key(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_keyboard::WlKeyboard,
        _: u32,
        event: WlKeyEvent,
    ) {
        self.handle_key(event, true);
    }
    fn release_key(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_keyboard::WlKeyboard,
        _: u32,
        event: WlKeyEvent,
    ) {
        self.handle_key(event, false);
    }
    fn update_modifiers(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_keyboard::WlKeyboard,
        _: u32,
        modifiers: WlModifiers,
        _layout: u32,
    ) {
        self.modifiers = Modifiers {
            shift: modifiers.shift,
            control: modifiers.ctrl,
            alt: modifiers.alt,
            logo: modifiers.logo,
        };
    }
}

impl State {
    fn handle_key(&mut self, event: WlKeyEvent, pressed: bool) {
        let modifiers = self.modifiers;
        let mapped = map_keysym(event.keysym);
        if pressed {
            if let Some(mapped) = mapped {
                self.dispatch(Event::KeyDown {
                    key: mapped,
                    modifiers,
                });
            }
            if !modifiers.has_command()
                && let Some(utf8) = event.utf8.as_deref()
            {
                for ch in utf8.chars() {
                    if (ch.is_control() && ch != '\t' && ch != '\n') || ch == '\r' {
                        continue;
                    }
                    self.dispatch(Event::Char { ch, modifiers });
                }
            }
        } else if let Some(mapped) = mapped {
            self.dispatch(Event::KeyUp {
                key: mapped,
                modifiers,
            });
        }
    }
}

impl PointerHandler for State {
    fn pointer_frame(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _pointer: &wl_pointer::WlPointer,
        events: &[PointerEvent],
    ) {
        for event in events {
            let in_popup = self
                .popup
                .as_ref()
                .map(|p| p.surface.wl_surface().id() == event.surface.id())
                .unwrap_or(false);
            let pos = if in_popup {
                let anchor = self.popup.as_ref().unwrap().anchor;
                Point::new(
                    event.position.0.floor() as i32 + anchor.x,
                    event.position.1.floor() as i32 + anchor.y,
                )
            } else {
                self.physical_to_logical(event.position.0, event.position.1)
            };

            match event.kind {
                PointerEventKind::Enter { .. } | PointerEventKind::Motion { .. } => {
                    if in_popup {
                        if let Some(p) = self.popup.as_mut() {
                            p.cursor = Some(pos);
                        }
                    } else {
                        self.cursor = Some(pos);
                    }
                    self.dispatch(Event::PointerMove { pos });
                    if in_popup {
                        if let Some(p) = self.popup.as_mut() {
                            p.needs_redraw = true;
                        }
                    }
                }
                PointerEventKind::Leave { .. } => {
                    if in_popup {
                        if let Some(p) = self.popup.as_mut() {
                            p.cursor = None;
                        }
                    } else {
                        self.cursor = None;
                    }
                    self.dispatch(Event::PointerLeave);
                }
                PointerEventKind::Press { button, .. } => {
                    let Some(b) = map_button(button) else { continue };
                    self.dispatch(Event::PointerDown { pos, button: b });
                    if in_popup
                        && let Some(p) = self.popup.as_mut()
                    {
                        p.needs_redraw = true;
                    }
                }
                PointerEventKind::Release { button, .. } => {
                    let Some(b) = map_button(button) else { continue };
                    self.dispatch(Event::PointerUp { pos, button: b });
                    if in_popup
                        && let Some(p) = self.popup.as_mut()
                    {
                        p.needs_redraw = true;
                    }
                }
                PointerEventKind::Axis { .. } => {
                    // Scroll wheel events — not surfaced yet.
                }
            }
        }
    }
}

impl ShmHandler for State {
    fn shm_state(&mut self) -> &mut Shm {
        &mut self.shm
    }
}

impl ProvidesRegistryState for State {
    fn registry(&mut self) -> &mut RegistryState {
        &mut self.registry_state
    }
    registry_handlers![OutputState, SeatState];
}

delegate_compositor!(State);
delegate_output!(State);
delegate_shm!(State);
delegate_seat!(State);
delegate_keyboard!(State);
delegate_pointer!(State);
delegate_xdg_shell!(State);
delegate_xdg_window!(State);
delegate_xdg_popup!(State);
delegate_registry!(State);

// -------------------------------------------------------------------- utils

fn sanitize(s: &str) -> String {
    s.chars()
        .map(|c| if c.is_ascii_alphanumeric() { c.to_ascii_lowercase() } else { '-' })
        .collect()
}

fn map_button(button: u32) -> Option<MouseButton> {
    // Linux input event codes for mouse buttons.
    match button {
        0x110 => Some(MouseButton::Left),
        0x111 => Some(MouseButton::Right),
        0x112 => Some(MouseButton::Middle),
        _ => None,
    }
}

fn map_keysym(keysym: Keysym) -> Option<Key> {
    use Keysym as K;
    let named = match keysym {
        K::Return | K::KP_Enter => NamedKey::Enter,
        K::BackSpace => NamedKey::Backspace,
        K::Delete | K::KP_Delete => NamedKey::Delete,
        K::Tab => NamedKey::Tab,
        K::Escape => NamedKey::Escape,
        K::space => NamedKey::Space,
        K::Left | K::KP_Left => NamedKey::Left,
        K::Right | K::KP_Right => NamedKey::Right,
        K::Up | K::KP_Up => NamedKey::Up,
        K::Down | K::KP_Down => NamedKey::Down,
        K::Home | K::KP_Home => NamedKey::Home,
        K::End | K::KP_End => NamedKey::End,
        K::Page_Up | K::KP_Page_Up => NamedKey::PageUp,
        K::Page_Down | K::KP_Page_Down => NamedKey::PageDown,
        _ => {
            let ch = keysym.key_char()?;
            return Some(Key::Char(ch));
        }
    };
    Some(Key::Named(named))
}

/// Reinterpret an `[u8]` framebuffer as `[u32]`. The SCTK slot pool gives us
/// raw bytes; the painter wants ARGB32 pixels.
fn bytes_as_u32_mut(bytes: &mut [u8]) -> &mut [u32] {
    let len = bytes.len() / 4;
    // SAFETY: a contiguous byte buffer whose length is a multiple of 4
    // aliases a `[u32]` of length `len`. ARGB32 in little-endian is the
    // natural memory order for `Color`'s 0xAARRGGBB.
    unsafe { std::slice::from_raw_parts_mut(bytes.as_mut_ptr() as *mut u32, len) }
}

// xdg_positioner has no incoming events; SCTK doesn't manage it for us
// because we create it ad-hoc per popup. An empty Dispatch impl
// satisfies the queue-handle requirement.
impl Dispatch<XdgPositioner, ()> for State {
    fn event(
        _state: &mut Self,
        _proxy: &XdgPositioner,
        _event: <XdgPositioner as Proxy>::Event,
        _data: &(),
        _conn: &Connection,
        _qhandle: &QueueHandle<Self>,
    ) {
    }
}

// The xdg_wm_dialog_v1 / xdg_dialog_v1 interfaces are sender-only: the
// client makes requests but receives no events. Empty Dispatch impls
// are enough to satisfy the queue-handle requirement on both objects.
impl Dispatch<XdgWmDialogV1, ()> for State {
    fn event(
        _state: &mut Self,
        _proxy: &XdgWmDialogV1,
        _event: <XdgWmDialogV1 as Proxy>::Event,
        _data: &(),
        _conn: &Connection,
        _qhandle: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<XdgDialogV1, ()> for State {
    fn event(
        _state: &mut Self,
        _proxy: &XdgDialogV1,
        _event: <XdgDialogV1 as Proxy>::Event,
        _data: &(),
        _conn: &Connection,
        _qhandle: &QueueHandle<Self>,
    ) {
    }
}

// Imports kept around for future expansion (buffer reuse, Arc-shared
// state across worker threads).
#[allow(dead_code)]
fn _unused(_b: Buffer, _arc: Arc<()>) {}
