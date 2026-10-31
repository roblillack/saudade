//! dnd — drop files from your file manager, or a link or an image out of a
//! web browser, onto the window. The drop zone highlights while a drag hovers
//! over it and then shows what the drop carried, form by form: the files by
//! their path (a folder, or how big a file), the links, and the files that come
//! by their contents — what the bytes are, and a thumbnail of a PNG image.
//!
//! Drag-and-drop works on every saudade backend: macOS, Windows and X11
//! through winit, and Wayland through smithay-client-toolkit. On Wayland,
//! Windows and X11 the highlight tracks the pointer as you drag and the drop
//! reports an exact position; on macOS winit doesn't report cursor
//! coordinates during a file drag, so the whole window behaves as a single
//! drop zone there.

use std::path::Path;

use saudade::{
    App, Color, Column, DragData, Event, EventCtx, FontFamily, FontStyle, Painter, Point, Rect,
    Size, Theme, Widget, WindowConfig,
};

const W: i32 = 480;
const H: i32 = 360;

/// The side of the square a thumbnail is shown in, in logical pixels.
const THUMB: i32 = 48;

/// The most pixels a side of a kept thumbnail has: enough to show it crisply
/// at four times the logical size.
const THUMB_PIXELS: u32 = 4 * THUMB as u32;

fn main() {
    let (config, root) = window();
    App::new(config, root).with_theme(Theme::windows_31()).run();
}

/// The window and the widget tree inside it, kept apart from `main` so
/// `examples/screenshots.rs` can render the same window offscreen.
pub fn window() -> (WindowConfig, impl Widget) {
    let root = Column::new()
        .with_background(Color::WHITE)
        .add_fill(DropZone::new());

    (WindowConfig::new("Drag & Drop", W, H).resizable(true), root)
}

/// Fills its slot with a sunken panel that floods navy (the Win 3.1 "selected"
/// look) while a drag hovers, and shows what was last dropped on it.
struct DropZone {
    bounds: Rect,
    hovering: bool,
    last: Option<Report>,
}

/// What a drop carried and where it landed, worked out once as it lands.
struct Report {
    pos: Point,
    /// One for each form the drop came in: files by path, links, files by
    /// their contents. Only the forms it carried.
    sections: Vec<Section>,
}

struct Section {
    title: String,
    items: Vec<Item>,
}

struct Item {
    /// What it is: a path, an address, a file's name.
    label: String,
    /// More about it, shown under it in gray; empty for nothing more to say.
    detail: String,
    thumb: Option<Thumb>,
}

/// A PNG image, flattened onto white and shrunk to at most [`THUMB_PIXELS`]
/// a side.
struct Thumb {
    w: u32,
    h: u32,
    pixels: Vec<u32>,
}

impl DropZone {
    fn new() -> Self {
        Self {
            bounds: Rect::new(0, 0, W, H),
            hovering: false,
            last: None,
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
        let size = theme.font_size;
        let fill = if self.hovering {
            theme.highlight_bg
        } else {
            Color::WHITE
        };
        p.fill_rect(area, fill);
        p.sunken_bevel(area, theme.highlight, theme.shadow);

        if self.hovering {
            p.text_centered(area, "Release to drop", size, theme.highlight_text);
            return;
        }
        let Some(report) = &self.last else {
            let prompt = "Drop files, links or images here";
            p.text_centered(area, prompt, size, theme.text);
            return;
        };

        let inner = area.inset(8);
        let saved = p.push_clip(inner);
        let line = (size * 1.5).round() as i32;
        let (x, mut y) = (inner.x, inner.y);
        let at = format!("Dropped at ({}, {})", report.pos.x, report.pos.y);
        let headline = if report.sections.is_empty() {
            format!("{at}: nothing that could be read.")
        } else {
            format!("{at}:")
        };
        p.text(x, y, &headline, size, theme.text);
        y += line;
        for section in &report.sections {
            y += line / 3;
            let bold = FontStyle::Bold;
            p.text_styled(
                x,
                y,
                &section.title,
                size,
                theme.text,
                FontFamily::Sans,
                bold,
            );
            y += line;
            for item in &section.items {
                let top = y;
                let mut text_x = x + 16;
                if let Some(thumb) = &item.thumb {
                    draw_thumb(p, thumb, Rect::new(text_x, top, THUMB, THUMB), theme);
                    text_x += THUMB + 8;
                }
                let width = inner.right() - text_x;
                p.text(
                    text_x,
                    y,
                    &elide(p, &item.label, size, width),
                    size,
                    theme.text,
                );
                y += line;
                if !item.detail.is_empty() {
                    let detail = elide(p, &item.detail, size, width);
                    p.text(text_x, y, &detail, size, theme.disabled_text);
                    y += line;
                }
                if item.thumb.is_some() {
                    y = y.max(top + THUMB + line / 3);
                }
            }
        }
        p.restore_clip(saved);
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
            // The payload landed: work out what it is and stop highlighting.
            Event::Drop { pos, data } => {
                self.hovering = false;
                self.last = Some(Report::new(*pos, data));
                ctx.request_paint();
            }
            _ => {}
        }
    }
}

impl Report {
    fn new(pos: Point, data: &DragData) -> Self {
        let mut sections = Vec::new();
        let mut section = |title: &str, items: Vec<Item>| {
            if !items.is_empty() {
                let title = format!("{title} ({})", items.len());
                sections.push(Section { title, items });
            }
        };
        let paths = data.paths.iter().map(|path| Item {
            label: path.display().to_string(),
            detail: describe_path(path),
            thumb: None,
        });
        section("Files by path", paths.collect());
        let links = data.urls.iter().map(|url| Item {
            label: url.clone(),
            detail: describe_link(url),
            thumb: None,
        });
        section("Links", links.collect());
        let files = data.files.iter().map(|file| Item {
            label: file.name.clone(),
            detail: describe_contents(&file.contents),
            thumb: thumbnail(&file.contents),
        });
        section("Files by contents", files.collect());
        Report { pos, sections }
    }
}

/// A file by its path: a folder, or how big a file.
fn describe_path(path: &Path) -> String {
    match std::fs::metadata(path) {
        Ok(meta) if meta.is_dir() => "Folder".to_string(),
        Ok(meta) => size(meta.len()),
        // Gone already, or not ours to look at: the path is all there is.
        Err(_) => String::new(),
    }
}

/// A link: what a `data:` address holds and how much, as the address itself
/// says little.
fn describe_link(url: &str) -> String {
    match url.split_once(',') {
        Some((head, body)) if head.starts_with("data:") => {
            let kind = head["data:".len()..].split(';').next().unwrap_or("");
            let kind = if kind.is_empty() { "text/plain" } else { kind };
            format!("{kind} data, {} encoded", size(body.len() as u64))
        }
        _ => String::new(),
    }
}

/// What a file's bytes are, as far as their first few tell, and how many.
fn describe_contents(bytes: &[u8]) -> String {
    format!("{} · {}", kind(bytes), size(bytes.len() as u64))
}

/// What `bytes` are: an image or document by its magic number, else text by
/// its first line, else the first few bytes in hex.
fn kind(bytes: &[u8]) -> String {
    const MAGIC: &[(&[u8], &str)] = &[
        (b"\x89PNG\r\n\x1a\n", "PNG image"),
        (b"\xff\xd8\xff", "JPEG image"),
        (b"GIF8", "GIF image"),
        (b"BM", "BMP image"),
        (b"%PDF-", "PDF document"),
        (b"PK\x03\x04", "ZIP archive"),
    ];
    if bytes.starts_with(MAGIC[0].0) && bytes.len() >= 24 {
        // The size is the first thing in the header chunk.
        let w = u32::from_be_bytes(bytes[16..20].try_into().unwrap());
        let h = u32::from_be_bytes(bytes[20..24].try_into().unwrap());
        return format!("PNG image, {w} × {h}");
    }
    if let Some((_, kind)) = MAGIC.iter().find(|(magic, _)| bytes.starts_with(magic)) {
        return kind.to_string();
    }
    if bytes.len() >= 12 && bytes.starts_with(b"RIFF") && &bytes[8..12] == b"WEBP" {
        return "WebP image".to_string();
    }
    match std::str::from_utf8(bytes) {
        Ok(text) => {
            let first = text.lines().map(str::trim).find(|l| !l.is_empty());
            let first: String = first.unwrap_or("").chars().take(40).collect();
            format!("Text, “{first}”")
        }
        Err(_) => {
            let hex: Vec<String> = bytes.iter().take(6).map(|b| format!("{b:02x}")).collect();
            format!("Bytes {} …", hex.join(" "))
        }
    }
}

/// `bytes` bytes, in the unit that reads best.
fn size(bytes: u64) -> String {
    const KB: f64 = 1024.0;
    match bytes {
        1 => "1 byte".to_string(),
        0..1024 => format!("{bytes} bytes"),
        _ if (bytes as f64) < KB * KB => format!("{:.1} KB", bytes as f64 / KB),
        _ => format!("{:.1} MB", bytes as f64 / (KB * KB)),
    }
}

/// `bytes` decoded as a PNG image, if they are one, for a thumbnail.
fn thumbnail(bytes: &[u8]) -> Option<Thumb> {
    let mut decoder = png::Decoder::new(bytes);
    // 8 bits per sample, palettes expanded: gray, gray + alpha, RGB or RGBA.
    decoder.set_transformations(png::Transformations::normalize_to_color8());
    let mut reader = decoder.read_info().ok()?;
    let mut buf = vec![0; reader.output_buffer_size()];
    let frame = reader.next_frame(&mut buf).ok()?;
    let (sw, sh) = (frame.width, frame.height);
    let samples = frame.color_type.samples();
    // A source pixel, as RGB over white.
    let rgb = |x: u32, y: u32| -> [f32; 3] {
        let at = y as usize * frame.line_size + x as usize * samples;
        let px = &buf[at..at + samples];
        let (color, alpha) = match samples {
            1 => ([px[0]; 3], 255),
            2 => ([px[0]; 3], px[1]),
            3 => ([px[0], px[1], px[2]], 255),
            _ => ([px[0], px[1], px[2]], px[3]),
        };
        let a = alpha as f32 / 255.0;
        color.map(|c| c as f32 * a + 255.0 * (1.0 - a))
    };
    // Shrunk by averaging the source pixels under each thumbnail pixel.
    let fit = (THUMB_PIXELS as f32 / sw.max(sh) as f32).min(1.0);
    let w = ((sw as f32 * fit).round() as u32).max(1);
    let h = ((sh as f32 * fit).round() as u32).max(1);
    let mut pixels = Vec::with_capacity((w * h) as usize);
    for ty in 0..h {
        let (y0, y1) = (ty * sh / h, ((ty + 1) * sh / h).max(ty * sh / h + 1));
        for tx in 0..w {
            let (x0, x1) = (tx * sw / w, ((tx + 1) * sw / w).max(tx * sw / w + 1));
            let mut sum = [0.0; 3];
            for y in y0..y1 {
                for x in x0..x1 {
                    for (s, c) in sum.iter_mut().zip(rgb(x, y)) {
                        *s += c;
                    }
                }
            }
            let n = ((x1 - x0) * (y1 - y0)) as f32;
            let [r, g, b] = sum.map(|s| (s / n).round() as u32);
            pixels.push(0xff00_0000 | r << 16 | g << 8 | b);
        }
    }
    Some(Thumb { w, h, pixels })
}

/// Show `thumb` as big as fits in `square`, framed, and centered in it.
fn draw_thumb(p: &mut Painter, thumb: &Thumb, square: Rect, theme: &Theme) {
    let longest = thumb.w.max(thumb.h) as f32;
    let w = ((thumb.w as f32 * square.w as f32 / longest).round() as i32).max(1);
    let h = ((thumb.h as f32 * square.h as f32 / longest).round() as i32).max(1);
    let area = Rect::new(
        square.x + (square.w - w) / 2,
        square.y + (square.h - h) / 2,
        w,
        h,
    );
    // Resampled to the device pixels the area covers, crisp at any scale.
    let content = Size::new(thumb.w as i32, thumb.h as i32);
    p.draw_resampled(area, content, 1.0, Color::WHITE, |p| {
        p.blit_argb(0, 0, thumb.w, thumb.h, &thumb.pixels);
    });
    let frame = Rect::new(area.x - 1, area.y - 1, area.w + 2, area.h + 2);
    p.stroke_rect(frame, theme.shadow);
}

/// `text` cut down to fit `width`, an ellipsis in place of its middle: the end
/// of a path or an address says the most.
fn elide(p: &Painter, text: &str, size: f32, width: i32) -> String {
    if p.measure_text(text, size).w <= width {
        return text.to_string();
    }
    let chars: Vec<char> = text.chars().collect();
    let cut = |keep: usize| -> String {
        let head: String = chars[..keep.div_ceil(2)].iter().collect();
        let tail: String = chars[chars.len() - keep / 2..].iter().collect();
        format!("{head}…{tail}")
    };
    // The most characters that fit, found by halving: a `data:` address can
    // run to megabytes.
    let (mut fits, mut over) = (0, chars.len());
    while over - fits > 1 {
        let mid = (fits + over) / 2;
        if p.measure_text(&cut(mid), size).w <= width {
            fits = mid;
        } else {
            over = mid;
        }
    }
    cut(fits)
}

// ============================================================================
// Tests — what a drop is described as is plain data, so it runs headless. Run
// with `cargo test --example dnd`.
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use saudade::DraggedFile;
    use std::path::PathBuf;

    /// A `w` × `h` RGBA image, all of it `rgba`, as PNG.
    fn png(w: u32, h: u32, rgba: [u8; 4]) -> Vec<u8> {
        let mut out = Vec::new();
        let mut encoder = png::Encoder::new(&mut out, w, h);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let pixels: Vec<u8> = (0..w * h).flat_map(|_| rgba).collect();
        let mut writer = encoder.write_header().unwrap();
        writer.write_image_data(&pixels).unwrap();
        writer.finish().unwrap();
        out
    }

    #[test]
    fn sorts_a_drop_into_its_forms() {
        let data = DragData {
            paths: vec![PathBuf::from("/nowhere/a.txt")],
            urls: vec!["https://example.com/a.png".into()],
            files: vec![DraggedFile {
                name: "a.png".into(),
                contents: png(2, 1, [255, 0, 0, 255]),
            }],
        };
        let report = Report::new(Point::new(3, 4), &data);
        let titles: Vec<&str> = report.sections.iter().map(|s| s.title.as_str()).collect();
        assert_eq!(
            titles,
            ["Files by path (1)", "Links (1)", "Files by contents (1)"]
        );
        let file = &report.sections[2].items[0];
        assert_eq!(file.label, "a.png");
        assert!(
            file.detail.starts_with("PNG image, 2 × 1 · "),
            "{}",
            file.detail
        );
        assert!(file.thumb.is_some());
        // A drop of nothing readable has nothing to show.
        let empty = Report::new(Point::new(0, 0), &DragData::default());
        assert!(empty.sections.is_empty());
    }

    #[test]
    fn tells_what_bytes_are() {
        assert_eq!(kind(b"\xff\xd8\xff\xe0rest"), "JPEG image");
        assert_eq!(kind(b"RIFF\0\0\0\0WEBPVP8 "), "WebP image");
        assert_eq!(kind(b"\n  Hello, world\nmore"), "Text, “Hello, world”");
        assert_eq!(kind(b"\x00\x01\xfe\xff"), "Bytes 00 01 fe ff …");
        assert_eq!(size(1), "1 byte");
        assert_eq!(size(1023), "1023 bytes");
        assert_eq!(size(1536), "1.5 KB");
        assert_eq!(size(3 << 20), "3.0 MB");
        assert_eq!(
            describe_link("data:image/png;base64,AAAA"),
            "image/png data, 4 bytes encoded"
        );
        assert_eq!(describe_link("https://example.com/"), "");
    }

    #[test]
    fn shrinks_an_image_onto_white() {
        // Half-transparent black over white is mid gray, and a big image is
        // shrunk to fit, its proportions kept.
        let thumb = thumbnail(&png(400, 100, [0, 0, 0, 128])).unwrap();
        assert_eq!((thumb.w, thumb.h), (THUMB_PIXELS, THUMB_PIXELS / 4));
        let gray = 0xff7f7f7f;
        assert!(
            thumb.pixels.iter().all(|&p| p == gray),
            "{:x}",
            thumb.pixels[0]
        );
        assert!(thumbnail(b"not a png").is_none());
    }
}
