//! dnd — drop files from your file manager onto the window. The drop zone
//! highlights while a drag hovers over it and lists the paths that land.
//!
//! File drag-and-drop works on every saudade backend: macOS, Windows and X11
//! through winit, and Wayland through smithay-client-toolkit. On Wayland the
//! highlight tracks the pointer as you drag and the drop reports an exact
//! position; on the winit backends winit doesn't report cursor coordinates
//! during a file drag, so the whole window behaves as a single drop zone.

use std::path::PathBuf;

use saudade::{App, Color, Column, Event, EventCtx, Painter, Rect, Theme, Widget, WindowConfig};

const W: i32 = 460;
const H: i32 = 320;

fn main() {
    let root = Column::new()
        .with_background(Color::WHITE)
        .add_fill(DropZone::new());

    App::new(WindowConfig::new("Drag & Drop", W, H).resizable(true), root)
        .with_theme(Theme::windows_31())
        .run();
}

/// Fills its slot with a sunken panel that floods navy (the Win 3.1 "selected"
/// look) while a file drag hovers, and lists whatever was last dropped on it.
struct DropZone {
    bounds: Rect,
    hovering: bool,
    dropped: Vec<PathBuf>,
}

impl DropZone {
    fn new() -> Self {
        Self {
            bounds: Rect::new(0, 0, W, H),
            hovering: false,
            dropped: Vec::new(),
        }
    }
}

impl Widget for DropZone {
    fn bounds(&self) -> Rect {
        self.bounds
    }

    fn layout(&mut self, bounds: Rect) {
        self.bounds = bounds;
    }

    fn paint(&mut self, p: &mut Painter, theme: &Theme) {
        let area = self.bounds.inset(8);
        let (fill, fg) = if self.hovering {
            (theme.highlight_bg, theme.highlight_text)
        } else {
            (Color::WHITE, theme.text)
        };
        p.fill_rect(area, fill);
        p.sunken_bevel(area, theme.highlight, theme.shadow);

        let inner = area.inset(8);
        if self.dropped.is_empty() {
            p.text_centered(area, "Drop files here", theme.font_size, fg);
            return;
        }

        let line_h = (theme.font_size * 1.5).round() as i32;
        let mut y = inner.y;
        p.text(inner.x, y, "Dropped:", theme.font_size, fg);
        y += line_h;
        for path in &self.dropped {
            if y > inner.bottom() - line_h {
                break;
            }
            p.text(inner.x, y, &path.display().to_string(), theme.font_size, fg);
            y += line_h;
        }
    }

    fn event(&mut self, event: &Event, ctx: &mut EventCtx) {
        match event {
            // A drag is hovering: opt in as a drop target (so the source sees a
            // valid drop here) and light up so the user sees it too.
            Event::DragEnter { .. } | Event::DragMove { .. } => {
                ctx.accept_drop();
                if !self.hovering {
                    self.hovering = true;
                    ctx.request_paint();
                }
            }
            // It left without dropping: back to normal.
            Event::DragLeave => {
                if self.hovering {
                    self.hovering = false;
                    ctx.request_paint();
                }
            }
            // The payload landed: take the paths and stop highlighting.
            Event::Drop { data, .. } => {
                self.hovering = false;
                self.dropped = data.paths.clone();
                ctx.request_paint();
            }
            _ => {}
        }
    }
}
