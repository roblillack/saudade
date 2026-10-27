//! macOS: a popup window with real per-pixel alpha.
//!
//! A menu panel never fills its window: the two corners inside the bend of its
//! L-shaped drop shadow are not part of it, and AppKit, which sizes a window in
//! points, may round it up a device pixel past the footprint it was asked for.
//! softbuffer 0.4 can't show the pixels a panel leaves out as see-through: its
//! CoreGraphics backend hands Core Animation a `CGImage` whose alpha byte is
//! ignored. (Its `master` has an alpha-mode API that fixes that, but no release
//! carries it yet.) So those pixels used to be filled in by redrawing the app
//! underneath — which has nothing to redraw where a menu hangs off its window,
//! and left a white margin out over the desktop.
//!
//! [`Surface`] does what softbuffer's backend does, with the alpha byte kept: a
//! Core Animation layer of its own over the window's view, whose contents are a
//! `CGImage` of the frame. The image is straight (unassociated) alpha, alpha
//! first in a host-order 32-bit pixel — `0xAARRGGBB`, the painter's own layout,
//! so nothing is converted on the way. That is the format softbuffer's `master`
//! uses for the same job; it says premultiplied did not work for it with
//! transparent windows. The window itself is see-through already (winit's
//! `with_transparent`), so wherever the image is, so is the popup.

use std::ptr;
use std::sync::Arc;

use objc2::rc::Retained;
use objc2_app_kit::NSView;
use objc2_core_foundation::{CFData, CFRetained, CGPoint};
use objc2_core_graphics::{
    CGBitmapInfo, CGColorRenderingIntent, CGColorSpace, CGDataProvider, CGImage, CGImageAlphaInfo,
    CGImageByteOrderInfo,
};
use objc2_quartz_core::{CALayer, CATransaction, kCAGravityTopLeft};
use winit::dpi::PhysicalSize;
use winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};
use winit::window::Window;

/// A layer over a popup window's view, and the frame painted for it.
pub(crate) struct Surface {
    /// Kept alive for as long as `layer` sits in its view.
    _win: Arc<Window>,
    /// The view's own layer, which ours is a sublayer of. Its contents belong
    /// to AppKit, so ours go on a layer of our own on top.
    root: Retained<CALayer>,
    layer: Retained<CALayer>,
    space: CFRetained<CGColorSpace>,
    /// The frame: `size.width × size.height` pixels, rows top-down and tightly
    /// packed, each `0xAARRGGBB`. Copied out on every present, so the image
    /// Core Animation holds on to never changes under it.
    pixels: Vec<u32>,
    size: PhysicalSize<u32>,
}

impl Surface {
    /// A layer of `size` device pixels over `win`'s view. `None` if the view
    /// can't be reached or won't take one, which leaves the window as it was
    /// for the caller to fall back on.
    pub(crate) fn new(win: Arc<Window>, size: PhysicalSize<u32>) -> Option<Self> {
        let RawWindowHandle::AppKit(handle) = win.window_handle().ok()?.as_raw() else {
            return None;
        };
        // SAFETY: winit owns this `NSView` for as long as the window lives,
        // which is past the end of this borrow — `win` is still holding it.
        let view: &NSView = unsafe { handle.ns_view.cast().as_ref() };
        view.setWantsLayer(true);
        let root = view.layer()?;
        let space = CGColorSpace::new_device_rgb()?;

        let layer = CALayer::new();
        // The origin top-left, where the painter's first row is, and the image
        // pinned there at its own size rather than stretched to the layer's.
        layer.setAnchorPoint(CGPoint::new(0.0, 0.0));
        layer.setGeometryFlipped(true);
        // SAFETY: an immutable string constant Core Animation exports.
        layer.setContentsGravity(unsafe { kCAGravityTopLeft });
        layer.setOpaque(false);
        root.addSublayer(&layer);

        let mut surface = Self {
            _win: win,
            root,
            layer,
            space,
            pixels: Vec::new(),
            size: PhysicalSize::new(0, 0),
        };
        surface.resize(size);
        Some(surface)
    }

    pub(crate) fn resize(&mut self, size: PhysicalSize<u32>) {
        let size = PhysicalSize::new(size.width.max(1), size.height.max(1));
        if size != self.size {
            self.pixels.clear();
            self.pixels
                .resize(size.width as usize * size.height as usize, 0);
            self.size = size;
        }
    }

    pub(crate) fn frame(&mut self) -> Option<Frame<'_>> {
        Some(Frame { surface: self })
    }

    fn present(&mut self) {
        let Some(image) = self.image() else {
            eprintln!("saudade: failed to make an image of a popup window's frame");
            return;
        };
        // Everything below in one transaction with actions off: a layer that
        // isn't a view's own animates any change to it by default, and a
        // quarter-second cross-fade between frames is not what a menu wants.
        CATransaction::begin();
        CATransaction::setDisableActions(true);
        // Following the view on every present rather than observing it: a
        // popup is resized and moved between displays only by saudade, and a
        // frame follows each time. The view's layer carries the window's
        // backing scale, which is how many of our pixels make a point.
        self.layer.setFrame(self.root.bounds());
        self.layer.setContentsScale(self.root.contentsScale());
        // SAFETY: a `CGImage` is one of the things `contents` may hold.
        unsafe { self.layer.setContents(Some(image.as_ref())) };
        CATransaction::commit();
    }

    /// The frame as a straight-alpha `CGImage`, over a copy of its pixels.
    fn image(&self) -> Option<CFRetained<CGImage>> {
        // SAFETY: `u32`s seen as their bytes: the same memory, four times the
        // length, and `u8` has no alignment to break.
        let bytes = unsafe {
            std::slice::from_raw_parts(self.pixels.as_ptr().cast::<u8>(), self.pixels.len() * 4)
        };
        let data = CFData::from_bytes(bytes);
        let provider = CGDataProvider::with_cf_data(Some(&data))?;
        let info = CGBitmapInfo(CGImageAlphaInfo::First.0 | CGImageByteOrderInfo::Order32Host.0);
        let width = self.size.width as usize;
        // SAFETY: `provider` holds `width × height` 32-bit pixels, tightly
        // packed, which is what the sizes describe; `decode` may be null.
        unsafe {
            CGImage::new(
                width,
                self.size.height as usize,
                8,
                32,
                width * 4,
                Some(&self.space),
                info,
                Some(&provider),
                ptr::null(),
                false,
                CGColorRenderingIntent::RenderingIntentDefault,
            )
        }
    }
}

impl Drop for Surface {
    fn drop(&mut self) {
        // The view outlives this — `self._win` holds it — and would otherwise
        // keep showing the last frame until the window goes.
        self.layer.removeFromSuperlayer();
    }
}

/// One frame of a [`Surface`]: paint into [`Self::pixels`], then
/// [`Self::present`].
pub(crate) struct Frame<'a> {
    surface: &'a mut Surface,
}

impl Frame<'_> {
    /// Rows are tightly packed.
    pub(crate) fn stride(&self) -> i32 {
        self.surface.size.width as i32
    }

    pub(crate) fn pixels(&mut self) -> &mut [u32] {
        &mut self.surface.pixels
    }

    pub(crate) fn present(self) {
        self.surface.present();
    }
}
