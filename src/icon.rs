//! Application icons: one baked SVG in, whatever each platform actually wants
//! out.
//!
//! An app names its icon once, as an [`SvgImage`] on
//! [`WindowConfig::icon`](crate::WindowConfig::icon). Nothing downstream takes
//! an SVG — every platform wants loose RGBA pixels, and each wants them at
//! sizes of its own choosing — but because the source is resolution-independent
//! geometry, each size is rasterized from the vectors rather than resampled off
//! one bitmap, so none of them is the blurry one.
//!
//! Where the icon ends up, and how big:
//!
//! * **Windows** — two `WM_SETICON` images: `ICON_SMALL` for the title bar and
//!   `ICON_BIG` for the taskbar and Alt-Tab. Both are rasterized at the
//!   window's DPI, so a 150% display gets a 24-pixel small icon rather than a
//!   16-pixel one stretched to fit.
//! * **X11** — one `_NET_WM_ICON` property (winit sets it). The window manager
//!   scales that single image down to whatever its title bar, task list and
//!   Alt-Tab switcher want, so it goes on deliberately larger than any of them.
//! * **Wayland** — `xdg_toplevel_icon_v1`, which lives in
//!   [`wayland`](crate::wayland) instead of here: it needs the compositor
//!   connection and an `shm` pool, and the compositor names the sizes it wants.
//! * **macOS** — the dock tile, via `NSApplication.applicationIconImage`. Mac
//!   windows have no title-bar icon of their own (that slot belongs to the
//!   document a window represents, which saudade windows don't have), so the
//!   dock is the whole story there. It is also per-*application* rather than
//!   per-window, which is why setting it is a separate call below.
//!
//! Every path here fails soft: an icon that can't be built or handed over
//! leaves the platform's default in place rather than taking the window down
//! with it.

use winit::window::Window;

use crate::svg::SvgImage;

/// Edge, in logical pixels, of the icon Windows draws in the title bar —
/// `SM_CXSMICON` at 100%, scaled by the window's DPI before rasterizing.
#[cfg(windows)]
const WINDOWS_SMALL: f32 = 16.0;

/// Edge, in logical pixels, of the icon Windows uses for the taskbar button and
/// the Alt-Tab switcher (`SM_CXICON`), likewise scaled by the window's DPI.
#[cfg(windows)]
const WINDOWS_BIG: f32 = 32.0;

/// Edge of the single `_NET_WM_ICON` image X11 window managers get. They only
/// ever scale it down — to ~16 px in a title bar, ~48 px in an Alt-Tab
/// switcher, and double either on a HiDPI desktop — so it is sized to cover the
/// largest of those rather than the most common.
#[cfg(all(unix, not(target_os = "macos")))]
const X11_EDGE: u32 = 64;

/// Edge of the macOS dock image. The tile itself is ~128 pt, but Cmd-Tab and
/// Mission Control draw the same image far larger, and all of them at 2x on a
/// Retina display — 512 is the size an `.icns` would carry for the same reason.
#[cfg(target_os = "macos")]
const DOCK_EDGE: u32 = 512;

/// Give `win` the app's icon, for whatever its platform draws around a window:
/// the title bar and taskbar on Windows, `_NET_WM_ICON` on X11.
///
/// Worth calling for every top-level window the runtime opens, dialogs
/// included — a dialog with the default icon beside a main window with the
/// app's reads as a different program. A no-op on macOS, where windows have no
/// icon and [`set_app_icon`] does the equivalent job once for the process.
pub(crate) fn set_window_icon(win: &Window, image: &SvgImage) {
    #[cfg(windows)]
    {
        use winit::platform::windows::WindowExtWindows;

        // The title bar is drawn by Windows at the *OS* scale factor, which is
        // not necessarily the scale saudade paints its own content at (an app
        // or the environment can pin that outright).
        let dpi = win.scale_factor() as f32;
        let edge = |logical: f32| (logical * dpi).round().max(1.0) as u32;
        win.set_window_icon(rasterize(image, edge(WINDOWS_SMALL)));
        win.set_taskbar_icon(rasterize(image, edge(WINDOWS_BIG)));
    }

    #[cfg(all(unix, not(target_os = "macos")))]
    {
        win.set_window_icon(rasterize(image, X11_EDGE));
    }

    #[cfg(target_os = "macos")]
    {
        let _ = (win, image);
    }
}

/// Give the *process* the app's icon: the macOS dock tile. A no-op elsewhere,
/// where an icon belongs to a window and [`set_window_icon`] places it.
///
/// Only for as long as the program runs — this sets the tile of the live
/// process, and does not touch whatever icon a bundle or a Finder alias carries
/// for the app on disk.
pub(crate) fn set_app_icon(image: &SvgImage) {
    #[cfg(target_os = "macos")]
    {
        set_dock_icon(image);
    }

    #[cfg(not(target_os = "macos"))]
    {
        let _ = image;
    }
}

/// Rasterize `image` into a square winit [`Icon`](winit::window::Icon) of
/// `edge` pixels, or `None` if winit rejects the pixels (it validates the
/// buffer length against the dimensions, which by construction agree here).
#[cfg(not(target_os = "macos"))]
fn rasterize(image: &SvgImage, edge: u32) -> Option<winit::window::Icon> {
    winit::window::Icon::from_rgba(image.rasterize_rgba(edge), edge, edge).ok()
}

/// Point the dock tile at `image`.
///
/// The trip through Core Graphics is the only way in: `NSImage` wants a
/// `CGImage`, and the way to get one from loose pixels is to hand them to a
/// bitmap context and snapshot it. The context allocates its own backing store
/// (passing a null `data` pointer) rather than borrowing our `Vec`, because the
/// snapshot shares that store copy-on-write and so has to outlive this
/// function.
#[cfg(target_os = "macos")]
fn set_dock_icon(image: &SvgImage) {
    use objc2::{AnyThread, MainThreadMarker};
    use objc2_app_kit::{NSApplication, NSImage};
    use objc2_core_foundation::CGSize;
    use objc2_core_graphics::{
        CGBitmapContextCreate, CGBitmapContextCreateImage, CGBitmapContextGetBytesPerRow,
        CGBitmapContextGetData, CGColorSpace, CGImageAlphaInfo,
    };

    // `sharedApplication` is main-thread-only, and so is the dock tile.
    let Some(mtm) = MainThreadMarker::new() else {
        return;
    };

    // Core Graphics has no straight-alpha pixel format, so fold each channel
    // into its own alpha on the way over.
    let mut rgba = image.rasterize_rgba(DOCK_EDGE);
    for px in rgba.as_chunks::<4>().0.iter_mut() {
        let a = px[3] as u32;
        for c in &mut px[..3] {
            *c = ((*c as u32 * a + 127) / 255) as u8;
        }
    }

    let Some(space) = CGColorSpace::new_device_rgb() else {
        return;
    };
    let side = DOCK_EDGE as usize;
    // A null `data` and a zero `bytesPerRow` leave both to Core Graphics: it
    // owns the buffer (so the image outlives us) and picks its own aligned
    // stride (so we copy row by row rather than in one block).
    let context = unsafe {
        CGBitmapContextCreate(
            std::ptr::null_mut(),
            side,
            side,
            8,
            0,
            Some(&space),
            CGImageAlphaInfo::PremultipliedLast.0,
        )
    };
    let Some(context) = context else {
        return;
    };
    let stride = CGBitmapContextGetBytesPerRow(Some(&context));
    let dst = CGBitmapContextGetData(Some(&context)).cast::<u8>();
    if dst.is_null() || stride < side * 4 {
        return;
    }
    for row in 0..side {
        // SAFETY: `dst` is the context's own `side` × `stride` store, and both
        // it and `rgba` are indexed within the row counts they were made with.
        unsafe {
            std::ptr::copy_nonoverlapping(
                rgba.as_ptr().add(row * side * 4),
                dst.add(row * stride),
                side * 4,
            );
        }
    }

    let Some(cg) = CGBitmapContextCreateImage(Some(&context)) else {
        return;
    };
    // The size is in points: one point per pixel makes the tile a 1x image at
    // its full resolution, which every consumer then scales down.
    let side = DOCK_EDGE as f64;
    let ns = NSImage::initWithCGImage_size(NSImage::alloc(), &cg, CGSize::new(side, side));
    // SAFETY: the setter is generated `unsafe` only because AppKit doesn't
    // annotate whether it takes null. It does — that is how an app clears the
    // tile — and we pass an image regardless.
    unsafe { NSApplication::sharedApplication(mtm).setApplicationIconImage(Some(&ns)) };
}
