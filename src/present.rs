//! Where a painted frame goes: the seam between saudade's pixel buffer and
//! the window it is shown in.
//!
//! softbuffer presents every window by default. It can't show a see-through
//! pixel on any platform — 0.4 has no alpha mode — and a popup window wants
//! them, for the parts of it a menu panel does not cover. A surface that asks
//! for [`Alpha::Straight`] is therefore presented by saudade itself where it
//! can be — a Core Animation layer on macOS ([`crate::macos_layer`]). Anywhere
//! else it is an opaque softbuffer surface, and [`Surface::honors_alpha`] says
//! so.
//!
//! The runtime paints into [`Frame::pixels`], a row-major `0xAARRGGBB` buffer
//! with tightly packed rows, and hands it back with [`Frame::present`].

use std::num::NonZeroU32;
use std::rc::Rc;

use winit::dpi::PhysicalSize;
use winit::window::Window;

/// What a surface has to do with the alpha byte of the pixels it is given.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Alpha {
    /// There is no alpha: every pixel is opaque, whatever the byte says. What
    /// a window that paints all of itself wants.
    Opaque,
    /// Straight (unassociated) alpha, the layout [`Color`](crate::Color)
    /// already has: a pixel is `0xAARRGGBB` with the colour *not* premultiplied
    /// by `AA`. What a popup window wants, so the parts of it a menu panel does
    /// not cover can be see-through.
    Straight,
}

#[cfg(target_os = "macos")]
use crate::macos_layer as layered;

/// Where one window's frames go: a softbuffer surface, or — for a see-through
/// popup on macOS — a presenter of saudade's own.
pub(crate) enum Surface {
    Soft(softbuffer::Surface<Rc<Window>, Rc<Window>>),
    #[cfg(target_os = "macos")]
    Layered(layered::Surface),
}

impl Surface {
    /// A surface of `size` device pixels for `win`, honouring `alpha` if it
    /// can — [`Self::honors_alpha`] says whether it did.
    pub(crate) fn new(
        context: &softbuffer::Context<Rc<Window>>,
        win: Rc<Window>,
        size: PhysicalSize<u32>,
        alpha: Alpha,
    ) -> Option<Self> {
        #[cfg(target_os = "macos")]
        if alpha == Alpha::Straight
            && let Some(surface) = layered::Surface::new(win.clone(), size)
        {
            return Some(Self::Layered(surface));
        }
        // Windows and X11 have nothing better than softbuffer to offer.
        #[cfg(not(target_os = "macos"))]
        let _ = alpha;
        let mut surface = softbuffer::Surface::new(context, win).ok()?;
        resize_soft(&mut surface, size);
        Some(Self::Soft(surface))
    }

    /// Whether this surface can actually keep the alpha byte it is given.
    pub(crate) fn honors_alpha(&self) -> bool {
        match self {
            // softbuffer 0.4 has no alpha mode: every backend of it shows a
            // pixel opaque, whatever its alpha byte says.
            Self::Soft(_) => false,
            #[cfg(target_os = "macos")]
            Self::Layered(_) => true,
        }
    }

    pub(crate) fn resize(&mut self, size: PhysicalSize<u32>) {
        match self {
            Self::Soft(surface) => resize_soft(surface, size),
            #[cfg(target_os = "macos")]
            Self::Layered(surface) => surface.resize(size),
        }
    }

    pub(crate) fn frame(&mut self) -> Option<Frame<'_>> {
        match self {
            Self::Soft(surface) => surface.buffer_mut().ok().map(Frame::Soft),
            #[cfg(target_os = "macos")]
            Self::Layered(surface) => surface.frame().map(Frame::Layered),
        }
    }
}

fn resize_soft(surface: &mut softbuffer::Surface<Rc<Window>, Rc<Window>>, size: PhysicalSize<u32>) {
    let w = NonZeroU32::new(size.width.max(1)).unwrap();
    let h = NonZeroU32::new(size.height.max(1)).unwrap();
    surface
        .resize(w, h)
        .expect("saudade: failed to resize surface");
}

/// One frame of a [`Surface`]: paint into [`Self::pixels`], then
/// [`Self::present`].
pub(crate) enum Frame<'a> {
    Soft(softbuffer::Buffer<'a, Rc<Window>, Rc<Window>>),
    #[cfg(target_os = "macos")]
    Layered(layered::Frame<'a>),
}

impl Frame<'_> {
    pub(crate) fn pixels(&mut self) -> &mut [u32] {
        match self {
            Self::Soft(buffer) => buffer,
            #[cfg(target_os = "macos")]
            Self::Layered(frame) => frame.pixels(),
        }
    }

    pub(crate) fn present(self) {
        match self {
            Self::Soft(buffer) => buffer.present().expect("saudade: failed to present buffer"),
            #[cfg(target_os = "macos")]
            Self::Layered(frame) => frame.present(),
        }
    }
}
