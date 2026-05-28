use crate::event::{Event, EventCtx};
use crate::geometry::{Color, Rect, Size};
use crate::painter::Painter;
use crate::theme::Theme;
use crate::widget::Widget;

/// A flat collection of widgets at absolute positions inside a fixed-size area.
///
/// This is the only container retrogui ships with right now: enough for
/// WINGs-style dialog layouts. A constraints/flexbox layout pass can be added
/// later without breaking the Widget trait.
pub struct Container {
    pub size: Size,
    pub background: Option<Color>,
    pub border: Option<Color>,
    children: Vec<Box<dyn Widget>>,
    /// Index of the child currently capturing the pointer, if any.
    captured: Option<usize>,
}

impl Container {
    pub fn new(width: i32, height: i32) -> Self {
        Self {
            size: Size::new(width, height),
            background: None,
            border: None,
            children: Vec::new(),
            captured: None,
        }
    }

    pub fn with_background(mut self, color: Color) -> Self {
        self.background = Some(color);
        self
    }

    pub fn with_border(mut self, color: Color) -> Self {
        self.border = Some(color);
        self
    }

    pub fn add(mut self, widget: impl Widget + 'static) -> Self {
        self.children.push(Box::new(widget));
        self
    }

    pub fn push(&mut self, widget: impl Widget + 'static) {
        self.children.push(Box::new(widget));
    }
}

impl Widget for Container {
    fn bounds(&self) -> Rect {
        Rect::new(0, 0, self.size.w, self.size.h)
    }

    fn paint(&mut self, painter: &mut Painter, theme: &Theme) {
        if let Some(bg) = self.background {
            painter.fill_rect(self.bounds(), bg);
        }
        for child in &mut self.children {
            child.paint(painter, theme);
        }
        if let Some(border) = self.border {
            painter.stroke_rect(self.bounds(), border);
        }
    }

    fn event(&mut self, event: &Event, ctx: &mut EventCtx) {
        // Captured child gets every event until it stops capturing.
        if let Some(idx) = self.captured {
            if let Some(child) = self.children.get_mut(idx) {
                child.event(event, ctx);
                if !child.captures_pointer() {
                    self.captured = None;
                }
            } else {
                self.captured = None;
            }
            return;
        }

        // Otherwise dispatch by hit-test, top-down (most-recently-added first).
        if let Some(pos) = event.position() {
            for (idx, child) in self.children.iter_mut().enumerate().rev() {
                if child.bounds().contains(pos) {
                    child.event(event, ctx);
                    if child.captures_pointer() {
                        self.captured = Some(idx);
                    }
                    return;
                }
            }
        } else {
            // Broadcast non-positional events (e.g. PointerLeave) to all.
            for child in &mut self.children {
                child.event(event, ctx);
            }
        }
    }

    fn captures_pointer(&self) -> bool {
        self.captured.is_some()
    }
}
