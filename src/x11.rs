//! X11: a dialog belongs to the window it opens over, and a display knows how
//! big it is — two things winit's X11 backend has no way to say.
//!
//! A native dialog is transient for the window it opens over
//! (`WM_TRANSIENT_FOR`), which is what has the window manager keep it above
//! that window, leave it out of the task list, and — on most — open it over
//! that window rather than wherever a new one would go. winit's
//! `with_parent_window` reparents the window into its parent instead, which
//! clips it to the parent's bounds, so [`make_dialog`] sets the property itself.
//!
//! The same goes for the frame: `with_enabled_buttons` is a no-op on X11, so a
//! dialog came with the minimize and maximize buttons of an application window,
//! and with resize handles on a window manager that goes by the Motif hints
//! rather than by the size `pin_dialog_size` pins. [`make_dialog`] narrows the
//! Motif hints to a frame that moves and closes, and nothing else. Weston's
//! window manager (WSLg's) heeds neither the pinned size nor the functions in
//! those hints, only the decorations: a dialog there loses the two buttons but
//! can still be resized by its edges.
//!
//! [`make_dialog`] also says the dialog is modal (`_NET_WM_STATE_MODAL`), which
//! has the window manager hand the keyboard to the dialog whenever the window
//! beneath it is clicked. Weston's doesn't read that either, and lets that
//! window take the keyboard: the runtime then drops the keys pressed there (see
//! `AppHandler::dispatch_key`), which would otherwise reach the dialog through
//! the widget tree.
//!
//! Not every window manager places a transient window over its owner, though,
//! and none has to, so [`centered_over`] works out where the dialog goes for it
//! to be created there — as `win32_popup` and `macos_popup` do.
//!
//! RandR has the physical size of each output, which winit reads a scale factor
//! off and then drops: [`display_dpi`] asks for it again.
//!
//! winit keeps its connection to the X server to itself, so all of this goes
//! over one of our own, opened the first time it is needed.

use std::sync::OnceLock;

use winit::dpi::{PhysicalPosition, PhysicalSize};
use winit::monitor::MonitorHandle;
use winit::platform::x11::MonitorHandleExtX11;
use winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};
use winit::window::Window;
use x11rb::errors::ReplyError;
use x11rb::protocol::randr::{ConnectionExt as _, Rotation};
use x11rb::protocol::xproto::{AtomEnum, PropMode};
use x11rb::rust_connection::RustConnection;
use x11rb::wrapper::ConnectionExt as _;

/// `_MOTIF_WM_HINTS`: which of the fields that follow are set, what the frame
/// lets the user do with the window, and what it draws to do it with.
const MWM_HINTS_FUNCTIONS: u32 = 1 << 0;
const MWM_HINTS_DECORATIONS: u32 = 1 << 1;
const MWM_FUNC_MOVE: u32 = 1 << 2;
const MWM_FUNC_CLOSE: u32 = 1 << 5;
const MWM_DECOR_BORDER: u32 = 1 << 1;
const MWM_DECOR_TITLE: u32 = 1 << 3;
const MWM_DECOR_MENU: u32 = 1 << 4;

x11rb::atom_manager! {
    Atoms: AtomsCookie {
        _MOTIF_WM_HINTS,
        _NET_WM_STATE,
        _NET_WM_STATE_MODAL,
    }
}

/// Make `win` a modal dialog of `owner`'s: transient for it, in a frame that
/// moves and closes it and nothing else. Called before `win` is first shown,
/// which is when the window manager reads all three.
pub(crate) fn make_dialog(win: &Window, owner: &Window) {
    let (Some(conn), Some(win), Some(owner)) = (connection(), xid(win), xid(owner)) else {
        return;
    };
    // A dialog the window manager doesn't know for one is still a dialog: a
    // failure here is left at that.
    let _ = set_dialog_hints(conn, win, owner);
}

fn set_dialog_hints(conn: &RustConnection, win: u32, owner: u32) -> Result<(), ReplyError> {
    conn.change_property32(
        PropMode::REPLACE,
        win,
        AtomEnum::WM_TRANSIENT_FOR,
        AtomEnum::WINDOW,
        &[owner],
    )?
    .ignore_error();
    let atoms = Atoms::new(conn)?.reply()?;
    // Before the window is mapped the state is the client's to write; winit
    // only ever asks for changes to it, once the window manager owns it.
    conn.change_property32(
        PropMode::REPLACE,
        win,
        atoms._NET_WM_STATE,
        AtomEnum::ATOM,
        &[atoms._NET_WM_STATE_MODAL],
    )?
    .ignore_error();
    conn.change_property32(
        PropMode::REPLACE,
        win,
        atoms._MOTIF_WM_HINTS,
        atoms._MOTIF_WM_HINTS,
        &[
            MWM_HINTS_FUNCTIONS | MWM_HINTS_DECORATIONS,
            MWM_FUNC_MOVE | MWM_FUNC_CLOSE,
            MWM_DECOR_BORDER | MWM_DECOR_TITLE | MWM_DECOR_MENU,
            0,
            0,
        ],
    )?
    .ignore_error();
    // winit shows the window over its own connection, so all three have to
    // have reached the server before this returns.
    conn.sync()
}

/// Where to create a dialog `size` big for it to come out in the middle of
/// `owner`, kept within the display `owner` is on, so its title bar can't land
/// off the screen.
///
/// The position is the frame's — where a window manager puts the top left
/// corner of the frame it wraps around a window created at it — and the frame
/// is taken to be the one around `owner`, which there is no knowing better
/// before the window is shown. Centering the dialog's frame on `owner`'s comes
/// to centering the two insides then.
pub(crate) fn centered_over(
    owner: &Window,
    size: PhysicalSize<u32>,
) -> Option<PhysicalPosition<i32>> {
    let outer = owner.outer_position().ok()?;
    let inside = owner.inner_size();
    let frame = owner.outer_size();
    let (w, h) = (size.width as i32, size.height as i32);
    let mut x = outer.x + (inside.width as i32 - w) / 2;
    let mut y = outer.y + (inside.height as i32 - h) / 2;
    if let Some(monitor) = owner.current_monitor() {
        // The top left last: a dialog bigger than the display keeps its title
        // bar on screen, to be dragged by.
        let (origin, screen) = (monitor.position(), monitor.size());
        let fw = w + (frame.width as i32 - inside.width as i32);
        let fh = h + (frame.height as i32 - inside.height as i32);
        x = x.min(origin.x + screen.width as i32 - fw).max(origin.x);
        y = y.min(origin.y + screen.height as i32 - fh).max(origin.y);
    }
    Some(PhysicalPosition::new(x, y))
}

/// How many device pixels `monitor` fits to the inch, going by the physical
/// size RandR reports for the output on it — `Painter::display_dpi`.
///
/// The size is the output's own, from its EDID, and so in the panel's
/// orientation; the pixels are the screen's, which a display turned on its side
/// has the other way round. `None` for an output that reports no size, as a
/// virtual one may not — the one WSLg gives an external display, say.
pub(crate) fn display_dpi(monitor: &MonitorHandle) -> Option<f32> {
    let conn = connection()?;
    // winit's monitor is a RandR CRTC, showing one output or several mirrored.
    let crtc = conn
        .randr_get_crtc_info(monitor.native_id(), x11rb::CURRENT_TIME)
        .ok()?
        .reply()
        .ok()?;
    let output = conn
        .randr_get_output_info(*crtc.outputs.first()?, x11rb::CURRENT_TIME)
        .ok()?
        .reply()
        .ok()?;
    let px = if crtc
        .rotation
        .intersects(Rotation::ROTATE90 | Rotation::ROTATE270)
    {
        crtc.height
    } else {
        crtc.width
    };
    let mm = output.mm_width;
    (px > 0 && mm > 0).then(|| px as f32 * 25.4 / mm as f32)
}

/// Our connection to the X server, or `None` if there is none to be had.
fn connection() -> Option<&'static RustConnection> {
    static CONNECTION: OnceLock<Option<RustConnection>> = OnceLock::new();
    CONNECTION
        .get_or_init(|| x11rb::connect(None).ok().map(|(conn, _screen)| conn))
        .as_ref()
}

/// The X window behind a winit window.
fn xid(win: &Window) -> Option<u32> {
    match win.window_handle().ok()?.as_raw() {
        RawWindowHandle::Xlib(handle) => u32::try_from(handle.window).ok(),
        RawWindowHandle::Xcb(handle) => Some(handle.window.get()),
        _ => None,
    }
}
