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

use std::cell::RefCell;

use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows_sys::Win32::UI::Shell::{DefSubclassProc, RemoveWindowSubclass, SetWindowSubclass};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    GW_OWNER, GetWindow, MA_NOACTIVATE, SendMessageW, WM_CLOSE, WM_MOUSEACTIVATE, WM_NCDESTROY,
    WM_NCLBUTTONDOWN, WM_NCMBUTTONDOWN, WM_NCRBUTTONDOWN, WM_NCXBUTTONDOWN,
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
}

/// `attrs` for a popup hanging off `owner`: owned by it, and shown without
/// taking the activation from it.
pub(crate) fn attributes(attrs: WindowAttributes, owner: &Window) -> WindowAttributes {
    let attrs = attrs.with_active(false);
    match hwnd(owner) {
        // winit's own `HWND` is the pointer as an integer.
        Some(owner) => attrs.with_owner_window(owner as isize),
        None => attrs,
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
/// Called once, right after the window is created.
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
            // A copy, as closing a popup can destroy it, which updates the list.
            let popups = POPUPS.with_borrow(|popups| popups.clone());
            for popup in popups {
                // SAFETY: plain calls on windows of this thread that are still
                // alive — a destroyed one has taken itself off the list.
                unsafe {
                    if GetWindow(popup, GW_OWNER) == hwnd {
                        SendMessageW(popup, WM_CLOSE, 0, 0);
                    }
                }
            }
            // SAFETY: forwarding the message we were handed, unchanged.
            unsafe { DefSubclassProc(hwnd, msg, wparam, lparam) }
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
