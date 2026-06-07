use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use crate::geometry::Color;
use crate::painter::Painter;

/// A loaded font, ready for glyph rasterization.
///
/// saudade owns no bundled bitmap font: we ask the host OS via fontdb for a
/// reasonable proportional sans-serif (MS Sans Serif on Windows, Tahoma /
/// Liberation Sans / DejaVu Sans elsewhere) and rasterize on demand with
/// fontdue. Glyph alpha is blended into the framebuffer.
///
/// Rasterizing an outline into a coverage bitmap is expensive, and a
/// retained-mode window repaints the *entire* visible text on every frame —
/// every scroll notch, every drag-resize step. So both the rasterized glyph
/// bitmaps and the per-glyph advance widths are memoized, keyed by the glyph
/// and the exact pixel size requested. After the first frame the working set
/// (the handful of characters actually on screen, at one or two sizes) is all
/// cache hits, turning each subsequent frame from "rasterize N glyphs" into
/// "blend N cached bitmaps". The caches use interior mutability so drawing can
/// stay `&self` (the painter only ever holds a shared reference to the font).
///
/// The glyph bitmaps are the memory-heavy half, so that cache is LRU-bounded
/// to [`GLYPH_CACHE_CAP`] entries: an app that cycles through many sizes (a
/// smooth zoom) or a large character range (CJK) keeps only the most recently
/// drawn glyphs instead of growing without limit. The advance cache holds a
/// single `f32` per entry, so it stays an unbounded plain map.
pub struct Font {
    inner: fontdue::Font,
    /// Rasterized glyphs, keyed by `(char, physical-size bits)`. The bitmaps
    /// are wrapped in `Rc` so a lookup can hand back a cheap clone and release
    /// the cache borrow before the (longer-lived) blend loop runs. LRU-bounded.
    glyphs: RefCell<LruCache<(char, u32), Rc<Glyph>>>,
    /// Per-glyph advance widths, keyed by `(char, size bits)`. Feeds both text
    /// measurement and the editor's caret-offset table; far cheaper than a full
    /// rasterize when only the advance is needed.
    advances: RefCell<HashMap<(char, u32), f32>>,
}

/// Upper bound on the number of distinct rasterized glyphs kept in memory at
/// once. The on-screen working set is a few hundred at most (printable ASCII
/// across one or two sizes), so this leaves generous headroom while still
/// capping memory when an app renders text at many sizes or over a wide script.
const GLYPH_CACHE_CAP: usize = 1024;

/// One rasterized glyph: fontdue's metrics plus its coverage bitmap.
struct Glyph {
    metrics: fontdue::Metrics,
    bitmap: Vec<u8>,
}

/// A small least-recently-used cache: a plain map plus a monotonic access
/// "clock" stamped on every entry. On overflow the entry with the oldest stamp
/// is evicted. Eviction scans the map (O(capacity)), but it only happens on a
/// miss that fills the cache — and a glyph rasterization, the thing a miss
/// triggers, dwarfs that scan — so the simplicity is worth more than an
/// intrusive-list O(1) variant here.
struct LruCache<K, V> {
    entries: HashMap<K, (V, u64)>,
    clock: u64,
    capacity: usize,
}

impl<K: Eq + std::hash::Hash + Copy, V: Clone> LruCache<K, V> {
    fn new(capacity: usize) -> Self {
        Self {
            entries: HashMap::new(),
            clock: 0,
            capacity: capacity.max(1),
        }
    }

    /// Fetch a value, marking it most-recently-used. Returns a clone so the
    /// caller doesn't hold a borrow of the cache.
    fn get(&mut self, key: &K) -> Option<V> {
        self.clock += 1;
        let stamp = self.clock;
        let slot = self.entries.get_mut(key)?;
        slot.1 = stamp;
        Some(slot.0.clone())
    }

    /// Insert (or overwrite) a value as most-recently-used, evicting the
    /// least-recently-used entry first if a *new* key would exceed capacity.
    fn insert(&mut self, key: K, value: V) {
        self.clock += 1;
        if self.entries.len() >= self.capacity
            && !self.entries.contains_key(&key)
            && let Some(lru) = self
                .entries
                .iter()
                .min_by_key(|(_, (_, stamp))| *stamp)
                .map(|(k, _)| *k)
        {
            self.entries.remove(&lru);
        }
        self.entries.insert(key, (value, self.clock));
    }
}

impl Font {
    fn new(inner: fontdue::Font) -> Self {
        Self {
            inner,
            glyphs: RefCell::new(LruCache::new(GLYPH_CACHE_CAP)),
            advances: RefCell::new(HashMap::new()),
        }
    }

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

    /// Load a font directly from an in-memory TTF/OTF byte buffer. Use this
    /// when you need deterministic glyph output independent of the host's
    /// installed fonts — for example, snapshot tests that bundle the font
    /// they render with via `include_bytes!`.
    pub fn from_bytes(data: Vec<u8>) -> Option<Self> {
        fontdue::Font::from_bytes(data, fontdue::FontSettings::default())
            .ok()
            .map(Self::new)
    }

    /// Cached advance width of a single glyph at `size` pixels. The first call
    /// for a `(char, size)` pair asks fontdue; the rest are map lookups.
    fn advance(&self, ch: char, size: f32) -> f32 {
        let key = (ch, size.to_bits());
        if let Some(a) = self.advances.borrow().get(&key) {
            return *a;
        }
        let a = self.inner.metrics(ch, size).advance_width;
        self.advances.borrow_mut().insert(key, a);
        a
    }

    /// Cached rasterization of a single glyph at `size_phys` physical pixels.
    /// Returns a shared handle so the caller can drop the cache borrow before
    /// iterating the bitmap.
    fn glyph(&self, ch: char, size_phys: f32) -> Rc<Glyph> {
        let key = (ch, size_phys.to_bits());
        if let Some(g) = self.glyphs.borrow_mut().get(&key) {
            return g;
        }
        let (metrics, bitmap) = self.inner.rasterize(ch, size_phys);
        let g = Rc::new(Glyph { metrics, bitmap });
        self.glyphs.borrow_mut().insert(key, g.clone());
        g
    }

    /// Measure a single line of text at the given pixel size. Returns
    /// (advance width, em height). The advance is summed from the per-glyph
    /// cache, so repeated measurements of the same text cost only map lookups.
    pub fn measure(&self, text: &str, size: f32) -> (f32, f32) {
        let width: f32 = text.chars().map(|ch| self.advance(ch, size)).sum();
        // The font's em height is more visually correct than max glyph height
        // when laying out lines of text. We use size as a proxy and pad a
        // little so descenders fit.
        (width, size * 1.2)
    }

    /// Cumulative caret x-offsets for `text` at `size` pixels. `out[i]` is the
    /// logical-pixel x where the caret sits *before* character `i`; `out[len]`
    /// is the end of the string. A single O(n) pass over the per-glyph advance
    /// cache — the value at each step is the running advance sum, ceiled, which
    /// matches the pixel width [`measure`](Self::measure) reports for the
    /// corresponding prefix. Replaces the editor's old O(n²) prefix-remeasure.
    pub fn cumulative_widths(&self, text: &str, size: f32) -> Vec<i32> {
        let mut out = Vec::with_capacity(text.len() + 1);
        let mut acc = 0.0_f32;
        out.push(0);
        for ch in text.chars() {
            acc += self.advance(ch, size);
            out.push(acc.ceil() as i32);
        }
        out
    }

    /// Draw one line of text at *physical* pixel coordinates. The caller
    /// (Painter::text) has already multiplied logical coords and font size by
    /// the DPI scale, so glyphs are rasterized once at their final on-screen
    /// pixel size — no resampling, no upscale blur.
    ///
    /// Glyphs are pulled from the rasterization cache, and any that fall
    /// entirely outside the painter's horizontal clip are skipped: the pen only
    /// advances rightward, so once a glyph starts past the clip's right edge the
    /// rest of the line is off-screen and the loop stops. This keeps a long line
    /// (a 500-column Markdown row) from blending hundreds of invisible glyphs.
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
        let (clip_lo, clip_hi) = painter.glyph_clip_x();
        let mut pen_x = x;
        for ch in text.chars() {
            let glyph = self.glyph(ch, size_phys);
            let metrics = &glyph.metrics;
            let glyph_x = pen_x + metrics.xmin as f32;
            // Everything from here rightward is past the visible span.
            if glyph_x >= clip_hi as f32 {
                break;
            }
            // This glyph ends before the visible span — advance past it without
            // blending (matters when content is scrolled left of the origin).
            if glyph_x + metrics.width as f32 <= clip_lo as f32 {
                pen_x += metrics.advance_width;
                continue;
            }
            let glyph_y = baseline - metrics.ymin as f32 - metrics.height as f32;
            for row in 0..metrics.height {
                let dy = glyph_y as i32 + row as i32;
                let src_row = row * metrics.width;
                for col in 0..metrics.width {
                    let alpha = glyph.bitmap[src_row + col];
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
    // fontdb's fontconfig loader hardcodes /etc/fonts/fonts.conf, but the
    // FreeBSD port installs fontconfig at /usr/local/etc/fonts/fonts.conf —
    // so on a stock FreeBSD desktop load_system_fonts ends up with zero
    // faces. Fall back to the conventional ports font directory.
    if db.faces().next().is_none() {
        db.load_fonts_dir("/usr/local/share/fonts");
    }

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
            return Some(Font::new(font));
        }
    }

    if monospace_fallback {
        for face in db.faces() {
            if face.monospaced
                && let Some(font) = load_face(&db, face.id)
            {
                return Some(Font::new(font));
            }
        }
    }

    // Last-ditch: any face we can find. Better something than nothing.
    for face in db.faces() {
        if let Some(font) = load_face(&db, face.id) {
            return Some(Font::new(font));
        }
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;

    impl<K: Eq + std::hash::Hash + Copy, V: Clone> LruCache<K, V> {
        fn len(&self) -> usize {
            self.entries.len()
        }
        fn contains(&self, key: &K) -> bool {
            self.entries.contains_key(key)
        }
    }

    #[test]
    fn evicts_the_least_recently_used_entry() {
        let mut cache: LruCache<i32, i32> = LruCache::new(2);
        cache.insert(1, 10);
        cache.insert(2, 20);
        // Touch key 1 so key 2 becomes the least-recently-used.
        assert_eq!(cache.get(&1), Some(10));
        // Inserting a third key overflows capacity → evict key 2, keep 1 and 3.
        cache.insert(3, 30);
        assert_eq!(cache.len(), 2);
        assert!(cache.contains(&1));
        assert!(!cache.contains(&2), "the untouched entry is evicted");
        assert!(cache.contains(&3));
    }

    #[test]
    fn overwriting_an_existing_key_never_evicts() {
        let mut cache: LruCache<i32, i32> = LruCache::new(2);
        cache.insert(1, 10);
        cache.insert(2, 20);
        // Re-inserting an existing key updates in place — the cache is full but
        // the key already lives there, so nothing is evicted.
        cache.insert(1, 11);
        assert_eq!(cache.len(), 2);
        assert_eq!(cache.get(&1), Some(11));
        assert!(cache.contains(&2));
    }

    #[test]
    fn a_miss_returns_none() {
        let mut cache: LruCache<i32, i32> = LruCache::new(4);
        assert_eq!(cache.get(&99), None);
    }
}
