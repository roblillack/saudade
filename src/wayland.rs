//! Pure SCTK (smithay-client-toolkit) Wayland backend.
//!
//! Used in place of winit when the process is started on a Wayland session
//! (`WAYLAND_DISPLAY` is set). The widget tree, the painter, and every other
//! piece of saudade stay the same — only the windowing + event loop differ.
//!
//! Why SCTK rather than winit's Wayland support: winit 0.30 still doesn't
//! implement `xdg_popup`, so popups would fall back to plain `xdg_toplevel`s
//! that the compositor places wherever it likes. Going through SCTK gives us
//! real popups with a positioner anchored to the parent — the same behavior
//! Chrome/Firefox have on Wayland.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use smithay_client_toolkit::compositor::{CompositorHandler, CompositorState};
use smithay_client_toolkit::data_device_manager::data_device::{DataDevice, DataDeviceHandler};
use smithay_client_toolkit::data_device_manager::data_offer::{DataOfferHandler, DragOffer};
use smithay_client_toolkit::data_device_manager::data_source::DataSourceHandler;
use smithay_client_toolkit::data_device_manager::{DataDeviceManagerState, WritePipe};
use smithay_client_toolkit::output::{OutputHandler, OutputState};
use smithay_client_toolkit::reexports::calloop::{
    EventLoop as CalloopLoop, LoopHandle, PostAction,
};
use smithay_client_toolkit::reexports::calloop_wayland_source::WaylandSource;
use smithay_client_toolkit::registry::{ProvidesRegistryState, RegistryState};
use smithay_client_toolkit::seat::keyboard::{
    KeyEvent as WlKeyEvent, KeyboardHandler, Keysym, Modifiers as WlModifiers,
};
use smithay_client_toolkit::seat::pointer::{
    AxisScroll, PointerEvent, PointerEventKind, PointerHandler,
};
use smithay_client_toolkit::seat::{Capability, SeatHandler, SeatState};
use smithay_client_toolkit::shell::WaylandSurface;
use smithay_client_toolkit::shell::xdg::popup::{Popup, PopupConfigure, PopupHandler};
use smithay_client_toolkit::shell::xdg::window::{
    Window as XdgWindow, WindowConfigure, WindowDecorations, WindowHandler,
};
use smithay_client_toolkit::shell::xdg::{XdgShell, XdgSurface};
use smithay_client_toolkit::shm::slot::{Buffer, SlotPool};
use smithay_client_toolkit::shm::{Shm, ShmHandler};
use smithay_client_toolkit::{
    delegate_compositor, delegate_data_device, delegate_keyboard, delegate_output,
    delegate_pointer, delegate_registry, delegate_seat, delegate_shm, delegate_xdg_popup,
    delegate_xdg_shell, delegate_xdg_window, registry_handlers,
};
use wayland_client::globals::registry_queue_init;
use wayland_client::protocol::wl_data_device::WlDataDevice;
use wayland_client::protocol::wl_data_device_manager::DndAction;
use wayland_client::protocol::wl_data_source::WlDataSource;
use wayland_client::protocol::{wl_keyboard, wl_output, wl_pointer, wl_seat, wl_shm, wl_surface};
use wayland_client::{Connection, Dispatch, Proxy, QueueHandle};
use wayland_protocols::wp::fractional_scale::v1::client::wp_fractional_scale_manager_v1::WpFractionalScaleManagerV1;
use wayland_protocols::wp::fractional_scale::v1::client::wp_fractional_scale_v1::{
    self, WpFractionalScaleV1,
};
use wayland_protocols::xdg::dialog::v1::client::xdg_dialog_v1::XdgDialogV1;
use wayland_protocols::xdg::dialog::v1::client::xdg_wm_dialog_v1::XdgWmDialogV1;
use wayland_protocols::xdg::shell::client::xdg_positioner::{Anchor, Gravity, XdgPositioner};
use wayland_protocols::xdg::shell::client::xdg_surface::XdgSurface as XdgSurfaceObj;

use crate::app::App;
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

pub(crate) fn run(app: App) {
    let (window_cfg, theme, root) = app.into_parts();

    let conn = Connection::connect_to_env().expect("saudade: Wayland connect failed");
    let (globals, event_queue) =
        registry_queue_init::<State>(&conn).expect("saudade: registry init failed");
    let qh: QueueHandle<State> = event_queue.handle();

    let mut event_loop: CalloopLoop<State> =
        CalloopLoop::try_new().expect("saudade: calloop init failed");
    let loop_handle = event_loop.handle();
    WaylandSource::new(conn.clone(), event_queue)
        .insert(loop_handle)
        .expect("saudade: WaylandSource insert failed");

    let compositor =
        CompositorState::bind(&globals, &qh).expect("saudade: wl_compositor not available");
    let xdg_shell = XdgShell::bind(&globals, &qh).expect("saudade: xdg_shell not available");
    let shm = Shm::bind(&globals, &qh).expect("saudade: wl_shm not available");
    // Optional: the dialog protocol is a "staging" extension. Compositors
    // that don't advertise it fall back to plain xdg_toplevel with
    // set_parent — still a real top-level, just without the explicit
    // "this is a dialog, hide min/max" hint.
    let xdg_dialog_mgr: Option<XdgWmDialogV1> =
        globals.bind::<XdgWmDialogV1, _, _>(&qh, 1..=1, ()).ok();
    // Optional: wp_fractional_scale_manager_v1 (a staging extension) lets the
    // compositor tell us the surface's *true* fractional scale (e.g. 1.5),
    // distinct from the integer buffer scale we render at. Purely
    // informational here — we keep rendering at the integer scale and let the
    // compositor resample. Compositors that don't advertise it leave us with
    // only the integer scale, which is the right fallback.
    let fractional_mgr: Option<WpFractionalScaleManagerV1> = globals
        .bind::<WpFractionalScaleManagerV1, _, _>(&qh, 1..=1, ())
        .ok();
    // The data-device manager is what carries drag-and-drop (we get file drops
    // through its drag offers). Compositors universally advertise it; if one
    // doesn't, we simply never receive drops and everything else still works.
    let data_device_manager = DataDeviceManagerState::bind(&globals, &qh).ok();

    let surface = compositor.create_surface(&qh);
    let window = xdg_shell.create_window(surface, WindowDecorations::RequestServer, &qh);
    window.set_title(&window_cfg.title);
    window.set_app_id(format!("saudade.{}", sanitize(&window_cfg.title)));
    // Subscribe the main surface to fractional-scale notifications, if the
    // compositor supports them. The returned object is kept alive in `State`
    // so the `preferred_scale` events keep arriving.
    let fractional_scale_obj = fractional_mgr
        .as_ref()
        .map(|mgr| mgr.get_fractional_scale(window.wl_surface(), &qh, ()));

    let initial_w = window_cfg.size.w.max(1) as u32;
    let initial_h = window_cfg.size.h.max(1) as u32;

    if window_cfg.resizable {
        window.set_min_size(Some((100, 60)));
    } else {
        // Fixed-size window: min == max tells the compositor the surface
        // is unresizable. Without the max hint, wlroots-based compositors
        // (river, …) report `max=(0x0)` → `is_fixed_size=false` and still
        // offer resize edges. Pinning both to the configured size mirrors
        // the winit backend's `with_resizable(false)`.
        window.set_min_size(Some((initial_w, initial_h)));
        window.set_max_size(Some((initial_w, initial_h)));
    }
    window.commit();

    let pool = SlotPool::new((initial_w * initial_h * 4) as usize * 4, &shm)
        .expect("saudade: slot pool init failed");

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
        fractional_scale_obj,
        fractional_scale: None,
        resizable: window_cfg.resizable,
        configured: false,
        needs_redraw: true,
        exit: false,

        keyboard: None,
        pointer: None,
        data_device_manager,
        data_device: None,
        drag: None,
        modifiers: Modifiers::default(),
        bg: BackgroundState::from_env(),
        cursor: None,

        popups: Vec::new(),
        qh: qh.clone(),
        loop_handle: event_loop.handle(),
    };
    drop(conn);

    while !state.exit {
        event_loop
            .dispatch(Duration::from_millis(16), &mut state)
            .expect("saudade: dispatch failed");
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
    /// `wp_fractional_scale_v1` for the main surface, held only to keep the
    /// `preferred_scale` events flowing. `None` when the compositor doesn't
    /// advertise the protocol.
    #[allow(dead_code)]
    fractional_scale_obj: Option<WpFractionalScaleV1>,
    /// True fractional display scale (e.g. 1.5) reported by the fractional-scale
    /// protocol, as opposed to `scale` — the integer buffer scale we actually
    /// rasterize at. `None` until reported / when unsupported, in which case
    /// callers fall back to the integer `scale`.
    fractional_scale: Option<f32>,
    /// Whether the window was configured resizable. A fixed window pins
    /// min == max size, so a programmatic resize ([`Self::apply_resize`])
    /// must move both hints to the new size; a resizable one leaves them be.
    resizable: bool,
    configured: bool,
    /// Set whenever something happened that needs a fresh frame on the
    /// main window. Drawing clears it; the next state change re-sets it.
    /// Without this flag we'd hammer the compositor with one buffer per
    /// loop iteration (~60Hz) and eventually get a BrokenPipe.
    needs_redraw: bool,
    exit: bool,

    keyboard: Option<wl_keyboard::WlKeyboard>,
    pointer: Option<wl_pointer::WlPointer>,
    /// Drag-and-drop plumbing. `data_device_manager` is the bound global;
    /// `data_device` is the per-seat device we read drag offers from, created
    /// once the first seat appears. `drag` tracks an in-flight drag so motion
    /// events know where they are and a drop knows which point to report.
    data_device_manager: Option<DataDeviceManagerState>,
    data_device: Option<DataDevice>,
    drag: Option<DragSession>,
    modifiers: Modifiers,
    /// Background pattern + color for the main window, toggled with the
    /// `p` / `c` debug keys. Popups/dialogs ignore it and stay white.
    bg: BackgroundState,
    /// Cursor position in *widget-tree logical coordinates* — i.e., the
    /// coordinates the widget tree expects (already converted from
    /// pointer pixels, and translated by the popup anchor when the
    /// cursor is over a popup).
    cursor: Option<Point>,

    /// Stack of popup windows, outermost first — a dropdown opened inside a
    /// dialog nests a second entry on top of it.
    popups: Vec<PopupState>,
    qh: QueueHandle<State>,
    /// Calloop handle, captured during startup so the keyboard-acquire
    /// path in `new_capability` can hand it to SCTK's `get_keyboard_with_repeat`
    /// — that's what arms the timer that turns held keys into repeated
    /// `KeyDown` / `Char` events.
    loop_handle: LoopHandle<'static, State>,
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

    /// The `xdg_surface` to parent a *nested* popup to (a dropdown opened inside
    /// this dialog, say).
    fn xdg_surface(&self) -> &XdgSurfaceObj {
        match self {
            ChildSurface::Popup(p) => p.xdg_surface(),
            ChildSurface::Dialog { window, .. } => window.xdg_surface(),
        }
    }
}

impl Drop for ChildSurface {
    fn drop(&mut self) {
        if let ChildSurface::Dialog {
            dialog_v1: Some(d), ..
        } = self
        {
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

/// State for a drag currently hovering over one of our surfaces. Wayland's data
/// device reports drag motion in surface-local coordinates without re-stating
/// which surface, so we remember the anchor offset of the surface the drag
/// entered (zero for the main window, the popup's anchor for a popup) and the
/// last reported position so a drop knows where to land.
struct DragSession {
    /// Offset added to surface-local drag coordinates to reach widget-tree
    /// logical coordinates — the same translation pointer events use.
    anchor: Point,
    /// Last reported position in widget-tree logical coordinates.
    pos: Point,
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
        for idx in 0..self.popups.len() {
            let should_draw = self.popups[idx].configured && self.popups[idx].needs_redraw;
            if should_draw && self.draw_popup(idx) {
                self.popups[idx].needs_redraw = false;
            }
        }
    }

    /// Mark every popup window dirty — the right thing after any dispatch, since
    /// one event can change widgets shown across several popups (e.g. closing a
    /// nested dropdown repaints the dialog beneath it).
    fn mark_popups_dirty(&mut self) {
        for p in &mut self.popups {
            p.needs_redraw = true;
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
            self.mark_popups_dirty();
        }
        if ctx.close_requested {
            self.exit = true;
        }
        if let Some(size) = ctx.resize_request {
            self.apply_resize(size);
        }
    }

    /// Resize the window to `size` logical (surface) pixels, at the widget's
    /// request. For a fixed window we move the min == max hints to the new size
    /// so the compositor lets it change; then we adopt the size, relayout, and
    /// repaint. The compositor echoes a configure with the same size, which the
    /// configure handler treats as a no-op transition.
    fn apply_resize(&mut self, size: Size) {
        let w = size.w.max(1) as u32;
        let h = size.h.max(1) as u32;
        if w == self.surface_w && h == self.surface_h {
            return;
        }
        if !self.resizable {
            self.window.set_min_size(Some((w, h)));
            self.window.set_max_size(Some((w, h)));
        }
        self.surface_w = w;
        self.surface_h = h;
        self.window.commit();
        self.relayout();
        self.needs_redraw = true;
        self.mark_popups_dirty();
    }

    fn draw_main(&mut self) {
        let scale = self.scale.max(1);
        let buf_w = (self.surface_w.max(1) * scale as u32) as i32;
        let buf_h = (self.surface_h.max(1) * scale as u32) as i32;
        let stride = buf_w * 4;
        let buffer = match self
            .pool
            .create_buffer(buf_w, buf_h, stride, wl_shm::Format::Argb8888)
        {
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
        let mut painter = Painter::with_popup_anchor(
            pixels,
            buf_w,
            buf_h,
            scale as f32,
            0,
            0,
            self.font.as_ref(),
            self.mono_font.as_ref(),
            None,
        );
        // Report the true display scale (e.g. 1.5) when the compositor gives it
        // to us; the buffer itself is still the integer `scale` and gets
        // resampled down. Falls back to the integer scale when unsupported.
        painter.set_system_scale(self.fractional_scale.unwrap_or(scale as f32));
        painter.fill_pattern(self.theme.background, self.bg.pattern, self.bg.color);
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

    /// Draw the popup window at `idx`. Returns true if anything was drawn.
    fn draw_popup(&mut self, idx: usize) -> bool {
        let scale = self.scale.max(1);
        let Some(p) = self.popups.get_mut(idx) else {
            return false;
        };
        let buf_w = (p.surface_w.max(1) * scale as u32) as i32;
        let buf_h = (p.surface_h.max(1) * scale as u32) as i32;
        let stride = buf_w * 4;
        let buffer = match p
            .pool
            .create_buffer(buf_w, buf_h, stride, wl_shm::Format::Argb8888)
        {
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

        let mut painter = Painter::with_popup_anchor(
            pixels,
            buf_w,
            buf_h,
            scale_f,
            origin_x,
            origin_y,
            self.font.as_ref(),
            self.mono_font.as_ref(),
            Some(anchor),
        );
        painter.set_system_scale(self.fractional_scale.unwrap_or(scale_f));
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

    /// Sync the popup window stack with the widget tree's active popups.
    /// Opens, destroys, or rebuilds windows so the stack matches the request
    /// list (outermost first). Keeping the longest matching prefix means
    /// opening a nested dropdown adds a window without disturbing the dialog
    /// beneath it.
    fn sync_popup(&mut self) {
        let mut requests = Vec::new();
        self.root.collect_popups(&mut requests);

        let keep = self
            .popups
            .iter()
            .zip(requests.iter())
            .take_while(|(p, req)| p.anchor == req.rect && p.surface.kind() == req.kind)
            .count();
        self.popups.truncate(keep);

        for req in requests.into_iter().skip(keep) {
            // Parent a nested popup to the current top of the stack (the dialog
            // it lives in); the first popup parents to the main window.
            let made = match self.popups.last() {
                Some(parent) => {
                    let parent_anchor = parent.anchor;
                    let parent_xdg = parent.surface.xdg_surface();
                    self.create_popup(&req, parent_anchor, Some(parent_xdg))
                }
                None => self.create_popup(&req, Rect::new(0, 0, 0, 0), None),
            };
            match made {
                Some(p) => self.popups.push(p),
                // A child popup must not outlive a parent we couldn't create.
                None => break,
            }
        }
    }

    /// Create one popup window for `request`. `parent_anchor` / `parent_xdg`
    /// describe the surface it nests into — `None` means the main window. Popup
    /// anchors are in root-widget coordinates, so the positioner is offset by
    /// the parent's anchor to land in the parent surface's local space.
    fn create_popup(
        &self,
        request: &PopupRequest,
        parent_anchor: Rect,
        parent_xdg: Option<&XdgSurfaceObj>,
    ) -> Option<PopupState> {
        let anchor = request.rect;
        // Buffer dimensions == surface dimensions (we don't set
        // buffer_scale). Anchor is already in surface coords.
        let phys_w = anchor.w.max(1) as u32;
        let phys_h = anchor.h.max(1) as u32;

        let surface = match request.kind {
            PopupKind::Popup => {
                // Build a positioner anchored to a 1×1 rect at the popup's
                // top-left *in the parent surface*. Gravity goes BottomRight so
                // the popup extends down/right from the anchor — same shape as a
                // classic dropdown menu.
                let rel_x = anchor.x - parent_anchor.x;
                let rel_y = anchor.y - parent_anchor.y;
                let positioner: XdgPositioner =
                    self.xdg_shell.xdg_wm_base().create_positioner(&self.qh, ());
                positioner.set_size(anchor.w.max(1), anchor.h.max(1));
                positioner.set_anchor_rect(rel_x, rel_y, 1, 1);
                positioner.set_anchor(Anchor::BottomLeft);
                positioner.set_gravity(Gravity::BottomRight);

                let parent = parent_xdg.unwrap_or_else(|| self.window.xdg_surface());
                let popup = match Popup::new(
                    parent,
                    &positioner,
                    &self.qh,
                    &self.compositor,
                    &self.xdg_shell,
                ) {
                    Ok(p) => p,
                    Err(_) => return None,
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
                ChildSurface::Dialog {
                    window: dialog,
                    dialog_v1,
                }
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
            Err(_) => return None,
        };

        Some(PopupState {
            surface,
            pool,
            anchor,
            surface_w: phys_w,
            surface_h: phys_h,
            configured: false,
            needs_redraw: true,
            cursor: None,
        })
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

    /// Offset to add to surface-local coordinates on `surface` to reach the
    /// widget tree's logical space: zero for the main window, the popup's
    /// anchor for one of the stacked popup surfaces. Mirrors the translation
    /// `pointer_frame` applies, so a drag and the pointer agree on coordinates.
    fn surface_anchor(&self, surface: &wl_surface::WlSurface) -> Point {
        self.popups
            .iter()
            .find(|p| p.surface.wl_surface().id() == surface.id())
            .map(|p| Point::new(p.anchor.x, p.anchor.y))
            .unwrap_or(Point::new(0, 0))
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
        self.mark_popups_dirty();
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
        if self.popups.iter().any(|p| {
            matches!(&p.surface, ChildSurface::Dialog { window: dialog, .. }
                if dialog.xdg_toplevel() == window.xdg_toplevel())
        }) {
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
        // and don't allow resizing. `set_min_size == set_max_size` is
        // only a *hint* — compositors such as Mutter still send
        // configures with user-dragged sizes and offer resize edges. We
        // deliberately IGNORE the proposed size and keep committing our
        // own fixed buffer (`surface_w`/`surface_h`, frozen at open
        // time). Since a Wayland client owns its buffer dimensions, the
        // window snaps back to our size and any resize drag has no
        // effect.
        if let Some(p) = self.popups.iter_mut().find(|p| {
            matches!(&p.surface, ChildSurface::Dialog { window: dialog, .. }
                if dialog.xdg_toplevel() == window.xdg_toplevel())
        }) {
            // surface_w / surface_h left untouched on purpose.
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
        if let Some(p) = self.popups.iter_mut().find(|p| {
            matches!(&p.surface, ChildSurface::Popup(existing)
                if existing.xdg_popup() == popup.xdg_popup())
        }) {
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
    fn new_seat(&mut self, _conn: &Connection, qh: &QueueHandle<Self>, seat: wl_seat::WlSeat) {
        // Create this seat's data device so drag offers start arriving. A drag
        // is delivered through the seat that owns the pointer; binding the
        // first seat is enough for the single-seat setups saudade targets.
        if self.data_device.is_none()
            && let Some(mgr) = self.data_device_manager.as_ref()
        {
            let device = mgr.get_data_device(qh, &seat);
            self.data_device = Some(device);
        }
    }
    fn new_capability(
        &mut self,
        _conn: &Connection,
        qh: &QueueHandle<Self>,
        seat: wl_seat::WlSeat,
        capability: Capability,
    ) {
        if capability == Capability::Keyboard && self.keyboard.is_none() {
            // Use the repeat-aware constructor so SCTK arms a calloop timer
            // based on the compositor's RepeatInfo. The callback fires once
            // per repeat tick with the same KeyEvent the original press
            // produced — we route it through `handle_key` so it walks the
            // same KeyDown / Char dispatch path as a fresh press.
            self.keyboard = self
                .seat_state
                .get_keyboard_with_repeat(
                    qh,
                    &seat,
                    None,
                    self.loop_handle.clone(),
                    Box::new(|state: &mut State, _kbd, event| {
                        state.handle_key(event, true);
                    }),
                )
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
            // On X11/Wayland AltGr is `ISO_Level3_Shift` (Mod5), reported
            // separately from Alt (Mod1) and not surfaced here, so it never
            // looks like `alt` and needs no special-casing.
            alt_graph: false,
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
            // Which surface is the event on — the main window, or one of the
            // stacked popups? Popup anchors are in root coords, so we add the
            // matching popup's anchor to land back in the widget tree's space.
            let popup_idx = self
                .popups
                .iter()
                .position(|p| p.surface.wl_surface().id() == event.surface.id());
            let pos = match popup_idx {
                Some(i) => {
                    let anchor = self.popups[i].anchor;
                    Point::new(
                        event.position.0.floor() as i32 + anchor.x,
                        event.position.1.floor() as i32 + anchor.y,
                    )
                }
                None => self.physical_to_logical(event.position.0, event.position.1),
            };

            match event.kind {
                PointerEventKind::Enter { .. } | PointerEventKind::Motion { .. } => {
                    match popup_idx {
                        Some(i) => self.popups[i].cursor = Some(pos),
                        None => self.cursor = Some(pos),
                    }
                    self.dispatch(Event::PointerMove { pos });
                    self.mark_popups_dirty();
                }
                PointerEventKind::Leave { .. } => {
                    match popup_idx {
                        Some(i) => self.popups[i].cursor = None,
                        None => self.cursor = None,
                    }
                    self.dispatch(Event::PointerLeave);
                }
                PointerEventKind::Press { button, .. } => {
                    let Some(b) = map_button(button) else {
                        continue;
                    };
                    self.dispatch(Event::PointerDown { pos, button: b });
                    self.mark_popups_dirty();
                }
                PointerEventKind::Release { button, .. } => {
                    let Some(b) = map_button(button) else {
                        continue;
                    };
                    self.dispatch(Event::PointerUp { pos, button: b });
                    self.mark_popups_dirty();
                }
                PointerEventKind::Axis {
                    horizontal,
                    vertical,
                    ..
                } => {
                    // Wayland already follows our sign convention: a positive
                    // axis value scrolls down / right, toward the content's
                    // end. Surface coordinates are logical, so the continuous
                    // (`absolute`) fallback needs no DPI scaling.
                    let lines = |axis: AxisScroll| {
                        if axis.discrete != 0 {
                            axis.discrete as f32 * WHEEL_LINES_PER_DETENT
                        } else {
                            axis.absolute as f32 / SCROLL_PIXELS_PER_LINE
                        }
                    };
                    let delta_x = lines(horizontal);
                    let delta_y = lines(vertical);
                    if delta_x != 0.0 || delta_y != 0.0 {
                        self.dispatch(Event::Scroll {
                            pos,
                            delta_x,
                            delta_y,
                        });
                        self.mark_popups_dirty();
                    }
                }
            }
        }
    }
}

impl DataDeviceHandler for State {
    fn enter(
        &mut self,
        conn: &Connection,
        _qh: &QueueHandle<Self>,
        _data_device: &WlDataDevice,
        x: f64,
        y: f64,
        surface: &wl_surface::WlSurface,
    ) {
        // Pull the offer the device just received. Cloning it out ends the
        // borrow of `self.data_device` so we can dispatch below.
        let offer = match self.data_device.as_ref() {
            Some(dd) => dd.data().drag_offer(),
            None => None,
        };
        let Some(offer) = offer else { return };

        // We only handle file drags. Reject anything without `text/uri-list`
        // so the compositor shows the "can't drop here" cursor over us.
        let accepts = offer.with_mime_types(|mimes| mimes.iter().any(|m| m == URI_LIST_MIME));
        if !accepts {
            offer.accept_mime_type(offer.serial, None);
            return;
        }
        // Accept a *copy* (never move — we must not make the source delete the
        // user's file) and tell the source we'll take the uri-list.
        offer.set_actions(DndAction::Copy, DndAction::Copy);
        offer.accept_mime_type(offer.serial, Some(URI_LIST_MIME.to_string()));
        let _ = conn.flush();

        let anchor = self.surface_anchor(surface);
        let pos = Point::new(x.floor() as i32 + anchor.x, y.floor() as i32 + anchor.y);
        self.drag = Some(DragSession { anchor, pos });
        self.dispatch(Event::DragEnter { pos });
        self.mark_popups_dirty();
    }

    fn leave(&mut self, _conn: &Connection, _qh: &QueueHandle<Self>, _data_device: &WlDataDevice) {
        if self.drag.take().is_some() {
            self.dispatch(Event::DragLeave);
            self.mark_popups_dirty();
        }
    }

    fn motion(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _data_device: &WlDataDevice,
        x: f64,
        y: f64,
    ) {
        let Some(anchor) = self.drag.as_ref().map(|d| d.anchor) else {
            return;
        };
        let pos = Point::new(x.floor() as i32 + anchor.x, y.floor() as i32 + anchor.y);
        if let Some(drag) = self.drag.as_mut() {
            drag.pos = pos;
        }
        self.dispatch(Event::DragMove { pos });
        self.mark_popups_dirty();
    }

    fn selection(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _data_device: &WlDataDevice,
    ) {
        // Clipboard selection — saudade uses arboard for the clipboard, so we
        // ignore the selection offer here.
    }

    fn drop_performed(
        &mut self,
        conn: &Connection,
        _qh: &QueueHandle<Self>,
        _data_device: &WlDataDevice,
    ) {
        let Some(drag) = self.drag.take() else { return };
        let pos = drag.pos;
        let offer = match self.data_device.as_ref() {
            Some(dd) => dd.data().drag_offer(),
            None => None,
        };
        let Some(offer) = offer else {
            // No payload offer (e.g. an internal drag): still report the drop so
            // a target can clear its highlight.
            self.dispatch(Event::Drop {
                pos,
                data: DragData::default(),
            });
            self.mark_popups_dirty();
            return;
        };

        // The drop is the protocol-guaranteed point at which the data is
        // readable. Ask for the uri-list and read it *asynchronously* off the
        // returned pipe via calloop — a blocking read here would freeze the UI
        // if the source were slow to write.
        let read_pipe = match offer.receive(URI_LIST_MIME.to_string()) {
            Ok(p) => p,
            Err(_) => {
                offer.finish();
                offer.destroy();
                self.dispatch(Event::Drop {
                    pos,
                    data: DragData::default(),
                });
                self.mark_popups_dirty();
                return;
            }
        };
        let _ = conn.flush();

        // Read the uri-list asynchronously off the pipe; `offer` is moved into
        // the reader closure and finished/destroyed once we hit EOF.
        let mut buf: Vec<u8> = Vec::new();
        let inserted =
            self.loop_handle
                .insert_source(read_pipe, move |_event, file, state: &mut State| {
                    use std::io::Read;
                    // SAFETY: we only read from the fd; calloop owns and closes it.
                    let f: &mut std::fs::File = unsafe { file.get_mut() };
                    let mut chunk = [0u8; 4096];
                    match f.read(&mut chunk) {
                        Ok(0) => {
                            // EOF: the source finished writing the uri-list.
                            let text = String::from_utf8_lossy(&buf);
                            let paths = parse_uri_list(text.as_ref());
                            offer.finish();
                            offer.destroy();
                            state.dispatch(Event::Drop {
                                pos,
                                data: DragData::from_paths(paths),
                            });
                            state.mark_popups_dirty();
                            PostAction::Remove
                        }
                        Ok(n) => {
                            buf.extend_from_slice(&chunk[..n]);
                            PostAction::Continue
                        }
                        Err(ref e)
                            if matches!(
                                e.kind(),
                                std::io::ErrorKind::Interrupted | std::io::ErrorKind::WouldBlock
                            ) =>
                        {
                            // Spurious wakeup / signal — wait for the next.
                            PostAction::Continue
                        }
                        Err(_) => {
                            offer.finish();
                            offer.destroy();
                            PostAction::Remove
                        }
                    }
                });
        if inserted.is_err() {
            // Couldn't register the reader; report an empty drop so the UI
            // doesn't get stuck mid-drag.
            self.dispatch(Event::Drop {
                pos,
                data: DragData::default(),
            });
            self.mark_popups_dirty();
        }
    }
}

impl DataOfferHandler for State {
    fn source_actions(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        offer: &mut DragOffer,
        _actions: DndAction,
    ) {
        // We only ever copy dropped files in — never move or link.
        offer.set_actions(DndAction::Copy, DndAction::Copy);
    }

    fn selected_action(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _offer: &mut DragOffer,
        _actions: DndAction,
    ) {
    }
}

// saudade only *receives* drops; it never creates a drag or clipboard source.
// `delegate_data_device!` still wires the `wl_data_source` dispatch (which
// requires this handler), so the methods are deliberate no-ops.
impl DataSourceHandler for State {
    fn accept_mime(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &WlDataSource,
        _: Option<String>,
    ) {
    }
    fn send_request(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &WlDataSource,
        _: String,
        _: WritePipe,
    ) {
    }
    fn cancelled(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &WlDataSource) {}
    fn dnd_dropped(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &WlDataSource) {}
    fn dnd_finished(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &WlDataSource) {}
    fn action(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &WlDataSource, _: DndAction) {}
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
delegate_data_device!(State);
delegate_keyboard!(State);
delegate_pointer!(State);
delegate_xdg_shell!(State);
delegate_xdg_window!(State);
delegate_xdg_popup!(State);
delegate_registry!(State);

// -------------------------------------------------------------------- utils

fn sanitize(s: &str) -> String {
    s.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect()
}

/// The MIME type that carries a newline-separated list of dragged file URIs.
/// It's the de-facto standard every file manager (Nautilus, Dolphin, Thunar, …)
/// offers for dragged files.
const URI_LIST_MIME: &str = "text/uri-list";

/// Parse an RFC 2483 `text/uri-list` payload into the local file paths it
/// references. Blank lines, comments (`#…`), and non-`file:` URIs are skipped.
fn parse_uri_list(text: &str) -> Vec<PathBuf> {
    text.lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .filter_map(file_uri_to_path)
        .collect()
}

/// Turn a single `file://[host]/path` URI into a [`PathBuf`], percent-decoding
/// the path. Returns `None` for URIs that aren't `file:` (we can't open a
/// remote resource as a path).
fn file_uri_to_path(uri: &str) -> Option<PathBuf> {
    let rest = uri.strip_prefix("file://")?;
    // Drop an optional authority (host) component: the path starts at the
    // first '/'. `file:///path` → empty authority, path "/path";
    // `file://host/path` → authority "host", path "/path".
    let path = &rest[rest.find('/')?..];
    Some(PathBuf::from(percent_decode(path)))
}

/// Decode `%XX` escapes in a URI path into raw bytes, then read the result as
/// UTF-8 (lossily — non-UTF-8 paths are rare and not worth refusing the drop).
fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            let hi = (bytes[i + 1] as char).to_digit(16);
            let lo = (bytes[i + 2] as char).to_digit(16);
            if let (Some(h), Some(l)) = (hi, lo) {
                out.push((h * 16 + l) as u8);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
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
        // ISO_Left_Tab is what xkb produces for Shift+Tab — without this
        // alias the focus-cycling layer never sees a Tab key event when
        // the user wants to walk focus backwards.
        K::Tab | K::ISO_Left_Tab => NamedKey::Tab,
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

// wp_fractional_scale_manager_v1 is sender-only (we only call
// get_fractional_scale on it). An empty Dispatch satisfies the queue handle.
impl Dispatch<WpFractionalScaleManagerV1, ()> for State {
    fn event(
        _state: &mut Self,
        _proxy: &WpFractionalScaleManagerV1,
        _event: <WpFractionalScaleManagerV1 as Proxy>::Event,
        _data: &(),
        _conn: &Connection,
        _qhandle: &QueueHandle<Self>,
    ) {
    }
}

// The compositor reports the surface's preferred fractional scale here, as a
// value in 120ths (1.5 → 180). We record the real scale so the UI can show it;
// rendering still uses the integer buffer scale and the compositor resamples.
impl Dispatch<WpFractionalScaleV1, ()> for State {
    fn event(
        state: &mut Self,
        _proxy: &WpFractionalScaleV1,
        event: wp_fractional_scale_v1::Event,
        _data: &(),
        _conn: &Connection,
        _qhandle: &QueueHandle<Self>,
    ) {
        if let wp_fractional_scale_v1::Event::PreferredScale { scale } = event {
            let f = scale as f32 / 120.0;
            if state.fractional_scale != Some(f) {
                state.fractional_scale = Some(f);
                state.needs_redraw = true;
                state.mark_popups_dirty();
            }
        }
    }
}

// Imports kept around for future expansion (buffer reuse, Arc-shared
// state across worker threads).
#[allow(dead_code)]
fn _unused(_b: Buffer, _arc: Arc<()>) {}

#[cfg(test)]
mod tests {
    use super::{file_uri_to_path, parse_uri_list, percent_decode};
    use std::path::PathBuf;

    #[test]
    fn parses_a_uri_list_into_paths() {
        // Real file managers terminate lines with CRLF and may include a
        // leading comment line; both must be tolerated.
        let payload = "#comment\r\nfile:///home/rob/a.txt\r\nfile:///tmp/b.png\r\n";
        assert_eq!(
            parse_uri_list(payload),
            vec![
                PathBuf::from("/home/rob/a.txt"),
                PathBuf::from("/tmp/b.png"),
            ]
        );
    }

    #[test]
    fn percent_escapes_are_decoded() {
        assert_eq!(
            file_uri_to_path("file:///tmp/a%20b%2Bc.txt"),
            Some(PathBuf::from("/tmp/a b+c.txt"))
        );
        assert_eq!(percent_decode("%41%42"), "AB");
        // A stray, truncated escape is left verbatim rather than panicking.
        assert_eq!(percent_decode("100%"), "100%");
    }

    #[test]
    fn drops_authority_and_non_file_uris() {
        assert_eq!(
            file_uri_to_path("file://localhost/etc/hosts"),
            Some(PathBuf::from("/etc/hosts"))
        );
        assert_eq!(file_uri_to_path("https://example.com/x"), None);
        assert!(parse_uri_list("https://example.com/x\n\n").is_empty());
    }
}
