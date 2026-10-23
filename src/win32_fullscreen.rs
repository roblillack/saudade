//! Windows: a fullscreen window keeps filling the display it is on.
//!
//! To Windows, winit's borderless fullscreen window is an ordinary window that
//! happens to cover a display, so Win+Shift+Left / Right moves it to the next
//! display like any other. winit follows that much: when the window is being
//! moved onto another display, its `WM_WINDOWPOSCHANGING` handler makes it that
//! display's size. Three things still went wrong:
//!
//! * Between displays of different density, `WM_DPICHANGED` follows the move,
//!   suggesting the window's old size rescaled. saudade's own handling of that
//!   message applies the suggestion, and then keeps the window's logical size —
//!   both right for an ordinary window, both undoing winit's fit here. A
//!   fullscreen window instead fills the display it is crossing to
//!   ([`follow_dpi_change`]).
//! * winit refits only when the display changes, so a display that changes
//!   resolution under a fullscreen window leaves it the old size; [`refit`]
//!   catches that, and anything else that leaves it not covering its display.
//! * Leaving fullscreen restores the placement winit saved on the display the
//!   window went fullscreen on — sending it back across, where crossing the
//!   density boundary rescales the restored size once more. [`leave`] carries
//!   that placement over to the display the window is on now and steers winit's
//!   restore there, so the window comes back where the user last put it, at the
//!   size it had, maximized again if it was.

use windows_sys::Win32::Foundation::{HWND, POINT, RECT};
use windows_sys::Win32::Graphics::Gdi::{
    GetMonitorInfoW, HMONITOR, MONITOR_DEFAULTTONEAREST, MONITOR_DEFAULTTOPRIMARY, MONITORINFO,
    MonitorFromPoint, MonitorFromRect, MonitorFromWindow,
};
use windows_sys::Win32::UI::HiDpi::{GetDpiForMonitor, MDT_EFFECTIVE_DPI};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    GetWindowPlacement, GetWindowRect, SW_SHOWMAXIMIZED, SetWindowPlacement, WINDOWPLACEMENT,
};
use winit::dpi::PhysicalSize;
use winit::window::Window;

use crate::win32_dpi::{hwnd, redirect_moves, set_window_rect, take_suggested};

/// Where a window went fullscreen from: the display, and the placement it had
/// there — its restored rect and whether it was maximized.
pub(crate) struct Home {
    monitor: HMONITOR,
    placement: WINDOWPLACEMENT,
}

/// Note `win`'s display and placement, just before it goes fullscreen.
pub(crate) fn remember(win: &Window) -> Option<Home> {
    let hwnd = hwnd(win)?;
    let mut placement: WINDOWPLACEMENT = unsafe { std::mem::zeroed() };
    placement.length = size_of::<WINDOWPLACEMENT>() as u32;
    // SAFETY: plain calls on a live window owned by this thread, with the
    // WINDOWPLACEMENT's length filled in as asked.
    if unsafe { GetWindowPlacement(hwnd, &mut placement) } == 0 {
        return None;
    }
    let monitor = unsafe { MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST) };
    Some(Home { monitor, placement })
}

/// Make a fullscreen window cover its display exactly again, if something
/// left it otherwise.
pub(crate) fn refit(win: &Window) {
    let Some(hwnd) = hwnd(win) else {
        return;
    };
    // SAFETY: plain call on a live window owned by this thread.
    let monitor = unsafe { MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST) };
    if let Some(display) = display(monitor)
        && window_rect(hwnd).is_some_and(|r| !same_rect(r, display.rect))
    {
        set_window_rect(hwnd, display.rect);
    }
}

/// Answer the `WM_DPICHANGED` a fullscreen window is handling by filling the
/// display it is crossing to, rather than taking the rect Windows suggests.
/// Like [`follow_dpi_change`](crate::win32_dpi::follow_dpi_change), the window
/// moves before winit's handler checks which display it is on, so that check
/// agrees with the move. Returns the display's size, for winit to keep: left
/// alone, winit puts back whatever size the window had before the message.
pub(crate) fn follow_dpi_change(win: &Window) -> Option<PhysicalSize<u32>> {
    let hwnd = hwnd(win)?;
    let suggested = take_suggested(hwnd)?;
    // SAFETY: `suggested` is a valid RECT for the duration of the call.
    let monitor = unsafe { MonitorFromRect(&suggested, MONITOR_DEFAULTTONEAREST) };
    let display = display(monitor)?;
    set_window_rect(hwnd, display.rect);
    let r = display.rect;
    Some(PhysicalSize::new(
        (r.right - r.left).max(1) as u32,
        (r.bottom - r.top).max(1) as u32,
    ))
}

/// Take `win` out of fullscreen. On the display it went fullscreen on, that
/// is winit's own restore. On another one, the placement saved on the way in
/// is carried over to this display first, and winit's restore — which would
/// put the window back on the old one — lands it there instead.
pub(crate) fn leave(win: &Window, home: Option<Home>) {
    let carried = home.and_then(|home| {
        let hwnd = hwnd(win)?;
        // SAFETY: plain call on a live window owned by this thread.
        let monitor = unsafe { MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST) };
        if monitor == home.monitor {
            return None;
        }
        let now = display(monitor)?;
        let to_screen = workspace_offset();
        let normal = carry(
            offset(home.placement.rcNormalPosition, to_screen),
            display(home.monitor).as_ref(),
            &now,
        );
        let mut placement = home.placement;
        placement.rcNormalPosition = offset(normal, (-to_screen.0, -to_screen.1));
        let maximized = home.placement.showCmd == SW_SHOWMAXIMIZED as u32;
        Some((hwnd, placement, if maximized { now.work } else { normal }))
    });
    let Some((hwnd, placement, landing)) = carried else {
        win.set_fullscreen(None);
        return;
    };
    redirect_moves(hwnd, landing, || win.set_fullscreen(None));
    // The window is on the right display now; this settles the rest — the
    // restored rect a maximized window keeps for later, and maximizing it
    // there rather than on the display it came from.
    // SAFETY: plain call on a live window owned by this thread, with a
    // WINDOWPLACEMENT obtained from GetWindowPlacement.
    unsafe { SetWindowPlacement(hwnd, &placement) };
}

/// What fullscreen handling needs to know about a display.
struct Display {
    /// The whole display, in virtual-screen pixels.
    rect: RECT,
    /// The part not taken by the taskbar and other desktop toolbars.
    work: RECT,
    /// Effective DPI: 96 at 100%.
    dpi: u32,
}

fn display(monitor: HMONITOR) -> Option<Display> {
    let mut info: MONITORINFO = unsafe { std::mem::zeroed() };
    info.cbSize = size_of::<MONITORINFO>() as u32;
    // SAFETY: `info` is a MONITORINFO with its size filled in, as asked; a
    // monitor that has gone away (unplugged since) makes the call fail.
    if unsafe { GetMonitorInfoW(monitor, &mut info) } == 0 {
        return None;
    }
    let (mut dpi_x, mut dpi_y) = (96, 96);
    // SAFETY: plain call with valid out-pointers; on failure the 96 stays.
    unsafe { GetDpiForMonitor(monitor, MDT_EFFECTIVE_DPI, &mut dpi_x, &mut dpi_y) };
    Some(Display {
        rect: info.rcMonitor,
        work: info.rcWork,
        dpi: dpi_x.max(1),
    })
}

/// What to add to workspace coordinates — the ones a WINDOWPLACEMENT is in,
/// which put the primary display's work area at 0,0 — to get screen ones.
fn workspace_offset() -> (i32, i32) {
    // SAFETY: plain call; the primary monitor always exists.
    let primary = unsafe { MonitorFromPoint(POINT { x: 0, y: 0 }, MONITOR_DEFAULTTOPRIMARY) };
    display(primary).map_or((0, 0), |d| {
        (d.work.left - d.rect.left, d.work.top - d.rect.top)
    })
}

fn offset(r: RECT, (dx, dy): (i32, i32)) -> RECT {
    RECT {
        left: r.left + dx,
        top: r.top + dy,
        right: r.right + dx,
        bottom: r.bottom + dy,
    }
}

fn window_rect(hwnd: HWND) -> Option<RECT> {
    let mut rect: RECT = unsafe { std::mem::zeroed() };
    // SAFETY: plain call on a live window with a valid out-pointer.
    (unsafe { GetWindowRect(hwnd, &mut rect) } != 0).then_some(rect)
}

fn same_rect(a: RECT, b: RECT) -> bool {
    (a.left, a.top, a.right, a.bottom) == (b.left, b.top, b.right, b.bottom)
}

/// `windowed`, a rect on display `from`, moved to display `to`. Its size and
/// its offset from the corner of the work area are scaled from one density to
/// the other (as Windows rescales a window crossing displays) and it is then
/// kept inside the new work area. Without `from` — that display has been
/// unplugged since — it is centered on the new one at the size it had.
fn carry(windowed: RECT, from: Option<&Display>, to: &Display) -> RECT {
    let k = from.map_or(1.0, |f| to.dpi as f64 / f.dpi as f64);
    let scaled = |v: i32| (v as f64 * k).round() as i32;
    let work_w = (to.work.right - to.work.left).max(1);
    let work_h = (to.work.bottom - to.work.top).max(1);
    let w = scaled(windowed.right - windowed.left).clamp(1, work_w);
    let h = scaled(windowed.bottom - windowed.top).clamp(1, work_h);
    let (x, y) = match from {
        Some(f) => (
            to.work.left + scaled(windowed.left - f.work.left),
            to.work.top + scaled(windowed.top - f.work.top),
        ),
        None => (
            to.work.left + (work_w - w) / 2,
            to.work.top + (work_h - h) / 2,
        ),
    };
    let x = x.clamp(to.work.left, to.work.right - w);
    let y = y.clamp(to.work.top, to.work.bottom - h);
    RECT {
        left: x,
        top: y,
        right: x + w,
        bottom: y + h,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect(left: i32, top: i32, w: i32, h: i32) -> RECT {
        RECT {
            left,
            top,
            right: left + w,
            bottom: top + h,
        }
    }

    fn display(rect: RECT, taskbar: i32, dpi: u32) -> Display {
        Display {
            rect,
            work: RECT {
                bottom: rect.bottom - taskbar,
                ..rect
            },
            dpi,
        }
    }

    /// The setup this was written against: a 4K display at 150% with a
    /// 1920×1200 one at 100% to its left, lower down.
    fn four_k() -> Display {
        display(rect(0, 0, 3840, 2160), 72, 144)
    }

    fn left() -> Display {
        display(rect(-1920, 952, 1920, 1200), 60, 96)
    }

    fn assert_rect(actual: RECT, expected: RECT) {
        let tuple = |r: RECT| (r.left, r.top, r.right, r.bottom);
        assert_eq!(tuple(actual), tuple(expected));
    }

    #[test]
    fn carries_offset_and_size_across_densities() {
        // 300,240 from the corner at 150% is 200,160 at 100%; 900×600 is 600×400.
        let r = carry(rect(300, 240, 900, 600), Some(&four_k()), &left());
        assert_rect(r, rect(-1920 + 200, 952 + 160, 600, 400));
        // And back again.
        let back = carry(r, Some(&left()), &four_k());
        assert_rect(back, rect(300, 240, 900, 600));
    }

    #[test]
    fn keeps_the_window_inside_the_new_work_area() {
        // Near the far corner of the 4K display: pushed back inside.
        let r = carry(rect(3000, 1800, 750, 300), Some(&four_k()), &left());
        assert_rect(r, rect(-500, 952 + 1140 - 200, 500, 200));
        // Bigger than the whole work area: shrunk to it.
        let r = carry(rect(0, 0, 3840, 2088), Some(&four_k()), &left());
        assert_rect(r, rect(-1920, 952, 1920, 1140));
    }

    #[test]
    fn a_vanished_display_centers_the_window() {
        let r = carry(rect(5000, 5000, 600, 400), None, &left());
        assert_rect(r, rect(-1920 + 660, 952 + 370, 600, 400));
    }
}
