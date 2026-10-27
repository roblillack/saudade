//! Where a painted frame goes: the seam between saudade's pixel buffer and
//! whatever the platform wants to be handed.
//!
//! **Experimental, uncommitted.** Three implementations sit behind one API so
//! they can be measured against each other:
//!
//! - [`soft`] — softbuffer, the default. A CPU buffer the window server reads
//!   directly (a `CGImage` over our memory on macOS, SHM on X11, a DIB on
//!   Windows). No GPU involved.
//! - [`gpu`] — `pixels`, behind the `pixels-backend` feature. The same CPU
//!   buffer, uploaded to a wgpu texture each frame and drawn as a fullscreen
//!   quad. Every window carries its own wgpu instance, adapter, device, queue
//!   and swapchain, because `pixels` exposes no way to share them.
//! - [`raw`] — wgpu directly, behind the `wgpu-backend` feature (which wins if
//!   both GPU features are on). One instance, adapter, device and queue for the
//!   whole process, a surface per window, and — where the surface allows a
//!   texture to be copied into — the frame written straight into the swapchain
//!   image, with no shader, pipeline or render pass at all.
//!
//! Build the GPU ones with:
//!
//! ```sh
//! cargo run --features pixels-backend --example present_bench
//! cargo run --features wgpu-backend --example present_bench
//! ```
//!
//! All three expose the same `Context`, `Surface` and `Frame`, which this
//! module wraps in its own [`Surface`] and [`Frame`]. The runtime paints into
//! [`Frame::pixels`], a row-major `0xAARRGGBB` buffer whose rows are
//! [`Frame::stride`] pixels apart, and hands it back with [`Frame::present`].
//!
//! The wrapper is there for the see-through pixels a popup wants, which
//! softbuffer 0.4 can show on no platform and a GPU swapchain can't on Windows.
//! A surface that asks for [`Alpha::Straight`] is presented by saudade itself
//! where it can be — a layered window on Windows ([`crate::win32_layered`]), a
//! Core Animation layer on macOS ([`crate::macos_layer`]) — whichever backend
//! is built in.

use std::sync::Arc;

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

#[cfg(not(any(feature = "pixels-backend", feature = "wgpu-backend")))]
use soft as backend;

#[cfg(all(feature = "pixels-backend", not(feature = "wgpu-backend")))]
use gpu as backend;

#[cfg(feature = "wgpu-backend")]
use raw as backend;

pub(crate) use backend::Context;

#[cfg(windows)]
use crate::win32_layered as layered;

#[cfg(target_os = "macos")]
use crate::macos_layer as layered;

/// Where one window's frames go: the backend's surface, or — for a
/// see-through popup on Windows or macOS — a presenter of saudade's own.
pub(crate) enum Surface {
    /// Boxed: a GPU backend's surface is a good deal bigger than a layered
    /// window's handful of handles.
    Backend(Box<backend::Surface>),
    #[cfg(any(windows, target_os = "macos"))]
    Layered(layered::Surface),
}

impl Surface {
    /// A surface of `size` device pixels for `win`, honouring `alpha` if it
    /// can — [`Self::honors_alpha`] says whether it did.
    pub(crate) fn new(
        context: &Context,
        win: Arc<Window>,
        size: PhysicalSize<u32>,
        alpha: Alpha,
    ) -> Option<Self> {
        #[cfg(any(windows, target_os = "macos"))]
        if alpha == Alpha::Straight
            && let Some(surface) = layered::Surface::new(win.clone(), size)
        {
            return Some(Self::Layered(surface));
        }
        backend::Surface::new(context, win, size, alpha).map(|s| Self::Backend(Box::new(s)))
    }

    /// Whether this surface can actually keep the alpha byte it is given.
    pub(crate) fn honors_alpha(&self) -> bool {
        match self {
            Self::Backend(surface) => surface.honors_alpha(),
            #[cfg(any(windows, target_os = "macos"))]
            Self::Layered(_) => true,
        }
    }

    /// What presents this surface's frames, for a benchmark to print: the
    /// backend's [`BACKEND`] name, unless saudade presents it itself.
    pub(crate) fn presenter(&self) -> &'static str {
        match self {
            Self::Backend(_) => BACKEND,
            #[cfg(any(windows, target_os = "macos"))]
            Self::Layered(_) => "layered",
        }
    }

    pub(crate) fn resize(&mut self, size: PhysicalSize<u32>) {
        match self {
            Self::Backend(surface) => surface.resize(size),
            #[cfg(any(windows, target_os = "macos"))]
            Self::Layered(surface) => surface.resize(size),
        }
    }

    pub(crate) fn frame(&mut self) -> Option<Frame<'_>> {
        match self {
            Self::Backend(surface) => surface.frame().map(Frame::Backend),
            #[cfg(any(windows, target_os = "macos"))]
            Self::Layered(surface) => surface.frame().map(Frame::Layered),
        }
    }
}

/// One frame of a [`Surface`]: paint into [`Self::pixels`], then
/// [`Self::present`].
pub(crate) enum Frame<'a> {
    Backend(backend::Frame<'a>),
    #[cfg(any(windows, target_os = "macos"))]
    Layered(layered::Frame<'a>),
}

impl Frame<'_> {
    /// Pixels from the start of one row to the start of the next.
    pub(crate) fn stride(&self) -> i32 {
        match self {
            Self::Backend(frame) => frame.stride(),
            #[cfg(any(windows, target_os = "macos"))]
            Self::Layered(frame) => frame.stride(),
        }
    }

    pub(crate) fn pixels(&mut self) -> &mut [u32] {
        match self {
            Self::Backend(frame) => frame.pixels(),
            #[cfg(any(windows, target_os = "macos"))]
            Self::Layered(frame) => frame.pixels(),
        }
    }

    pub(crate) fn present(self) {
        match self {
            Self::Backend(frame) => frame.present(),
            #[cfg(any(windows, target_os = "macos"))]
            Self::Layered(frame) => frame.present(),
        }
    }
}

/// The name of the backend in use, for a benchmark to print.
pub(crate) const BACKEND: &str = if cfg!(feature = "wgpu-backend") {
    "wgpu"
} else if cfg!(feature = "pixels-backend") {
    "pixels"
} else {
    "softbuffer"
};

#[cfg(not(any(feature = "pixels-backend", feature = "wgpu-backend")))]
mod soft {
    use super::{Alpha, Arc, PhysicalSize, Window};
    use std::num::NonZeroU32;

    /// softbuffer's per-display state, shared by every surface on it.
    pub(crate) struct Context(softbuffer::Context<Arc<Window>>);

    impl Context {
        pub(crate) fn new(win: &Arc<Window>) -> Option<Self> {
            softbuffer::Context::new(win.clone()).ok().map(Self)
        }
    }

    pub(crate) struct Surface {
        inner: softbuffer::Surface<Arc<Window>, Arc<Window>>,
        /// EXPERIMENTAL: with `SAUDADE_SCRATCH=1`, paint into a plain heap
        /// buffer of our own and memcpy it into the window's buffer at present
        /// time, rather than painting into the window's buffer directly. The
        /// question it answers: how much of the frame is spent because the
        /// buffer we paint into is one the window server also reads?
        scratch: Option<Vec<u32>>,
    }

    impl Surface {
        pub(crate) fn new(
            context: &Context,
            win: Arc<Window>,
            size: PhysicalSize<u32>,
            // Opaque either way: see `honors_alpha`.
            _alpha: Alpha,
        ) -> Option<Self> {
            let mut surface = softbuffer::Surface::new(&context.0, win).ok()?;
            let (w, h) = nonzero(size);
            surface
                .resize(w, h)
                .expect("saudade: failed to resize surface");
            let scratch = std::env::var("SAUDADE_SCRATCH")
                .is_ok_and(|v| v != "0")
                .then(Vec::new);
            Some(Self {
                inner: surface,
                scratch,
            })
        }

        /// Never: softbuffer 0.4 has no alpha mode, and every backend of it
        /// shows a pixel opaque whatever its alpha byte says.
        pub(crate) fn honors_alpha(&self) -> bool {
            false
        }

        pub(crate) fn resize(&mut self, size: PhysicalSize<u32>) {
            let (w, h) = nonzero(size);
            self.inner
                .resize(w, h)
                .expect("saudade: failed to resize surface");
        }

        pub(crate) fn frame(&mut self) -> Option<Frame<'_>> {
            let buffer = self.inner.buffer_mut().ok()?;
            match self.scratch.as_mut() {
                None => Some(Frame {
                    buffer,
                    scratch: None,
                }),
                Some(scratch) => {
                    let want = buffer.len();
                    if scratch.len() != want {
                        scratch.resize(want, 0);
                    }
                    Some(Frame {
                        buffer,
                        scratch: Some(scratch),
                    })
                }
            }
        }
    }

    fn nonzero(size: PhysicalSize<u32>) -> (NonZeroU32, NonZeroU32) {
        (
            NonZeroU32::new(size.width.max(1)).unwrap(),
            NonZeroU32::new(size.height.max(1)).unwrap(),
        )
    }

    pub(crate) struct Frame<'a> {
        buffer: softbuffer::Buffer<'a, Arc<Window>, Arc<Window>>,
        scratch: Option<&'a mut Vec<u32>>,
    }

    impl Frame<'_> {
        /// Rows are tightly packed: softbuffer 0.4 pads none.
        pub(crate) fn stride(&self) -> i32 {
            self.buffer.width().get() as i32
        }

        pub(crate) fn pixels(&mut self) -> &mut [u32] {
            match self.scratch.as_mut() {
                Some(scratch) => scratch,
                None => &mut self.buffer,
            }
        }

        pub(crate) fn present(mut self) {
            if let Some(scratch) = self.scratch.take() {
                let n = self.buffer.len().min(scratch.len());
                self.buffer[..n].copy_from_slice(&scratch[..n]);
            }
            self.buffer
                .present()
                .expect("saudade: failed to present buffer");
        }
    }
}

#[cfg(all(feature = "pixels-backend", not(feature = "wgpu-backend")))]
mod gpu {
    use super::{Alpha, Arc, PhysicalSize, Window};
    use pixels::wgpu;
    use pixels::{Pixels, PixelsBuilder, SurfaceTexture};

    /// Nothing to share: `pixels` builds a wgpu instance, adapter, device and
    /// queue per surface, with no API for handing it ones that already exist.
    pub(crate) struct Context;

    impl Context {
        pub(crate) fn new(_win: &Arc<Window>) -> Option<Self> {
            Some(Self)
        }
    }

    pub(crate) struct Surface {
        pixels: Pixels<'static>,
        win: Arc<Window>,
        size: PhysicalSize<u32>,
        straight_alpha: bool,
    }

    impl Surface {
        pub(crate) fn new(
            _context: &Context,
            win: Arc<Window>,
            size: PhysicalSize<u32>,
            alpha: Alpha,
        ) -> Option<Self> {
            let (w, h) = (size.width.max(1), size.height.max(1));
            // `SAUDADE_PIXELS_VSYNC=1` puts the swapchain back on `AutoVsync`.
            // Off by default: with it on, `render` blocks until the compositor
            // takes the last frame, which lands in the middle of the interval
            // a benchmark is trying to time.
            let vsync = std::env::var("SAUDADE_PIXELS_VSYNC").is_ok_and(|v| v != "0");
            let build = |alpha_mode| {
                PixelsBuilder::new(w, h, SurfaceTexture::new(w, h, win.clone()))
                    // BGRA, so the texture's bytes are `Color`'s `0xAARRGGBB`
                    // as-is. sRGB on both sides of the blit round-trips the
                    // values untouched.
                    .texture_format(wgpu::TextureFormat::Bgra8UnormSrgb)
                    // The quad's own alpha *is* the window's alpha; don't let
                    // it blend against the clear colour on the way out.
                    .blend_state(wgpu::BlendState::REPLACE)
                    .clear_color(wgpu::Color::TRANSPARENT)
                    .alpha_mode(alpha_mode)
                    .enable_vsync(vsync)
                    .build()
            };
            let want = match alpha {
                Alpha::Straight => wgpu::CompositeAlphaMode::PostMultiplied,
                Alpha::Opaque => wgpu::CompositeAlphaMode::Auto,
            };
            let (pixels, straight_alpha) = match build(want) {
                Ok(p) => (p, alpha == Alpha::Straight),
                // The surface cannot composite straight alpha — take an opaque
                // one and let the caller fall back to redrawing the underlay.
                Err(_) => (build(wgpu::CompositeAlphaMode::Auto).ok()?, false),
            };
            Some(Self {
                pixels,
                win,
                size: PhysicalSize::new(w, h),
                straight_alpha,
            })
        }

        pub(crate) fn honors_alpha(&self) -> bool {
            self.straight_alpha
        }

        pub(crate) fn resize(&mut self, size: PhysicalSize<u32>) {
            let (w, h) = (size.width.max(1), size.height.max(1));
            if self.size == PhysicalSize::new(w, h) {
                return;
            }
            self.size = PhysicalSize::new(w, h);
            let _ = self.pixels.resize_buffer(w, h);
            let _ = self.pixels.resize_surface(w, h);
        }

        pub(crate) fn frame(&mut self) -> Option<Frame<'_>> {
            Some(Frame {
                pixels: &mut self.pixels,
                win: &self.win,
            })
        }
    }

    pub(crate) struct Frame<'a> {
        pixels: &'a mut Pixels<'static>,
        win: &'a Arc<Window>,
    }

    impl Frame<'_> {
        /// `pixels` hands back a tightly packed buffer — no row padding.
        pub(crate) fn stride(&self) -> i32 {
            self.pixels.texture().width() as i32
        }

        pub(crate) fn pixels(&mut self) -> &mut [u32] {
            let bytes = self.pixels.frame_mut();
            // SAFETY: the buffer is a `Vec<u8>` of `w * h * 4` bytes holding
            // BGRA texels, which is `Color`'s `0xAARRGGBB` on a little-endian
            // target. The prefix assert catches an allocator that hands back a
            // buffer that isn't 4-aligned, which no mainstream one does.
            let (prefix, pixels, _) = unsafe { bytes.align_to_mut::<u32>() };
            assert!(prefix.is_empty(), "saudade: pixel buffer is not 4-aligned");
            pixels
        }

        pub(crate) fn present(self) {
            // winit wants this before a frame goes out; softbuffer's macOS
            // backend does it inside `present`, wgpu leaves it to the caller.
            self.win.pre_present_notify();
            if let Err(err) = self.pixels.render() {
                eprintln!("saudade: failed to present buffer: {err}");
            }
        }
    }
}

#[cfg(feature = "wgpu-backend")]
mod raw {
    use super::{Alpha, Arc, PhysicalSize, Window};
    use std::rc::Rc;

    /// Shared by every window in the process: one instance, one adapter, one
    /// device, one queue. This is the whole point of driving wgpu directly —
    /// opening a menu then costs a surface and a texture, not a GPU device.
    /// Every [`Surface`] holds a handle to it, so presenting needs nothing
    /// passed in and the three backends keep the same shape.
    struct Shared {
        instance: wgpu::Instance,
        adapter: wgpu::Adapter,
        device: wgpu::Device,
        queue: wgpu::Queue,
        /// Built once, and only for the fallback path — see [`Blit`].
        shader: wgpu::ShaderModule,
        sampler: wgpu::Sampler,
    }

    pub(crate) struct Context(Rc<Shared>);

    impl Context {
        pub(crate) fn new(win: &Arc<Window>) -> Option<Self> {
            let instance =
                wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
            // An adapter is picked against a surface it has to be able to
            // present on, so borrow the first window for the question and drop
            // the surface again; the real ones come from `Surface::new`.
            let probe = instance.create_surface(win.clone()).ok()?;
            let adapter =
                pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
                    power_preference: wgpu::PowerPreference::default(),
                    force_fallback_adapter: false,
                    compatible_surface: Some(&probe),
                }))
                .ok()?;
            let (device, queue) =
                pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
                    label: Some("saudade"),
                    required_limits: adapter.limits(),
                    ..Default::default()
                }))
                .ok()?;
            drop(probe);

            let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("saudade blit"),
                source: wgpu::ShaderSource::Wgsl(BLIT_WGSL.into()),
            });
            let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
                label: Some("saudade blit"),
                // The buffer is the same size as the surface, so this only ever
                // samples texel centres — but nearest keeps it honest if a
                // rounding difference ever puts it half a texel out.
                mag_filter: wgpu::FilterMode::Nearest,
                min_filter: wgpu::FilterMode::Nearest,
                ..Default::default()
            });

            Some(Self(Rc::new(Shared {
                instance,
                adapter,
                device,
                queue,
                shader,
                sampler,
            })))
        }
    }

    /// A fullscreen triangle that samples the frame. Only built when the
    /// surface will not accept a copy — on Metal it never is.
    struct Blit {
        pipeline: wgpu::RenderPipeline,
        texture: wgpu::Texture,
        bind_group: wgpu::BindGroup,
        layout: wgpu::BindGroupLayout,
    }

    pub(crate) struct Surface {
        shared: Rc<Shared>,
        surface: wgpu::Surface<'static>,
        win: Arc<Window>,
        format: wgpu::TextureFormat,
        alpha_mode: wgpu::CompositeAlphaMode,
        present_mode: wgpu::PresentMode,
        usage: wgpu::TextureUsages,
        size: PhysicalSize<u32>,
        /// Pixels per row, rounded up so a row is a whole number of 256-byte
        /// blocks — the alignment a texture write wants.
        stride: u32,
        /// What the painter paints into.
        buffer: Vec<u32>,
        /// `None` when the frame goes straight into the swapchain image.
        blit: Option<Blit>,
        straight_alpha: bool,
    }

    impl Surface {
        pub(crate) fn new(
            context: &Context,
            win: Arc<Window>,
            size: PhysicalSize<u32>,
            alpha: Alpha,
        ) -> Option<Self> {
            let shared = context.0.clone();
            let surface = shared.instance.create_surface(win.clone()).ok()?;
            let caps = surface.get_capabilities(&shared.adapter);

            // Match softbuffer's pixels byte for byte: BGRA in memory, which is
            // `Color`'s `0xAARRGGBB`, and sRGB so nothing is re-encoded on the
            // way to the compositor.
            let format = if caps.formats.contains(&wgpu::TextureFormat::Bgra8UnormSrgb) {
                wgpu::TextureFormat::Bgra8UnormSrgb
            } else {
                *caps.formats.first()?
            };
            let want_alpha = match alpha {
                Alpha::Straight => wgpu::CompositeAlphaMode::PostMultiplied,
                Alpha::Opaque => wgpu::CompositeAlphaMode::Opaque,
            };
            let alpha_mode = if caps.alpha_modes.contains(&want_alpha) {
                want_alpha
            } else {
                caps.alpha_modes[0]
            };
            let straight_alpha = alpha_mode == wgpu::CompositeAlphaMode::PostMultiplied;
            let vsync = std::env::var("SAUDADE_GPU_VSYNC").is_ok_and(|v| v != "0");
            let present_mode = if vsync {
                wgpu::PresentMode::AutoVsync
            } else {
                wgpu::PresentMode::AutoNoVsync
            };

            // The fast path: hand the frame to the swapchain image itself. It
            // needs a surface that can be a copy destination, which Metal can
            // and some others cannot; `SAUDADE_WGPU_MODE=quad` forces the
            // fallback for comparison.
            let forced = std::env::var("SAUDADE_WGPU_MODE").unwrap_or_default();
            let direct = forced != "quad" && caps.usages.contains(wgpu::TextureUsages::COPY_DST);
            let usage = if direct {
                wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_DST
            } else {
                wgpu::TextureUsages::RENDER_ATTACHMENT
            };

            let mut this = Self {
                shared,
                surface,
                win,
                format,
                alpha_mode,
                present_mode,
                usage,
                size: PhysicalSize::new(0, 0),
                stride: 0,
                buffer: Vec::new(),
                blit: None,
                straight_alpha,
            };
            this.reconfigure(size, !direct);
            Some(this)
        }

        pub(crate) fn honors_alpha(&self) -> bool {
            self.straight_alpha
        }

        fn reconfigure(&mut self, size: PhysicalSize<u32>, blit: bool) {
            let (w, h) = (size.width.max(1), size.height.max(1));
            self.size = PhysicalSize::new(w, h);
            // 256 bytes = 64 pixels: the row alignment a buffer-to-texture copy
            // wants. `Queue::write_texture` stages the data itself and takes
            // any stride, so `SAUDADE_WGPU_PAD=0` asks for tight rows — worth a
            // look, since the padding is memory the painter also has to clear.
            self.stride = if std::env::var("SAUDADE_WGPU_PAD").is_ok_and(|v| v == "0") {
                w
            } else {
                w.div_ceil(64) * 64
            };
            self.buffer.clear();
            self.buffer.resize(self.stride as usize * h as usize, 0);

            self.surface.configure(
                &self.shared.device,
                &wgpu::SurfaceConfiguration {
                    usage: self.usage,
                    format: self.format,
                    width: w,
                    height: h,
                    present_mode: self.present_mode,
                    desired_maximum_frame_latency: 2,
                    alpha_mode: self.alpha_mode,
                    view_formats: vec![],
                },
            );

            if !blit {
                self.blit = None;
                return;
            }
            let texture = self.shared.device.create_texture(&wgpu::TextureDescriptor {
                label: Some("saudade frame"),
                size: wgpu::Extent3d {
                    width: w,
                    height: h,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: self.format,
                usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                view_formats: &[],
            });
            let layout =
                match self.blit.take() {
                    Some(blit) => blit.layout,
                    None => self.shared.device.create_bind_group_layout(
                        &wgpu::BindGroupLayoutDescriptor {
                            label: Some("saudade blit"),
                            entries: &[
                                wgpu::BindGroupLayoutEntry {
                                    binding: 0,
                                    visibility: wgpu::ShaderStages::FRAGMENT,
                                    ty: wgpu::BindingType::Texture {
                                        sample_type: wgpu::TextureSampleType::Float {
                                            filterable: false,
                                        },
                                        view_dimension: wgpu::TextureViewDimension::D2,
                                        multisampled: false,
                                    },
                                    count: None,
                                },
                                wgpu::BindGroupLayoutEntry {
                                    binding: 1,
                                    visibility: wgpu::ShaderStages::FRAGMENT,
                                    ty: wgpu::BindingType::Sampler(
                                        wgpu::SamplerBindingType::NonFiltering,
                                    ),
                                    count: None,
                                },
                            ],
                        },
                    ),
                };
            let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
            let bind_group = self
                .shared
                .device
                .create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("saudade blit"),
                    layout: &layout,
                    entries: &[
                        wgpu::BindGroupEntry {
                            binding: 0,
                            resource: wgpu::BindingResource::TextureView(&view),
                        },
                        wgpu::BindGroupEntry {
                            binding: 1,
                            resource: wgpu::BindingResource::Sampler(&self.shared.sampler),
                        },
                    ],
                });
            let pipeline_layout =
                self.shared
                    .device
                    .create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                        label: Some("saudade blit"),
                        bind_group_layouts: &[Some(&layout)],
                        immediate_size: 0,
                    });
            let pipeline =
                self.shared
                    .device
                    .create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                        label: Some("saudade blit"),
                        layout: Some(&pipeline_layout),
                        vertex: wgpu::VertexState {
                            module: &self.shared.shader,
                            entry_point: Some("vs"),
                            buffers: &[],
                            compilation_options: Default::default(),
                        },
                        fragment: Some(wgpu::FragmentState {
                            module: &self.shared.shader,
                            entry_point: Some("fs"),
                            // The frame's own alpha is the window's alpha: write it
                            // through rather than blending against the target.
                            targets: &[Some(wgpu::ColorTargetState {
                                format: self.format,
                                blend: Some(wgpu::BlendState::REPLACE),
                                write_mask: wgpu::ColorWrites::ALL,
                            })],
                            compilation_options: Default::default(),
                        }),
                        primitive: Default::default(),
                        depth_stencil: None,
                        multisample: Default::default(),
                        multiview_mask: None,
                        cache: None,
                    });
            self.blit = Some(Blit {
                pipeline,
                texture,
                bind_group,
                layout,
            });
        }

        pub(crate) fn frame(&mut self) -> Option<Frame<'_>> {
            Some(Frame { surface: self })
        }
    }

    pub(crate) struct Frame<'a> {
        surface: &'a mut Surface,
    }

    impl Frame<'_> {
        pub(crate) fn stride(&self) -> i32 {
            self.surface.stride as i32
        }

        pub(crate) fn pixels(&mut self) -> &mut [u32] {
            &mut self.surface.buffer
        }

        pub(crate) fn present(self) {
            self.surface.present_frame();
        }
    }

    impl Surface {
        fn present_frame(&mut self) {
            let (w, h) = (self.size.width, self.size.height);
            let bytes = {
                let ptr = self.buffer.as_ptr().cast::<u8>();
                // SAFETY: `u32` is four `u8`s, and the slice is live for the call.
                unsafe { std::slice::from_raw_parts(ptr, self.buffer.len() * 4) }
            };
            let layout = wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(self.stride * 4),
                rows_per_image: Some(h),
            };
            let extent = wgpu::Extent3d {
                width: w,
                height: h,
                depth_or_array_layers: 1,
            };

            use wgpu::CurrentSurfaceTexture as Acquired;
            let frame = match self.surface.get_current_texture() {
                Acquired::Success(frame) | Acquired::Suboptimal(frame) => frame,
                // The surface moved out from under us (a resize, a display
                // change): reconfigure and take one more run at it.
                Acquired::Outdated | Acquired::Lost => {
                    let blit = self.blit.is_some();
                    self.reconfigure(self.size, blit);
                    match self.surface.get_current_texture() {
                        Acquired::Success(frame) | Acquired::Suboptimal(frame) => frame,
                        _ => return,
                    }
                }
                // Nothing is on screen to draw to, or the frame is not ready.
                Acquired::Occluded | Acquired::Timeout | Acquired::Validation => return,
            };

            match self.blit.as_ref() {
                // Straight into the swapchain image: one write, no pass.
                None => {
                    self.shared.queue.write_texture(
                        wgpu::TexelCopyTextureInfo {
                            texture: &frame.texture,
                            mip_level: 0,
                            origin: wgpu::Origin3d::ZERO,
                            aspect: wgpu::TextureAspect::All,
                        },
                        bytes,
                        layout,
                        extent,
                    );
                    self.shared.queue.submit([]);
                }
                // Fallback: upload to our own texture and draw it over the
                // swapchain image with a fullscreen triangle.
                Some(blit) => {
                    self.shared.queue.write_texture(
                        wgpu::TexelCopyTextureInfo {
                            texture: &blit.texture,
                            mip_level: 0,
                            origin: wgpu::Origin3d::ZERO,
                            aspect: wgpu::TextureAspect::All,
                        },
                        bytes,
                        layout,
                        extent,
                    );
                    let view = frame
                        .texture
                        .create_view(&wgpu::TextureViewDescriptor::default());
                    let mut encoder = self.shared.device.create_command_encoder(
                        &wgpu::CommandEncoderDescriptor {
                            label: Some("saudade blit"),
                        },
                    );
                    {
                        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                            label: Some("saudade blit"),
                            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                                view: &view,
                                depth_slice: None,
                                resolve_target: None,
                                ops: wgpu::Operations {
                                    load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                                    store: wgpu::StoreOp::Store,
                                },
                            })],
                            depth_stencil_attachment: None,
                            timestamp_writes: None,
                            occlusion_query_set: None,
                            multiview_mask: None,
                        });
                        pass.set_pipeline(&blit.pipeline);
                        pass.set_bind_group(0, &blit.bind_group, &[]);
                        pass.draw(0..3, 0..1);
                    }
                    self.shared.queue.submit([encoder.finish()]);
                }
            }

            self.win.pre_present_notify();
            frame.present();
        }

        pub(crate) fn resize(&mut self, size: PhysicalSize<u32>) {
            let size = PhysicalSize::new(size.width.max(1), size.height.max(1));
            if self.size == size {
                return;
            }
            let blit = self.blit.is_some();
            self.reconfigure(size, blit);
        }
    }

    /// A fullscreen triangle, and the frame sampled onto it. Only used where a
    /// surface refuses to be a copy destination.
    const BLIT_WGSL: &str = r#"
struct VsOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@vertex
fn vs(@builtin(vertex_index) i: u32) -> VsOut {
    // (-1,-1), (3,-1), (-1,3): one triangle covering the whole target.
    let x = f32(i32(i) / 2) * 4.0 - 1.0;
    let y = f32(i32(i) & 1) * 4.0 - 1.0;
    var out: VsOut;
    out.pos = vec4<f32>(x, -y, 0.0, 1.0);
    out.uv = vec2<f32>((x + 1.0) * 0.5, (y + 1.0) * 0.5);
    return out;
}

@group(0) @binding(0) var frame: texture_2d<f32>;
@group(0) @binding(1) var samp: sampler;

@fragment
fn fs(in: VsOut) -> @location(0) vec4<f32> {
    return textureSample(frame, samp, in.uv);
}
"#;
}
