use crate::geometry::Color;
use crate::painter::Painter;

/// A loaded font, ready for glyph rasterization.
///
/// retrogui owns no bundled bitmap font: we ask the host OS via fontdb for a
/// reasonable proportional sans-serif (MS Sans Serif on Windows, Tahoma /
/// Liberation Sans / DejaVu Sans elsewhere) and rasterize on demand with
/// fontdue. Glyph alpha is blended into the framebuffer.
pub struct Font {
    inner: fontdue::Font,
}

impl Font {
    /// Try to load a system sans-serif font. Returns `None` if no candidate
    /// face could be loaded — text drawing then becomes a no-op.
    pub fn load_system() -> Option<Self> {
        const SANS_FAMILIES: &[&str] = &[
            "MS Sans Serif",
            "Microsoft Sans Serif",
            "Tahoma",
            "Segoe UI",
            "Arial",
            "Helvetica",
            "Geneva",
            "DejaVu Sans",
            "Liberation Sans",
        ];
        load_family_chain(SANS_FAMILIES, false)
    }

    /// Try to load a fixed-width font for plain-text editors / code displays.
    /// Walks the same set of fallbacks Notepad and friends used through the
    /// nineties down to modern Linux replacements.
    pub fn load_monospace() -> Option<Self> {
        const MONO_FAMILIES: &[&str] = &[
            "Lucida Console",
            "Consolas",
            "Courier New",
            "Courier",
            "Liberation Mono",
            "DejaVu Sans Mono",
            "Menlo",
            "Monaco",
        ];
        load_family_chain(MONO_FAMILIES, true)
    }

    /// Measure a single line of text at the given pixel size. Returns
    /// (advance width, em height).
    pub fn measure(&self, text: &str, size: f32) -> (f32, f32) {
        let mut width = 0.0_f32;
        let mut height = 0.0_f32;
        for ch in text.chars() {
            let m = self.inner.metrics(ch, size);
            width += m.advance_width;
            height = height.max(m.height as f32);
        }
        // The font's em height is more visually correct than max glyph height
        // when laying out lines of text. We use size as a proxy and pad a
        // little so descenders fit.
        (width, size * 1.2)
    }

    /// Draw one line of text at *physical* pixel coordinates. The caller
    /// (Painter::text) has already multiplied logical coords and font size by
    /// the DPI scale, so glyphs are rasterized once at their final on-screen
    /// pixel size — no resampling, no upscale blur.
    pub(crate) fn draw_phys(
        &self,
        painter: &mut Painter,
        text: &str,
        x: f32,
        y: f32,
        size_phys: f32,
        color: Color,
    ) -> f32 {
        let baseline = y + size_phys;
        let mut pen_x = x;
        for ch in text.chars() {
            let (metrics, bitmap) = self.inner.rasterize(ch, size_phys);
            let glyph_x = pen_x + metrics.xmin as f32;
            let glyph_y = baseline - metrics.ymin as f32 - metrics.height as f32;
            for row in 0..metrics.height {
                let dy = glyph_y as i32 + row as i32;
                for col in 0..metrics.width {
                    let alpha = bitmap[row * metrics.width + col];
                    if alpha == 0 {
                        continue;
                    }
                    let dx = glyph_x as i32 + col as i32;
                    painter.blend_pixel_phys(dx, dy, color, alpha);
                }
            }
            pen_x += metrics.advance_width;
        }
        pen_x
    }
}

fn load_face(db: &fontdb::Database, id: fontdb::ID) -> Option<fontdue::Font> {
    let mut data: Option<Vec<u8>> = None;
    db.with_face_data(id, |bytes, _| data = Some(bytes.to_vec()));
    let data = data?;
    fontdue::Font::from_bytes(data, fontdue::FontSettings::default()).ok()
}

/// Search `db` for the first family name in `families` that resolves to a
/// loadable face. When `monospace_fallback` is true, after exhausting the
/// named families the search also accepts any face whose face record claims
/// monospace — useful so we don't accidentally drop into a proportional font
/// when none of the well-known mono families are installed.
fn load_family_chain(families: &[&str], monospace_fallback: bool) -> Option<Font> {
    let mut db = fontdb::Database::new();
    db.load_system_fonts();

    for family in families {
        let query = fontdb::Query {
            families: &[fontdb::Family::Name(family)],
            weight: fontdb::Weight::NORMAL,
            stretch: fontdb::Stretch::Normal,
            style: fontdb::Style::Normal,
        };
        if let Some(id) = db.query(&query)
            && let Some(font) = load_face(&db, id)
        {
            return Some(Font { inner: font });
        }
    }

    if monospace_fallback {
        for face in db.faces() {
            if face.monospaced
                && let Some(font) = load_face(&db, face.id)
            {
                return Some(Font { inner: font });
            }
        }
    }

    // Last-ditch: any face we can find. Better something than nothing.
    for face in db.faces() {
        if let Some(font) = load_face(&db, face.id) {
            return Some(Font { inner: font });
        }
    }

    None
}
