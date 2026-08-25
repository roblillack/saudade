//! Saudade — a minimal, retained-mode GUI library for small Win 3.1-styled
//! programs and utilities. Targets Wayland, X11, Windows, macOS.
//!
//! The library stays intentionally small:
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
//! use saudade::*;
//!
//! let root = Container::new(200, 80)
//!     .with_background(Color::WHITE)
//!     .add(Label::new(Rect::new(10, 10, 180, 16), "Hello, world!"))
//!     .add(
//!         Button::new(Rect::new(60, 40, 80, 24), "OK")
//!             .default(true)
//!             .on_click(|cx| cx.close()),
//!     );
//!
//! App::new(WindowConfig::new("Hello", 200, 80), root).run();
//! ```

// Let the crate refer to itself as `saudade`, so the `include_svg!` macro —
// which expands to absolute `::saudade::…` paths for use by *downstream* crates
// — also works when saudade uses it internally (e.g. the dialog icons).
extern crate self as saudade;

mod accel;
mod app;
mod background;
pub mod chrome;
#[cfg(target_os = "macos")]
mod coretext;
mod event;
mod font;
mod geometry;
pub mod mock;
mod painter;
mod svg;
mod theme;
#[cfg(all(unix, not(target_os = "macos")))]
mod wayland;
mod widget;
mod widgets;

pub use accel::{Accel, AccelMods, ModifierScheme};
pub use app::{App, WindowConfig};
pub use background::{BackgroundPattern, PATTERN_COLORS};
pub use chrome::{WindowChrome, WindowFrame};
pub use event::{Cursor, DragData, Event, EventCtx, Key, Modifiers, MouseButton, NamedKey};
pub use font::{Font, FontFamily, FontSet, FontStyle};
pub use geometry::{Color, Point, Rect, Size};
pub use painter::Painter;
// `include_svg!` reads an SVG at compile time and expands to a const
// `SvgImage`; the runtime side just fills the baked polygons (no SVG parser in
// the binary). The macro emits paths like `::saudade::SvgImage`, so these
// re-exports are also the names its output references.
pub use saudade_macros::include_svg;
pub use svg::{FillRule, SvgImage, SvgPolygon};
pub use theme::Theme;
pub use widget::{PopupKind, PopupRequest, Widget};
pub use widgets::{
    Bevel, Button, Checkbox, Column, Container, Dialog, DialogIcon, Dropdown, FileDialog,
    FileFilter, FocusLabel, Image, Label, List, ListIcon, ListItem, Menu, MenuBar, MenuItem, Modal,
    Orientation, ProgressBar, Row, SCROLLBAR_THICKNESS, ScrollBar, Slider, TextEditor, TextInput,
};
