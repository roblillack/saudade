//! Windows: put a window where `WM_DPICHANGED` suggests, ahead of winit 0.30.
//!
//! When a window is moved onto a display of another density, Windows sends it
//! `WM_DPICHANGED` along with a suggested rect for it at the new density — on
//! the new display by construction. winit 0.30's handler works out a rect of
//! its own instead, then checks that it lands on "the display with the new
//! DPI", which it asks `MonitorFromWindow` for. But the message arrives
//! *before* the move it announces has been committed, so that is still the
//! display the window is leaving: a rect that has correctly landed on the new
//! display gets nudged back onto the old one, Windows answers with a
//! `WM_DPICHANGED` back to the old density, and the window flaps between the
//! two for as long as the pointer stays near the boundary — growing by the
//! ratio between the displays whenever Windows' move loop rescales the rect it
//! is tracking, until the surface behind it is too big to allocate.
//!
//! winit fixed this on `master` by applying the suggested rect as it is
//! (rust-windowing/winit#4341), but no 0.30 release carries that yet
//! (rust-windowing/winit#4600). Until one does, the main window is subclassed
//! so the suggested rect can be read off the message, and
//! [`follow_dpi_change`] applies it from the `ScaleFactorChanged` event winit
//! sends from inside its handler. That is before winit's check, which then asks
//! about a window that is already on the right display — and agrees with it,
//! or nudges the rect toward it rather than away. Against a winit that applies
//! the suggested rect itself, this moves the window to where winit is about to
//! put it anyway.

use std::cell::Cell;

use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows_sys::Win32::UI::Shell::{DefSubclassProc, RemoveWindowSubclass, SetWindowSubclass};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    SWP_NOACTIVATE, SWP_NOZORDER, SetWindowPos, WM_DPICHANGED, WM_NCDESTROY,
};
use winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};
use winit::window::Window;

/// Tells this subclass apart from any other on the same window.
const SUBCLASS_ID: usize = 1;

thread_local! {
    /// The window handling a `WM_DPICHANGED` right now, and the rect that
    /// message suggests for it. Taken by [`follow_dpi_change`], so a scale
    /// change that winit queues and replays later — from outside the message
    /// it came with — is left alone.
    static SUGGESTED: Cell<Option<(HWND, RECT)>> = const { Cell::new(None) };
}

/// Subclass `win` so [`follow_dpi_change`] can see the rect its
/// `WM_DPICHANGED` suggests. Called once, right after the window is created.
pub(crate) fn watch_dpi_changes(win: &Window) {
    let Some(hwnd) = hwnd(win) else {
        return;
    };
    // SAFETY: `hwnd` is a live window owned by this thread, which is all
    // `SetWindowSubclass` asks; the subclass removes itself on `WM_NCDESTROY`.
    // If it fails, the window behaves as it would without it.
    unsafe { SetWindowSubclass(hwnd, Some(subclass_proc), SUBCLASS_ID, 0) };
}

/// Move `win` to the rect suggested by the `WM_DPICHANGED` it is handling. A
/// no-op outside one.
pub(crate) fn follow_dpi_change(win: &Window) {
    let Some(hwnd) = hwnd(win) else {
        return;
    };
    let Some((target, rect)) = SUGGESTED.take() else {
        return;
    };
    if target != hwnd {
        return;
    }
    // SAFETY: plain call on a live window owned by this thread.
    unsafe {
        SetWindowPos(
            hwnd,
            std::ptr::null_mut(),
            rect.left,
            rect.top,
            rect.right - rect.left,
            rect.bottom - rect.top,
            SWP_NOZORDER | SWP_NOACTIVATE,
        )
    };
}

fn hwnd(win: &Window) -> Option<HWND> {
    match win.window_handle().ok()?.as_raw() {
        RawWindowHandle::Win32(handle) => Some(handle.hwnd.get() as HWND),
        _ => None,
    }
}

unsafe extern "system" fn subclass_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
    _id: usize,
    _data: usize,
) -> LRESULT {
    match msg {
        WM_DPICHANGED => {
            // SAFETY: for this message `lparam` points at the suggested RECT.
            let rect = unsafe { *(lparam as *const RECT) };
            // Kept for a nested message's sake, so the one around it finds its
            // own rect again once that has been handled.
            let outer = SUGGESTED.replace(Some((hwnd, rect)));
            // SAFETY: forwarding the message we were handed, unchanged.
            let result = unsafe { DefSubclassProc(hwnd, msg, wparam, lparam) };
            SUGGESTED.set(outer);
            result
        }
        WM_NCDESTROY => {
            // SAFETY: removing the subclass installed by `watch_dpi_changes`,
            // then forwarding the message we were handed, unchanged.
            unsafe {
                RemoveWindowSubclass(hwnd, Some(subclass_proc), SUBCLASS_ID);
                DefSubclassProc(hwnd, msg, wparam, lparam)
            }
        }
        // SAFETY: forwarding the message we were handed, unchanged.
        _ => unsafe { DefSubclassProc(hwnd, msg, wparam, lparam) },
    }
}
