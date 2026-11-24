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

use std::cell::Cell;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;
use std::time::{Duration, Instant};

use smithay_client_toolkit::compositor::{CompositorHandler, CompositorState, Region};
use smithay_client_toolkit::data_device_manager::data_device::{DataDevice, DataDeviceHandler};
use smithay_client_toolkit::data_device_manager::data_offer::{DataOfferHandler, DragOffer};
use smithay_client_toolkit::data_device_manager::data_source::{DataSourceHandler, DragSource};
use smithay_client_toolkit::data_device_manager::{DataDeviceManagerState, WritePipe};
use smithay_client_toolkit::output::{OutputHandler, OutputState};
use smithay_client_toolkit::reexports::calloop::{
    EventLoop as CalloopLoop, LoopHandle, PostAction, channel,
};
use smithay_client_toolkit::reexports::calloop_wayland_source::WaylandSource;
use smithay_client_toolkit::reexports::csd_frame::{WindowManagerCapabilities, WindowState};
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
    DecorationMode, Window as XdgWindow, WindowConfigure, WindowDecorations, WindowHandler,
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
use wayland_protocols::wp::viewporter::client::wp_viewport::{self, WpViewport};
use wayland_protocols::wp::viewporter::client::wp_viewporter::{self, WpViewporter};
use wayland_protocols::xdg::dialog::v1::client::xdg_dialog_v1::XdgDialogV1;
use wayland_protocols::xdg::dialog::v1::client::xdg_wm_dialog_v1::XdgWmDialogV1;
use wayland_protocols::xdg::shell::client::xdg_positioner::{Anchor, Gravity, XdgPositioner};
use wayland_protocols::xdg::shell::client::xdg_surface::XdgSurface as XdgSurfaceObj;
use wayland_protocols::xdg::shell::client::xdg_toplevel::ResizeEdge;
use wayland_protocols::xdg::toplevel_icon::v1::client::xdg_toplevel_icon_manager_v1::{
    self, XdgToplevelIconManagerV1,
};
use wayland_protocols::xdg::toplevel_icon::v1::client::xdg_toplevel_icon_v1::XdgToplevelIconV1;

use crate::app::{App, KeySwallow};
use crate::appearance::{Appearance, Themes};
use crate::background::BackgroundState;
use crate::chrome::{self, Edges, Hit, TitleButton, WindowChrome, WindowFrame};
use crate::event::{
    Cursor, DragData, DropAction, Event, EventCtx, Key, Modifiers, MouseButton, NamedKey,
    SCROLL_PIXELS_PER_LINE, WHEEL_LINES_PER_DETENT,
};
use crate::font::{Font, FontFamily, FontSet};
use crate::geometry::{Color, Point, Rect, Size};
use crate::mime_drop::{self, Reading, URI_LIST};
use crate::painter::Painter;
use crate::svg::SvgImage;
use crate::widget::{PopupKind, PopupRequest, Widget};

/// Icon sizes handed to the compositor before it has said which it wants —
/// the ladder a desktop icon theme ships, from a title bar's 16 up to the
/// large tiles a window overview draws. Compositors that do announce their
/// preferences replace this with exactly those (see the
/// `xdg_toplevel_icon_manager_v1` dispatch); the ones that announce nothing
/// pick from here.
const DEFAULT_ICON_SIZES: &[u32] = &[16, 24, 32, 48, 64, 128];

/// How far apart two [`ui_for`] results have to be to count as a different
/// scale: far enough to clear the rounding of the division that produces them,
/// near enough that no scale a compositor can report is swallowed — the
/// smallest step the fractional-scale protocol can express is a 120th.
const UI_EPSILON: f32 = 1e-4;

pub(crate) fn run(app: App) {
    let (window_cfg, mut themes, root, app_scale) = app.into_parts();

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

    // Ask the settings portal for the desktop's light or dark mode, before
    // the window opens so it opens in the right theme, and hear about every
    // change after through a channel the loop wakes on.
    if themes.follows_system() {
        let (sender, changes) = channel::channel();
        if let Some(appearance) = crate::portal::watch(move |a| sender.send(a).is_ok()) {
            themes.follow(appearance);
        }
        event_loop
            .handle()
            .insert_source(changes, |event, _, state: &mut State| {
                if let channel::Event::Msg(appearance) = event {
                    state.follow_system(appearance);
                }
            })
            .expect("saudade: portal source insert failed");
    }

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
    // distinct from the integer buffer scale we render at. That fractional
    // figure is the one the display is really presented at, so it is what the
    // scale to draw at is derived from (see `ui_for`); we go on rasterizing at
    // the integer scale and let the compositor resample. Compositors that
    // don't advertise it leave us with only the integer scale, which is the
    // right fallback.
    let fractional_mgr: Option<WpFractionalScaleManagerV1> = globals
        .bind::<WpFractionalScaleManagerV1, _, _>(&qh, 1..=1, ())
        .ok();
    // Optional: wp_viewporter is what lets us rasterize at the *fractional*
    // scale instead of the integer one. `wl_surface.set_buffer_scale` takes
    // only an integer, so without this a display presented at 125% means
    // drawing at 200% and handing the compositor a buffer it has to rescale by
    // 0.625 every frame — which undoes the painter's whole business of putting
    // each edge on a device pixel, since the grid it snapped to is no longer
    // the panel's: blurred by a compositor that filters as it scales, and
    // dropped or doubled a row at a time by one that doesn't. A viewport
    // states the surface size outright (`wp_viewport.set_destination`), so the
    // buffer can be exactly as many device pixels as the display will present
    // — 1:1 with the panel, the way a window on Windows or X11 at 125%
    // already is, and the same 1.667x scale there. Compositors without it keep
    // the oversampled path.
    let viewporter: Option<WpViewporter> = globals.bind::<WpViewporter, _, _>(&qh, 1..=1, ()).ok();
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
    // Optional: xdg_toplevel_icon_v1 (another staging extension) is how a
    // toplevel gets an icon of its own on Wayland. Without it a compositor has
    // only the app_id below to go on, and shows whatever icon the matching
    // desktop-entry file names — or a placeholder, for a program that hasn't
    // been installed with one.
    let toplevel_icon_mgr: Option<XdgToplevelIconManagerV1> = globals
        .bind::<XdgToplevelIconManagerV1, _, _>(&qh, 1..=1, ())
        .ok();

    // Server-side decorations, please — and where the compositor won't draw
    // them, we draw our own (see `Decor`). `SAUDADE_DECORATIONS=client` skips
    // the asking and always draws our own, the way a compositor without
    // server-side decorations has us do.
    let decorations = decoration_request();
    let surface = compositor.create_surface(&qh);
    let window = xdg_shell.create_window(surface, decorations, &qh);
    window.set_title(&window_cfg.title);
    window.set_app_id(
        window_cfg
            .app_id
            .clone()
            .unwrap_or_else(|| default_app_id(&window_cfg.title)),
    );
    // Subscribe the main surface to fractional-scale notifications, if the
    // compositor supports them. The returned object is kept alive in `State`
    // so the `preferred_scale` events keep arriving.
    let fractional_scale_obj = fractional_mgr
        .as_ref()
        .map(|mgr| mgr.get_fractional_scale(window.wl_surface(), &qh, ()));
    let viewport = viewporter
        .as_ref()
        .map(|vp| vp.get_viewport(window.wl_surface(), &qh, ()));

    // How big a saudade logical pixel is, as far as it can be known before the
    // compositor has said anything about the display: the pin, else the ladder
    // at a factor of 1. Both knobs are read once, the way the winit runtime
    // reads them, and the first `preferred_scale` / buffer-scale event the
    // compositor sends re-derives it (see `State::adopt_scale`) — which, the
    // ladder being a ratio against the platform's own unit, usually lands on
    // the same number.
    let scale_pin = crate::ui_scale::pinned(app_scale);
    let reference_dpi = crate::ui_scale::reference_dpi();
    let ui = ui_for(1.0, scale_pin, reference_dpi);

    let initial_w = surface_units(window_cfg.size.w, ui);
    let initial_h = surface_units(window_cfg.size.h, ui);

    if window_cfg.fullscreen {
        // Asked before the first commit, so the first configure already comes
        // at the display's size. (The size hints, sent below, leave a fixed
        // window's equal min / max out while it is fullscreen.)
        window.set_fullscreen(None);
    }

    let pool = SlotPool::new((initial_w * initial_h * 4) as usize * 4, &shm)
        .expect("saudade: slot pool init failed");

    let [font, serif_font, mono_font] =
        [FontFamily::Sans, FontFamily::Serif, FontFamily::Mono].map(Font::system);
    let mut state = State {
        registry_state: RegistryState::new(&globals),
        seat_state: SeatState::new(&globals, &qh),
        output_state: OutputState::new(&globals, &qh),
        compositor,
        shm,
        xdg_shell,
        xdg_dialog_mgr,

        window,
        title: window_cfg.title.clone(),
        decor: Decor::new(if window_cfg.resizable {
            WindowFrame::Resizable
        } else {
            WindowFrame::Fixed
        }),
        decorations,
        min_size: window_cfg.min_size,
        sent_hints: None,
        root,
        themes,
        font,
        serif_font,
        mono_font,

        pool,
        surface_w: initial_w,
        surface_h: initial_h,
        scale: 1,
        fractional_scale_obj,
        fractional_scale: None,
        viewporter,
        viewport,
        ui,
        scale_pin,
        reference_dpi,
        output_dpi: None,
        output: None,
        resizable: window_cfg.resizable,
        fullscreen: window_cfg.fullscreen,
        windowed_size: window_cfg.fullscreen.then_some((initial_w, initial_h)),
        configured: false,
        needs_redraw: true,
        exit: false,
        tick_requested: false,

        keyboard: None,
        keyboard_surface: None,
        pointer: None,
        seat: None,
        chrome_grab: false,
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
        toplevel_icon_mgr,
        icon_image: window_cfg.icon,
        icon_sizes: Vec::new(),
        icons: Vec::new(),
        modifiers: Modifiers::default(),
        buttons_held: 0,
        bg: BackgroundState::from_env(),
        cursor: None,
        cursor_shape: Cursor::Default,

        popups: Vec::new(),
        qh: qh.clone(),
        loop_handle: event_loop.handle(),
        swallow: KeySwallow::default(),
    };
    drop(conn);

    // The size hints, then the commit the compositor answers with the first
    // configure.
    state.apply_size_hints();
    state.window.commit();

    // Give the toplevel an icon straight away, at the sizes a desktop usually
    // wants. A compositor that has its own preferences announces them on the
    // manager and we redo this from those (see the dispatch); one that
    // announces none keeps what it gets here.
    state.set_toplevel_icon(DEFAULT_ICON_SIZES);

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
    /// The main window's title, as the decorations we draw show it.
    title: String,
    /// The decorations we draw around the main window, if the compositor
    /// leaves that to us.
    decor: Decor,
    /// What every toplevel asks the compositor for in the way of decorations —
    /// see [`decoration_request`].
    decorations: WindowDecorations,
    /// The smallest size, in saudade pixels, a resizable window can be
    /// resized to — `None` for a small default.
    min_size: Option<Size>,
    /// The main window's min / max size hints as last sent, so that they are
    /// sent again only when they change.
    sent_hints: Option<SizeHints>,
    root: Box<dyn Widget>,
    /// The light and dark themes, and which one the desktop's appearance —
    /// or `SAUDADE_THEME` — currently calls for.
    themes: Themes,
    font: Option<Rc<Font>>,
    serif_font: Option<Rc<Font>>,
    mono_font: Option<Rc<Font>>,

    pool: SlotPool,
    /// Surface (logical) dimensions reported by the compositor. The buffer we
    /// attach is `surface_w * scale` × `surface_h * scale` physical pixels, and
    /// the widget tree lays out into [`Self::logical_size`] — these over
    /// [`Self::ui`], a surface unit not being a saudade pixel.
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
    /// `wp_viewporter`, kept to make a viewport for each surface we create —
    /// the popups and the drag icon as well as the main window. `None` when the
    /// compositor doesn't advertise it, which is what puts every surface on the
    /// oversampled integer-scale path: see [`Self::raster_scale`].
    viewporter: Option<WpViewporter>,
    /// The main surface's viewport, through which its size in surface units is
    /// stated outright, leaving the buffer free to be exactly as many device
    /// pixels as the display presents.
    viewport: Option<WpViewport>,
    /// Surface-local units one saudade logical pixel spans: the scale to draw
    /// at over the factor the display is presented at. The widget tree is laid
    /// out in saudade pixels, and every size and position the compositor is
    /// told about is this many times one — see [`ui_for`].
    ui: f32,
    /// A scale the app or the environment pinned, drawn at verbatim in place of
    /// the ladder. Read once at startup.
    scale_pin: Option<f32>,
    /// The density a logical pixel is aimed at — `REFERENCE_DPI` unless
    /// `SAUDADE_UI_DPI` says otherwise. Read once at startup.
    ///
    /// [`REFERENCE_DPI`]: crate::ui_scale
    reference_dpi: f32,
    /// Panel pixels the display fits to the inch, from the physical size and
    /// current mode `wl_output` reports for it. A property of the glass, so it
    /// moves only when the window changes display or that display changes
    /// mode; what the painter is told is [`Self::device_dpi`], which counts the
    /// pixels we actually draw. `None` for a display that reports no size.
    output_dpi: Option<f32>,
    /// The output the main surface is on, as `wl_surface.enter` named it: what
    /// [`Self::output_dpi`] is read from.
    output: Option<wl_output::WlOutput>,
    /// Whether the window was configured resizable. A fixed window pins
    /// min == max size, so a programmatic resize ([`Self::apply_resize`])
    /// must move both hints to the new size; a resizable one leaves them be.
    resizable: bool,
    /// Whether the window is fullscreen: as the compositor last configured it,
    /// or as we have since asked it to be (see [`Self::apply_fullscreen`]).
    fullscreen: bool,
    /// The surface size to return to on leaving fullscreen, recorded on the
    /// way in. Compositors needn't suggest a size when they unfullscreen a
    /// window, and the surface would otherwise keep the display's.
    windowed_size: Option<(u32, u32)>,
    configured: bool,
    /// Set whenever something happened that needs a fresh frame on the
    /// main window. Drawing clears it; the next state change re-sets it.
    /// Without this flag we'd hammer the compositor with one buffer per
    /// loop iteration (~60Hz) and eventually get a BrokenPipe.
    needs_redraw: bool,
    exit: bool,
    /// Push-based tick keep-alive: set whenever a dispatch called
    /// [`EventCtx::request_tick`], so a widget can drive an animation without
    /// any `wants_ticks()` forwarding. Cleared just before each tick we fire.
    tick_requested: bool,

    keyboard: Option<wl_keyboard::WlKeyboard>,
    /// The surface holding the keyboard, as `wl_keyboard.enter` named it: the
    /// main window's or a popup's. A key arrives without saying where it was
    /// typed, so this is what tells one typed into the window beneath a dialog
    /// — see [`Self::keyboard_under_dialog`].
    keyboard_surface: Option<wl_surface::WlSurface>,
    pointer: Option<wl_pointer::WlPointer>,
    /// The seat the pointer belongs to: what a move, a resize, or the window
    /// menu begun from the decorations we draw is asked for on.
    seat: Option<wl_seat::WlSeat>,
    /// Whether the buttons held went down on the decorations rather than the
    /// content: until they are all up, the pointer is theirs, not the widget
    /// tree's.
    chrome_grab: bool,
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
    /// `xdg_toplevel_icon` plumbing. `toplevel_icon_mgr` is the bound global —
    /// `None` on compositors without the protocol, where the app_id and its
    /// desktop-entry file are the only icon there is. `icon_image` is the
    /// vector artwork every size is rasterized from, and `icons` holds each
    /// icon object we have set, with its shm storage: see [`ToplevelIcon`] for
    /// why none of them is ever dropped. `icon_sizes` accumulates the sizes
    /// the compositor announces a preference for, until the `done` that ends
    /// the run.
    toplevel_icon_mgr: Option<XdgToplevelIconManagerV1>,
    icon_image: Option<SvgImage>,
    icon_sizes: Vec<u32>,
    icons: Vec<ToplevelIcon>,
    modifiers: Modifiers,
    /// Mouse buttons held down, on any of our surfaces: while one is, each key
    /// is followed by [`State::rehover`]. Cleared when the pointer leaves, and
    /// when an outbound drag takes the press over (its release then goes to
    /// the compositor's drag, never to us).
    buttons_held: u32,
    /// Background pattern + color for the main window, toggled with the
    /// `p` / `c` debug keys. Popups/dialogs ignore it and stay plain.
    bg: BackgroundState,
    /// Cursor position in *widget-tree logical coordinates* — i.e., the
    /// coordinates the widget tree expects (already converted from
    /// pointer pixels, and translated by the popup anchor when the
    /// cursor is over a popup).
    cursor: Option<Point>,
    /// Pointer shape last requested via `wp_cursor_shape`. Tracked so an
    /// unchanged hover doesn't re-issue a `set_shape`, and reset to
    /// [`Cursor::Default`] on leave (the compositor restores its own default
    /// when the pointer exits, so the next enter must re-apply from scratch).
    cursor_shape: Cursor,

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

    /// The toplevel to parent a dialog opened over this one to — `None` for a
    /// menu or dropdown, which a dialog doesn't belong to.
    fn dialog_window(&self) -> Option<&XdgWindow> {
        match self {
            ChildSurface::Popup(_) => None,
            ChildSurface::Dialog { window, .. } => Some(window),
        }
    }
}

/// A popup's viewport outlives nothing: the stack is rebuilt every time a menu
/// opens, so one left behind would leak a compositor object per open. `destroy`
/// is the one viewport request still allowed once the surface is gone, so the
/// order these fields drop in doesn't matter.
impl Drop for PopupState {
    fn drop(&mut self) {
        if let Some(vp) = &self.viewport {
            vp.destroy();
        }
    }
}

/// Likewise for the chip that follows the cursor through a drag.
impl Drop for DragIcon {
    fn drop(&mut self) {
        if let Some(vp) = &self.viewport {
            vp.destroy();
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
    /// A dialog's decorations, where we draw them; never on for a menu.
    decor: Decor,
    /// A dialog's title, for those decorations.
    title: String,
    /// The popup's viewport, where the compositor has `wp_viewporter`: what
    /// lets its buffer be the fractional size the display presents rather than
    /// an integer multiple of it. See [`State::raster_scale`].
    viewport: Option<WpViewport>,
    pool: SlotPool,
    anchor: Rect,
    /// Popup surface dimensions, in the compositor's units — the popup's
    /// `anchor` is the same rectangle in saudade pixels. Buffer is
    /// `surface_w * scale` × `surface_h * scale` physical pixels.
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
    /// The surface the drag is over: what its surface-local coordinates are
    /// translated from to reach the widget tree's — the same translation
    /// pointer events get.
    surface: wl_surface::WlSurface,
    /// Last reported position in widget-tree logical coordinates.
    pos: Point,
    /// What the widget under the cursor said a drop would do on the last
    /// `DragEnter`/`DragMove`, `None` if it wouldn't take one. Tracked so we
    /// only re-tell the offer when the answer actually changes as the drag
    /// moves within the window.
    accepted: Option<DropAction>,
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
    /// Chip size in saudade logical pixels — what the painter draws it in.
    logical_w: i32,
    logical_h: i32,
    /// The same chip in surface-local units — what the compositor is told the
    /// chip measures, through its viewport or its buffer scale.
    surface_w: i32,
    surface_h: i32,
    /// And in the device pixels it is rasterized in. Rounded *up* from the
    /// surface size so the buffer can never come out short of what the painter
    /// draws into it; without a viewport it is the surface size times `scale`,
    /// the integer multiple `wl_surface.set_buffer_scale` demands.
    buf_w: i32,
    buf_h: i32,
    /// The chip's viewport, where the compositor has `wp_viewporter`.
    viewport: Option<WpViewport>,
    /// Side length of the square badge at the chip's left edge.
    badge: i32,
    /// Integer buffer scale the chip is rendered at.
    scale: i32,
    /// Buffer pixels to a logical one: the surface units a logical pixel spans,
    /// times `scale`.
    paint_scale: f32,
    /// Currently painted feedback, so re-renders are skipped when unchanged.
    feedback: DragFeedback,
}

/// How soon a second click has to follow the first to make a double click on
/// the decorations we draw — Canoe's figure, as the decorations are Canoe's.
const DOUBLE_CLICK: Duration = Duration::from_millis(400);

/// What each toplevel asks the compositor for in the way of decorations:
/// server-side ones, unless `SAUDADE_DECORATIONS=client` says to draw our own
/// whatever the compositor could do.
fn decoration_request() -> WindowDecorations {
    match std::env::var("SAUDADE_DECORATIONS") {
        Ok(v) if v.eq_ignore_ascii_case("client") => WindowDecorations::ClientOnly,
        _ => WindowDecorations::RequestServer,
    }
}

/// Client-side decorations: the Canoe frame we draw around a toplevel — the
/// main window or a dialog — ourselves, when the compositor leaves decorating
/// windows to their clients. GNOME's Mutter does, as does Weston and any
/// compositor without `zxdg_decoration_manager_v1`; others do when they answer
/// our request for server-side decorations with `client_side`. It is the same
/// chrome [`MockBackend::render_framed`](crate::mock::MockBackend::render_framed)
/// frames screenshots in.
///
/// The frame sits in the buffer around the content, with room outside it for
/// the drop shadow. The window geometry we report is the frame, so the
/// compositor sizes, places and snaps the window by it and leaves the shadow
/// out; the input region is the frame plus the strip of shadow its edges can
/// be grabbed by, so a click anywhere else on the shadow goes to whatever is
/// underneath.
struct Decor {
    /// Whether we draw them: the compositor wants client-side decorations, and
    /// the window isn't fullscreen.
    on: bool,
    frame: WindowFrame,
    /// Whether the window has the focus, for the title bar's colors.
    active: bool,
    /// Maximized or tiled: no shadow, and no edges to resize by.
    docked: bool,
    maximized: bool,
    /// What the compositor says it can do. The buttons for what it can't are
    /// left out.
    caps: WindowManagerCapabilities,
    /// The title-bar button a left press went down on, until its release, and
    /// whether the pointer is over it still: it is drawn down only then, and
    /// does what it does only if released there.
    held: Option<TitleButton>,
    over_held: bool,
    /// When the title bar and the close box were last clicked: the next click
    /// within [`DOUBLE_CLICK`] is a double click.
    title_click: Option<Instant>,
    close_click: Option<Instant>,
    /// The window geometry and input region last sent, so they are sent again
    /// only when they change. `None` until the decorations are first drawn: a
    /// window that never has any never sends either.
    sent: Option<(Rect, Option<Rect>)>,
}

impl Decor {
    fn new(frame: WindowFrame) -> Self {
        Self {
            on: false,
            frame,
            active: true,
            docked: false,
            maximized: false,
            caps: WindowManagerCapabilities::all(),
            held: None,
            over_held: false,
            title_click: None,
            close_click: None,
            sent: None,
        }
    }

    /// Take in what a configure says about the window. Returns whether that
    /// moves anything the size hints depend on.
    fn configure(&mut self, configure: &WindowConfigure) -> bool {
        let was = (self.on, self.docked);
        self.on = configure.decoration_mode == DecorationMode::Client && !configure.is_fullscreen();
        self.active = configure.is_activated();
        self.maximized = configure.is_maximized();
        self.docked = self.maximized || configure.state.intersects(WindowState::TILED);
        self.caps = configure.capabilities;
        was != (self.on, self.docked)
    }

    fn can_minimize(&self) -> bool {
        self.frame != WindowFrame::Dialog && self.caps.contains(WindowManagerCapabilities::MINIMIZE)
    }

    fn can_maximize(&self) -> bool {
        self.frame == WindowFrame::Resizable
            && self.caps.contains(WindowManagerCapabilities::MAXIMIZE)
    }

    /// The chrome to draw, titled `title`.
    fn chrome(&self, title: &str) -> WindowChrome {
        WindowChrome::new(title, self.frame)
            .with_active(self.active)
            .docked(self.docked)
            .with_buttons(self.can_minimize(), self.can_maximize())
            .with_pressed(self.held.filter(|_| self.over_held))
    }

    /// Where everything goes on a surface holding `content` (surface units)
    /// inside these decorations, at `ui` surface units to a saudade pixel.
    fn layout(&self, content: (u32, u32), ui: f32) -> DecorLayout {
        let (w, h) = (content.0.max(1), content.1.max(1));
        if !self.on {
            return DecorLayout {
                surface: (w, h),
                content_size: (w, h),
                content: (0, 0),
                geometry: Rect::new(0, 0, w as i32, h as i32),
                input: None,
                extra: (0, 0),
            };
        }
        let chrome = self.chrome("");
        // Rounded up, so the room kept for the frame is never short of the
        // frame drawn in it.
        let su = |logical: i32| (logical.max(0) as f32 * ui).ceil() as u32;
        let f = chrome.frame_insets();
        let s = chrome.shadow_insets();
        let (fl, ft, fr, fb) = (su(f.left), su(f.top), su(f.right), su(f.bottom));
        let (sl, st, sr, sb) = (su(s.left), su(s.top), su(s.right), su(s.bottom));
        let geometry = Rect::new(
            sl as i32,
            st as i32,
            (fl + w + fr) as i32,
            (ft + h + fb) as i32,
        );
        let grab = su(chrome.resize_grab()).min(sl.min(st).min(sr).min(sb)) as i32;
        DecorLayout {
            surface: (sl + fl + w + fr + sr, st + ft + h + fb + sb),
            content_size: (w, h),
            content: (sl + fl, st + ft),
            geometry,
            input: Some(Rect::new(
                geometry.x - grab,
                geometry.y - grab,
                geometry.w + 2 * grab,
                geometry.h + 2 * grab,
            )),
            extra: (fl + fr, ft + fb),
        }
    }
}

/// Where everything sits on a toplevel's surface, in surface units: see
/// [`Decor::layout`]. Without decorations, the content is all there is.
#[derive(Clone, Copy)]
struct DecorLayout {
    /// The whole surface: content, frame and shadow.
    surface: (u32, u32),
    /// The content's size, and its top-left corner.
    content_size: (u32, u32),
    content: (u32, u32),
    /// The window geometry: the frame, without the shadow.
    geometry: Rect,
    /// The input region, `None` for all of the surface.
    input: Option<Rect>,
    /// What the frame adds to the content's width and height: what the window
    /// geometry — and with it every size the compositor and we tell each
    /// other — measures beyond the content.
    extra: (u32, u32),
}

impl DecorLayout {
    /// The content's top-left corner in device pixels, at `raster` of them to
    /// a surface unit: where the widget tree is drawn from.
    fn content_px(&self, raster: f32) -> (i32, i32) {
        let px = |v: u32| (v as f32 * raster).round() as i32;
        (px(self.content.0), px(self.content.1))
    }

    /// The decorations' geometry in device pixels, at `raster` of them to a
    /// surface unit, drawn at `paint_scale` device pixels to a saudade one.
    fn metrics(&self, raster: f32, paint_scale: f32, chrome: &WindowChrome) -> chrome::Metrics {
        let px = |v: i32| (v as f32 * raster).round() as i32;
        let span = |from: i32, len: i32| (px(from), px(from + len) - px(from));
        let (cx, cw) = span(self.content.0 as i32, self.content_size.0 as i32);
        let (cy, ch) = span(self.content.1 as i32, self.content_size.1 as i32);
        let g = self.geometry;
        let (fx, fw) = span(g.x, g.w);
        let (fy, fh) = span(g.y, g.h);
        let buffer = Size::new(
            px(self.surface.0 as i32).max(1),
            px(self.surface.1 as i32).max(1),
        );
        chrome::metrics_around(
            Rect::new(fx, fy, fw, fh),
            Rect::new(cx, cy, cw, ch),
            buffer,
            paint_scale,
            chrome,
        )
    }

    /// Send the window geometry and input region this layout calls for, if
    /// they aren't what `decor` sent last. Both are double-buffered: they take
    /// effect with the next commit of `surface`.
    fn sync(
        &self,
        decor: &mut Decor,
        compositor: &CompositorState,
        xdg: &XdgSurfaceObj,
        surface: &wl_surface::WlSurface,
    ) {
        if !decor.on && decor.sent.is_none() {
            return;
        }
        let want = (self.geometry, self.input);
        if decor.sent == Some(want) {
            return;
        }
        let g = self.geometry;
        xdg.set_window_geometry(g.x, g.y, g.w.max(1), g.h.max(1));
        match self
            .input
            .and_then(|r| Some((r, Region::new(compositor).ok()?)))
        {
            Some((r, region)) => {
                region.add(r.x, r.y, r.w, r.h);
                surface.set_input_region(Some(region.wl_region()));
            }
            None => surface.set_input_region(None),
        }
        decor.sent = Some(want);
    }
}

/// The xdg-shell edge a resize from `edges` moves.
fn resize_edge(edges: Edges) -> ResizeEdge {
    match (edges.top, edges.bottom, edges.left, edges.right) {
        (true, _, true, _) => ResizeEdge::TopLeft,
        (true, _, _, true) => ResizeEdge::TopRight,
        (_, true, true, _) => ResizeEdge::BottomLeft,
        (_, true, _, true) => ResizeEdge::BottomRight,
        (true, ..) => ResizeEdge::Top,
        (_, true, ..) => ResizeEdge::Bottom,
        (_, _, true, _) => ResizeEdge::Left,
        _ => ResizeEdge::Right,
    }
}

/// The pointer over a resize handle moving `edges`.
fn resize_cursor(edges: Edges) -> Cursor {
    match (edges.top || edges.bottom, edges.left || edges.right) {
        (true, true) if edges.top == edges.left => Cursor::NwseResize,
        (true, true) => Cursor::NeswResize,
        (true, false) => Cursor::NsResize,
        _ => Cursor::EwResize,
    }
}

/// A toplevel's min and max size hints, in surface units: `None` for none.
type SizeHints = (Option<(u32, u32)>, Option<(u32, u32)>);

/// Which toplevel a decorations event is about: the main window, or the
/// dialog at an index of the popup stack.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Toplevel {
    Main,
    Dialog(usize),
}

/// One `xdg_toplevel_icon_v1` we have handed the compositor, plus everything
/// that has to stay alive behind it.
///
/// The protocol is strict about lifetime: a buffer added to an icon must not be
/// destroyed, and the shm storage behind it must not be rewritten, for as long
/// as that icon exists — `no_buffer` is a fatal protocol error, and the
/// compositor may read the pixels at any point, since it never sends
/// `wl_buffer.release` for them. So the pool is private to this one icon (never
/// the frame pool, which recycles slots every redraw), written once at
/// construction, and neither it nor the icon object is ever dropped: the
/// compositor may still be holding the previous icon when it is handed a new
/// one. There are at most two of these in a process's life, at a few dozen KB
/// each.
struct ToplevelIcon {
    _obj: XdgToplevelIconV1,
    _pool: SlotPool,
    _buffers: Vec<Buffer>,
}

impl State {
    /// Rasterize the app's icon at each of `sizes` and set the result on the
    /// toplevel, replacing any icon set before.
    ///
    /// Every size is a fresh rasterization of the same vectors rather than a
    /// resample of one bitmap — the reason an icon is specified as an
    /// [`SvgImage`] in the first place. All buffers go on at scale 1, so the
    /// sizes are read as both surface-local and pixel sizes; a compositor that
    /// wants a HiDPI icon asks for the larger size rather than the same size at
    /// a larger scale.
    ///
    /// Does nothing when the app named no icon, or when the compositor lacks
    /// `xdg_toplevel_icon_v1` — there the desktop-entry file behind the app_id
    /// is the only icon source, and it isn't ours to write.
    fn set_toplevel_icon(&mut self, sizes: &[u32]) {
        // Both taken by value up front, so the borrow of `self` ends here and
        // the new icon can be pushed onto `self.icons` at the bottom.
        let (Some(mgr), Some(image)) = (self.toplevel_icon_mgr.clone(), self.icon_image) else {
            return;
        };
        let mut sizes: Vec<u32> = sizes.iter().copied().filter(|&s| s > 0).collect();
        sizes.sort_unstable();
        sizes.dedup();
        if sizes.is_empty() {
            return;
        }

        let bytes: usize = sizes.iter().map(|&s| s as usize * s as usize * 4).sum();
        let Ok(mut pool) = SlotPool::new(bytes, &self.shm) else {
            return;
        };
        let obj = mgr.create_icon(&self.qh, ());
        let mut buffers = Vec::with_capacity(sizes.len());
        for size in sizes {
            let edge = size as i32;
            let Ok((buffer, canvas)) =
                pool.create_buffer(edge, edge, edge * 4, wl_shm::Format::Argb8888)
            else {
                continue;
            };
            let rgba = image.rasterize_rgba(size);
            write_premultiplied(&rgba, bytes_as_u32_mut(canvas));
            obj.add_buffer(buffer.wl_buffer(), 1);
            buffers.push(buffer);
        }
        if buffers.is_empty() {
            obj.destroy();
            return;
        }

        mgr.set_icon(self.window.xdg_toplevel(), Some(&obj));
        // `set_icon` is double-buffered like any other toplevel state: it takes
        // effect on the next commit of the toplevel's surface. Before the first
        // configure that commit is the one the initial handshake is waiting on,
        // and the frame it answers with carries the icon along — so only a call
        // made later needs a commit of its own, and gets one here rather than
        // sitting invisible until something else happens to redraw.
        if self.configured {
            self.window.commit();
        }
        self.icons.push(ToplevelIcon {
            _obj: obj,
            _pool: pool,
            _buffers: buffers,
        });
    }

    /// Per-loop housekeeping: sync popup window state with the widget
    /// tree, then redraw any surface that asked for it. Idle iterations
    /// (no state changes since the last frame) do nothing — without
    /// gating on these flags we'd attach one buffer per loop tick and
    /// drown the compositor.
    fn tick(&mut self) {
        self.sync_popup();

        // Animation: while any widget asks for ticks, fan one out each
        // loop iteration (~60 Hz). Idle widgets ignore the event, so
        // the cost is a single function call per widget per frame. Two
        // triggers: a `wants_ticks()` pull (steady-state, e.g. a blinking
        // caret) or a pushed `request_tick` latch (transient, e.g. a held
        // scrollbar arrow). The latch is cleared before dispatch so the tick
        // handler re-arms it only while it still needs more.
        if self.root.wants_ticks() || self.tick_requested {
            self.tick_requested = false;
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

    /// The desktop switched to `appearance`: repaint every surface in the
    /// matching theme, unless it is the one showing already or the
    /// environment pinned the other.
    fn follow_system(&mut self, appearance: Appearance) {
        if self.themes.follow(appearance) {
            self.needs_redraw = true;
            self.mark_popups_dirty();
        }
    }

    /// The scale the display is presented at, as the compositor describes it:
    /// the fractional scale where the protocol is there to report it, else the
    /// integer buffer scale. The counterpart of the factor winit reports, and
    /// what [`Painter::system_scale`] hands to widgets.
    fn os_scale(&self) -> f32 {
        self.fractional_scale
            .unwrap_or(self.scale.max(1) as f32)
            .max(0.01)
    }

    /// Device pixels one surface unit is rasterized at.
    ///
    /// With a viewport that is the scale the display is presented at, fraction
    /// and all: the buffer comes out exactly the size the compositor will put
    /// on the glass, so a device pixel *is* a panel pixel and nothing is
    /// resampled. Without one the buffer size can only be stated as an integer
    /// multiple of the surface (`wl_surface.set_buffer_scale`), so a fractional
    /// display means rasterizing at the integer scale above it and letting the
    /// compositor scale the result down.
    fn raster_scale(&self) -> f32 {
        if self.viewport.is_some() {
            self.os_scale()
        } else {
            self.scale.max(1) as f32
        }
    }

    /// Buffer pixels one saudade logical pixel spans: the surface units it is
    /// worth, times the device pixels one of those is rasterized at. What the
    /// painter multiplies the widget tree's coordinates by.
    fn paint_scale(&self) -> f32 {
        self.ui * self.raster_scale()
    }

    /// The buffer that covers `(w, h)` surface units, in device pixels.
    fn buffer_size(&self, w: u32, h: u32) -> (i32, i32) {
        let raster = self.raster_scale();
        let px = |n: u32| ((n.max(1) as f32 * raster).round() as i32).max(1);
        (px(w), px(h))
    }

    /// State the size of `surface` in surface units and the scale its buffer is
    /// to be read at, then hand back nothing — the caller commits.
    ///
    /// Two ways to say it, and they are exclusive: a viewport states the
    /// surface size outright, so the buffer is whatever it is and
    /// `set_buffer_scale` stays at its default 1; without one the buffer scale
    /// is the only thing that can say it, and the buffer must be that integer
    /// multiple of the surface size.
    fn apply_surface_scale(
        surface: &wl_surface::WlSurface,
        viewport: Option<&WpViewport>,
        surface_w: u32,
        surface_h: u32,
        integer_scale: i32,
    ) {
        match viewport {
            Some(vp) => {
                vp.set_destination(surface_w.max(1) as i32, surface_h.max(1) as i32);
                surface.set_buffer_scale(1);
            }
            None => surface.set_buffer_scale(integer_scale.max(1)),
        }
    }

    /// The main surface in saudade logical pixels — what the widget tree is
    /// laid out into, the compositor's own units over [`Self::ui`].
    fn logical_size(&self) -> Size {
        Size::new(
            (self.surface_w.max(1) as f32 / self.ui).round().max(1.0) as i32,
            (self.surface_h.max(1) as f32 / self.ui).round().max(1.0) as i32,
        )
    }

    /// Re-derive [`Self::ui`] from the scale the compositor now reports, and
    /// carry the window's logical size across the change: a window holding 320
    /// saudade pixels goes on holding 320 of them, over however many surface
    /// units those now come to. The winit runtime's `adopt_scale`, in the one
    /// unit Wayland gives us to say it in.
    ///
    /// Called from both halves of the scale, the integer buffer one and the
    /// fractional one, since either can move on its own — and ends in a
    /// relayout either way, which is what the buffer scale moving on its own
    /// needs.
    fn adopt_scale(&mut self) {
        let ui = ui_for(self.os_scale(), self.scale_pin, self.reference_dpi);
        // Compared with a hair of slack: the ladder divides out to the same
        // ratio at every factor, but not to the same *bits* — 20/12 over 1.25
        // lands a binary step off 16/12 over 1 — and re-deriving a number the
        // window is already laid out in would drop the popup stack for nothing.
        if (ui - self.ui).abs() > UI_EPSILON {
            let was = self.ui;
            self.ui = ui;
            // Every open popup was sized in the unit that just changed.
            // Dropping the stack lets `sync_popup` rebuild it in the new one,
            // from the same requests the widget tree is still making.
            self.popups.clear();
            // The size a fullscreen window comes back to is in surface units
            // as well, so it is re-counted whether or not we are fullscreen
            // now — the window that holds it may not leave fullscreen until
            // long after the scale moved.
            self.windowed_size = self.windowed_size.map(|size| requantize(size, was, ui));
            // Fullscreen itself has no size of ours to preserve: the display
            // decides it until the configure that releases the window.
            let mut commit = false;
            if !self.fullscreen {
                let (w, h) = requantize((self.surface_w, self.surface_h), was, ui);
                if (w, h) != (self.surface_w, self.surface_h) {
                    self.surface_w = w;
                    self.surface_h = h;
                    commit = true;
                }
            }
            // The size hints are in surface units too, and so are the
            // decorations they count in.
            commit |= self.apply_size_hints();
            if commit {
                self.window.commit();
            }
        }
        self.relayout();
        self.needs_redraw = true;
        self.mark_popups_dirty();
    }

    /// Panel pixels the display the window is on fits to the inch, worked out
    /// from the physical size and current mode `wl_output` reports for it: the
    /// Wayland counterpart of the EDID figure the Windows and macOS runtimes
    /// ask the platform for. `None` where the output reports no size — a
    /// projector, a virtual output — or has no current mode.
    ///
    /// Both figures are in the output's own, untransformed orientation, so a
    /// rotated display needs no special case: the widths divide out the same.
    fn compute_output_dpi(&self) -> Option<f32> {
        let output = self.output.clone().or_else(|| {
            // Before any `wl_surface.enter`, and on a compositor that sends
            // none, the one output there is will do; with several there is
            // nothing to choose between them yet.
            let mut outputs = self.output_state.outputs();
            let only = outputs.next()?;
            outputs.next().is_none().then_some(only)
        })?;
        let info = self.output_state.info(&output)?;
        let px = info.modes.iter().find(|m| m.current)?.dimensions.0;
        let mm = info.physical_size.0;
        (px > 0 && mm > 0).then(|| px as f32 * 25.4 / mm as f32)
    }

    /// Re-read [`Self::output_dpi`], repainting if it moved: a window that has
    /// landed on a display of another density has to draw anything sized in
    /// millimetres again, scale factor or no scale factor.
    fn refresh_output_dpi(&mut self) {
        let dpi = self.compute_output_dpi();
        if dpi != self.output_dpi {
            self.output_dpi = dpi;
            self.needs_redraw = true;
            self.mark_popups_dirty();
        }
    }

    /// The density to hand the painter — [`Painter::display_dpi`], which counts
    /// the device pixels the painter *draws* in, and those are the buffer's,
    /// not the panel's.
    ///
    /// They are the same figure wherever we rasterize at the scale the display
    /// is presented at — which, with a viewport, is everywhere. Without one a
    /// fractional display is oversampled at the integer scale above it and
    /// resampled on the way to the glass, so an inch of buffer holds
    /// `raster_scale / os_scale` more pixels than an inch of panel: exactly the
    /// oversampling a Mac's framebuffer has over its panel in a scaled mode,
    /// which [`Painter::display_dpi`] already answers for there.
    fn device_dpi(&self) -> Option<f32> {
        Some(self.output_dpi? * self.raster_scale() / self.os_scale())
    }

    fn relayout(&mut self) {
        // The widget tree is laid out in saudade logical pixels, of which a
        // Wayland surface unit is `ui` (see `ui_for`) — the surface coordinates
        // the compositor configures us in are the platform's own unit, not
        // ours. The buffer scale on top of that is the painter's business,
        // applied when the tree is handed one.
        let size = self.logical_size();
        self.root.layout(Rect::new(0, 0, size.w, size.h));
    }

    /// Dispatch `event` into the widget tree and apply the requests it left on
    /// the [`EventCtx`], returning that context so the caller can read the
    /// requests it didn't act on: `accepted_drop` for the incoming-drag events
    /// (turned into accept/reject on the drag offer) and `swallow_key` for the
    /// keyboard path. Most callers ignore the result.
    fn dispatch(&mut self, event: Event) -> EventCtx {
        let mut ctx = EventCtx::with_fullscreen(self.fullscreen);
        ctx.modifiers = self.modifiers;
        ctx.dragging_out = self.drag_source.is_some();
        self.root.event(&event, &mut ctx);
        if ctx.paint_requested {
            self.needs_redraw = true;
            self.mark_popups_dirty();
        }
        // Push-based tick keep-alive (see `EventCtx::request_tick`): latch the
        // request so the next `tick()` keeps the clock running, regardless of
        // how deeply the requesting widget is nested.
        if ctx.tick_requested {
            self.tick_requested = true;
        }
        if ctx.close_requested {
            self.exit = true;
        }
        if let Some(size) = ctx.resize_request {
            self.apply_resize(size);
        }
        if let Some(fullscreen) = ctx.fullscreen_request {
            self.apply_fullscreen(fullscreen);
        }
        if let Some(title) = ctx.title_request.take() {
            // Not double-buffered state: the compositor takes it on receipt,
            // with no commit needed. Decorations of our own show it as well.
            self.window.set_title(title.clone());
            self.title = title;
            if self.decor.on {
                self.needs_redraw = true;
            }
        }
        if let Some((data, actions)) = ctx.drag_request.take() {
            self.begin_drag(data, &actions);
        }
        ctx
    }

    /// Start an outbound drag carrying `data`'s file paths, its drop doing one
    /// of `actions`, in response to a widget's [`EventCtx::start_drag`]. Needs
    /// the data-device plumbing plus the latest pointer press (its serial +
    /// surface): Wayland only lets a client begin a drag off the implicit grab
    /// a button press established, so this is expected to be called while that
    /// button is still held. Bails quietly if any of that is missing or the
    /// payload has no paths — there's nothing the user could have grabbed.
    fn begin_drag(&mut self, data: DragData, actions: &[DropAction]) {
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

        // Offer only the uri-list, and the actions the widget asked for. A
        // move is the target's to carry out: we never delete anything when a
        // drop ends in one.
        let actions = actions
            .iter()
            .fold(DndAction::empty(), |all, a| all | dnd_action(*a));
        let source = mgr.create_drag_and_drop_source(&self.qh, [URI_LIST], actions);
        source.start_drag(dd, &origin, icon.as_ref().map(|i| &i.surface), serial);
        self.drag_payload = paths_to_uri_list(&data.paths).into_bytes();
        self.drag_source = Some(source);
        self.buttons_held = 0;
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
        self.set_cursor_shape(Shape::NoDrop);
    }

    /// Lay out the cursor-following drag icon and allocate its surface + pool,
    /// but don't paint it yet — the surface only becomes a drag icon once
    /// `start_drag` assigns the role, so [`Self::paint_drag_icon`] is called
    /// (which commits) afterwards. Returns `None` if there's no font, so callers
    /// drag without an icon.
    fn create_drag_icon(&self, data: &DragData) -> Option<DragIcon> {
        let label = drag_icon_label(&data.paths);
        let font = self.font.as_ref()?;
        let (text_w, text_h) = font.measure(&label, self.themes.current().font_size);
        let text_h = text_h.ceil() as i32;

        let badge = text_h.max(DRAG_ICON_BADGE_MIN);
        let logical_w =
            DRAG_ICON_PAD + badge + DRAG_ICON_GAP + text_w.ceil() as i32 + DRAG_ICON_PAD;
        let logical_h = DRAG_ICON_PAD + badge.max(text_h) + DRAG_ICON_PAD;
        let scale = self.scale.max(1);
        // The chip's own size in the compositor's units, rounded *up* so the
        // buffer can never come out a pixel short of what the painter draws
        // into it.
        let surface_w = (logical_w as f32 * self.ui).ceil().max(1.0) as i32;
        let surface_h = (logical_h as f32 * self.ui).ceil().max(1.0) as i32;
        // The chip's buffer, in the device pixels it is really rasterized in.
        let raster = self.raster_scale();
        let buf_w = ((surface_w as f32 * raster).ceil() as i32).max(1);
        let buf_h = ((surface_h as f32 * raster).ceil() as i32).max(1);
        // Room for two buffers so a re-render (on a feedback change) can grab a
        // fresh slot while the compositor still holds the previous one.
        let pool_bytes = (buf_w * buf_h * 4) as usize * 2;
        let pool = SlotPool::new(pool_bytes, &self.shm).ok()?;
        let surface = self.compositor.create_surface(&self.qh);
        let viewport = self
            .viewporter
            .as_ref()
            .map(|vp| vp.get_viewport(&surface, &self.qh, ()));

        Some(DragIcon {
            surface,
            pool,
            label,
            logical_w,
            logical_h,
            surface_w,
            surface_h,
            buf_w,
            buf_h,
            viewport,
            badge,
            scale,
            paint_scale: self.paint_scale(),
            feedback: DragFeedback::NoDrop,
        })
    }

    /// (Re)paint the drag icon for its current `feedback` and commit it. Safe to
    /// call repeatedly during a drag — the icon surface stays mapped, so each
    /// commit just swaps what follows the cursor. A no-op if the pool's buffer
    /// is momentarily busy (the previous frame stays up until the next change).
    fn paint_drag_icon(&self, icon: &mut DragIcon) {
        let (buf_w, buf_h) = (icon.buf_w, icon.buf_h);
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
                icon.paint_scale,
                0,
                0,
                FontSet {
                    sans: self.font.as_deref(),
                    serif: self.serif_font.as_deref(),
                    mono: self.mono_font.as_deref(),
                },
                None,
            );
            painter.set_system_scale(self.os_scale());
            let theme = self.themes.current();
            let area = Rect::new(0, 0, icon.logical_w, icon.logical_h);
            painter.fill_rect(area, theme.background);
            painter.raised_bevel(area, theme.highlight, theme.shadow);
            let badge = Rect::new(DRAG_ICON_PAD, DRAG_ICON_PAD, icon.badge, icon.badge);
            draw_drag_badge(&mut painter, badge, icon.feedback);
            painter.text(
                DRAG_ICON_PAD + icon.badge + DRAG_ICON_GAP,
                DRAG_ICON_PAD,
                &icon.label,
                theme.font_size,
                theme.text,
            );
        }
        let _ = buffer.attach_to(&icon.surface);
        Self::apply_surface_scale(
            &icon.surface,
            icon.viewport.as_ref(),
            icon.surface_w.max(1) as u32,
            icon.surface_h.max(1) as u32,
            icon.scale,
        );
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
            // Hand the pointer back to its normal cursor, and forget the shape
            // we were showing so the next move re-applies whatever the widget
            // under the pointer wants (it may not be the arrow).
            self.set_cursor_shape(Shape::Default);
            self.cursor_shape = Cursor::Default;
        }
    }

    /// Set the pointer to `shape` via `wp_cursor_shape` — the mechanism behind
    /// both a widget's requested cursor (hand over a button, I-beam over a text
    /// field) and the copy / no-drop feedback shown while we're a drag source.
    /// `set_shape` wants the latest `wl_pointer.enter` serial, which SCTK tracks
    /// for us. A no-op when the protocol is unavailable or no serial is on
    /// record; if the compositor declines to honor it (e.g. mid-drag), the
    /// cursor simply stays put.
    fn set_cursor_shape(&self, shape: Shape) {
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

    /// Reconcile the pointer shape after a [`PointerMove`](Event::PointerMove)
    /// dispatch: show whatever the widget under the pointer asked for (or the
    /// arrow if none did). The last shape is remembered so an unchanged hover
    /// doesn't re-issue a request — except when `force` is set, which always
    /// emits the shape.
    ///
    /// `force` is used on `wl_pointer.enter`: Wayland leaves the pointer image
    /// *undefined* when the pointer enters a surface and makes the client
    /// responsible for setting it, so we can't trust our remembered shape (it
    /// may have been the previous surface's) and must re-establish it — even
    /// when it's the plain arrow. A subsequent motion that doesn't change the
    /// shape can still skip the request.
    ///
    /// An in-flight *outbound* drag drives the cursor itself (copy / no-drop),
    /// so we stay out of its way while one is live.
    fn apply_cursor(&mut self, requested: Option<Cursor>, force: bool) {
        if self.drag_source.is_some() {
            return;
        }
        let want = requested.unwrap_or(Cursor::Default);
        if !force && self.cursor_shape == want {
            return;
        }
        self.cursor_shape = want;
        self.set_cursor_shape(wayland_shape(want));
    }

    /// Resize the window to `size` saudade logical pixels, at the widget's
    /// request — carried to the compositor in its own units. For a fixed window
    /// we move the min == max hints to the new size so the compositor lets it
    /// change; then we adopt the size, relayout, and repaint. The compositor
    /// echoes a configure with the same size, which the configure handler
    /// treats as a no-op transition.
    fn apply_resize(&mut self, size: Size) {
        let w = surface_units(size.w, self.ui);
        let h = surface_units(size.h, self.ui);
        if self.fullscreen {
            // The display decides the size for now; this is the size to take
            // once the window leaves fullscreen.
            self.windowed_size = Some((w, h));
            return;
        }
        if w == self.surface_w && h == self.surface_h {
            return;
        }
        self.surface_w = w;
        self.surface_h = h;
        self.apply_size_hints();
        self.window.commit();
        self.relayout();
        self.needs_redraw = true;
        self.mark_popups_dirty();
    }

    /// Ask the compositor to make the main window fullscreen on whichever
    /// output it chooses (the one the window is on), or to take it out again,
    /// at a widget's request. The compositor answers with a configure carrying
    /// the new size, which [`WindowHandler::configure`] adopts; until then the
    /// window reports the state it was asked for.
    fn apply_fullscreen(&mut self, fullscreen: bool) {
        if fullscreen == self.fullscreen {
            return;
        }
        self.fullscreen = fullscreen;
        if fullscreen {
            self.windowed_size = Some((self.surface_w, self.surface_h));
            self.apply_size_hints();
            self.window.set_fullscreen(None);
        } else {
            self.window.unset_fullscreen();
            self.apply_size_hints();
        }
        self.window.commit();
    }

    /// Send the main window's min / max size hints, if they aren't what was
    /// sent last; returns whether they were sent. They are double-buffered, so
    /// the caller commits — or leaves it to the frame about to be drawn.
    ///
    /// A resizable window gets the configured minimum, or a small default so it
    /// can't be collapsed to nothing. A fixed one gets min == max, which tells
    /// the compositor it can't be resized — without the max hint,
    /// wlroots-based compositors (river, …) report `max=(0x0)` →
    /// `is_fixed_size=false` and still offer resize edges — pinned to the size
    /// it has, or returns to from fullscreen; while it is fullscreen it gets
    /// none, which would contradict the display's size. Either way the hints
    /// measure the window geometry, so they count in the frame of any
    /// decorations we draw.
    fn apply_size_hints(&mut self) -> bool {
        let extra = self.main_layout().extra;
        let grow = |(w, h): (u32, u32)| Some((w + extra.0, h + extra.1));
        let hints = if self.resizable {
            let min = self
                .min_size
                .map(|m| (surface_units(m.w, self.ui), surface_units(m.h, self.ui)))
                .unwrap_or((100, 60));
            (grow(min), None)
        } else if self.fullscreen {
            (None, None)
        } else {
            let size = grow(
                self.windowed_size
                    .unwrap_or((self.surface_w, self.surface_h)),
            );
            (size, size)
        };
        if self.sent_hints == Some(hints) {
            return false;
        }
        self.window.set_min_size(hints.0);
        self.window.set_max_size(hints.1);
        self.sent_hints = Some(hints);
        true
    }

    /// Where everything sits on the main window's surface.
    fn main_layout(&self) -> DecorLayout {
        self.decor.layout((self.surface_w, self.surface_h), self.ui)
    }

    /// Where everything sits on a toplevel's surface. A popup index that
    /// isn't a dialog's — a stack rebuilt since — is laid out undecorated.
    fn toplevel_layout(&self, t: Toplevel) -> DecorLayout {
        match t {
            Toplevel::Main => self.main_layout(),
            Toplevel::Dialog(i) => match self.popups.get(i) {
                Some(p) => p.decor.layout((p.surface_w, p.surface_h), self.ui),
                None => Decor::new(WindowFrame::Dialog).layout((1, 1), self.ui),
            },
        }
    }

    /// The toplevel `surface` is: the main window's, or a dialog's. `None` for
    /// a menu's, or a surface we don't know.
    fn toplevel_of(&self, surface: &wl_surface::WlSurface) -> Option<Toplevel> {
        if surface.id() == self.window.wl_surface().id() {
            return Some(Toplevel::Main);
        }
        let i = self
            .popups
            .iter()
            .position(|p| p.surface.wl_surface().id() == surface.id())?;
        (self.popups[i].surface.kind() == PopupKind::Dialog).then_some(Toplevel::Dialog(i))
    }

    /// A toplevel's decorations, and the window they belong to.
    fn toplevel_parts(&mut self, t: Toplevel) -> Option<(&mut Decor, &XdgWindow)> {
        match t {
            Toplevel::Main => Some((&mut self.decor, &self.window)),
            Toplevel::Dialog(i) => {
                let PopupState { decor, surface, .. } = self.popups.get_mut(i)?;
                match surface {
                    ChildSurface::Dialog { window, .. } => Some((decor, &*window)),
                    ChildSurface::Popup(_) => None,
                }
            }
        }
    }

    /// Repaint a toplevel.
    fn redraw_toplevel(&mut self, t: Toplevel) {
        match t {
            Toplevel::Main => self.needs_redraw = true,
            Toplevel::Dialog(i) => {
                if let Some(p) = self.popups.get_mut(i) {
                    p.needs_redraw = true;
                }
            }
        }
    }

    fn draw_main(&mut self) {
        let scale = self.scale.max(1);
        // Read off `self` before the pool hands out the canvas, which borrows
        // it for as long as the painter lives.
        let paint_scale = self.paint_scale();
        let raster = self.raster_scale();
        let system_scale = self.os_scale();
        let display_dpi = self.device_dpi();
        let layout = self.main_layout();
        let decorations = self.decor.on.then(|| {
            let chrome = self.decor.chrome(&self.title);
            let m = layout.metrics(raster, paint_scale, &chrome);
            (chrome, m)
        });
        let (origin_x, origin_y) = layout.content_px(raster);
        let (buf_w, buf_h) = self.buffer_size(layout.surface.0, layout.surface.1);
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
        let fonts = FontSet {
            sans: self.font.as_deref(),
            serif: self.serif_font.as_deref(),
            mono: self.mono_font.as_deref(),
        };

        // Buffer holds physical pixels; the painter multiplies the widget
        // tree's logical coords by `paint_scale` to land on them — the surface
        // units a saudade pixel is worth, times the integer buffer scale —
        // from the content's corner, inside the decorations if there are any.
        {
            let mut painter = Painter::with_popup_anchor(
                pixels,
                buf_w,
                buf_h,
                paint_scale,
                origin_x,
                origin_y,
                fonts,
                None,
            )
            .with_display_dpi(display_dpi);
            // Report the scale the display is presented at (e.g. 1.25), which
            // is neither the integer scale the buffer is rasterized at nor the
            // scale the UI is drawn at: widgets asking what the *display* is
            // set to want this one.
            painter.set_system_scale(system_scale);
            let theme = self.themes.current();
            painter.fill_pattern(theme.background, self.bg.pattern, self.bg.ink(theme));
            if let Some((_, m)) = &decorations {
                let c = m.content;
                painter.set_clip_phys(c.x, c.y, c.w, c.h);
            }
            self.root.paint(&mut painter, theme);
        }
        if let Some((chrome, m)) = &decorations {
            // The frame around it, drawn 1:1 in the device pixels its metrics
            // are already in, and the shadow, see-through.
            let mut painter = Painter::new(pixels, buf_w, buf_h, 1.0, 0, 0, fonts);
            chrome::paint_decorations(&mut painter, m, chrome);
            crate::painter::premultiply(pixels);
        }

        let surface = self.window.wl_surface();
        layout.sync(
            &mut self.decor,
            &self.compositor,
            self.window.xdg_surface(),
            surface,
        );
        let _ = buffer.attach_to(surface);
        // damage_buffer takes buffer-pixel coordinates.
        surface.damage_buffer(0, 0, buf_w, buf_h);
        // Say how big the surface is, so the compositor doesn't scale what we
        // drew — it is already at the display's own resolution.
        Self::apply_surface_scale(
            surface,
            self.viewport.as_ref(),
            layout.surface.0,
            layout.surface.1,
            scale,
        );
        surface.frame(&self.qh, surface.clone());
        surface.commit();
    }

    /// Draw the popup window at `idx`. Returns true if anything was drawn.
    fn draw_popup(&mut self, idx: usize) -> bool {
        let scale = self.scale.max(1);
        let paint_scale = self.paint_scale();
        let raster = self.raster_scale();
        let system_scale = self.os_scale();
        let display_dpi = self.device_dpi();
        let layout = self.toplevel_layout(Toplevel::Dialog(idx));
        let decorations = match self.popups.get(idx) {
            Some(p) => p.decor.on.then(|| {
                let chrome = p.decor.chrome(&p.title);
                let m = layout.metrics(raster, paint_scale, &chrome);
                (chrome, m)
            }),
            None => return false,
        };
        let (content_x, content_y) = layout.content_px(raster);
        let buffer = self.buffer_size(layout.surface.0, layout.surface.1);
        let Some(p) = self.popups.get_mut(idx) else {
            return false;
        };
        let (buf_w, buf_h) = buffer;
        let stride = buf_w * 4;
        let buf = match p
            .pool
            .create_buffer(buf_w, buf_h, stride, wl_shm::Format::Argb8888)
        {
            Ok((b, _)) => b,
            Err(_) => return false,
        };
        let canvas = match p.pool.canvas(&buf) {
            Some(c) => c,
            None => return false,
        };
        let pixels = bytes_as_u32_mut(canvas);
        // The anchor is in the root widget's saudade pixels, so it reaches
        // buffer pixels through the same `paint_scale` the painter draws with.
        let anchor = p.anchor;
        let origin_x = content_x - (anchor.x as f32 * paint_scale).round() as i32;
        let origin_y = content_y - (anchor.y as f32 * paint_scale).round() as i32;
        let clip_w = (anchor.w as f32 * paint_scale).round() as i32;
        let clip_h = (anchor.h as f32 * paint_scale).round() as i32;
        // A menu panel never fills its buffer: the L-shaped drop shadow leaves
        // the two corners inside its bend uncovered, and the buffer is rounded
        // twice — the anchor to whole surface units, then those to device
        // pixels — where the footprint above is rounded once, so it can come
        // out a pixel wider or taller (140 logical at a 4/3 UI scale on a 200%
        // display: 187 surface units, 374 pixels, a 373-pixel footprint). An
        // `Argb8888` buffer carries alpha, so an `xdg_popup` leaves all of that
        // transparent and only draws the overlay pass, instead of redrawing the
        // app underneath and leaving the sliver past the footprint showing the
        // background fill. A dialog is a real, opaque window and keeps the
        // redraw.
        let transparent = p.surface.kind() == PopupKind::Popup;

        let fonts = FontSet {
            sans: self.font.as_deref(),
            serif: self.serif_font.as_deref(),
            mono: self.mono_font.as_deref(),
        };
        {
            let mut painter = Painter::with_popup_anchor(
                pixels,
                buf_w,
                buf_h,
                paint_scale,
                origin_x,
                origin_y,
                fonts,
                Some(anchor),
            )
            .with_display_dpi(display_dpi);
            painter.set_system_scale(system_scale);
            let theme = self.themes.current();
            if transparent {
                painter.fill(Color::TRANSPARENT);
                painter.set_clip_phys(0, 0, clip_w.max(buf_w), clip_h.max(buf_h));
                self.root.paint_overlay(&mut painter, theme);
            } else {
                painter.fill(theme.background);
                painter.set_clip_phys(content_x, content_y, clip_w, clip_h);
                self.root.paint(&mut painter, theme);
            }
        }
        if let Some((chrome, m)) = &decorations {
            let mut painter = Painter::new(pixels, buf_w, buf_h, 1.0, 0, 0, fonts);
            chrome::paint_decorations(&mut painter, m, chrome);
        }
        if transparent || decorations.is_some() {
            crate::painter::premultiply(pixels);
        }

        if let ChildSurface::Dialog { window, .. } = &p.surface {
            layout.sync(
                &mut p.decor,
                &self.compositor,
                window.xdg_surface(),
                window.wl_surface(),
            );
        }
        let surface = p.surface.wl_surface();
        let _ = buf.attach_to(surface);
        surface.damage_buffer(0, 0, buf_w, buf_h);
        Self::apply_surface_scale(
            surface,
            p.viewport.as_ref(),
            layout.surface.0,
            layout.surface.1,
            scale,
        );
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
            match self.create_popup(&req, self.popups.last()) {
                Some(p) => self.popups.push(p),
                // A child popup must not outlive a parent we couldn't create.
                None => break,
            }
        }
    }

    /// Create one popup window for `request`. `parent` is the popup it nests
    /// into — `None` means the main window. Popup anchors are in root-widget
    /// coordinates, so the positioner is offset by the parent's anchor to land
    /// in the parent surface's local space.
    fn create_popup(
        &self,
        request: &PopupRequest,
        parent: Option<&PopupState>,
    ) -> Option<PopupState> {
        let parent_anchor = parent.map_or(Rect::new(0, 0, 0, 0), |p| p.anchor);
        let anchor = request.rect;
        // The anchor is in the root widget's saudade pixels; every size and
        // offset the compositor is handed below — the positioner's rects, the
        // surface size, a dialog's hints — is in its own surface units.
        let surf_w = surface_units(anchor.w, self.ui);
        let surf_h = surface_units(anchor.h, self.ui);

        let surface = match request.kind {
            PopupKind::Popup => {
                // Build a positioner anchored to a 1×1 rect at the popup's
                // top-left *in the parent surface*. Gravity goes BottomRight so
                // the popup extends down/right from the anchor — same shape as a
                // classic dropdown menu.
                // The positioner's rect is relative to the parent's window
                // geometry, which with decorations of our own on it is the
                // frame: the content starts the frame's border and title bar in.
                let geometry = match parent {
                    None => self.main_layout(),
                    Some(p) => p.decor.layout((p.surface_w, p.surface_h), self.ui),
                };
                let (frame_x, frame_y) = (
                    geometry.content.0 as i32 - geometry.geometry.x,
                    geometry.content.1 as i32 - geometry.geometry.y,
                );
                let rel_x = surface_offset(anchor.x - parent_anchor.x, self.ui) + frame_x;
                let rel_y = surface_offset(anchor.y - parent_anchor.y, self.ui) + frame_y;
                let positioner: XdgPositioner =
                    self.xdg_shell.xdg_wm_base().create_positioner(&self.qh, ());
                positioner.set_size(surf_w as i32, surf_h as i32);
                positioner.set_anchor_rect(rel_x, rel_y, 1, 1);
                positioner.set_anchor(Anchor::BottomLeft);
                positioner.set_gravity(Gravity::BottomRight);

                let parent =
                    parent.map_or_else(|| self.window.xdg_surface(), |p| p.surface.xdg_surface());
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
                // close button (or we do, if it won't — see `Decor`), and
                // we ask it to enforce a fixed size
                // (set_min_size == set_max_size disables the resize
                // affordances). `set_parent` makes the dialog transient
                // to the window it belongs to — the dialog beneath it, or
                // the main window, as on the other platforms. WSLg's
                // compositor, for one, centers it over that window and
                // activates it in that window's place. If the compositor
                // advertises `xdg_wm_dialog_v1` we additionally register
                // the toplevel as a dialog and ask for modal semantics —
                // that's what tells wlroots-based compositors (river,
                // labwc, …) to drop the minimize / maximize controls
                // from the SSD chrome.
                let dialog_surface = self.compositor.create_surface(&self.qh);
                let dialog =
                    self.xdg_shell
                        .create_window(dialog_surface, self.decorations, &self.qh);
                let title = request.title.as_deref().unwrap_or("Dialog");
                dialog.set_title(title);
                let owner = parent
                    .and_then(|p| p.surface.dialog_window())
                    .unwrap_or(&self.window);
                dialog.set_parent(Some(owner));
                dialog.set_min_size(Some((surf_w, surf_h)));
                dialog.set_max_size(Some((surf_w, surf_h)));

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
        let max_scale = self.raster_scale().ceil().max(1.0) as u32;
        let pool_bytes = (surf_w * surf_h * max_scale * max_scale * 4) as usize * 2;
        let pool = match SlotPool::new(pool_bytes, &self.shm) {
            Ok(p) => p,
            Err(_) => return None,
        };

        let viewport = self
            .viewporter
            .as_ref()
            .map(|vp| vp.get_viewport(surface.wl_surface(), &self.qh, ()));

        Some(PopupState {
            surface,
            decor: Decor::new(WindowFrame::Dialog),
            title: request.title.as_deref().unwrap_or("Dialog").to_string(),
            viewport,
            pool,
            anchor,
            surface_w: surf_w,
            surface_h: surf_h,
            configured: false,
            needs_redraw: true,
            cursor: None,
        })
    }

    /// Surface-local coordinates in the widget tree's saudade pixels. The
    /// pointer and the data device both report positions "in surface
    /// coordinates" — the compositor's own unit, `ui` of them to a saudade
    /// pixel — and `anchor` carries the offset of the surface they are on in
    /// the root widget's space: a popup's anchor, or zero for the main window.
    fn surface_to_logical(&self, surface_x: f64, surface_y: f64, anchor: Point) -> Point {
        let ui = self.ui.max(0.01) as f64;
        Point::new(
            (surface_x / ui).floor() as i32 + anchor.x,
            (surface_y / ui).floor() as i32 + anchor.y,
        )
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

    /// A position on `surface` (surface-local, as the pointer and the data
    /// device report it) in the widget tree's saudade pixels: measured from
    /// the corner of the content, inside any decorations we draw around it,
    /// and offset by the surface's anchor.
    fn surface_point(&self, surface: &wl_surface::WlSurface, x: f64, y: f64) -> Point {
        let (cx, cy) = match self.toplevel_of(surface) {
            Some(t) => self.toplevel_layout(t).content,
            None => (0, 0),
        };
        let anchor = self.surface_anchor(surface);
        self.surface_to_logical(x - cx as f64, y - cy as f64, anchor)
    }

    /// What a surface-local position on toplevel `t` lands on: the content, or
    /// a part of the decorations we draw around it.
    fn decor_hit(&self, t: Toplevel, x: f64, y: f64) -> Hit {
        let decor = match t {
            Toplevel::Main => &self.decor,
            Toplevel::Dialog(i) => match self.popups.get(i) {
                Some(p) => &p.decor,
                None => return Hit::Content,
            },
        };
        if !decor.on {
            return Hit::Content;
        }
        let raster = self.raster_scale();
        let chrome = decor.chrome("");
        let m = self
            .toplevel_layout(t)
            .metrics(raster, self.paint_scale(), &chrome);
        let px = |v: f64| (v * raster as f64).floor() as i32;
        chrome::hit_test(&m, &chrome, Point::new(px(x), px(y)))
    }

    /// Close toplevel `t`, as its close box or the compositor asked.
    ///
    /// The main window gives the root a chance to react before we quit: a
    /// widget used directly as the window root (rather than hosted in a
    /// `Modal`) gets its `on_cancel` here, so a dialog-as-window can revert
    /// pending edits on close, matching Escape and the dialog close path. Most
    /// roots leave this a no-op.
    ///
    /// A dialog gets an Escape, so the dialog widget's dismiss path runs
    /// (which clears `open`; the next `sync_popup` tear-down then destroys the
    /// toplevel).
    fn close_toplevel(&mut self, t: Toplevel) {
        match t {
            Toplevel::Main => {
                let mut ctx = EventCtx::with_fullscreen(self.fullscreen);
                self.root.on_cancel(&mut ctx);
                self.exit = true;
            }
            Toplevel::Dialog(_) => {
                let mods = self.modifiers;
                self.dispatch(Event::KeyDown {
                    key: Key::Named(NamedKey::Escape),
                    modifiers: mods,
                });
            }
        }
    }

    /// Dispatch a [`DragMove`](Event::DragMove) where the incoming drag now
    /// is — after it moved, or the modifiers changed what a drop there would
    /// do — and re-tell the offer if the answer changed: a drag can cross
    /// from a drop zone to plain content within one window.
    fn drag_moved(&mut self, conn: &Connection) {
        let Some(pos) = self.drag.as_ref().map(|d| d.pos) else {
            return;
        };
        let accepted = self.dispatch(Event::DragMove { pos }).accepted_drop;
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

    /// Tell an incoming drag offer whether we'll take it. Accepting asks for
    /// the action the widget named, falling back to a copy where the source
    /// doesn't offer it, and names the form a drop reads first (see
    /// [`mime_drop`]); rejecting clears the action and mime so the source sees
    /// "no drop" over us. Flushed right away so the source's cursor updates
    /// without waiting for the next loop turn.
    fn apply_drag_offer(&self, offer: &DragOffer, accepted: Option<DropAction>, conn: &Connection) {
        if let Some(action) = accepted {
            let form = offer.with_mime_types(|types| mime_drop::first(types).map(str::to_string));
            let preferred = dnd_action(action);
            offer.set_actions(DndAction::Copy | preferred, preferred);
            offer.accept_mime_type(offer.serial, form);
        } else {
            offer.set_actions(DndAction::empty(), DndAction::empty());
            offer.accept_mime_type(offer.serial, None);
        }
        let _ = conn.flush();
    }

    /// Read the next form of the dropped `offer` (see [`Reading`]), or, with
    /// none left, end the drag and hand the widgets what it carried. Each form
    /// is read *asynchronously* off the pipe the source writes it to, via
    /// calloop — a blocking read here would freeze the UI if the source were
    /// slow to write — and the next one asked for once it is in.
    fn read_drop(&mut self, offer: DragOffer, mut reading: Reading, pos: Point) {
        let Some(form) = reading.next() else {
            offer.finish();
            offer.destroy();
            self.dispatch(Event::Drop {
                pos,
                data: reading.finish(),
            });
            self.mark_popups_dirty();
            return;
        };
        // A form the source can't hand over is one fewer to read.
        let Ok(read_pipe) = offer.receive(form.clone()) else {
            return self.read_drop(offer, reading, pos);
        };
        // Shared with the reader, which takes it once the form is in; left
        // here if the reader can't be registered.
        let pending = Rc::new(Cell::new(Some((offer, reading))));
        let theirs = pending.clone();
        let mut buf: Vec<u8> = Vec::new();
        let inserted =
            self.loop_handle
                .insert_source(read_pipe, move |_event, file, state: &mut State| {
                    use std::io::Read;
                    // SAFETY: we only read from the fd; calloop owns and closes it.
                    let f: &mut std::fs::File = unsafe { file.get_mut() };
                    let mut chunk = [0u8; 64 * 1024];
                    match f.read(&mut chunk) {
                        // EOF: the source finished writing the form.
                        Ok(0) => {}
                        Ok(n) => {
                            buf.extend_from_slice(&chunk[..n]);
                            return PostAction::Continue;
                        }
                        Err(ref e)
                            if matches!(
                                e.kind(),
                                std::io::ErrorKind::Interrupted | std::io::ErrorKind::WouldBlock
                            ) =>
                        {
                            // Spurious wakeup / signal — wait for the next.
                            return PostAction::Continue;
                        }
                        // A broken pipe: nothing of this form.
                        Err(_) => buf.clear(),
                    }
                    if let Some((offer, mut reading)) = theirs.take() {
                        reading.take(&form, std::mem::take(&mut buf));
                        state.read_drop(offer, reading, pos);
                    }
                    PostAction::Remove
                });
        if inserted.is_err()
            && let Some((offer, reading)) = pending.take()
        {
            // Couldn't watch the pipe: skip the form rather than leave the UI
            // stuck mid-drag.
            self.read_drop(offer, reading, pos);
        }
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
        // Where the compositor reports no fractional scale this is also the
        // factor the display is presented at, so the scale to draw at moves
        // with it; `adopt_scale` relayouts and repaints either way.
        self.adopt_scale();
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
        surface: &wl_surface::WlSurface,
        output: &wl_output::WlOutput,
    ) {
        // Only the main surface says which display the window is on; a popup or
        // a drag icon rides along with it.
        if surface.id() == self.window.wl_surface().id() {
            self.output = Some(output.clone());
            self.refresh_output_dpi();
        }
    }

    fn surface_leave(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        surface: &wl_surface::WlSurface,
        output: &wl_output::WlOutput,
    ) {
        // Dragged off the display we had recorded: wait for the enter that
        // names the next one rather than keeping a figure for the old one.
        if surface.id() == self.window.wl_surface().id()
            && self.output.as_ref().is_some_and(|o| o.id() == output.id())
        {
            self.output = None;
            self.refresh_output_dpi();
        }
    }
}

impl OutputHandler for State {
    fn output_state(&mut self) -> &mut OutputState {
        &mut self.output_state
    }
    fn new_output(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {
        // The first output's figures are what a window with no `enter` yet goes
        // on, so a display appearing can be the one that answers the density.
        self.refresh_output_dpi();
    }

    fn update_output(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {
        // A mode change moves the density even though the window never moved.
        self.refresh_output_dpi();
    }

    fn output_destroyed(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        output: wl_output::WlOutput,
    ) {
        if self.output.as_ref().is_some_and(|o| o.id() == output.id()) {
            self.output = None;
        }
        self.refresh_output_dpi();
    }
}

impl WindowHandler for State {
    fn request_close(&mut self, _: &Connection, _: &QueueHandle<Self>, window: &XdgWindow) {
        if window.xdg_toplevel() == self.window.xdg_toplevel() {
            self.close_toplevel(Toplevel::Main);
            return;
        }
        if let Some(i) = self.popups.iter().position(|p| {
            matches!(&p.surface, ChildSurface::Dialog { window: dialog, .. }
                if dialog.xdg_toplevel() == window.xdg_toplevel())
        }) {
            self.close_toplevel(Toplevel::Dialog(i));
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
            let current = (self.surface_w.max(1), self.surface_h.max(1));
            // Whether our decorations came on or went off: a size suggested
            // for a fixed window was then worked out from hints that didn't
            // count them in (or did), so it is no size to take.
            let decor_moved = self.decor.configure(&configure);
            // The compositor's sizes are the window geometry's, which takes in
            // the frame of any decorations we draw.
            let extra = self.main_layout().extra;
            let suggested = match configure.new_size {
                (Some(w), Some(h)) if !(decor_moved && !self.resizable) => Some((
                    w.get().saturating_sub(extra.0).max(1),
                    h.get().saturating_sub(extra.1).max(1),
                )),
                _ => None,
            };
            let fullscreen = configure.is_fullscreen();
            let (w, h) = if fullscreen {
                // The compositor may also have fullscreened us on its own (a
                // keybinding): remember where to come back to either way.
                if self.windowed_size.is_none() {
                    self.windowed_size = Some(current);
                }
                suggested.unwrap_or(current)
            } else if let Some(windowed) = self.windowed_size.take() {
                // Out of fullscreen. Nothing obliges the compositor to suggest
                // the old size, so go back to it ourselves — exactly, for a
                // fixed window, whose size hints go back on too.
                let size = if self.resizable {
                    suggested.unwrap_or(windowed)
                } else {
                    windowed
                };
                self.mark_popups_dirty();
                size
            } else {
                suggested.unwrap_or(current)
            };
            self.fullscreen = fullscreen;
            self.surface_w = w;
            self.surface_h = h;
            // Sent with the frame drawn in reply to this configure.
            self.apply_size_hints();
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
        let ui = self.ui;
        if let Some(p) = self.popups.iter_mut().find(|p| {
            matches!(&p.surface, ChildSurface::Dialog { window: dialog, .. }
                if dialog.xdg_toplevel() == window.xdg_toplevel())
        }) {
            // surface_w / surface_h left untouched on purpose. The fixed size
            // hints measure the window geometry, though, so decorations of our
            // own coming on or going off move them.
            if p.decor.configure(&configure) {
                let extra = p.decor.layout((p.surface_w, p.surface_h), ui).extra;
                let size = Some((p.surface_w + extra.0, p.surface_h + extra.1));
                window.set_min_size(size);
                window.set_max_size(size);
            }
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
            self.seat = Some(seat.clone());
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
        surface: &wl_surface::WlSurface,
        _serial: u32,
        _raw: &[u32],
        _keysyms: &[Keysym],
    ) {
        self.keyboard_surface = Some(surface.clone());
    }
    fn leave(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_keyboard::WlKeyboard,
        surface: &wl_surface::WlSurface,
        _: u32,
    ) {
        if self.keyboard_surface.as_ref() == Some(surface) {
            self.keyboard_surface = None;
        }
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
        self.rehover_while_held();
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
        self.rehover_while_held();
    }
    fn update_modifiers(
        &mut self,
        conn: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_keyboard::WlKeyboard,
        _: u32,
        modifiers: WlModifiers,
        _layout: u32,
    ) {
        let modifiers = Modifiers {
            shift: modifiers.shift,
            control: modifiers.ctrl,
            alt: modifiers.alt,
            // On X11/Wayland AltGr is `ISO_Level3_Shift` (Mod5), reported
            // separately from Alt (Mod1) and not surfaced here, so it never
            // looks like `alt` and needs no special-casing.
            alt_graph: false,
            logo: modifiers.logo,
        };
        // Shown under the pointer at once, as a move would — and to a drag
        // over the window, whose drop they may turn from a move to a copy.
        if modifiers != self.modifiers {
            self.modifiers = modifiers;
            self.rehover();
            self.drag_moved(conn);
        }
    }
}

impl State {
    /// Dispatch a `PointerMove` where the pointer last was, though it hasn't
    /// moved, and show the cursor the widgets ask for there. Hover feedback
    /// that depends on more than the position — the modifiers, or a drag a
    /// key has just called off — so keeps up with it.
    fn rehover(&mut self) {
        let pos = self
            .cursor
            .or_else(|| self.popups.iter().find_map(|p| p.cursor));
        let Some(pos) = pos else {
            return;
        };
        let ctx = self.dispatch(Event::PointerMove { pos });
        self.apply_cursor(ctx.cursor_request, false);
        self.mark_popups_dirty();
    }

    /// After a key: a drag shows at once what the key did to it (Escape
    /// calling it off, say).
    fn rehover_while_held(&mut self) {
        if self.buttons_held > 0 {
            self.rehover();
        }
    }

    fn handle_key(&mut self, event: WlKeyEvent, pressed: bool) {
        let modifiers = self.modifiers;
        let mapped = map_keysym(event.keysym);
        if pressed {
            // A key pressed in a window with a dialog open over it, which would
            // reach that dialog through the widget tree: a compositor that
            // heeds neither the dialog's parent nor its modality lets that
            // window take the keyboard. The release still goes through:
            // nothing arms on a press it hasn't seen, and one that went down in
            // the dialog ends there.
            if self.keyboard_under_dialog() {
                return;
            }
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

    /// Whether a dialog is open over the surface holding the keyboard: the
    /// main window, or one of the popups. `false` for a surface we no longer
    /// know, such as a dialog just closed whose `leave` hasn't come yet.
    fn keyboard_under_dialog(&self) -> bool {
        let Some(surface) = &self.keyboard_surface else {
            return false;
        };
        let above = if surface == self.window.wl_surface() {
            0
        } else if let Some(idx) = self
            .popups
            .iter()
            .position(|p| p.surface.wl_surface() == surface)
        {
            idx + 1
        } else {
            return false;
        };
        self.popups[above..]
            .iter()
            .any(|p| p.surface.kind() == PopupKind::Dialog)
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
            // stacked popups? `surface_point` lands the position in the widget
            // tree's space from either: inside any decorations, and offset by
            // a popup's anchor, popup anchors being in root coords.
            let popup_idx = self
                .popups
                .iter()
                .position(|p| p.surface.wl_surface().id() == event.surface.id());
            let (x, y) = event.position;
            let pos = self.surface_point(&event.surface, x, y);

            // Over decorations we draw, the pointer is theirs: unless a
            // button that went down on the content is still held, which keeps
            // the pointer with the widget tree wherever it goes — and the
            // other way round.
            if !matches!(event.kind, PointerEventKind::Leave { .. })
                && let Some(t) = self.toplevel_of(&event.surface)
            {
                let hit = self.decor_hit(t, x, y);
                if self.chrome_grab || (self.buttons_held == 0 && hit != Hit::Content) {
                    self.chrome_pointer(t, event, hit);
                    continue;
                }
            }

            match event.kind {
                PointerEventKind::Enter { .. } | PointerEventKind::Motion { .. } => {
                    match popup_idx {
                        Some(i) => self.popups[i].cursor = Some(pos),
                        None => self.cursor = Some(pos),
                    }
                    let ctx = self.dispatch(Event::PointerMove { pos });
                    // Show the shape the widget under the pointer asked for. On
                    // enter we *must* set it — Wayland leaves the pointer image
                    // undefined there — so force it then; the enter serial SCTK
                    // just recorded is what `set_shape` rides on.
                    let entered = matches!(event.kind, PointerEventKind::Enter { .. });
                    self.apply_cursor(ctx.cursor_request, entered);
                    self.mark_popups_dirty();
                }
                PointerEventKind::Leave { .. } => {
                    match popup_idx {
                        Some(i) => self.popups[i].cursor = None,
                        None => self.cursor = None,
                    }
                    self.buttons_held = 0;
                    // A title-bar button held down pops back up, and does
                    // nothing: its release won't come to us.
                    self.chrome_grab = false;
                    if let Some(t) = self.toplevel_of(&event.surface)
                        && let Some((decor, _)) = self.toplevel_parts(t)
                        && decor.held.take().is_some()
                    {
                        self.redraw_toplevel(t);
                    }
                    self.dispatch(Event::PointerLeave);
                    // The pointer image is the next surface's responsibility now;
                    // forget what we'd shown so the next enter re-establishes it
                    // unconditionally (see `apply_cursor`'s `force`).
                    self.cursor_shape = Cursor::Default;
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
                    self.buttons_held += 1;
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
                    self.buttons_held = self.buttons_held.saturating_sub(1);
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
                    // end. The continuous (`absolute`) fallback is a distance
                    // in surface units, so it crosses into saudade pixels the
                    // same way a position does before being counted in lines.
                    let per_line = self.ui.max(0.01) * SCROLL_PIXELS_PER_LINE;
                    let lines = |axis: AxisScroll| {
                        if axis.discrete != 0 {
                            axis.discrete as f32 * WHEEL_LINES_PER_DETENT
                        } else {
                            axis.absolute as f32 / per_line
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

impl State {
    /// A pointer event on the decorations of toplevel `t`, which `hit` says
    /// what part of. (A leave never comes here: it goes the way of one on the
    /// content.)
    fn chrome_pointer(&mut self, t: Toplevel, event: &PointerEvent, hit: Hit) {
        match event.kind {
            PointerEventKind::Enter { .. } | PointerEventKind::Motion { .. } => {
                // Off the content and onto the decorations: the widget tree
                // has seen the last of the pointer for now.
                if !self.chrome_grab {
                    let was = match t {
                        Toplevel::Main => self.cursor.take(),
                        Toplevel::Dialog(i) => self.popups.get_mut(i).and_then(|p| p.cursor.take()),
                    };
                    if was.is_some() {
                        self.dispatch(Event::PointerLeave);
                        self.mark_popups_dirty();
                    }
                }
                // A title-bar button held down shows down only while the
                // pointer is on it.
                if let Some((decor, _)) = self.toplevel_parts(t)
                    && let Some(button) = decor.held
                {
                    let over = hit == Hit::Button(button);
                    if over != decor.over_held {
                        decor.over_held = over;
                        self.redraw_toplevel(t);
                    }
                }
                let cursor = match hit {
                    Hit::Resize(edges) if !self.chrome_grab => resize_cursor(edges),
                    _ => Cursor::Default,
                };
                let entered = matches!(event.kind, PointerEventKind::Enter { .. });
                self.apply_cursor(Some(cursor), entered);
            }
            PointerEventKind::Press { button, serial, .. } => {
                self.buttons_held += 1;
                // Another button, while the first is still held: nothing.
                if self.chrome_grab {
                    return;
                }
                self.chrome_grab = true;
                if let Some(b) = map_button(button) {
                    self.chrome_press(t, hit, b, serial, event.position);
                }
            }
            PointerEventKind::Release { button, .. } => {
                self.buttons_held = self.buttons_held.saturating_sub(1);
                if self.buttons_held == 0 {
                    self.chrome_grab = false;
                }
                if map_button(button) == Some(MouseButton::Left) {
                    self.chrome_release(t, hit);
                }
            }
            PointerEventKind::Leave { .. } | PointerEventKind::Axis { .. } => {}
        }
    }

    /// A button went down on the decorations of toplevel `t`, on `hit`, at
    /// surface-local `pos`. What Canoe does with the same press: the title bar
    /// moves the window, and maximizes or restores it on a double click; the
    /// edges resize it; a right click brings up the window menu; and the
    /// buttons go down, to act once they come up.
    fn chrome_press(
        &mut self,
        t: Toplevel,
        hit: Hit,
        button: MouseButton,
        serial: u32,
        pos: (f64, f64),
    ) {
        let Some(seat) = self.seat.clone() else {
            return;
        };
        let geometry = self.toplevel_layout(t).geometry;
        let Some((decor, window)) = self.toplevel_parts(t) else {
            return;
        };
        // Moves, resizes and the window menu are the compositor's: it takes
        // the press over, and its release never comes to us.
        let mut handed_over = true;
        match (hit, button) {
            (Hit::Resize(edges), MouseButton::Left) => {
                window.resize(&seat, serial, resize_edge(edges));
            }
            (Hit::Title, MouseButton::Left) => {
                let now = Instant::now();
                let double = decor
                    .title_click
                    .take()
                    .is_some_and(|at| now.duration_since(at) <= DOUBLE_CLICK);
                if double {
                    handed_over = false;
                    if decor.can_maximize() {
                        toggle_maximized(decor, window);
                    }
                } else {
                    decor.title_click = Some(now);
                    window.move_(&seat, serial);
                }
            }
            (Hit::Title | Hit::Button(_), MouseButton::Right)
                if decor.caps.contains(WindowManagerCapabilities::WINDOW_MENU) =>
            {
                // At the pointer, in window-geometry coordinates.
                let x = pos.0 as i32 - geometry.x;
                let y = pos.1 as i32 - geometry.y;
                window.show_window_menu(&seat, serial, (x, y));
            }
            (Hit::Button(b), MouseButton::Left) => {
                handed_over = false;
                decor.held = Some(b);
                decor.over_held = true;
                self.redraw_toplevel(t);
            }
            _ => handed_over = false,
        }
        if handed_over {
            self.buttons_held = 0;
            self.chrome_grab = false;
        }
    }

    /// The left button came up on the decorations of toplevel `t`, on `hit`.
    /// A title-bar button it went down on acts if it comes up there too: the
    /// close box on the second of a double click — Windows 3.1's control-menu
    /// box, as on Canoe — the other two at once.
    fn chrome_release(&mut self, t: Toplevel, hit: Hit) {
        let Some((decor, window)) = self.toplevel_parts(t) else {
            return;
        };
        let Some(button) = decor.held.take() else {
            return;
        };
        let fire = decor.over_held && hit == Hit::Button(button);
        decor.over_held = false;
        let close_click = decor.close_click.take();
        let mut close = false;
        if fire {
            match button {
                TitleButton::Close => {
                    let now = Instant::now();
                    close = close_click.is_some_and(|at| now.duration_since(at) <= DOUBLE_CLICK);
                    if !close {
                        decor.close_click = Some(now);
                    }
                }
                TitleButton::Minimize => window.set_minimized(),
                TitleButton::Maximize => toggle_maximized(decor, window),
            }
        }
        self.redraw_toplevel(t);
        if close {
            self.close_toplevel(t);
        }
    }
}

/// Ask the compositor to maximize `window`, or to restore it if `decor` says
/// it is maximized already.
fn toggle_maximized(decor: &Decor, window: &XdgWindow) {
    if decor.maximized {
        window.unset_maximized();
    } else {
        window.set_maximized();
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

        // Reject a drag that carries nothing a drop reads (see `mime_drop`)
        // outright so the compositor shows "can't drop here" over us.
        let readable = offer.with_mime_types(|types| mime_drop::first(types).is_some());
        if !readable {
            offer.accept_mime_type(offer.serial, None);
            return;
        }

        let pos = self.surface_point(surface, x, y);
        self.drag = Some(DragSession {
            surface: surface.clone(),
            pos,
            accepted: None,
        });
        // Offer the drag to the widget tree first; only accept it if a widget
        // opts in via `EventCtx::accept_drop`. Accepting unconditionally would
        // tell the source every window is a drop target, even ones with no drop
        // zone — so the source's cursor would wrongly read "copy" over us.
        let accepted = self.dispatch(Event::DragEnter { pos }).accepted_drop;
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
        let Some(surface) = self.drag.as_ref().map(|d| d.surface.clone()) else {
            return;
        };
        let pos = self.surface_point(&surface, x, y);
        if let Some(drag) = self.drag.as_mut() {
            drag.pos = pos;
        }
        self.drag_moved(conn);
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
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _data_device: &WlDataDevice,
    ) {
        let Some(drag) = self.drag.take() else { return };
        let offer = match self.data_device.as_ref() {
            Some(dd) => dd.data().drag_offer(),
            None => None,
        };
        let Some(offer) = offer else {
            // No payload offer (e.g. an internal drag): still report the drop so
            // a target can clear its highlight.
            self.dispatch(Event::Drop {
                pos: drag.pos,
                data: DragData::default(),
            });
            self.mark_popups_dirty();
            return;
        };
        // A drop the compositor left on "ask" isn't settled: the target is to
        // name what it did. The widget does its own thing with it once it
        // has the data; to the source, that's a copy.
        if offer.selected_action == DndAction::Ask {
            offer.set_actions(DndAction::Copy, DndAction::Copy);
        }
        // The drop is the protocol-guaranteed point at which the data is
        // readable.
        let reading = Reading::new(offer.with_mime_types(<[String]>::to_vec));
        self.read_drop(offer, reading, drag.pos);
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
        // A copy until the widget under the drag says what a drop there
        // would do, on enter.
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
        if !ours || mime != URI_LIST {
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
        let valid = !action.is_empty();
        // The badge is the feedback that actually shows on wlroots; the cursor
        // shape is the bonus for compositors that honor it mid-drag.
        self.update_drag_feedback(if valid {
            DragFeedback::Copy
        } else {
            DragFeedback::NoDrop
        });
        let shape = if action.contains(DndAction::Move) {
            Shape::Move
        } else if action.contains(DndAction::Ask) {
            Shape::ContextMenu
        } else if valid {
            Shape::Copy
        } else {
            Shape::NoDrop
        };
        self.set_cursor_shape(shape);
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

/// Surface-local units one saudade logical pixel spans on a display presented
/// at `os_scale`: the scale to draw at — `pin`, else the ladder in
/// [`ui_scale`](crate::ui_scale) at the density `reference_dpi` a logical pixel
/// is aimed at — over the compositor's own unit.
///
/// A Wayland surface coordinate is one of the platform's logical units, the
/// same nominally-96-dpi thing a Windows or X11 one is, so the default ladder
/// puts 4/3 of them to a saudade pixel on any display; only the snap to a
/// twelfth, or a pinned scale, makes the answer depend on `os_scale` at all.
/// The integer buffer scale does not come into it — that is how finely the
/// surface is rasterized, not how big anything on it is.
fn ui_for(os_scale: f32, pin: Option<f32>, reference_dpi: f32) -> f32 {
    // A factor no compositor could report is taken for 1 rather than divided
    // by: an infinite one would otherwise take the drawn scale away to nothing
    // and leave the window no unit to lay anything out in.
    let os = if os_scale.is_finite() && os_scale > 0.0 {
        os_scale
    } else {
        1.0
    };
    let drawn = pin
        .unwrap_or_else(|| crate::ui_scale::effective_for(os, reference_dpi))
        .max(0.01);
    (drawn / os).max(0.01)
}

/// A surface-local `(w, h)` re-expressed at `ui`, having been counted at `was`:
/// the same number of saudade pixels, in the unit the compositor now works in.
fn requantize(size: (u32, u32), was: f32, ui: f32) -> (u32, u32) {
    let logical = |n: u32| (n.max(1) as f32 / was.max(0.01)).round().max(1.0) as i32;
    (
        surface_units(logical(size.0), ui),
        surface_units(logical(size.1), ui),
    )
}

/// `logical` saudade pixels as a surface-local extent — a size, never nothing.
fn surface_units(logical: i32, ui: f32) -> u32 {
    (logical.max(1) as f32 * ui).round().max(1.0) as u32
}

/// `logical` saudade pixels as a surface-local offset, which unlike a size may
/// be negative: a popup anchored left of or above its parent.
fn surface_offset(logical: i32, ui: f32) -> i32 {
    (logical as f32 * ui).round() as i32
}

/// The app_id of a program that names none: the file name the executable was started
/// by, as winit takes it for `WM_CLASS` on X11, so the two agree — and usually
/// match the `.desktop` file the program is installed with. The title stands
/// in when `argv[0]` is missing or not UTF-8.
fn default_app_id(title: &str) -> String {
    std::env::args_os()
        .next()
        .as_ref()
        .and_then(|path| std::path::Path::new(path).file_name())
        .and_then(|name| name.to_str())
        .map(str::to_owned)
        .unwrap_or_else(|| title.to_owned())
}

/// Serialize file paths into an RFC 2483 `text/uri-list` payload — the inverse
/// of [`mime_drop::parse_uri_list`] — for handing to a drop target. Each path
/// becomes one CRLF-terminated `file://` URI with its bytes percent-encoded.
/// Relative paths are skipped: a `file:` URI must be absolute, and a widget
/// hands us absolute paths anyway.
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
/// through the parser's decoding unchanged.
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

/// The data device's name for `action`.
fn dnd_action(action: DropAction) -> DndAction {
    match action {
        DropAction::Copy => DndAction::Copy,
        DropAction::Move => DndAction::Move,
        DropAction::Ask => DndAction::Ask,
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

/// Map a saudade [`Cursor`] onto a `wp_cursor_shape` [`Shape`]. The two
/// vocabularies are both CSS-derived, so every saudade variant has a direct
/// equivalent (the one rename is [`Cursor::Hand`] → [`Shape::Pointer`]).
fn wayland_shape(cursor: Cursor) -> Shape {
    match cursor {
        Cursor::Default => Shape::Default,
        Cursor::Hand => Shape::Pointer,
        Cursor::Text => Shape::Text,
        Cursor::VerticalText => Shape::VerticalText,
        Cursor::Crosshair => Shape::Crosshair,
        Cursor::Move => Shape::Move,
        Cursor::Grab => Shape::Grab,
        Cursor::Grabbing => Shape::Grabbing,
        Cursor::NotAllowed => Shape::NotAllowed,
        Cursor::NoDrop => Shape::NoDrop,
        Cursor::Copy => Shape::Copy,
        Cursor::Alias => Shape::Alias,
        Cursor::ContextMenu => Shape::ContextMenu,
        Cursor::Help => Shape::Help,
        Cursor::Progress => Shape::Progress,
        Cursor::Wait => Shape::Wait,
        Cursor::Cell => Shape::Cell,
        Cursor::ZoomIn => Shape::ZoomIn,
        Cursor::ZoomOut => Shape::ZoomOut,
        Cursor::AllScroll => Shape::AllScroll,
        Cursor::ColResize => Shape::ColResize,
        Cursor::RowResize => Shape::RowResize,
        Cursor::EwResize => Shape::EwResize,
        Cursor::NsResize => Shape::NsResize,
        Cursor::NeswResize => Shape::NeswResize,
        Cursor::NwseResize => Shape::NwseResize,
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
        K::F1 => NamedKey::F1,
        K::F2 => NamedKey::F2,
        K::F3 => NamedKey::F3,
        K::F4 => NamedKey::F4,
        K::F5 => NamedKey::F5,
        K::F6 => NamedKey::F6,
        K::F7 => NamedKey::F7,
        K::F8 => NamedKey::F8,
        K::F9 => NamedKey::F9,
        K::F10 => NamedKey::F10,
        K::F11 => NamedKey::F11,
        K::F12 => NamedKey::F12,
        K::F13 => NamedKey::F13,
        K::F14 => NamedKey::F14,
        K::F15 => NamedKey::F15,
        K::F16 => NamedKey::F16,
        K::F17 => NamedKey::F17,
        K::F18 => NamedKey::F18,
        K::F19 => NamedKey::F19,
        K::F20 => NamedKey::F20,
        K::F21 => NamedKey::F21,
        K::F22 => NamedKey::F22,
        K::F23 => NamedKey::F23,
        K::F24 => NamedKey::F24,
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

/// Convert straight-alpha RGBA8 (what [`SvgImage::rasterize_rgba`] returns)
/// into the `wl_shm` `Argb8888` a Wayland buffer wants: `0xAARRGGBB` per pixel,
/// with the color channels premultiplied by alpha.
fn write_premultiplied(rgba: &[u8], dst: &mut [u32]) {
    for (px, out) in rgba.as_chunks::<4>().0.iter().zip(dst.iter_mut()) {
        let a = px[3] as u32;
        let scaled = |c: u8| (c as u32 * a + 127) / 255;
        *out = (a << 24) | (scaled(px[0]) << 16) | (scaled(px[1]) << 8) | scaled(px[2]);
    }
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
impl Dispatch<WpViewporter, ()> for State {
    fn event(
        _state: &mut Self,
        _proxy: &WpViewporter,
        _event: wp_viewporter::Event,
        _data: &(),
        _conn: &Connection,
        _qhandle: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<WpViewport, ()> for State {
    fn event(
        _state: &mut Self,
        _proxy: &WpViewport,
        _event: wp_viewport::Event,
        _data: &(),
        _conn: &Connection,
        _qhandle: &QueueHandle<Self>,
    ) {
    }
}

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
// value in 120ths (1.5 → 180). That is the factor the display is really
// presented at, so it is what the scale to draw at is derived from (see
// `ui_for`) and what `Painter::system_scale` reports; rendering still uses the
// integer buffer scale and the compositor resamples.
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
                // The scale the UI is drawn at hangs off this one, so take the
                // window's logical size across to the new unit.
                state.adopt_scale();
            }
        }
    }
}

// The compositor announces the icon sizes it would rather have as a run of
// `icon_size` events terminated by `done` (a compositor with no preference
// still sends the `done`). Because the artwork is vectors, we can give it
// exactly those — so collect the run and re-set the icon at the end of it,
// over the ladder guessed at startup.
impl Dispatch<XdgToplevelIconManagerV1, ()> for State {
    fn event(
        state: &mut Self,
        _proxy: &XdgToplevelIconManagerV1,
        event: xdg_toplevel_icon_manager_v1::Event,
        _data: &(),
        _conn: &Connection,
        _qhandle: &QueueHandle<Self>,
    ) {
        match event {
            xdg_toplevel_icon_manager_v1::Event::IconSize { size } if size > 0 => {
                state.icon_sizes.push(size as u32);
            }
            xdg_toplevel_icon_manager_v1::Event::Done => {
                // Taken rather than read, so a compositor that announces a
                // second time replaces its preferences instead of adding to
                // them.
                let sizes = std::mem::take(&mut state.icon_sizes);
                if !sizes.is_empty() {
                    state.set_toplevel_icon(&sizes);
                }
            }
            _ => {}
        }
    }
}

// xdg_toplevel_icon_v1 is sender-only — we create it, fill it, and hand it
// over. An empty Dispatch satisfies the queue handle.
impl Dispatch<XdgToplevelIconV1, ()> for State {
    fn event(
        _state: &mut Self,
        _proxy: &XdgToplevelIconV1,
        _event: <XdgToplevelIconV1 as Proxy>::Event,
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

#[cfg(test)]
mod tests {
    use super::{
        Decor, UI_EPSILON, paths_to_uri_list, percent_encode_path, requantize, resize_cursor,
        resize_edge, surface_offset, surface_units, ui_for,
    };
    use crate::chrome::{Edges, WindowFrame};
    use crate::event::Cursor;
    use crate::geometry::Rect;
    use crate::mime_drop::parse_uri_list;
    use std::path::PathBuf;
    use wayland_protocols::xdg::shell::client::xdg_toplevel::ResizeEdge;

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
        assert_eq!(parse_uri_list(&paths_to_uri_list(&paths)).paths, paths);
    }

    #[test]
    fn keeps_unreserved_and_slash_but_escapes_the_rest() {
        assert_eq!(percent_encode_path(b"/a-b_c.d~e/f"), "/a-b_c.d~e/f");
        assert_eq!(percent_encode_path(b"a b"), "a%20b");
        assert_eq!(percent_encode_path(b"#?%"), "%23%3F%25");
    }

    // ------------------------------------------------- scale

    /// Scales a compositor can present a surface at: the integer ones, and the
    /// fractional ones `wp_fractional_scale_v1` carries in 120ths.
    const SCALES: &[f32] = &[1.0, 1.25, 1.5, 1.75, 2.0, 2.5, 3.0];

    /// Surface units to a saudade pixel, at the default reference density.
    fn decorated(frame: WindowFrame) -> Decor {
        let mut decor = Decor::new(frame);
        decor.on = true;
        decor
    }

    #[test]
    fn without_decorations_the_content_is_the_whole_surface() {
        let layout = Decor::new(WindowFrame::Resizable).layout((300, 200), 4.0 / 3.0);
        assert_eq!(layout.surface, (300, 200));
        assert_eq!(layout.content, (0, 0));
        assert_eq!(layout.geometry, Rect::new(0, 0, 300, 200));
        assert_eq!(layout.input, None);
        assert_eq!(layout.extra, (0, 0));
    }

    /// The frame is the window geometry, the shadow lies outside it, and the
    /// content sits inside the frame's border and title bar: at a scale of 1,
    /// 4 px of border all round, a 19 px title bar and the line beneath it,
    /// with 20 px of shadow at the sides and bottom and 10 above.
    #[test]
    fn decorations_put_the_frame_in_the_geometry_and_the_shadow_outside() {
        let layout = decorated(WindowFrame::Resizable).layout((300, 200), 1.0);
        assert_eq!(layout.extra, (8, 28));
        assert_eq!(layout.geometry, Rect::new(20, 10, 308, 228));
        assert_eq!(layout.content, (24, 34));
        assert_eq!(layout.surface, (348, 258));
        // The edges can be grabbed a little way into the shadow.
        assert_eq!(layout.input, Some(Rect::new(14, 4, 320, 240)));
    }

    #[test]
    fn a_docked_window_has_no_shadow_and_no_edges_to_grab() {
        let mut decor = decorated(WindowFrame::Resizable);
        decor.docked = true;
        let layout = decor.layout((300, 200), 1.0);
        assert_eq!(layout.geometry, Rect::new(0, 0, 308, 228));
        assert_eq!(layout.surface, (308, 228));
        assert_eq!(layout.input, Some(layout.geometry));
    }

    #[test]
    fn a_fixed_window_has_a_thin_frame_and_nothing_to_grab_outside_it() {
        let layout = decorated(WindowFrame::Fixed).layout((300, 200), 1.0);
        assert_eq!(layout.extra, (2, 22));
        assert_eq!(layout.input, Some(layout.geometry));
    }

    /// Counted in surface units, the room kept for the frame is rounded up,
    /// so it never comes out short of the frame drawn in it.
    #[test]
    fn the_frame_is_never_short_of_room_at_a_fractional_unit() {
        let ui = ui(1.0);
        let layout = decorated(WindowFrame::Resizable).layout((300, 200), ui);
        let (left, top) = (
            layout.content.0 as i32 - layout.geometry.x,
            layout.content.1 as i32 - layout.geometry.y,
        );
        assert!(left as f32 >= 4.0 * ui);
        assert!(top as f32 >= 24.0 * ui);
    }

    #[test]
    fn resize_handles_map_to_their_edge_and_cursor() {
        let edges = |top, bottom, left, right| Edges {
            top,
            bottom,
            left,
            right,
        };
        assert_eq!(
            resize_edge(edges(true, false, true, false)),
            ResizeEdge::TopLeft
        );
        assert_eq!(
            resize_edge(edges(false, true, false, true)),
            ResizeEdge::BottomRight
        );
        assert_eq!(
            resize_edge(edges(false, false, false, true)),
            ResizeEdge::Right
        );
        assert_eq!(
            resize_edge(edges(true, false, false, false)),
            ResizeEdge::Top
        );
        assert_eq!(
            resize_cursor(edges(true, false, true, false)),
            Cursor::NwseResize
        );
        assert_eq!(
            resize_cursor(edges(false, true, false, true)),
            Cursor::NwseResize
        );
        assert_eq!(
            resize_cursor(edges(true, false, false, true)),
            Cursor::NeswResize
        );
        assert_eq!(
            resize_cursor(edges(false, true, false, false)),
            Cursor::NsResize
        );
        assert_eq!(
            resize_cursor(edges(false, false, true, false)),
            Cursor::EwResize
        );
    }

    fn ui(os_scale: f32) -> f32 {
        ui_for(os_scale, None, 72.0)
    }

    /// The point of the whole exercise: a surface unit is one of the platform's
    /// own, nominally 96-dpi ones, so a saudade pixel is 4/3 of it — whatever
    /// the compositor is presenting the display at, and whatever integer scale
    /// it has us rasterizing into.
    #[test]
    fn a_saudade_pixel_is_a_third_over_a_surface_unit_at_every_scale() {
        for &os in SCALES {
            assert!((ui(os) - 4.0 / 3.0).abs() < UI_EPSILON, "at {os}");
        }
    }

    /// Which is the same as saying the scale drawn at is the one the winit
    /// runtime derives for the same factor: `ui` times the factor itself.
    #[test]
    fn the_scale_drawn_at_is_the_ladders() {
        for &os in SCALES {
            let drawn = ui(os) * os;
            let wanted = crate::ui_scale::effective_for(os, 72.0);
            assert!((drawn - wanted).abs() < UI_EPSILON, "at {os}: {drawn}");
        }
    }

    /// A pinned scale is the scale drawn at, so what it takes in surface units
    /// moves with the display instead: pinning 2.0 draws at 2.0 on a 125%
    /// display as much as on a 100% one.
    #[test]
    fn a_pin_is_the_drawn_scale_whatever_the_display_says() {
        for &os in SCALES {
            let drawn = ui_for(os, Some(2.0), 72.0) * os;
            assert!((drawn - 2.0).abs() < UI_EPSILON, "at {os}: {drawn}");
        }
    }

    /// The reference density moves the ladder here too: at the platforms' own
    /// 96 dpi a saudade pixel *is* a surface unit.
    #[test]
    fn the_reference_density_moves_the_ladder() {
        for &os in SCALES {
            assert!((ui_for(os, None, 96.0) - 1.0).abs() < UI_EPSILON, "at {os}");
        }
    }

    /// Nothing a compositor can say leaves us with a scale we can't draw at.
    #[test]
    fn a_nonsense_scale_does_not_produce_a_nonsense_unit() {
        for os in [0.0, -1.0, f32::NAN, f32::INFINITY] {
            assert!(ui(os) > 0.0, "at {os}");
        }
        assert!(ui_for(1.0, Some(0.0), 72.0) > 0.0);
    }

    /// A window keeps the logical size it was asked for: the surface units it
    /// takes divide back out to the same number of saudade pixels.
    #[test]
    fn a_window_size_survives_the_round_trip() {
        let u = ui(1.25);
        for logical in [1, 13, 100, 320, 427, 1080] {
            let surface = surface_units(logical, u);
            let back = (surface as f32 / u).round() as i32;
            assert_eq!(back, logical, "{logical} -> {surface}");
        }
    }

    /// An extent is never nothing — a zero-sized buffer is a protocol error,
    /// and a zero-sized surface has nowhere to put the window.
    #[test]
    fn an_extent_is_always_at_least_one_unit() {
        for logical in [i32::MIN, -5, 0, 1] {
            assert!(surface_units(logical, ui(1.0)) >= 1, "{logical}");
        }
        assert!(surface_units(1, 0.001) >= 1);
    }

    /// Re-counting a size in a new unit keeps the logical size it stood for:
    /// a window holding 320 saudade pixels goes on holding 320 of them when
    /// the scale moves under it.
    #[test]
    fn a_size_re_counted_in_a_new_unit_is_the_same_logical_size() {
        let (was, now) = (ui_for(1.0, Some(2.0), 72.0), ui_for(1.25, Some(2.0), 72.0));
        let before = (surface_units(320, was), surface_units(200, was));
        let after = requantize(before, was, now);
        assert_eq!(after, (surface_units(320, now), surface_units(200, now)));
        // And re-counting into the unit it is already in changes nothing.
        assert_eq!(requantize(before, was, was), before);
    }

    /// An offset, unlike an extent, keeps its sign: a popup can sit left of or
    /// above the parent it is anchored to.
    #[test]
    fn an_offset_keeps_its_sign() {
        let u = ui(1.0);
        assert_eq!(surface_offset(0, u), 0);
        assert_eq!(surface_offset(3, u), 4);
        assert_eq!(surface_offset(-3, u), -4);
        assert_eq!(surface_offset(-300, u), -400);
    }
}
