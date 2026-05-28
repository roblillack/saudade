#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Point {
    pub x: i32,
    pub y: i32,
}

impl Point {
    pub const fn new(x: i32, y: i32) -> Self {
        Self { x, y }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Size {
    pub w: i32,
    pub h: i32,
}

impl Size {
    pub const fn new(w: i32, h: i32) -> Self {
        Self { w, h }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

impl Rect {
    pub const fn new(x: i32, y: i32, w: i32, h: i32) -> Self {
        Self { x, y, w, h }
    }

    pub const fn from_xywh(x: i32, y: i32, w: i32, h: i32) -> Self {
        Self::new(x, y, w, h)
    }

    pub fn contains(&self, p: Point) -> bool {
        p.x >= self.x && p.x < self.x + self.w && p.y >= self.y && p.y < self.y + self.h
    }

    pub fn inset(&self, n: i32) -> Self {
        Self {
            x: self.x + n,
            y: self.y + n,
            w: (self.w - 2 * n).max(0),
            h: (self.h - 2 * n).max(0),
        }
    }

    pub fn right(&self) -> i32 {
        self.x + self.w
    }

    pub fn bottom(&self) -> i32 {
        self.y + self.h
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Color(pub u32);

impl Color {
    pub const fn argb(a: u8, r: u8, g: u8, b: u8) -> Self {
        Color(((a as u32) << 24) | ((r as u32) << 16) | ((g as u32) << 8) | b as u32)
    }

    pub const fn rgb(r: u8, g: u8, b: u8) -> Self {
        Self::argb(0xFF, r, g, b)
    }

    pub const fn alpha(self) -> u8 {
        (self.0 >> 24) as u8
    }

    pub const fn red(self) -> u8 {
        (self.0 >> 16) as u8
    }

    pub const fn green(self) -> u8 {
        (self.0 >> 8) as u8
    }

    pub const fn blue(self) -> u8 {
        self.0 as u8
    }

    pub const TRANSPARENT: Color = Color(0);
    pub const BLACK: Color = Color::rgb(0x00, 0x00, 0x00);
    pub const WHITE: Color = Color::rgb(0xFF, 0xFF, 0xFF);
    pub const LIGHT_GRAY: Color = Color::rgb(0xC0, 0xC0, 0xC0);
    pub const MID_GRAY: Color = Color::rgb(0x80, 0x80, 0x80);
    pub const DARK_GRAY: Color = Color::rgb(0x40, 0x40, 0x40);
    pub const NAVY: Color = Color::rgb(0x00, 0x00, 0x80);
    pub const RED: Color = Color::rgb(0xCC, 0x00, 0x00);
    pub const GREEN: Color = Color::rgb(0x00, 0xA0, 0x00);
    pub const YELLOW: Color = Color::rgb(0xCC, 0xCC, 0x00);
}
