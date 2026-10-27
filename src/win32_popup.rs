//! Windows: a menu or dropdown belongs to the window it drops from.
//!
//! saudade hosts every popup in a window of its own, and to Windows a winit
//! window is an application window unless told otherwise: unowned, with a
//! taskbar button of its own, taking the activation when it is shown. Two
//! things followed from that:
//!
//! * Opening a menu took the activation from the window it belongs to, whose
//!   title bar went grey for as long as the menu was up.
//! * Sharing that window in a video call left the menu out. Capture tools work
//!   out which other windows are part of a shared one from their styles and
//!   ownership: WebRTC's — behind Chrome, Teams, Slack and most others — takes
//!   in the `WS_POPUP` windows whose root owner is the shared window, and
//!   Windows' own capture API the `WS_POPUP` windows stacked right above it. An
//!   unowned, non-popup window is another app's, which happens to lie on top.
//!
//! A native menu or combo box list is neither: it is a `WS_POPUP` window owned
//! by the window it drops from, and it never takes the activation. [`attributes`]
//! makes ours the same, and [`never_activate`] keeps a click on one from
//! activating it after all. The keyboard stays with the window beneath, and
//! reaches the widget tree from there, as it does on X11.
//!
//! That leaves the runtime to learn some other way what the popup losing the
//! keyboard used to tell it — that the user is done with the menu. Another app
//! or another window of this one taking over still shows, as the window beneath
//! losing the keyboard instead. A click on that window's own title bar or frame
//! doesn't move the keyboard anywhere, so [`dismiss_popups_on_frame_click`]
//! catches it there.
//!
//! A dialog is [`owned_by`] the window it belongs to as well, as a native one
//! is: it stays above that window, is hidden along with it, has no taskbar
//! button of its own, hands the activation back to it on closing, and shows up
//! when that window is shared. An owned window is a `WS_POPUP` one, though, and
//! Windows puts those at the top left of the primary display unless told where
//! to go — winit doesn't — so [`center_on_owner`] does.
//!
//! A native dialog is modal as well: its owner is disabled for as long as it is
//! open, so a click on that window doesn't activate it, and the keyboard and
//! the active title bar stay with the dialog. [`disable_owner`] does the same
//! for ours, and [`enable_owner`] undoes it before the dialog goes: Windows
//! picks the window to hand the activation to as the dialog is destroyed, and
//! passes over a disabled one.
//!
//! A click on a disabled window reaches none of the handlers that close the
//! popups open over the dialog, so the subclass of
//! [`dismiss_popups_on_frame_click`] does that too. Without any open, the
//! click is left to Windows, which beeps and flashes the dialog's title bar —
//! kept from winit, which would take it for the dialog losing the keyboard.

use std::cell::RefCell;

use windows_sys::Win32::Foundation::{FALSE, HWND, LPARAM, LRESULT, RECT, TRUE, WPARAM};
use windows_sys::Win32::Graphics::Gdi::{
    GetMonitorInfoW, MONITOR_DEFAULTTONEAREST, MONITORINFO, MonitorFromWindow,
};
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{EnableWindow, GetActiveWindow};
use windows_sys::Win32::UI::Shell::{DefSubclassProc, RemoveWindowSubclass, SetWindowSubclass};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    DefWindowProcW, GW_OWNER, GetWindow, GetWindowRect, HTERROR, MA_NOACTIVATE, SWP_NOACTIVATE,
    SWP_NOSIZE, SWP_NOZORDER, SendMessageW, SetWindowPos, WM_CLOSE, WM_LBUTTONDOWN, WM_MBUTTONDOWN,
    WM_MOUSEACTIVATE, WM_NCACTIVATE, WM_NCDESTROY, WM_NCLBUTTONDOWN, WM_NCMBUTTONDOWN,
    WM_NCRBUTTONDOWN, WM_NCXBUTTONDOWN, WM_RBUTTONDOWN, WM_SETCURSOR, WM_XBUTTONDOWN,
};
use winit::platform::windows::WindowAttributesExtWindows;
use winit::window::{Window, WindowAttributes};

use crate::win32_dpi::hwnd;

/// Tells these subclasses apart from any other on the same window.
const SUBCLASS_ID: usize = 1;

thread_local! {
    /// Every popup window [`never_activate`] has been called on, until it is
    /// destroyed.
    static POPUPS: RefCell<Vec<HWND>> = const { RefCell::new(Vec::new()) };

    /// Every dialog [`disable_owner`] has disabled the owner of, until
    /// [`enable_owner`] enables it again.
    static DIALOGS: RefCell<Vec<HWND>> = const { RefCell::new(Vec::new()) };
}

/// `attrs` for a popup hanging off `owner`: owned by it, and shown without
/// taking the activation from it.
pub(crate) fn attributes(attrs: WindowAttributes, owner: &Window) -> WindowAttributes {
    owned_by(attrs.with_active(false), owner)
}

/// `attrs` for a window owned by `owner`.
pub(crate) fn owned_by(attrs: WindowAttributes, owner: &Window) -> WindowAttributes {
    match hwnd(owner) {
        // winit's own `HWND` is the pointer as an integer.
        Some(owner) => attrs.with_owner_window(owner as isize),
        None => attrs,
    }
}

/// Move `win` to the middle of the window that owns it, kept within the work
/// area of the display that one is on, so its title bar can't land under the
/// taskbar or off the screen. A window without an owner is left alone.
pub(crate) fn center_on_owner(win: &Window) {
    let Some(hwnd) = hwnd(win) else {
        return;
    };
    // SAFETY: plain calls on live windows of this thread, with out-pointers to
    // locals; `cbSize` is set as `GetMonitorInfoW` requires.
    unsafe {
        let owner = GetWindow(hwnd, GW_OWNER);
        let mut outer: RECT = std::mem::zeroed();
        let mut own: RECT = std::mem::zeroed();
        if owner.is_null() || GetWindowRect(owner, &mut outer) == 0 {
            return;
        }
        if GetWindowRect(hwnd, &mut own) == 0 {
            return;
        }
        let (w, h) = (own.right - own.left, own.bottom - own.top);
        let mut x = (outer.left + outer.right - w) / 2;
        let mut y = (outer.top + outer.bottom - h) / 2;
        let mut info: MONITORINFO = std::mem::zeroed();
        info.cbSize = size_of::<MONITORINFO>() as u32;
        let monitor = MonitorFromWindow(owner, MONITOR_DEFAULTTONEAREST);
        if GetMonitorInfoW(monitor, &mut info) != 0 {
            // The top left last: a dialog bigger than the work area keeps its
            // title bar on screen, to be dragged by.
            let work = info.rcWork;
            x = x.min(work.right - w).max(work.left);
            y = y.min(work.bottom - h).max(work.top);
        }
        SetWindowPos(
            hwnd,
            std::ptr::null_mut(),
            x,
            y,
            0,
            0,
            SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE,
        );
    }
}

/// Disable the window that owns the dialog `win`, so it can't be activated
/// while the dialog is open. Called once the dialog is shown, holding the
/// activation. An owner that is disabled already is left to whatever disabled
/// it, as `DialogBox` leaves it.
pub(crate) fn disable_owner(win: &Window) {
    let Some(hwnd) = hwnd(win) else {
        return;
    };
    // SAFETY: plain calls on live windows of this thread.
    unsafe {
        let owner = GetWindow(hwnd, GW_OWNER);
        // Zero for a window that was enabled until now.
        if !owner.is_null() && EnableWindow(owner, FALSE) == 0 {
            DIALOGS.with_borrow_mut(|dialogs| dialogs.push(hwnd));
        }
    }
}

/// Enable the window [`disable_owner`] disabled for the dialog `win` again.
/// Called before the dialog is destroyed, so it is the window Windows hands the
/// activation back to.
pub(crate) fn enable_owner(win: &Window) {
    let Some(hwnd) = hwnd(win) else {
        return;
    };
    let disabled = DIALOGS.with_borrow_mut(|dialogs| {
        let i = dialogs.iter().position(|&dialog| dialog == hwnd);
        i.map(|i| dialogs.swap_remove(i)).is_some()
    });
    if disabled {
        // SAFETY: plain calls on live windows of this thread.
        unsafe { EnableWindow(GetWindow(hwnd, GW_OWNER), TRUE) };
    }
}

/// Keep a click on the popup `win` from activating it. Called once, right after
/// the window is created.
pub(crate) fn never_activate(win: &Window) {
    let Some(hwnd) = hwnd(win) else {
        return;
    };
    // SAFETY: `hwnd` is a live window owned by this thread, which is all
    // `SetWindowSubclass` asks; the subclass removes itself on `WM_NCDESTROY`.
    // If it fails, a click on the popup activates it, as it would without.
    if unsafe { SetWindowSubclass(hwnd, Some(popup_proc), SUBCLASS_ID, 0) } != 0 {
        POPUPS.with_borrow_mut(|popups| popups.push(hwnd));
    }
}

/// Have a click on `win`'s title bar or frame close the popups it owns. They are
/// sent `WM_CLOSE`, which the runtime answers for a popup as it does Escape.
/// While a dialog has `win` disabled, a click anywhere on it closes the popups
/// of the window holding the activation — that dialog — instead. Called once,
/// right after the window is created.
pub(crate) fn dismiss_popups_on_frame_click(win: &Window) {
    let Some(hwnd) = hwnd(win) else {
        return;
    };
    // SAFETY: as in `never_activate`.
    unsafe { SetWindowSubclass(hwnd, Some(owner_proc), SUBCLASS_ID, 0) };
}

unsafe extern "system" fn popup_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
    _id: usize,
    _data: usize,
) -> LRESULT {
    match msg {
        // The click itself still arrives, as it does at a window that is active.
        WM_MOUSEACTIVATE => MA_NOACTIVATE as LRESULT,
        WM_NCDESTROY => {
            POPUPS.with_borrow_mut(|popups| popups.retain(|&popup| popup != hwnd));
            // SAFETY: removing the subclass installed by `never_activate`, then
            // forwarding the message we were handed, unchanged.
            unsafe {
                RemoveWindowSubclass(hwnd, Some(popup_proc), SUBCLASS_ID);
                DefSubclassProc(hwnd, msg, wparam, lparam)
            }
        }
        // SAFETY: forwarding the message we were handed, unchanged.
        _ => unsafe { DefSubclassProc(hwnd, msg, wparam, lparam) },
    }
}

unsafe extern "system" fn owner_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
    _id: usize,
    _data: usize,
) -> LRESULT {
    match msg {
        // Before the press is handled: on the caption that is the start of a
        // window move, which the popup is not to come along on.
        WM_NCLBUTTONDOWN | WM_NCRBUTTONDOWN | WM_NCMBUTTONDOWN | WM_NCXBUTTONDOWN => {
            close_popups_owned_by(hwnd);
            // SAFETY: forwarding the message we were handed, unchanged.
            unsafe { DefSubclassProc(hwnd, msg, wparam, lparam) }
        }
        // A press on the window while a dialog has it disabled, which it gets
        // no message of its own for: only this one, with `HTERROR` for where it
        // landed. With popups open over the dialog, it is a press outside them,
        // so they close, and it goes no further: `DefWindowProc` would hand the
        // activation to the topmost enabled window this one owns, directly or
        // not — one of those very popups. Without, `DefWindowProc` beeps and
        // flashes the dialog's title bar, as for a native dialog.
        // SAFETY: a plain call, about the calling thread.
        WM_SETCURSOR
            if is_press_on_disabled(lparam)
                && close_popups_owned_by(unsafe { GetActiveWindow() }) =>
        {
            TRUE as LRESULT
        }
        // That flash toggles the dialog's title bar between active and
        // inactive with `WM_NCACTIVATE`, the message a change of activation
        // starts with, for a second or so. winit reports each toggle as the
        // dialog losing or taking the keyboard, and a popup opened over it
        // meanwhile closes on the next loss. So winit isn't told of the title
        // bar going inactive while the dialog has its owner disabled. When the
        // dialog really loses the activation, it loses the keyboard as well,
        // and winit learns of that from the `WM_KILLFOCUS` that follows, which
        // a flash doesn't send.
        WM_NCACTIVATE
            if wparam == FALSE as WPARAM
                && DIALOGS.with_borrow(|dialogs| dialogs.contains(&hwnd)) =>
        {
            // SAFETY: handing the message we were handed, unchanged, to the
            // default procedure, which draws the title bar, past winit's.
            unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
        }
        WM_NCDESTROY => {
            // SAFETY: removing the subclass installed by
            // `dismiss_popups_on_frame_click`, then forwarding the message we
            // were handed, unchanged.
            unsafe {
                RemoveWindowSubclass(hwnd, Some(owner_proc), SUBCLASS_ID);
                DefSubclassProc(hwnd, msg, wparam, lparam)
            }
        }
        // SAFETY: forwarding the message we were handed, unchanged.
        _ => unsafe { DefSubclassProc(hwnd, msg, wparam, lparam) },
    }
}

/// Send `WM_CLOSE` to the popups `owner` owns. Whether there were any.
fn close_popups_owned_by(owner: HWND) -> bool {
    // A copy, as closing a popup can destroy it, which updates the list.
    let popups = POPUPS.with_borrow(|popups| popups.clone());
    let mut closed = false;
    for popup in popups {
        // SAFETY: plain calls on windows of this thread that are still alive —
        // a destroyed one has taken itself off the list.
        unsafe {
            if GetWindow(popup, GW_OWNER) == owner {
                SendMessageW(popup, WM_CLOSE, 0, 0);
                closed = true;
            }
        }
    }
    closed
}

/// Whether the `WM_SETCURSOR` with `lparam` is for a mouse button going down on
/// a disabled window: the low word is the hit-test result, the high word the
/// mouse message that prompted it.
fn is_press_on_disabled(lparam: LPARAM) -> bool {
    let hit = lparam as u16 as i16 as i32;
    let msg = (lparam >> 16) as u16 as u32;
    hit == HTERROR
        && matches!(
            msg,
            WM_LBUTTONDOWN | WM_RBUTTONDOWN | WM_MBUTTONDOWN | WM_XBUTTONDOWN
        )
}
