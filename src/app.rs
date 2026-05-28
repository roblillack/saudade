use std::num::NonZeroU32;
use std::rc::Rc;

use winit::dpi::{LogicalSize, PhysicalPosition, PhysicalSize};
use winit::event::{ElementState, Event as WinitEvent, MouseButton as WinitMouseButton, WindowEvent};
use winit::event_loop::{ControlFlow, EventLoop};
use winit::window::WindowBuilder;

use crate::event::{Event, EventCtx, MouseButton};
use crate::font::Font;
use crate::geometry::{Point, Size};
use crate::painter::Painter;
use crate::theme::Theme;
use crate::widget::Widget;

pub struct WindowConfig {
    pub title: String,
    pub size: Size,
    pub resizable: bool,
}

impl WindowConfig {
    pub fn new(title: impl Into<String>, width: i32, height: i32) -> Self {
        Self {
            title: title.into(),
            size: Size::new(width, height),
            resizable: false,
        }
    }

    pub fn resizable(mut self, resizable: bool) -> Self {
        self.resizable = resizable;
        self
    }
}

/// Top-level entry point. Owns the window configuration, the theme, and the
/// root widget tree, and drives the winit event loop until the user closes the
/// window or a widget calls [`EventCtx::close`](crate::event::EventCtx::close).
pub struct App {
    window: WindowConfig,
    theme: Theme,
    root: Box<dyn Widget>,
}

impl App {
    pub fn new(window: WindowConfig, root: impl Widget + 'static) -> Self {
        Self {
            window,
            theme: Theme::default(),
            root: Box::new(root),
        }
    }

    pub fn with_theme(mut self, theme: Theme) -> Self {
        self.theme = theme;
        self
    }

    pub fn run(self) {
        let App {
            window: window_cfg,
            theme,
            mut root,
        } = self;

        let event_loop = EventLoop::new().expect("retrogui: failed to create event loop");
        // Request the window in *logical* units. winit + the compositor pick
        // the physical buffer size based on the monitor's scale_factor, so we
        // get the right physical pixels for the design size on every DPI.
        let win = WindowBuilder::new()
            .with_title(&window_cfg.title)
            .with_inner_size(LogicalSize::new(
                window_cfg.size.w as f64,
                window_cfg.size.h as f64,
            ))
            .with_resizable(window_cfg.resizable)
            .build(&event_loop)
            .expect("retrogui: failed to create window");
        let win = Rc::new(win);

        let context = softbuffer::Context::new(win.clone())
            .expect("retrogui: failed to create softbuffer context");
        let mut surface = softbuffer::Surface::new(&context, win.clone())
            .expect("retrogui: failed to create softbuffer surface");

        let font = Font::load_system();
        let logical_size = window_cfg.size;

        let mut physical = win.inner_size();
        let mut scale = win.scale_factor() as f32;
        resize_surface(&mut surface, physical);

        let mut cursor: Option<Point> = None;
        let mut needs_redraw = true;

        event_loop
            .run(move |event, elwt| {
                elwt.set_control_flow(ControlFlow::Wait);

                match event {
                    WinitEvent::WindowEvent { event, .. } => match event {
                        WindowEvent::CloseRequested => elwt.exit(),
                        WindowEvent::Resized(new_size) => {
                            physical = new_size;
                            resize_surface(&mut surface, physical);
                            needs_redraw = true;
                        }
                        WindowEvent::ScaleFactorChanged { scale_factor, .. } => {
                            scale = scale_factor as f32;
                            physical = win.inner_size();
                            resize_surface(&mut surface, physical);
                            needs_redraw = true;
                        }
                        WindowEvent::CursorMoved { position, .. } => {
                            let (origin_x, origin_y) = origin(logical_size, scale, physical);
                            let pos = physical_to_logical(position, scale, origin_x, origin_y);
                            cursor = Some(pos);
                            dispatch(
                                &mut root,
                                &Event::PointerMove { pos },
                                &mut needs_redraw,
                                elwt,
                            );
                        }
                        WindowEvent::CursorLeft { .. } => {
                            cursor = None;
                            dispatch(&mut root, &Event::PointerLeave, &mut needs_redraw, elwt);
                        }
                        WindowEvent::MouseInput {
                            state,
                            button: winit_button,
                            ..
                        } => {
                            let Some(pos) = cursor else { return };
                            let Some(button) = map_button(winit_button) else {
                                return;
                            };
                            let event = match state {
                                ElementState::Pressed => Event::PointerDown { pos, button },
                                ElementState::Released => Event::PointerUp { pos, button },
                            };
                            dispatch(&mut root, &event, &mut needs_redraw, elwt);
                        }
                        WindowEvent::RedrawRequested => {
                            let (origin_x, origin_y) = origin(logical_size, scale, physical);
                            let mut surface_buf = surface
                                .buffer_mut()
                                .expect("retrogui: failed to acquire surface buffer");
                            let mut painter = Painter::new(
                                &mut surface_buf,
                                physical.width as i32,
                                physical.height as i32,
                                scale,
                                origin_x,
                                origin_y,
                                font.as_ref(),
                            );
                            // Clear the whole physical buffer so any letterbox
                            // area around the content (when the window has
                            // been resized larger than the design) shows the
                            // theme background instead of garbage.
                            painter.fill(theme.background);
                            root.paint(&mut painter, &theme);

                            surface_buf
                                .present()
                                .expect("retrogui: failed to present buffer");
                            needs_redraw = false;
                        }
                        _ => {}
                    },
                    WinitEvent::AboutToWait => {
                        if needs_redraw {
                            win.request_redraw();
                        }
                    }
                    _ => {}
                }
            })
            .expect("retrogui: event loop error");
    }
}

fn dispatch(
    root: &mut Box<dyn Widget>,
    event: &Event,
    needs_redraw: &mut bool,
    elwt: &winit::event_loop::EventLoopWindowTarget<()>,
) {
    let mut ctx = EventCtx::new();
    root.event(event, &mut ctx);
    if ctx.paint_requested {
        *needs_redraw = true;
    }
    if ctx.close_requested {
        elwt.exit();
    }
}

fn map_button(button: WinitMouseButton) -> Option<MouseButton> {
    match button {
        WinitMouseButton::Left => Some(MouseButton::Left),
        WinitMouseButton::Right => Some(MouseButton::Right),
        WinitMouseButton::Middle => Some(MouseButton::Middle),
        _ => None,
    }
}

fn resize_surface(
    surface: &mut softbuffer::Surface<Rc<winit::window::Window>, Rc<winit::window::Window>>,
    size: PhysicalSize<u32>,
) {
    let w = NonZeroU32::new(size.width.max(1)).unwrap();
    let h = NonZeroU32::new(size.height.max(1)).unwrap();
    surface
        .resize(w, h)
        .expect("retrogui: failed to resize surface");
}

/// Center the design within the physical buffer. At the natural DPI the
/// content fills the window exactly (origin = 0); only a resized-larger
/// window produces a non-zero offset, and the surrounding area becomes
/// background letterbox — content is never stretched.
fn origin(logical: Size, scale: f32, physical: PhysicalSize<u32>) -> (i32, i32) {
    let content_w = (logical.w as f32 * scale).round() as i32;
    let content_h = (logical.h as f32 * scale).round() as i32;
    let ox = ((physical.width as i32 - content_w) / 2).max(0);
    let oy = ((physical.height as i32 - content_h) / 2).max(0);
    (ox, oy)
}

fn physical_to_logical(
    pos: PhysicalPosition<f64>,
    scale: f32,
    origin_x: i32,
    origin_y: i32,
) -> Point {
    let s = scale.max(0.01) as f64;
    let x = ((pos.x - origin_x as f64) / s).floor() as i32;
    let y = ((pos.y - origin_y as f64) / s).floor() as i32;
    Point::new(x, y)
}
