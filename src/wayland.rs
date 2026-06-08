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
use smithay_client_toolkit::data_device_manager::data_source::{DataSourceHandler, DragSource};
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
    AxisScroll, PointerData, PointerEvent, PointerEventKind, PointerHandler,
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
use wayland_protocols::wp::cursor_shape::v1::client::wp_cursor_shape_device_v1::{
    Shape, WpCursorShapeDeviceV1,
};
use wayland_protocols::wp::cursor_shape::v1::client::wp_cursor_shape_manager_v1::WpCursorShapeManagerV1;
use wayland_protocols::wp::fractional_scale::v1::client::wp_fractional_scale_manager_v1::WpFractionalScaleManagerV1;
use wayland_protocols::wp::fractional_scale::v1::client::wp_fractional_scale_v1::{
    self, WpFractionalScaleV1,
};
use wayland_protocols::xdg::dialog::v1::client::xdg_dialog_v1::XdgDialogV1;
use wayland_protocols::xdg::dialog::v1::client::xdg_wm_dialog_v1::XdgWmDialogV1;
use wayland_protocols::xdg::shell::client::xdg_positioner::{Anchor, Gravity, XdgPositioner};
use wayland_protocols::xdg::shell::client::xdg_surface::XdgSurface as XdgSurfaceObj;

use crate::app::{App, KeySwallow};
use crate::background::BackgroundState;
use crate::event::{
    DragData, Event, EventCtx, Key, Modifiers, MouseButton, NamedKey, SCROLL_PIXELS_PER_LINE,
    WHEEL_LINES_PER_DETENT,
};
use crate::font::Font;
use crate::geometry::{Color, Point, Rect, Size};
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
    // Optional: wp_cursor_shape lets us swap the pointer to a themed drag cursor
    // while we're a drag source. v1 already has every shape we use; compositors
    // without it just keep their default cursor during a drag.
    let cursor_shape_mgr: Option<WpCursorShapeManagerV1> = globals
        .bind::<WpCursorShapeManagerV1, _, _>(&qh, 1..=1, ())
        .ok();

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
        drag_grab_serial: None,
        drag_origin_surface: None,
        drag_source: None,
        drag_payload: Vec::new(),
        drag_icon: None,
        cursor_shape_mgr,
        cursor_shape_device: None,
        modifiers: Modifiers::default(),
        bg: BackgroundState::from_env(),
        cursor: None,

        popups: Vec::new(),
        qh: qh.clone(),
        loop_handle: event_loop.handle(),
        swallow: KeySwallow::default(),
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
    /// Outbound drag-and-drop (us as the *source*). `drag_grab_serial` /
    /// `drag_origin_surface` remember the latest pointer press — Wayland
    /// requires both to start a drag, since the press's implicit grab is what
    /// the drag rides on. `drag_source` keeps the live `wl_data_source` alive
    /// for the duration of the drag, and `drag_payload` is the serialized
    /// `text/uri-list` we hand the target when it asks (`send_request`).
    drag_grab_serial: Option<u32>,
    drag_origin_surface: Option<wl_surface::WlSurface>,
    drag_source: Option<DragSource>,
    drag_payload: Vec<u8>,
    /// The cursor-following icon for the active outbound drag, if any.
    drag_icon: Option<DragIcon>,
    /// `wp_cursor_shape` plumbing used to swap the pointer to a drag cursor
    /// (`copy` over a valid target, `no_drop` elsewhere) while we're the drag
    /// source. `mgr` is the bound global; `device` is the per-pointer handle we
    /// call `set_shape` on. Both `None` if the compositor lacks the protocol —
    /// the drag still works, just with the compositor's default cursor.
    cursor_shape_mgr: Option<WpCursorShapeManagerV1>,
    cursor_shape_device: Option<WpCursorShapeDeviceV1>,
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
    /// Tracks a key press a widget asked to swallow until release (via
    /// [`EventCtx::swallow_key_until_release`]).
    swallow: KeySwallow,
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
    /// Whether a widget under the cursor opted in to the drop on the last
    /// `DragEnter`/`DragMove`. Tracked so we only re-tell the offer when the
    /// answer actually changes as the drag moves within the window.
    accepted: bool,
}

/// Whether the spot under the cursor will accept the drop. River (and other
/// wlroots compositors) revoke our pointer focus the instant a drag starts and
/// then ignore `wp_cursor_shape.set_shape`, so we can't change the real cursor
/// mid-drag; instead we bake this state into the drag icon as a badge.
#[derive(Clone, Copy, PartialEq, Eq)]
enum DragFeedback {
    /// Over a target that accepts the drop — a green `+` badge.
    Copy,
    /// Nowhere valid to drop (over our own window, empty desktop, a
    /// non-accepting app) — a red `−` badge.
    NoDrop,
}

/// The little surface that follows the cursor during an *outbound* drag — the
/// visual feedback for "you're carrying this": the file's name plus a badge
/// showing whether it can be dropped here. We keep the pool and enough layout
/// to *re-render* it as the feedback changes (the icon surface stays mapped for
/// the whole drag, so re-committing it updates what the user sees), and tear it
/// all down when the drag ends.
struct DragIcon {
    surface: wl_surface::WlSurface,
    pool: SlotPool,
    /// File name shown on the chip.
    label: String,
    /// Chip size in logical pixels (the buffer is this times `scale`).
    logical_w: i32,
    logical_h: i32,
    /// Side length of the square badge at the chip's left edge.
    badge: i32,
    /// Integer buffer scale the chip is rendered at.
    scale: i32,
    /// Currently painted feedback, so re-renders are skipped when unchanged.
    feedback: DragFeedback,
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

    /// Dispatch `event` into the widget tree and apply the requests it left on
    /// the [`EventCtx`], returning that context so the caller can read the
    /// requests it didn't act on: `accepts_drop` for the incoming-drag events
    /// (turned into accept/reject on the drag offer) and `swallow_key` for the
    /// keyboard path. Most callers ignore the result.
    fn dispatch(&mut self, event: Event) -> EventCtx {
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
        if let Some(data) = ctx.drag_request.take() {
            self.begin_drag(data);
        }
        ctx
    }

    /// Start an outbound drag carrying `data`'s file paths, in response to a
    /// widget's [`EventCtx::start_drag`]. Needs the data-device plumbing plus
    /// the latest pointer press (its serial + surface): Wayland only lets a
    /// client begin a drag off the implicit grab a button press established, so
    /// this is expected to be called while that button is still held. Bails
    /// quietly if any of that is missing or the payload has no paths — there's
    /// nothing the user could have grabbed.
    fn begin_drag(&mut self, data: DragData) {
        let (Some(mgr), Some(dd), Some(serial)) = (
            self.data_device_manager.as_ref(),
            self.data_device.as_ref(),
            self.drag_grab_serial,
        ) else {
            return;
        };
        if data.paths.is_empty() {
            return;
        }
        // The origin must be one of our surfaces; fall back to the main window
        // if we somehow never recorded a press surface.
        let origin = self
            .drag_origin_surface
            .clone()
            .unwrap_or_else(|| self.window.wl_surface().clone());

        // The surface that rides the cursor so the user can see what they're
        // carrying. Laid out before we borrow the device/manager (needs only
        // `&self`); `None` if we couldn't allocate it — the drag still works.
        let icon = self.create_drag_icon(&data);

        // Offer only the uri-list, copy-only (we never want a move to make the
        // source side delete the user's files).
        let source = mgr.create_drag_and_drop_source(&self.qh, [URI_LIST_MIME], DndAction::Copy);
        source.start_drag(dd, &origin, icon.as_ref().map(|i| &i.surface), serial);
        self.drag_payload = paths_to_uri_list(&data.paths).into_bytes();
        self.drag_source = Some(source);
        // Now that `start_drag` has given the surface the drag-icon role, paint
        // and commit it. It starts as "no-drop" (nothing valid under the cursor
        // yet); `action` upgrades it to "copy" once a target accepts.
        if let Some(mut icon) = icon {
            self.paint_drag_icon(&mut icon);
            self.drag_icon = Some(icon);
        }
        // Also ask for a themed copy/no-drop cursor. River ignores this mid-drag
        // (it revokes our pointer focus), but compositors that render DnD cursors
        // (GNOME, KDE) honor it — so the cursor is right there and the icon badge
        // covers wlroots.
        self.set_drag_cursor(Shape::NoDrop);
    }

    /// Lay out the cursor-following drag icon and allocate its surface + pool,
    /// but don't paint it yet — the surface only becomes a drag icon once
    /// `start_drag` assigns the role, so [`Self::paint_drag_icon`] is called
    /// (which commits) afterwards. Returns `None` if there's no font, so callers
    /// drag without an icon.
    fn create_drag_icon(&self, data: &DragData) -> Option<DragIcon> {
        let label = drag_icon_label(&data.paths);
        let font = self.font.as_ref()?;
        let (text_w, text_h) = font.measure(&label, self.theme.font_size);
        let text_h = text_h.ceil() as i32;

        let badge = text_h.max(DRAG_ICON_BADGE_MIN);
        let logical_w =
            DRAG_ICON_PAD + badge + DRAG_ICON_GAP + text_w.ceil() as i32 + DRAG_ICON_PAD;
        let logical_h = DRAG_ICON_PAD + badge.max(text_h) + DRAG_ICON_PAD;
        let scale = self.scale.max(1);
        // Room for two buffers so a re-render (on a feedback change) can grab a
        // fresh slot while the compositor still holds the previous one.
        let pool_bytes = (logical_w * scale * logical_h * scale * 4) as usize * 2;
        let pool = SlotPool::new(pool_bytes, &self.shm).ok()?;
        let surface = self.compositor.create_surface(&self.qh);

        Some(DragIcon {
            surface,
            pool,
            label,
            logical_w,
            logical_h,
            badge,
            scale,
            feedback: DragFeedback::NoDrop,
        })
    }

    /// (Re)paint the drag icon for its current `feedback` and commit it. Safe to
    /// call repeatedly during a drag — the icon surface stays mapped, so each
    /// commit just swaps what follows the cursor. A no-op if the pool's buffer
    /// is momentarily busy (the previous frame stays up until the next change).
    fn paint_drag_icon(&self, icon: &mut DragIcon) {
        let buf_w = icon.logical_w * icon.scale;
        let buf_h = icon.logical_h * icon.scale;
        let stride = buf_w * 4;
        let Ok((buffer, _)) =
            icon.pool
                .create_buffer(buf_w, buf_h, stride, wl_shm::Format::Argb8888)
        else {
            return;
        };
        let Some(canvas) = icon.pool.canvas(&buffer) else {
            return;
        };
        let pixels = bytes_as_u32_mut(canvas);
        {
            let mut painter = Painter::with_popup_anchor(
                pixels,
                buf_w,
                buf_h,
                icon.scale as f32,
                0,
                0,
                self.font.as_ref(),
                self.mono_font.as_ref(),
                None,
            );
            painter.set_system_scale(self.fractional_scale.unwrap_or(icon.scale as f32));
            let area = Rect::new(0, 0, icon.logical_w, icon.logical_h);
            painter.fill_rect(area, self.theme.background);
            painter.raised_bevel(area, self.theme.highlight, self.theme.shadow);
            let badge = Rect::new(DRAG_ICON_PAD, DRAG_ICON_PAD, icon.badge, icon.badge);
            draw_drag_badge(&mut painter, badge, icon.feedback);
            painter.text(
                DRAG_ICON_PAD + icon.badge + DRAG_ICON_GAP,
                DRAG_ICON_PAD,
                &icon.label,
                self.theme.font_size,
                self.theme.text,
            );
        }
        let _ = buffer.attach_to(&icon.surface);
        icon.surface.set_buffer_scale(icon.scale);
        icon.surface.damage_buffer(0, 0, buf_w, buf_h);
        icon.surface.commit();
    }

    /// Swap the drag icon's badge to reflect whether the spot under the cursor
    /// accepts the drop, repainting only when it actually changed.
    fn update_drag_feedback(&mut self, feedback: DragFeedback) {
        let Some(mut icon) = self.drag_icon.take() else {
            return;
        };
        if icon.feedback != feedback {
            icon.feedback = feedback;
            self.paint_drag_icon(&mut icon);
        }
        self.drag_icon = Some(icon);
    }

    /// Tear down the active outbound drag source once it's no longer needed
    /// (the target finished reading, or the drag was cancelled). Destroying the
    /// `wl_data_source` is the client's responsibility after `dnd_finished` /
    /// `cancelled`; we only do so for *our* source.
    fn end_drag_source(&mut self, source: &WlDataSource) {
        if self
            .drag_source
            .as_ref()
            .is_some_and(|s| s.inner() == source)
        {
            if let Some(s) = self.drag_source.take() {
                s.inner().destroy();
            }
            if let Some(icon) = self.drag_icon.take() {
                icon.surface.destroy();
            }
            self.drag_payload.clear();
            // Hand the pointer back to its normal cursor.
            self.set_drag_cursor(Shape::Default);
        }
    }

    /// Set the pointer to `shape` via `wp_cursor_shape` — used to show the drag
    /// cursor (copy / no-drop) while we're the drag source. `set_shape` wants
    /// the latest `wl_pointer.enter` serial, which SCTK tracks for us. A no-op
    /// when the protocol is unavailable or no serial is on record; if the
    /// compositor declines to honor it mid-drag, the cursor simply stays put.
    fn set_drag_cursor(&self, shape: Shape) {
        let (Some(device), Some(pointer)) =
            (self.cursor_shape_device.as_ref(), self.pointer.as_ref())
        else {
            return;
        };
        if let Some(serial) = pointer
            .data::<PointerData>()
            .and_then(|d| d.latest_enter_serial())
        {
            device.set_shape(serial, shape);
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

    /// Tell an incoming drag offer whether we'll take it. Accepting asks for a
    /// *copy* (never move — we must not make the source delete the user's file)
    /// and names the uri-list; rejecting clears the action and mime so the
    /// source sees "no drop" over us. Flushed right away so the source's cursor
    /// updates without waiting for the next loop turn.
    fn apply_drag_offer(&self, offer: &DragOffer, accepted: bool, conn: &Connection) {
        if accepted {
            offer.set_actions(DndAction::Copy, DndAction::Copy);
            offer.accept_mime_type(offer.serial, Some(URI_LIST_MIME.to_string()));
        } else {
            offer.set_actions(DndAction::empty(), DndAction::empty());
            offer.accept_mime_type(offer.serial, None);
        }
        let _ = conn.flush();
    }

    /// Create this seat's data device (once) so drag offers start arriving. A
    /// drag is delivered through the seat that owns the pointer; one device is
    /// enough for the single-seat setups saudade targets.
    ///
    /// This must be driven from `new_capability`, not only `new_seat`:
    /// `SeatHandler::new_seat` fires only for seats *hotplugged* after startup.
    /// The seat that already exists when the app launches — the usual case — is
    /// bound directly inside `SeatState::new` and never routed through
    /// `new_seat` (the registry's `new_global` is "not called during initial
    /// enumeration of globals"). `new_capability` *does* fire for that
    /// pre-existing seat, so creating the device there is what actually makes
    /// file drops work on a normal Wayland session.
    fn ensure_data_device(&mut self, qh: &QueueHandle<Self>, seat: &wl_seat::WlSeat) {
        if self.data_device.is_none()
            && let Some(mgr) = self.data_device_manager.as_ref()
        {
            self.data_device = Some(mgr.get_data_device(qh, seat));
        }
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
            // Give the root a chance to react before we quit: a widget used
            // directly as the window root (rather than hosted in a `Modal`)
            // gets its `on_cancel` here, so a dialog-as-window can revert
            // pending edits on close, matching Escape and the dialog-popup
            // close path below. Most roots leave this a no-op.
            let mut ctx = EventCtx::new();
            self.root.on_cancel(&mut ctx);
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
        // A seat hotplugged after startup: wire up its data device so drag
        // offers start arriving. The seat that already exists at launch never
        // reaches here — `new_capability` covers that one (see
        // `ensure_data_device`).
        self.ensure_data_device(qh, &seat);
    }
    fn new_capability(
        &mut self,
        _conn: &Connection,
        qh: &QueueHandle<Self>,
        seat: wl_seat::WlSeat,
        capability: Capability,
    ) {
        // Create the seat's data device the first time we see any capability on
        // it. This is the path that catches the seat already present at startup
        // (the common case), which `new_seat` is never called for; without it
        // no drag offers ever arrive and file drops silently do nothing.
        self.ensure_data_device(qh, &seat);
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
            // Pair the pointer with a cursor-shape device so a drag can swap in
            // the copy / no-drop cursor. Sender-only; no events come back.
            if let (Some(mgr), Some(ptr)) = (self.cursor_shape_mgr.as_ref(), self.pointer.as_ref())
            {
                self.cursor_shape_device = Some(mgr.get_pointer(ptr, qh, ()));
            }
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
            // A key whose press a widget asked to swallow (e.g. the letter that
            // fired a menu item) is dropped wholesale — including the repeat
            // timer's re-presses — until release, so neither the `KeyDown` nor
            // its text reaches the tree.
            if self.swallow.drops_press(mapped) {
                return;
            }
            if let Some(m) = mapped {
                let ctx = self.dispatch(Event::KeyDown { key: m, modifiers });
                if ctx.swallow_key {
                    self.swallow.begin(mapped);
                    return;
                }
            }
            if !modifiers.has_command()
                && let Some(utf8) = event.utf8.as_deref()
            {
                for ch in utf8.chars() {
                    if (ch.is_control() && ch != '\t' && ch != '\n') || ch == '\r' {
                        continue;
                    }
                    let ctx = self.dispatch(Event::Char { ch, modifiers });
                    if ctx.swallow_key {
                        self.swallow.begin(mapped);
                        return;
                    }
                }
            }
        } else if self.swallow.ends_on_release(mapped) {
            // The release that ends a swallowed press: consume it and arm the
            // key for normal handling again.
        } else if let Some(m) = mapped {
            self.dispatch(Event::KeyUp { key: m, modifiers });
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
                PointerEventKind::Press { button, serial, .. } => {
                    let Some(b) = map_button(button) else {
                        continue;
                    };
                    // Remember the press: a drag the widget may start during the
                    // following motion rides on this press's implicit grab, and
                    // `start_drag` needs both the serial and the surface it
                    // happened on.
                    self.drag_grab_serial = Some(serial);
                    self.drag_origin_surface = Some(event.surface.clone());
                    self.dispatch(Event::PointerDown {
                        pos,
                        button: b,
                        modifiers: self.modifiers,
                    });
                    self.mark_popups_dirty();
                }
                PointerEventKind::Release { button, .. } => {
                    let Some(b) = map_button(button) else {
                        continue;
                    };
                    self.dispatch(Event::PointerUp {
                        pos,
                        button: b,
                        modifiers: self.modifiers,
                    });
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
        // outright so the compositor shows "can't drop here" over us.
        let has_uri = offer.with_mime_types(|mimes| mimes.iter().any(|m| m == URI_LIST_MIME));
        if !has_uri {
            offer.accept_mime_type(offer.serial, None);
            return;
        }

        let anchor = self.surface_anchor(surface);
        let pos = Point::new(x.floor() as i32 + anchor.x, y.floor() as i32 + anchor.y);
        self.drag = Some(DragSession {
            anchor,
            pos,
            accepted: false,
        });
        // Offer the drag to the widget tree first; only accept it if a widget
        // opts in via `EventCtx::accept_drop`. Accepting unconditionally would
        // tell the source every window is a drop target, even ones with no drop
        // zone — so the source's cursor would wrongly read "copy" over us.
        let accepted = self.dispatch(Event::DragEnter { pos }).accepts_drop;
        if let Some(d) = self.drag.as_mut() {
            d.accepted = accepted;
        }
        self.apply_drag_offer(&offer, accepted, conn);
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
        conn: &Connection,
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
        // Re-evaluate acceptance for the new position — a drag can cross from a
        // drop zone to plain content within one window — and only re-tell the
        // offer when the answer flipped.
        let accepted = self.dispatch(Event::DragMove { pos }).accepts_drop;
        let changed = self.drag.as_ref().is_some_and(|d| d.accepted != accepted);
        if let Some(drag) = self.drag.as_mut() {
            drag.accepted = accepted;
        }
        if changed
            && let Some(offer) = self
                .data_device
                .as_ref()
                .and_then(|dd| dd.data().drag_offer())
        {
            self.apply_drag_offer(&offer, accepted, conn);
        }
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

// saudade is a drag source only for outbound file drags started via
// `EventCtx::start_drag` (we never create a clipboard/selection source). The
// handler serves the uri-list when a target asks and tears the source down when
// the drag ends; the clipboard-source callbacks stay no-ops.
impl DataSourceHandler for State {
    fn accept_mime(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        source: &WlDataSource,
        mime: Option<String>,
    ) {
        // The compositor reports whether the destination under the cursor
        // accepts one of our mime types — `Some` means a real target, `None`
        // means none. It's a more immediate signal than `action`, so mirror it
        // into the badge too.
        if self
            .drag_source
            .as_ref()
            .is_some_and(|s| s.inner() == source)
        {
            self.update_drag_feedback(if mime.is_some() {
                DragFeedback::Copy
            } else {
                DragFeedback::NoDrop
            });
        }
    }
    fn send_request(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        source: &WlDataSource,
        mime: String,
        write_pipe: WritePipe,
    ) {
        // The target is reading our drag's payload. Only serve the uri-list,
        // and only for the source we actually started. The payload is a handful
        // of paths — well under a pipe's buffer — so a direct write can't block
        // the UI in practice.
        let ours = self
            .drag_source
            .as_ref()
            .is_some_and(|s| s.inner() == source);
        if !ours || mime != URI_LIST_MIME {
            return;
        }
        use std::io::Write;
        use std::os::fd::OwnedFd;
        let mut file = std::fs::File::from(OwnedFd::from(write_pipe));
        let _ = file.write_all(&self.drag_payload);
    }
    fn cancelled(&mut self, _: &Connection, _: &QueueHandle<Self>, source: &WlDataSource) {
        // The drag was aborted (dropped on empty space, Escape, …). Discard it.
        self.end_drag_source(source);
    }
    fn dnd_dropped(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &WlDataSource) {
        // The target accepted the drop; it may still be reading via
        // `send_request`. Keep the source alive until `dnd_finished`.
    }
    fn dnd_finished(&mut self, _: &Connection, _: &QueueHandle<Self>, source: &WlDataSource) {
        // Transfer complete: the source has done its job, so destroy it.
        self.end_drag_source(source);
    }
    fn action(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        source: &WlDataSource,
        action: DndAction,
    ) {
        // The compositor reports the action the current target would take as the
        // pointer moves: a real action (copy/move) means a valid drop target is
        // under the cursor, an empty action means there isn't one. Reflect that
        // in the cursor so the user can see where a drop will land.
        if !self
            .drag_source
            .as_ref()
            .is_some_and(|s| s.inner() == source)
        {
            return;
        }
        let valid = action.contains(DndAction::Copy) || action.contains(DndAction::Move);
        // The badge is the feedback that actually shows on wlroots; the cursor
        // shape is the bonus for compositors that honor it mid-drag.
        self.update_drag_feedback(if valid {
            DragFeedback::Copy
        } else {
            DragFeedback::NoDrop
        });
        let shape = if action.contains(DndAction::Move) {
            Shape::Move
        } else if valid {
            Shape::Copy
        } else {
            Shape::NoDrop
        };
        self.set_drag_cursor(shape);
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

/// Serialize file paths into an RFC 2483 `text/uri-list` payload — the inverse
/// of [`parse_uri_list`] — for handing to a drop target. Each path becomes one
/// CRLF-terminated `file://` URI with its bytes percent-encoded. Relative paths
/// are skipped: a `file:` URI must be absolute, and a widget hands us absolute
/// paths anyway.
fn paths_to_uri_list(paths: &[PathBuf]) -> String {
    use std::os::unix::ffi::OsStrExt;
    let mut out = String::new();
    for path in paths {
        if !path.is_absolute() {
            continue;
        }
        out.push_str("file://");
        out.push_str(&percent_encode_path(path.as_os_str().as_bytes()));
        out.push_str("\r\n");
    }
    out
}

/// Percent-encode a path for a `file:` URI: keep the RFC 3986 unreserved set
/// (`A–Z a–z 0–9 - . _ ~`) and the path separator `/` verbatim, escape every
/// other byte as `%XX`. Operates on raw bytes so non-UTF-8 paths round-trip
/// through [`percent_decode`] unchanged.
fn percent_encode_path(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789ABCDEF";
    let mut out = String::with_capacity(bytes.len());
    for &b in bytes {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'_' | b'~' | b'/') {
            out.push(b as char);
        } else {
            out.push('%');
            out.push(HEX[(b >> 4) as usize] as char);
            out.push(HEX[(b & 0x0f) as usize] as char);
        }
    }
    out
}

/// Drag-icon layout, in logical pixels: padding around the chip, the gap
/// between the badge and the label, and a floor on the badge size so it stays
/// legible with tiny fonts.
const DRAG_ICON_PAD: i32 = 6;
const DRAG_ICON_GAP: i32 = 6;
const DRAG_ICON_BADGE_MIN: i32 = 12;

/// Paint the drop-feedback badge into `area`: a white checkmark on a green
/// square when the target accepts the drop, a white cross on a red square when
/// it doesn't. The glyph strokes are drawn in physical pixels (inside
/// [`Painter::physical`]) so the diagonals stay crisp at any scale.
fn draw_drag_badge(painter: &mut Painter, area: Rect, feedback: DragFeedback) {
    let (bg, accept) = match feedback {
        DragFeedback::Copy => (Color::GREEN, true),
        DragFeedback::NoDrop => (Color::RED, false),
    };
    painter.fill_rect(area, bg);
    painter.physical(area, |p, r| {
        let stroke = (r.w / 7).max(2);
        // A point at fraction (fx, fy) across the badge, in physical pixels.
        let at = |fx: f32, fy: f32| -> (i32, i32) {
            (
                r.x + (r.w as f32 * fx).round() as i32,
                r.y + (r.h as f32 * fy).round() as i32,
            )
        };
        if accept {
            // Checkmark: a short stroke down into the valley, then a long one up.
            let (ax, ay) = at(0.20, 0.52);
            let (bx, by) = at(0.42, 0.72);
            let (cx, cy) = at(0.80, 0.26);
            draw_thick_line(p, ax, ay, bx, by, stroke, Color::WHITE);
            draw_thick_line(p, bx, by, cx, cy, stroke, Color::WHITE);
        } else {
            // Cross: the two diagonals of an X.
            let (ax, ay) = at(0.27, 0.27);
            let (bx, by) = at(0.73, 0.73);
            let (cx, cy) = at(0.73, 0.27);
            let (dx, dy) = at(0.27, 0.73);
            draw_thick_line(p, ax, ay, bx, by, stroke, Color::WHITE);
            draw_thick_line(p, cx, cy, dx, dy, stroke, Color::WHITE);
        }
    });
}

/// Draw a `thickness`-wide line between two points by stepping along the longer
/// axis and stamping a square at each step — enough for the small badge glyphs.
/// Expects physical-pixel coordinates (call inside [`Painter::physical`]).
fn draw_thick_line(
    painter: &mut Painter,
    x0: i32,
    y0: i32,
    x1: i32,
    y1: i32,
    thickness: i32,
    color: Color,
) {
    let steps = (x1 - x0).abs().max((y1 - y0).abs()).max(1);
    let half = thickness / 2;
    for i in 0..=steps {
        let t = i as f32 / steps as f32;
        let x = x0 + ((x1 - x0) as f32 * t).round() as i32;
        let y = y0 + ((y1 - y0) as f32 * t).round() as i32;
        painter.fill_rect(Rect::new(x - half, y - half, thickness, thickness), color);
    }
}

/// The label shown on the cursor-following drag icon: the single file's name,
/// or "N items" for a multi-file drag.
fn drag_icon_label(paths: &[PathBuf]) -> String {
    match paths {
        [] => String::new(),
        [one] => one
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| one.display().to_string()),
        many => format!("{} items", many.len()),
    }
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

// wp_cursor_shape_manager_v1 and its per-pointer device are both sender-only
// (we only call get_pointer / set_shape). Empty Dispatch impls satisfy the
// queue handle.
impl Dispatch<WpCursorShapeManagerV1, ()> for State {
    fn event(
        _state: &mut Self,
        _proxy: &WpCursorShapeManagerV1,
        _event: <WpCursorShapeManagerV1 as Proxy>::Event,
        _data: &(),
        _conn: &Connection,
        _qhandle: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<WpCursorShapeDeviceV1, ()> for State {
    fn event(
        _state: &mut Self,
        _proxy: &WpCursorShapeDeviceV1,
        _event: <WpCursorShapeDeviceV1 as Proxy>::Event,
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
    use super::{
        file_uri_to_path, parse_uri_list, paths_to_uri_list, percent_decode, percent_encode_path,
    };
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

    #[test]
    fn serializes_paths_to_a_crlf_uri_list() {
        // Each path becomes one `file://` line, CRLF-terminated, with special
        // characters percent-encoded — the format a drop target expects.
        let list = paths_to_uri_list(&[
            PathBuf::from("/home/rob/a.txt"),
            PathBuf::from("/tmp/a b+c.txt"),
        ]);
        assert_eq!(
            list,
            "file:///home/rob/a.txt\r\nfile:///tmp/a%20b%2Bc.txt\r\n"
        );
    }

    #[test]
    fn skips_relative_paths_that_cant_be_file_uris() {
        // A `file:` URI must be absolute; a relative path has no valid encoding.
        assert_eq!(paths_to_uri_list(&[PathBuf::from("relative/x")]), "");
    }

    #[test]
    fn encoding_round_trips_through_the_parser() {
        // What we emit as a source must parse back to the same paths when we're
        // the target — the encoder and decoder agree on the escaping.
        let paths = vec![
            PathBuf::from("/tmp/plain.txt"),
            PathBuf::from("/tmp/with space & symbols#1.txt"),
            PathBuf::from("/home/rob/Bilder/Größe.png"),
        ];
        assert_eq!(parse_uri_list(&paths_to_uri_list(&paths)), paths);
    }

    #[test]
    fn keeps_unreserved_and_slash_but_escapes_the_rest() {
        assert_eq!(percent_encode_path(b"/a-b_c.d~e/f"), "/a-b_c.d~e/f");
        assert_eq!(percent_encode_path(b"a b"), "a%20b");
        assert_eq!(percent_encode_path(b"#?%"), "%23%3F%25");
    }
}
