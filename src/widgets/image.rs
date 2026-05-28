use crate::geometry::{Color, Rect};
use crate::painter::Painter;
use crate::theme::Theme;
use crate::widget::Widget;

/// Static ARGB32 pixel buffer drawn at an absolute position. Alpha == 0 means
/// "transparent — skip the pixel". This is the workhorse for small retro
/// glyphs/logos that you draw procedurally.
pub struct Image {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
    pub pixels: Vec<u32>,
}

impl Image {
    pub fn new(x: i32, y: i32, width: i32, height: i32) -> Self {
        let len = (width.max(0) * height.max(0)) as usize;
        Self {
            x,
            y,
            width,
            height,
            pixels: vec![0; len],
        }
    }

    pub fn from_pixels(x: i32, y: i32, width: i32, height: i32, pixels: Vec<u32>) -> Self {
        debug_assert_eq!(pixels.len(), (width * height) as usize);
        Self {
            x,
            y,
            width,
            height,
            pixels,
        }
    }

    pub fn set_pixel(&mut self, px: i32, py: i32, color: Color) {
        if px < 0 || py < 0 || px >= self.width || py >= self.height {
            return;
        }
        self.pixels[(py * self.width + px) as usize] = color.0;
    }

    pub fn fill_rect(&mut self, rect: Rect, color: Color) {
        let x0 = rect.x.max(0);
        let y0 = rect.y.max(0);
        let x1 = rect.right().min(self.width);
        let y1 = rect.bottom().min(self.height);
        for y in y0..y1 {
            let row = (y * self.width) as usize;
            for x in x0..x1 {
                self.pixels[row + x as usize] = color.0;
            }
        }
    }
}

impl Widget for Image {
    fn bounds(&self) -> Rect {
        Rect::new(self.x, self.y, self.width, self.height)
    }

    fn paint(&mut self, painter: &mut Painter, _theme: &Theme) {
        for py in 0..self.height {
            for px in 0..self.width {
                let color = Color(self.pixels[(py * self.width + px) as usize]);
                if color.alpha() == 0 {
                    continue;
                }
                painter.pixel(self.x + px, self.y + py, color);
            }
        }
    }
}
