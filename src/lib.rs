//! retrogui — a minimal, retained-mode GUI library for small Win 3.1-styled
//! utilities (about boxes, simple dialogs, system info viewers).
//!
//! The library follows the architecture sketched in `retrogui.md` but stays
//! intentionally small:
//!
//! * the runtime drives winit + softbuffer
//! * widgets are ordinary Rust values implementing [`Widget`]
//! * events are typed (no integer message IDs)
//! * widgets request repaint / window close via [`EventCtx`]
//! * the default [`Theme`] paints chrome that matches Windows 3.1
//!
//! ## Minimal example
//!
//! ```no_run
//! use retrogui::*;
//!
//! let root = Container::new(200, 80)
//!     .with_background(Color::WHITE)
//!     .add(Label::new(10, 10, "Hello, world!"))
//!     .add(
//!         Button::new(Rect::new(60, 40, 80, 24), "OK")
//!             .default(true)
//!             .on_click(|cx| cx.close()),
//!     );
//!
//! App::new(WindowConfig::new("Hello", 200, 80), root).run();
//! ```

mod app;
mod event;
mod font;
mod geometry;
pub mod mock;
mod painter;
mod theme;
mod wayland;
mod widget;
mod widgets;

pub use app::{App, WindowConfig};
pub use event::{Event, EventCtx, Key, Modifiers, MouseButton, NamedKey};
pub use font::Font;
pub use geometry::{Color, Point, Rect, Size};
pub use painter::Painter;
pub use theme::Theme;
pub use widget::{PopupKind, PopupRequest, Widget};
pub use widgets::{
    Bevel, Button, Column, Container, Dialog, DialogIcon, Image, Label, Menu, MenuBar, MenuItem,
    Orientation, SCROLLBAR_THICKNESS, ScrollBar, TextEditor,
};
