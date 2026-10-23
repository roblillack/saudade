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
//! (rust-windowing/winit#4600). Until one does, the main window and every
//! dialog are subclassed so the suggested rect can be read off the message, and
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
    SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SWP_NOZORDER, SetWindowPos, WINDOWPOS, WM_DPICHANGED,
    WM_NCDESTROY, WM_WINDOWPOSCHANGING,
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

    /// While set, every move or resize of that window lands at that rect
    /// instead — see [`redirect_moves`].
    static REDIRECT: Cell<Option<(HWND, RECT)>> = const { Cell::new(None) };
}

/// Run `f`, with every move and resize it makes `hwnd` go through landing at
/// `rect` instead. For winit calls that place the window somewhere saudade
/// has no say over — leaving fullscreen restores a placement winit saved — the
/// rect is rewritten in `WM_WINDOWPOSCHANGING`, before winit or Windows act on
/// it, so the window never shows up anywhere else on the way.
pub(crate) fn redirect_moves(hwnd: HWND, rect: RECT, f: impl FnOnce()) {
    let outer = REDIRECT.replace(Some((hwnd, rect)));
    f();
    REDIRECT.set(outer);
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
    if let Some(rect) = take_suggested(hwnd) {
        set_window_rect(hwnd, rect);
    }
}

/// The rect suggested by the `WM_DPICHANGED` that `hwnd` is handling right
/// now, if it is handling one. Taken: a second call answers `None`.
pub(crate) fn take_suggested(hwnd: HWND) -> Option<RECT> {
    match SUGGESTED.take() {
        Some((target, rect)) if target == hwnd => Some(rect),
        _ => None,
    }
}

/// Place `hwnd` at `rect`, outer bounds in physical pixels.
pub(crate) fn set_window_rect(hwnd: HWND, rect: RECT) {
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

pub(crate) fn hwnd(win: &Window) -> Option<HWND> {
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
        WM_WINDOWPOSCHANGING => {
            if let Some((target, rect)) = REDIRECT.get()
                && target == hwnd
            {
                // SAFETY: for this message `lparam` points at the WINDOWPOS
                // about to be applied, which the receiver may change.
                let pos = unsafe { &mut *(lparam as *mut WINDOWPOS) };
                pos.x = rect.left;
                pos.y = rect.top;
                pos.cx = rect.right - rect.left;
                pos.cy = rect.bottom - rect.top;
                pos.flags &= !(SWP_NOMOVE | SWP_NOSIZE);
            }
            // SAFETY: forwarding the message we were handed (redirected).
            unsafe { DefSubclassProc(hwnd, msg, wparam, lparam) }
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
