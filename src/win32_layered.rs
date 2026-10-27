//! Windows: a popup window with real per-pixel alpha.
//!
//! A menu panel never fills its window: the two corners inside the bend of its
//! L-shaped drop shadow are not part of it. softbuffer's Win32 backend can't
//! show the pixels it leaves out as see-through, so those used to be filled in
//! by redrawing the app underneath — which has nothing to redraw where a menu
//! hangs off its window, and left a square of the theme's background out over
//! the desktop.
//!
//! A *layered* window can: it is shown from a premultiplied 32-bit bitmap
//! handed over whole with `UpdateLayeredWindow`, which is how Windows' own menus
//! carried their shadows before the compositor drew them. [`Surface`] is one of
//! those. The painter draws straight into a DIB section, and presenting
//! premultiplies it and gives it to the window. A fully transparent pixel is no
//! part of the window for the mouse either: a click there goes to whatever is
//! underneath, as it does on macOS.
//!
//! winit owns the window's extended style, and writes it back whole whenever a
//! window flag of its own changes — showing the window is one — which would
//! take `WS_EX_LAYERED` off again. A subclass keeps it on: Windows asks the
//! window before a style change takes effect (`WM_STYLECHANGING`), and the
//! answer puts the bit back.

use std::ptr;
use std::rc::Rc;

use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, SIZE, WPARAM};
use windows_sys::Win32::Graphics::Gdi::{
    AC_SRC_ALPHA, AC_SRC_OVER, BI_RGB, BITMAPINFO, BITMAPINFOHEADER, BLENDFUNCTION,
    CreateCompatibleDC, CreateDIBSection, DIB_RGB_COLORS, DeleteDC, DeleteObject, GdiFlush,
    HBITMAP, HDC, HGDIOBJ, SelectObject,
};
use windows_sys::Win32::UI::Shell::{DefSubclassProc, RemoveWindowSubclass, SetWindowSubclass};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    GWL_EXSTYLE, GetWindowLongW, STYLESTRUCT, SetWindowLongW, ULW_ALPHA, UpdateLayeredWindow,
    WM_NCDESTROY, WM_STYLECHANGING, WS_EX_LAYERED,
};
use winit::dpi::PhysicalSize;
use winit::window::Window;

use crate::win32_dpi::hwnd;

/// Tells this subclass apart from any other on the same window.
const SUBCLASS_ID: usize = 1;

/// A layered window's contents: a top-down 32-bit DIB section selected into a
/// memory DC, which is what `UpdateLayeredWindow` reads from.
pub(crate) struct Surface {
    /// Kept alive for as long as its contents are ours to update.
    _win: Rc<Window>,
    hwnd: HWND,
    dc: HDC,
    /// The DIB section, or null before the first one is made.
    bitmap: HBITMAP,
    /// What `dc` held before the first DIB section went in, to be put back
    /// before the DC is deleted.
    previous: HGDIOBJ,
    /// The DIB section's pixels: `size.width × size.height` of them, rows
    /// top-down and tightly packed, each `0xAARRGGBB` — BGRA in memory, which
    /// is what both the painter and `UpdateLayeredWindow` want.
    bits: *mut u32,
    size: PhysicalSize<u32>,
}

impl Surface {
    /// Make `win` a layered window of `size` device pixels. `None` if Windows
    /// won't, which leaves the window as it was for the caller to fall back on.
    pub(crate) fn new(win: Rc<Window>, size: PhysicalSize<u32>) -> Option<Self> {
        let hwnd = hwnd(&win)?;
        // SAFETY: a memory DC compatible with the screen, checked for null.
        let dc = unsafe { CreateCompatibleDC(ptr::null_mut()) };
        if dc.is_null() {
            return None;
        }
        let mut surface = Self {
            _win: win,
            hwnd,
            dc,
            bitmap: ptr::null_mut(),
            previous: ptr::null_mut(),
            bits: ptr::null_mut(),
            size: PhysicalSize::new(0, 0),
        };
        if !surface.allocate(size) {
            return None;
        }
        // SAFETY: `hwnd` is a live window owned by this thread, which is all
        // `SetWindowSubclass` asks; the subclass removes itself on
        // `WM_NCDESTROY`, or when the surface goes first. The style is read,
        // and written back with one bit more.
        unsafe {
            if SetWindowSubclass(hwnd, Some(keep_layered), SUBCLASS_ID, 0) == 0 {
                // Without the subclass winit would drop the style again the
                // next time it touches it, and the window would vanish.
                return None;
            }
            let style = GetWindowLongW(hwnd, GWL_EXSTYLE) as u32;
            SetWindowLongW(hwnd, GWL_EXSTYLE, (style | WS_EX_LAYERED) as i32);
        }
        Some(surface)
    }

    pub(crate) fn resize(&mut self, size: PhysicalSize<u32>) {
        let size = PhysicalSize::new(size.width.max(1), size.height.max(1));
        if size != self.size {
            // Should Windows refuse, the old bitmap stays, at its old size.
            self.allocate(size);
        }
    }

    pub(crate) fn frame(&mut self) -> Option<Frame<'_>> {
        // SAFETY: no arguments. Flushes any GDI work still queued against the
        // DIB section before the painter writes to its memory directly, as
        // Windows asks of a DIB section's bits.
        unsafe { GdiFlush() };
        Some(Frame { surface: self })
    }

    /// Swap in a DIB section of `size`, which must not be zero. False, with
    /// nothing changed, if Windows won't make one.
    fn allocate(&mut self, size: PhysicalSize<u32>) -> bool {
        let size = PhysicalSize::new(size.width.max(1), size.height.max(1));
        // SAFETY: a zeroed `BITMAPINFO` is a valid one to fill in.
        let mut info: BITMAPINFO = unsafe { std::mem::zeroed() };
        info.bmiHeader = BITMAPINFOHEADER {
            biSize: size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: size.width as i32,
            // Negative: rows run top-down, as the painter lays them out.
            biHeight: -(size.height as i32),
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB,
            ..info.bmiHeader
        };
        let mut bits = ptr::null_mut();
        // SAFETY: `info` describes a 32-bit top-down bitmap and `bits` is an
        // out-pointer to a local; the result is checked before use.
        let bitmap = unsafe {
            CreateDIBSection(
                self.dc,
                &info,
                DIB_RGB_COLORS,
                &mut bits,
                ptr::null_mut(),
                0,
            )
        };
        if bitmap.is_null() || bits.is_null() {
            return false;
        }
        // SAFETY: `dc` and `bitmap` are both ours. The bitmap it held before is
        // deleted only once it is no longer selected into the DC; the one the
        // DC came with is kept, to be put back when the DC goes.
        unsafe {
            let previous = SelectObject(self.dc, bitmap);
            if self.bitmap.is_null() {
                self.previous = previous;
            } else {
                DeleteObject(self.bitmap);
            }
        }
        self.bitmap = bitmap;
        self.bits = bits.cast();
        self.size = size;
        true
    }

    fn pixels(&mut self) -> &mut [u32] {
        let len = self.size.width as usize * self.size.height as usize;
        // SAFETY: `bits` is the DIB section's memory, `len` 32-bit pixels of it,
        // which lives until the section is deleted — not before `self` is
        // borrowed mutably again.
        unsafe { std::slice::from_raw_parts_mut(self.bits, len) }
    }

    fn present(&mut self) {
        crate::painter::premultiply(self.pixels());
        let size = SIZE {
            cx: self.size.width as i32,
            cy: self.size.height as i32,
        };
        let origin = POINT { x: 0, y: 0 };
        let blend = BLENDFUNCTION {
            BlendOp: AC_SRC_OVER as u8,
            BlendFlags: 0,
            SourceConstantAlpha: 255,
            // Per-pixel alpha, premultiplied — which the pass above made it.
            AlphaFormat: AC_SRC_ALPHA as u8,
        };
        // SAFETY: a live window of this thread, the memory DC holding the DIB
        // section, and pointers to locals. No destination DC (the screen's
        // default palette) and no destination point (the window stays where
        // it is). The size is the window's own: the bitmap follows the window
        // through `resize`.
        let shown = unsafe {
            UpdateLayeredWindow(
                self.hwnd,
                ptr::null_mut(),
                ptr::null(),
                &size,
                self.dc,
                &origin,
                0,
                &blend,
                ULW_ALPHA,
            )
        };
        if shown == 0 {
            eprintln!(
                "saudade: failed to update a layered popup window: {}",
                std::io::Error::last_os_error()
            );
        }
    }
}

impl Drop for Surface {
    fn drop(&mut self) {
        // SAFETY: the window is still alive — `self._win` holds it — so the
        // subclass can come off it; then the DC gets its original bitmap back
        // before the DIB section and the DC itself are deleted.
        unsafe {
            RemoveWindowSubclass(self.hwnd, Some(keep_layered), SUBCLASS_ID);
            if !self.bitmap.is_null() {
                SelectObject(self.dc, self.previous);
                DeleteObject(self.bitmap);
            }
            DeleteDC(self.dc);
        }
    }
}

/// One frame of a [`Surface`]: paint into [`Self::pixels`], then
/// [`Self::present`].
pub(crate) struct Frame<'a> {
    surface: &'a mut Surface,
}

impl Frame<'_> {
    pub(crate) fn pixels(&mut self) -> &mut [u32] {
        self.surface.pixels()
    }

    pub(crate) fn present(self) {
        self.surface.present();
    }
}

unsafe extern "system" fn keep_layered(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
    _id: usize,
    _data: usize,
) -> LRESULT {
    match msg {
        // `wparam` says which style is changing; it is a negative index.
        WM_STYLECHANGING if wparam as i32 == GWL_EXSTYLE => {
            // SAFETY: for `WM_STYLECHANGING`, `lparam` points at the change
            // about to be made, whose new style the window may alter.
            unsafe { (*(lparam as *mut STYLESTRUCT)).styleNew |= WS_EX_LAYERED };
            // SAFETY: forwarding the message we were handed.
            unsafe { DefSubclassProc(hwnd, msg, wparam, lparam) }
        }
        WM_NCDESTROY => {
            // SAFETY: removing the subclass installed by `Surface::new`, then
            // forwarding the message we were handed, unchanged.
            unsafe {
                RemoveWindowSubclass(hwnd, Some(keep_layered), SUBCLASS_ID);
                DefSubclassProc(hwnd, msg, wparam, lparam)
            }
        }
        // SAFETY: forwarding the message we were handed, unchanged.
        _ => unsafe { DefSubclassProc(hwnd, msg, wparam, lparam) },
    }
}
