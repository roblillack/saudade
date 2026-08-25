use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use crate::geometry::Color;
use crate::painter::Painter;

/// Which face of a font to draw with.
///
/// saudade renders text in real OS-installed faces — when an app asks for
/// **bold** or *italic*, the host's actual bold / italic / bold-italic face of
/// the same family is rasterized, never a synthesized (smeared / sheared)
/// approximation of the regular face. A style the system has no face for falls
/// back to the nearest *real* face the family does provide (see
/// [`Font::load_sans`] and `resolve_style`).
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
pub enum FontStyle {
    /// The upright, regular-weight face — what `Painter::text` uses.
    #[default]
    Regular = 0,
    /// Bold weight, upright.
    Bold = 1,
    /// Regular weight, italic (or oblique) slant.
    Italic = 2,
    /// Bold weight *and* italic slant.
    BoldItalic = 3,
}

impl FontStyle {
    /// The style for a `(bold, italic)` flag pair — handy when a renderer tracks
    /// the two emphases independently and needs the combined face.
    pub fn new(bold: bool, italic: bool) -> Self {
        match (bold, italic) {
            (false, false) => FontStyle::Regular,
            (true, false) => FontStyle::Bold,
            (false, true) => FontStyle::Italic,
            (true, true) => FontStyle::BoldItalic,
        }
    }

    /// Whether this style carries bold weight.
    pub fn is_bold(self) -> bool {
        matches!(self, FontStyle::Bold | FontStyle::BoldItalic)
    }

    /// Whether this style carries an italic / oblique slant.
    pub fn is_italic(self) -> bool {
        matches!(self, FontStyle::Italic | FontStyle::BoldItalic)
    }
}

/// Which family of font to draw with: a proportional sans-serif, a proportional
/// serif, or a fixed-width monospace. The styled draw / measure entry points take
/// one of these alongside a [`FontStyle`]; the plain `text` / `measure_text`
/// default to [`Sans`](FontFamily::Sans).
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
pub enum FontFamily {
    /// Proportional sans-serif — the UI default.
    #[default]
    Sans,
    /// Proportional serif.
    Serif,
    /// Fixed-width monospace.
    Mono,
}

/// The set of font families a [`Painter`](crate::Painter) draws with — a
/// sans-serif, a serif, and a monospace face, each selectable per draw via
/// [`FontFamily`]. Any family may be absent (`None`): drawing or measuring in a
/// missing one is a no-op / zero. The painter borrows these for a paint pass;
/// the backend owns the `Font`s. Build it field-by-field (`FontSet { sans: ...,
/// ..Default::default() }`) or start from [`FontSet::default`] (all empty).
#[derive(Clone, Copy, Default)]
pub struct FontSet<'a> {
    /// Proportional sans-serif — the default for `text` / `measure_text`.
    pub sans: Option<&'a Font>,
    /// Proportional serif.
    pub serif: Option<&'a Font>,
    /// Fixed-width monospace — used by the text editors and by `text_styled`
    /// with [`FontFamily::Mono`].
    pub mono: Option<&'a Font>,
}

/// The number of distinct faces a [`Font`] can hold — one per [`FontStyle`].
pub(crate) const STYLE_COUNT: usize = 4;

/// Pick the [`FontStyle`] to actually draw for a requested one, given which
/// faces are present (`present[s as usize]`). The regular face is always loaded,
/// so this always resolves to a real, loaded face — we never fake an absent
/// bold/italic by transforming the regular glyph. A missing bold-italic prefers
/// to keep as much of the request as a real face allows (bold, then italic)
/// before giving up to regular.
fn resolve_style(present: [bool; STYLE_COUNT], style: FontStyle) -> FontStyle {
    use FontStyle::*;
    let has = |s: FontStyle| present[s as usize];
    match style {
        Regular => Regular,
        Bold => {
            if has(Bold) {
                Bold
            } else {
                Regular
            }
        }
        Italic => {
            if has(Italic) {
                Italic
            } else {
                Regular
            }
        }
        BoldItalic => {
            if has(BoldItalic) {
                BoldItalic
            } else if has(Bold) {
                Bold
            } else if has(Italic) {
                Italic
            } else {
                Regular
            }
        }
    }
}

/// A loaded font family, ready for glyph rasterization in any of its
/// [`FontStyle`]s.
///
/// saudade owns no bundled bitmap font: we ask the host OS via fontdb for a
/// reasonable proportional sans-serif (MS Sans Serif on Windows, Tahoma /
/// Liberation Sans / DejaVu Sans elsewhere) and rasterize on demand with
/// fontdue. Glyph alpha is blended into the framebuffer. For emphasis we load
/// the family's *real* bold, italic, and bold-italic faces alongside the regular
/// one — bold text is the system's bold face, not the regular face smeared a
/// pixel wider. The bold/italic faces are optional: a family the host ships
/// without them simply falls back to the regular face (see `resolve_style`).
///
/// Rasterizing an outline into a coverage bitmap is expensive, and a
/// retained-mode window repaints the *entire* visible text on every frame —
/// every scroll notch, every drag-resize step. So both the rasterized glyph
/// bitmaps and the per-glyph advance widths are memoized, keyed by the glyph,
/// the exact pixel size requested, *and* the style. After the first frame the
/// working set (the handful of characters actually on screen, at one or two
/// sizes) is all cache hits, turning each subsequent frame from "rasterize N
/// glyphs" into "blend N cached bitmaps". The caches use interior mutability so
/// drawing can stay `&self` (the painter only ever holds a shared reference to
/// the font).
///
/// The glyph bitmaps are the memory-heavy half, so that cache is LRU-bounded
/// to [`GLYPH_CACHE_CAP`] entries: an app that cycles through many sizes (a
/// smooth zoom) or a large character range (CJK) keeps only the most recently
/// drawn glyphs instead of growing without limit. The advance cache holds a
/// single `f32` per entry, so it stays an unbounded plain map.
pub struct Font {
    /// Where glyphs come from: font files parsed by fontdue, or the host's own
    /// text stack.
    faces: Faces,
    /// Rasterized glyphs, keyed by `(char, physical-size bits, style)`. The
    /// bitmaps are wrapped in `Rc` so a lookup can hand back a cheap clone and
    /// release the cache borrow before the (longer-lived) blend loop runs.
    /// LRU-bounded. The `style` in the key is always a *resolved* style, so two
    /// requests that fall back to the same real face share a cache entry.
    glyphs: RefCell<LruCache<GlyphKey, Rc<Glyph>>>,
    /// Per-glyph advance widths, keyed by `(char, size bits, style)`. Feeds both
    /// text measurement and the editor's caret-offset table; far cheaper than a
    /// full rasterize when only the advance is needed.
    advances: RefCell<HashMap<AdvanceKey, f32>>,
}

/// Cache key: a glyph at a logical size and DPI scale, in a resolved style.
/// Sizes are `f32::to_bits`, so the key stays hashable. The scale is part of it
/// because a glyph is rasterized at `size * scale` — the same 13px label is a
/// different bitmap on a Retina display — and because a face may draw a size
/// differently depending on the scale it is asked for it at (see
/// [`crate::coretext`]).
type GlyphKey = (char, u32, u32, FontStyle);

/// Cache key for an advance width, which depends only on the logical size:
/// advances scale linearly with the DPI, so there is one entry per size.
type AdvanceKey = (char, u32, FontStyle);

/// Upper bound on the number of distinct rasterized glyphs kept in memory at
/// once. The on-screen working set is a few hundred at most (printable ASCII
/// across one or two sizes and styles), so this leaves generous headroom while
/// still capping memory when an app renders text at many sizes or over a wide
/// script.
const GLYPH_CACHE_CAP: usize = 1024;

/// One rasterized glyph: where to put it, and its coverage bitmap.
struct Glyph {
    metrics: GlyphMetrics,
    bitmap: Vec<u8>,
}

/// What the blend loop needs to place a rasterized glyph: the bitmap's size,
/// where it sits relative to the pen and the baseline, and how far the pen moves
/// afterwards. This is the subset of `fontdue::Metrics` this crate ever reads,
/// named in its own right so a platform rasterizer can fill it in without
/// pretending to be fontdue.
#[derive(Clone, Copy, Default)]
pub(crate) struct GlyphMetrics {
    /// Bitmap size in pixels.
    pub(crate) width: usize,
    pub(crate) height: usize,
    /// Offset from the pen to the bitmap's left edge.
    pub(crate) xmin: i32,
    /// Offset from the baseline *up* to the bitmap's bottom edge — negative for
    /// a glyph that descends below it.
    pub(crate) ymin: i32,
    /// How far the pen advances after drawing.
    pub(crate) advance: f32,
}

impl From<fontdue::Metrics> for GlyphMetrics {
    fn from(m: fontdue::Metrics) -> Self {
        Self {
            width: m.width,
            height: m.height,
            xmin: m.xmin,
            ymin: m.ymin,
            advance: m.advance_width,
        }
    }
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

/// Where a [`Font`]'s glyphs come from.
///
/// Two sources answer the same three questions — which styles are real, how wide
/// is a glyph, what does it look like — so everything above this enum (the
/// caches, measurement, the blend loop) is written once.
enum Faces {
    /// Faces parsed out of font files by fontdue, indexed by `FontStyle as
    /// usize`. Slot 0 (`Regular`) is always `Some`; the bold / italic /
    /// bold-italic slots are `Some` only when the host actually provides that
    /// face. This is the path for fonts an app supplies as bytes, and for every
    /// platform whose own text stack we don't speak.
    Outlines(Box<[Option<fontdue::Font>; STYLE_COUNT]>),
    /// The host's text stack, asked for a system font by role rather than for a
    /// file. See [`crate::coretext`] for why a Mac needs this to draw a bold
    /// heading at all.
    #[cfg(target_os = "macos")]
    Native(crate::coretext::Family),
}

impl Faces {
    /// Which styles have a real face behind them — drives [`resolve_style`], so
    /// a style with no face falls back to one that exists instead of being
    /// quietly drawn as regular.
    fn presence(&self) -> [bool; STYLE_COUNT] {
        match self {
            Faces::Outlines(faces) => std::array::from_fn(|i| faces[i].is_some()),
            #[cfg(target_os = "macos")]
            Faces::Native(family) => family.presence(),
        }
    }

    /// Advance width of one glyph at `size` pixels in an already-resolved style.
    fn advance(&self, ch: char, size: f32, style: FontStyle) -> f32 {
        match self {
            Faces::Outlines(faces) => match &faces[style as usize] {
                Some(face) => face.metrics(ch, size).advance_width,
                None => 0.0,
            },
            #[cfg(target_os = "macos")]
            Faces::Native(family) => family.advance(ch, size, style),
        }
    }

    /// Rasterize one glyph for a `size`-logical-pixel run drawn at DPI `scale`,
    /// in an already-resolved style: an 8-bit coverage bitmap, top row first,
    /// `size * scale` pixels tall in round numbers. Its metrics are in physical
    /// pixels, and its advance is exactly `scale` times the logical advance
    /// [`Faces::advance`] reports, so measurement and drawing agree at any DPI.
    fn rasterize(
        &self,
        ch: char,
        size: f32,
        scale: f32,
        style: FontStyle,
    ) -> (GlyphMetrics, Vec<u8>) {
        match self {
            Faces::Outlines(faces) => match &faces[style as usize] {
                // An outline face has one shape at every size, so the physical
                // size is all it needs.
                Some(face) => {
                    let (metrics, bitmap) = face.rasterize(ch, size * scale);
                    (metrics.into(), bitmap)
                }
                None => (GlyphMetrics::default(), Vec::new()),
            },
            #[cfg(target_os = "macos")]
            Faces::Native(family) => family.rasterize(ch, size, scale, style),
        }
    }

    /// The outline faces, when that is what this font is made of. The
    /// `with_*_bytes` builders attach faces to a font built from bytes; there is
    /// nothing to attach them to on a font backed by the host's text stack.
    fn outlines_mut(&mut self) -> Option<&mut [Option<fontdue::Font>; STYLE_COUNT]> {
        match self {
            Faces::Outlines(faces) => Some(&mut **faces),
            #[cfg(target_os = "macos")]
            Faces::Native(_) => None,
        }
    }
}

impl Font {
    /// Build a font from just a regular face. Bold / italic slots start empty;
    /// attach them with [`with_bold_bytes`](Self::with_bold_bytes) and friends,
    /// or let the system loader fill them in.
    fn from_face(regular: fontdue::Font) -> Self {
        Self::from_faces(Faces::Outlines(Box::new([Some(regular), None, None, None])))
    }

    fn from_faces(faces: Faces) -> Self {
        Self {
            faces,
            glyphs: RefCell::new(LruCache::new(GLYPH_CACHE_CAP)),
            advances: RefCell::new(HashMap::new()),
        }
    }

    /// Try to load a system sans-serif font, with its bold / italic / bold-italic
    /// faces. Returns `None` if no candidate family could be loaded — text
    /// drawing then becomes a no-op.
    pub fn load_sans() -> Option<Self> {
        #[cfg(target_os = "macos")]
        if let Some(family) = crate::coretext::Family::system() {
            return Some(Self::from_faces(Faces::Native(family)));
        }
        load_family_chain(SANS_FAMILIES, false)
    }

    /// Try to load a system serif font, with its bold / italic / bold-italic
    /// faces. Returns `None` if no candidate family could be loaded.
    pub fn load_serif() -> Option<Self> {
        // There is no interface *serif* to ask for by role, so the named chain
        // does the choosing either way — Core Text just draws it better, and can
        // instantiate a variable family's weights (New York) where a font
        // database sees only one face.
        #[cfg(target_os = "macos")]
        if let Some(font) = pick_family(SERIF_FAMILIES, |family| {
            crate::coretext::Family::named(family).map(|f| Self::from_faces(Faces::Native(f)))
        }) {
            return Some(font);
        }
        load_family_chain(SERIF_FAMILIES, false)
    }

    /// Try to load a fixed-width font (with its styled faces) for plain-text
    /// editors / code displays. Returns `None` if no candidate family could be
    /// loaded.
    pub fn load_monospace() -> Option<Self> {
        // The user's own fixed-pitch preference, which is what a Mac's own text
        // views are set in.
        #[cfg(target_os = "macos")]
        if let Some(family) = crate::coretext::Family::fixed_pitch() {
            return Some(Self::from_faces(Faces::Native(family)));
        }
        load_family_chain(MONO_FAMILIES, true)
    }

    /// Load a font family from an in-memory TTF/OTF byte buffer, used as its
    /// regular (sans) face. Use this when you need deterministic glyph output
    /// independent of the host's installed fonts — for example, snapshot tests
    /// that bundle the font they render with via `include_bytes!`. The result
    /// has only a regular face; attach the emphasis faces with
    /// [`with_bold_bytes`](Self::with_bold_bytes),
    /// [`with_italic_bytes`](Self::with_italic_bytes), and
    /// [`with_bold_italic_bytes`](Self::with_bold_italic_bytes).
    pub fn from_sans_bytes(data: Vec<u8>) -> Option<Self> {
        fontdue::Font::from_bytes(data, fontdue::FontSettings::default())
            .ok()
            .map(Self::from_face)
    }

    /// Attach a bold face from an in-memory TTF/OTF buffer (builder style).
    /// A buffer that fails to parse leaves the slot empty (bold then falls back
    /// to the regular face). Pairs with [`from_sans_bytes`](Self::from_sans_bytes)
    /// for deterministic, font-bundling snapshot tests.
    pub fn with_bold_bytes(self, data: Vec<u8>) -> Self {
        self.with_style_bytes(FontStyle::Bold, data)
    }

    /// Attach an italic (or oblique) face from an in-memory buffer. See
    /// [`with_bold_bytes`](Self::with_bold_bytes).
    pub fn with_italic_bytes(self, data: Vec<u8>) -> Self {
        self.with_style_bytes(FontStyle::Italic, data)
    }

    /// Attach a bold-italic face from an in-memory buffer. See
    /// [`with_bold_bytes`](Self::with_bold_bytes).
    pub fn with_bold_italic_bytes(self, data: Vec<u8>) -> Self {
        self.with_style_bytes(FontStyle::BoldItalic, data)
    }

    fn with_style_bytes(mut self, style: FontStyle, data: Vec<u8>) -> Self {
        if let (Some(faces), Ok(face)) = (
            self.faces.outlines_mut(),
            fontdue::Font::from_bytes(data, fontdue::FontSettings::default()),
        ) {
            faces[style as usize] = Some(face);
        }
        self
    }

    /// Which styles have a face loaded — drives [`resolve_style`].
    fn presence(&self) -> [bool; STYLE_COUNT] {
        self.faces.presence()
    }

    /// The style to actually draw a requested one in: an absent bold / italic
    /// resolves down to a style that exists (ultimately the always-present
    /// regular one), and that resolved style is what the caches are keyed on.
    fn resolved(&self, style: FontStyle) -> FontStyle {
        resolve_style(self.presence(), style)
    }

    /// Cached advance width of a single glyph at `size` pixels in `style`. The
    /// first call for a `(char, size, style)` triple asks fontdue; the rest are
    /// map lookups.
    fn advance(&self, ch: char, size: f32, style: FontStyle) -> f32 {
        let resolved = self.resolved(style);
        let key = (ch, size.to_bits(), resolved);
        if let Some(a) = self.advances.borrow().get(&key) {
            return *a;
        }
        let a = self.faces.advance(ch, size, resolved);
        self.advances.borrow_mut().insert(key, a);
        a
    }

    /// Cached rasterization of one glyph of a `size`-logical-pixel run drawn at
    /// DPI `scale`. Returns a shared handle so the caller can drop the cache
    /// borrow before iterating the bitmap.
    fn glyph(&self, ch: char, size: f32, scale: f32, style: FontStyle) -> Rc<Glyph> {
        let resolved = self.resolved(style);
        let key = (ch, size.to_bits(), scale.to_bits(), resolved);
        if let Some(g) = self.glyphs.borrow_mut().get(&key) {
            return g;
        }
        let (metrics, bitmap) = self.faces.rasterize(ch, size, scale, resolved);
        let g = Rc::new(Glyph { metrics, bitmap });
        self.glyphs.borrow_mut().insert(key, g.clone());
        g
    }

    /// Measure a single line of regular-weight text at the given pixel size.
    /// Returns (advance width, em height). See [`measure_styled`](Self::measure_styled).
    pub fn measure(&self, text: &str, size: f32) -> (f32, f32) {
        self.measure_styled(text, size, FontStyle::Regular)
    }

    /// Measure a single line of text drawn in `style` at the given pixel size.
    /// Returns (advance width, em height). The advance is summed from the
    /// per-glyph cache, so repeated measurements of the same text cost only map
    /// lookups. Bold and italic faces have their own metrics, so measuring with
    /// the same style the text is drawn in keeps wrapping pixel-accurate.
    pub fn measure_styled(&self, text: &str, size: f32, style: FontStyle) -> (f32, f32) {
        let width: f32 = text.chars().map(|ch| self.advance(ch, size, style)).sum();
        // The font's em height is more visually correct than max glyph height
        // when laying out lines of text. We use size as a proxy and pad a
        // little so descenders fit.
        (width, size * 1.2)
    }

    /// Cumulative caret x-offsets for `text` at `size` pixels in `style`.
    /// `out[i]` is the logical-pixel x where the caret sits *before* character
    /// `i`; `out[len]` is the end of the string. A single O(n) pass over the
    /// per-glyph advance cache — the value at each step is the running advance
    /// sum, ceiled, which matches the pixel width
    /// [`measure_styled`](Self::measure_styled) reports for the corresponding
    /// prefix. Replaces the editor's old O(n²) prefix-remeasure.
    pub fn cumulative_widths(&self, text: &str, size: f32, style: FontStyle) -> Vec<i32> {
        let mut out = Vec::with_capacity(text.len() + 1);
        let mut acc = 0.0_f32;
        out.push(0);
        for ch in text.chars() {
            acc += self.advance(ch, size, style);
            out.push(acc.ceil() as i32);
        }
        out
    }

    /// Draw one line of text in `style` at *physical* pixel coordinates. The
    /// caller (Painter::text) has already multiplied logical coords and font
    /// size by the DPI scale, so glyphs are rasterized once at their final
    /// on-screen pixel size — no resampling, no upscale blur.
    ///
    /// Glyphs are pulled from the rasterization cache, and any that fall
    /// entirely outside the painter's horizontal clip are skipped: the pen only
    /// advances rightward, so once a glyph starts past the clip's right edge the
    /// rest of the line is off-screen and the loop stops. This keeps a long line
    /// (a 500-column Markdown row) from blending hundreds of invisible glyphs.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn draw_phys(
        &self,
        painter: &mut Painter,
        text: &str,
        x: f32,
        y: f32,
        size: f32,
        scale: f32,
        color: Color,
        style: FontStyle,
    ) -> f32 {
        let baseline = y + size * scale;
        let (clip_lo, clip_hi) = painter.glyph_clip_x();
        let mut pen_x = x;
        for ch in text.chars() {
            let glyph = self.glyph(ch, size, scale, style);
            let metrics = &glyph.metrics;
            let glyph_x = pen_x + metrics.xmin as f32;
            // Everything from here rightward is past the visible span.
            if glyph_x >= clip_hi as f32 {
                break;
            }
            // This glyph ends before the visible span — advance past it without
            // blending (matters when content is scrolled left of the origin).
            if glyph_x + metrics.width as f32 <= clip_lo as f32 {
                pen_x += metrics.advance;
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
            pen_x += metrics.advance;
        }
        pen_x
    }
}

/// Load the face `id` names out of `db`, as the face it actually is.
///
/// A font *collection* (`.ttc`) packs several faces into one file, and fontdb
/// hands back the whole file plus the index of the face inside it — macOS ships
/// most of its families this way, Helvetica.ttc holding the regular, bold and
/// oblique faces together. Dropping that index leaves fontdue on face 0, so
/// every emphasis face of a collection quietly rasterized as its regular one:
/// bold text that measured wider than it looked.
fn load_face(db: &fontdb::Database, id: fontdb::ID) -> Option<fontdue::Font> {
    let mut data: Option<(Vec<u8>, u32)> = None;
    db.with_face_data(id, |bytes, index| data = Some((bytes.to_vec(), index)));
    let (data, collection_index) = data?;
    fontdue::Font::from_bytes(
        data,
        fontdue::FontSettings {
            collection_index,
            ..fontdue::FontSettings::default()
        },
    )
    .ok()
}

/// The weight at or above which we consider a face "bold". OS/2 weight classes
/// put Regular at 400 and Bold at 700; SemiBold (600) and up read as bold enough
/// to serve as the bold face, while Medium (500) does not.
const BOLD_WEIGHT_THRESHOLD: u16 = 600;

/// Query `db` for a `family` face in the requested weight/style and load it —
/// but only if the face fontdb hands back *actually* carries that emphasis.
///
/// fontdb's `query` runs the CSS font-matching algorithm, which returns the
/// closest available face even when nothing matches: ask a family with no italic
/// for its italic and you get the upright face back. Accepting that would let a
/// regular face masquerade as bold or italic — exactly the synthesized-looking
/// fakery we want to avoid — so we re-read the matched face's own weight/style
/// and reject a mismatch. The caller then leaves that slot empty and falls back
/// to a real face at draw time.
fn query_verified(
    db: &fontdb::Database,
    family: &str,
    weight: fontdb::Weight,
    style: fontdb::Style,
) -> Option<fontdue::Font> {
    let query = fontdb::Query {
        families: &[fontdb::Family::Name(family)],
        weight,
        stretch: fontdb::Stretch::Normal,
        style,
    };
    let id = db.query(&query)?;
    let info = db.face(id)?;

    let want_bold = weight.0 >= BOLD_WEIGHT_THRESHOLD;
    let want_slanted = style != fontdb::Style::Normal;
    let is_bold = info.weight.0 >= BOLD_WEIGHT_THRESHOLD;
    let is_slanted = info.style != fontdb::Style::Normal;
    if is_bold != want_bold || is_slanted != want_slanted {
        return None;
    }

    load_face(db, id)
}

/// Build a [`Font`] for `family`, loading its regular face plus whichever of the
/// bold / italic / bold-italic faces the host actually ships. Returns `None`
/// when the family has no usable regular face.
fn load_styled_family(db: &fontdb::Database, family: &str) -> Option<Font> {
    let regular = query_verified(db, family, fontdb::Weight::NORMAL, fontdb::Style::Normal)?;
    let mut font = Font::from_face(regular);
    let mut attach = |style: FontStyle, weight, slant| {
        if let (Some(faces), Some(face)) = (
            font.faces.outlines_mut(),
            query_verified(db, family, weight, slant),
        ) {
            faces[style as usize] = Some(face);
        }
    };
    attach(FontStyle::Bold, fontdb::Weight::BOLD, fontdb::Style::Normal);
    attach(
        FontStyle::Italic,
        fontdb::Weight::NORMAL,
        fontdb::Style::Italic,
    );
    attach(
        FontStyle::BoldItalic,
        fontdb::Weight::BOLD,
        fontdb::Style::Italic,
    );
    Some(font)
}

// ---------------------------------------------------------------------------
// The candidate families, per platform
// ---------------------------------------------------------------------------
//
// Each chain starts with the host's *own* UI font, so an app looks like it
// belongs on the desktop it was opened on rather than like a port, and ends in
// the faces that turn up nearly everywhere. Names are tried in order, except
// that a family shipping no bold face loses to a later one that does — see
// [`pick_family`].
//
// A note on why the newest system font is not always what gets used: some of
// them are *variable* fonts (macOS's San Francisco, its New York), a single
// file whose weight is an axis rather than a set of faces. fontdb reports one
// weight-400 face for the whole file, so there is no bold face to load and no
// honest way to draw a heading in it. They are still listed first — they are
// the right answer the moment their weights become reachable — and passed over
// in the meantime for the newest system face that has a real bold.

/// Proportional sans: the UI face nearly everything is drawn in.
///
/// A safety net on this platform: [`Font::load_sans`] asks Core Text for the
/// interface font by *role*, and only falls back to searching for families by
/// name if that somehow fails. So the system font is not listed here — it has no
/// public family name, and going through a font database is exactly what loses
/// its weights.
#[cfg(target_os = "macos")]
const SANS_FAMILIES: &[&str] = &[
    // The two faces that held the interface job before San Francisco.
    "Helvetica Neue",
    "Lucida Grande",
    "Avenir Next",
    "Helvetica",
    // Bundled with macOS, and the generic tail.
    "Arial",
    "DejaVu Sans",
    "Liberation Sans",
];

/// Proportional sans: the UI face nearly everything is drawn in.
#[cfg(target_os = "windows")]
const SANS_FAMILIES: &[&str] = &[
    // Vista onwards, then the XP-era shell font, then the 3.1 original.
    "Segoe UI",
    "Tahoma",
    "Microsoft Sans Serif",
    "MS Sans Serif",
    "Arial",
    "DejaVu Sans",
    "Liberation Sans",
];

/// Proportional sans: the UI face nearly everything is drawn in.
#[cfg(not(any(target_os = "macos", target_os = "windows")))]
const SANS_FAMILIES: &[&str] = &[
    // GNOME's UI font, Ubuntu's, then what most distributions default to.
    "Cantarell",
    "Ubuntu",
    "Noto Sans",
    "DejaVu Sans",
    "Liberation Sans",
    "Nimbus Sans",
    "FreeSans",
    // A host with the Microsoft fonts installed can still have them.
    "Arial",
    "Helvetica",
];

/// Proportional serif, for the odd body of running text.
#[cfg(target_os = "macos")]
const SERIF_FAMILIES: &[&str] = &[
    "New York",
    "Times New Roman",
    "Palatino",
    "Charter",
    "Times",
    "Georgia",
    "Noto Serif",
    "DejaVu Serif",
    "Liberation Serif",
];

/// Proportional serif, for the odd body of running text.
#[cfg(target_os = "windows")]
const SERIF_FAMILIES: &[&str] = &[
    "Times New Roman",
    "Georgia",
    "Palatino Linotype",
    "MS Serif",
    "Noto Serif",
    "DejaVu Serif",
    "Liberation Serif",
];

/// Proportional serif, for the odd body of running text.
#[cfg(not(any(target_os = "macos", target_os = "windows")))]
const SERIF_FAMILIES: &[&str] = &[
    "Noto Serif",
    "DejaVu Serif",
    "Liberation Serif",
    "Nimbus Roman",
    "FreeSerif",
    "Times New Roman",
    "Georgia",
];

/// Fixed-width fallbacks; as with the sans, Core Text is asked for the user's
/// own fixed-pitch font by role first.
#[cfg(target_os = "macos")]
const MONO_FAMILIES: &[&str] = &[
    "Menlo",
    "Monaco",
    "Andale Mono",
    "Courier New",
    "Noto Sans Mono",
    "DejaVu Sans Mono",
    "Liberation Mono",
];

/// Fixed-width, for the text editors and anything showing code.
#[cfg(target_os = "windows")]
const MONO_FAMILIES: &[&str] = &[
    "Cascadia Mono",
    "Consolas",
    "Lucida Console",
    "Courier New",
    "Courier",
    "Noto Sans Mono",
    "DejaVu Sans Mono",
    "Liberation Mono",
];

/// Fixed-width, for the text editors and anything showing code.
#[cfg(not(any(target_os = "macos", target_os = "windows")))]
const MONO_FAMILIES: &[&str] = &[
    "Noto Sans Mono",
    "DejaVu Sans Mono",
    "Liberation Mono",
    "Ubuntu Mono",
    "Nimbus Mono PS",
    "FreeMono",
    "Courier New",
];

/// Choose which of `families` to dress the UI in, `load` resolving a family
/// name to whatever faces the host has for it.
///
/// Priority order still decides between families that can do the same job, but
/// a family that ships no bold face at all loses to one further down the list
/// that does: emphasis is worth more to a UI than any particular typeface, and
/// a heading with no bold face behind it is indistinguishable from body text.
/// (macOS is where this bites — its Microsoft Sans Serif is regular-only, and
/// it sits one line above Tahoma, which has a real bold.) A chain with no bold
/// anywhere still yields its first hit, since the alternative is no text at all.
fn pick_family(families: &[&str], load: impl Fn(&str) -> Option<Font>) -> Option<Font> {
    let mut unemphasized = None;
    for family in families {
        let Some(font) = load(family) else { continue };
        if font.presence()[FontStyle::Bold as usize] {
            return Some(font);
        }
        if unemphasized.is_none() {
            unemphasized = Some(font);
        }
    }
    unemphasized
}

/// Search `db` for the first family name in `families` that resolves to a
/// loadable regular face, returning that family with all of its emphasis faces.
/// When `monospace_fallback` is true, after exhausting the named families the
/// search also accepts any face whose record claims monospace — useful so we
/// don't accidentally drop into a proportional font when none of the well-known
/// mono families are installed. The fallback faces are regular-only (no styled
/// variants); a named family is the path that carries bold/italic.
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

    if let Some(font) = pick_family(families, |family| load_styled_family(&db, family)) {
        return Some(font);
    }

    if monospace_fallback {
        for face in db.faces() {
            if face.monospaced
                && let Some(font) = load_face(&db, face.id)
            {
                return Some(Font::from_face(font));
            }
        }
    }

    // Last-ditch: any face we can find. Better something than nothing.
    for face in db.faces() {
        if let Some(font) = load_face(&db, face.id) {
            return Some(Font::from_face(font));
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

    /// The bundled snapshot fixtures, read at run time: they are excluded from
    /// the published crate, so a test that needs them stands aside when they
    /// are not there rather than failing.
    fn fixture(name: &str) -> Option<Vec<u8>> {
        std::fs::read(format!("tests/fonts/{name}")).ok()
    }

    fn be32(v: u32) -> [u8; 4] {
        v.to_be_bytes()
    }

    fn read16(b: &[u8], at: usize) -> u16 {
        u16::from_be_bytes([b[at], b[at + 1]])
    }

    fn read32(b: &[u8], at: usize) -> u32 {
        u32::from_be_bytes([b[at], b[at + 1], b[at + 2], b[at + 3]])
    }

    /// Splice whole TrueType files into one collection (`.ttc`), the packaging
    /// macOS ships its families in. Nothing in `tests/fonts` is a collection,
    /// and the face-index bug only shows up inside one, so build it here: a
    /// `ttcf` header pointing at each font's table directory, with every table
    /// copied over and its offset rewritten to where it landed.
    fn collection(fonts: &[&[u8]]) -> Vec<u8> {
        let header = 12 + 4 * fonts.len();
        let dir_len = |f: &[u8]| 12 + 16 * read16(f, 4) as usize;
        let mut dirs = Vec::new();
        let mut at = header;
        for f in fonts {
            dirs.push(at);
            at += dir_len(f);
        }

        let mut out = vec![0u8; at];
        out[0..4].copy_from_slice(b"ttcf");
        out[4..8].copy_from_slice(&be32(0x0001_0000));
        out[8..12].copy_from_slice(&be32(fonts.len() as u32));
        for (i, dir) in dirs.iter().enumerate() {
            out[12 + 4 * i..16 + 4 * i].copy_from_slice(&be32(*dir as u32));
        }

        for (f, dir) in fonts.iter().zip(&dirs) {
            let tables = read16(f, 4) as usize;
            out[*dir..dir + 12].copy_from_slice(&f[0..12]);
            for t in 0..tables {
                let src = 12 + 16 * t;
                let from = read32(f, src + 8) as usize;
                let len = read32(f, src + 12) as usize;
                while !out.len().is_multiple_of(4) {
                    out.push(0);
                }
                let to = out.len();
                out.extend_from_slice(&f[from..from + len]);
                let rec = dir + 12 + 16 * t;
                out[rec..rec + 8].copy_from_slice(&f[src..src + 8]);
                out[rec + 8..rec + 12].copy_from_slice(&be32(to as u32));
                out[rec + 12..rec + 16].copy_from_slice(&be32(len as u32));
            }
        }
        out
    }

    #[test]
    fn each_candidate_chain_is_the_one_for_this_platform() {
        for (role, chain) in [
            ("sans", SANS_FAMILIES),
            ("serif", SERIF_FAMILIES),
            ("mono", MONO_FAMILIES),
        ] {
            assert!(!chain.is_empty(), "{role} has no candidates at all");
            let mut seen = std::collections::HashSet::new();
            for family in chain {
                assert!(seen.insert(*family), "{role} lists {family} twice");
            }
            // Whatever the host is, the chain has to end somewhere that exists
            // on a bare machine, or an unfamiliar desktop gets no text at all.
            assert!(
                chain
                    .iter()
                    .any(|f| f.starts_with("DejaVu") || f.starts_with("Liberation")),
                "{role} has no near-universal fallback"
            );
        }

        // And that the cfg arm compiled in is this platform's, not a neighbour's.
        #[cfg(target_os = "macos")]
        {
            // A Mac gets its system fonts from Core Text by role, so these are
            // the fallbacks — led by the interface face that came before San
            // Francisco, rather than by a name for the system font itself.
            assert_eq!(SANS_FAMILIES[0], "Helvetica Neue");
            assert!(MONO_FAMILIES.contains(&"Menlo"));
        }
        #[cfg(target_os = "windows")]
        {
            assert_eq!(SANS_FAMILIES[0], "Segoe UI");
            assert!(MONO_FAMILIES.contains(&"Consolas"));
        }
        #[cfg(not(any(target_os = "macos", target_os = "windows")))]
        {
            assert_eq!(SANS_FAMILIES[0], "Cantarell");
            assert!(MONO_FAMILIES.contains(&"Noto Sans Mono"));
        }
    }

    #[test]
    fn a_face_is_loaded_from_its_own_slot_in_a_collection() {
        let (Some(sans), Some(mono)) = (fixture("DejaVuSans.ttf"), fixture("DejaVuSansMono.ttf"))
        else {
            return;
        };
        // Two very different faces in one file: the proportional one first, so
        // reading the collection's face 0 by mistake is measurable.
        let mut db = fontdb::Database::new();
        db.load_font_data(collection(&[&sans, &mono]));
        assert_eq!(db.faces().count(), 2, "both faces are registered");

        let monospaced = db
            .faces()
            .find(|f| f.monospaced)
            .expect("the collection carries the mono face")
            .id;
        let face = load_face(&db, monospaced).expect("the mono face loads");
        // A fixed-width face gives `i` and `M` the same advance; the
        // proportional face at index 0 does not.
        assert_eq!(
            face.metrics('i', 24.0).advance_width,
            face.metrics('M', 24.0).advance_width,
            "the face loaded is the mono one, not the collection's first face"
        );
    }

    /// A `Font` with only a regular face, and one that also has a bold face.
    fn stub(bytes: &[u8], bold: Option<&[u8]>) -> Font {
        let font = Font::from_sans_bytes(bytes.to_vec()).expect("fixture parses");
        match bold {
            Some(b) => font.with_bold_bytes(b.to_vec()),
            None => font,
        }
    }

    #[test]
    fn a_family_with_a_real_bold_face_wins_over_one_without() {
        let (Some(sans), Some(mono)) = (fixture("DejaVuSans.ttf"), fixture("DejaVuSansMono.ttf"))
        else {
            return;
        };
        // "first" comes earlier in the chain but ships no bold; "second" does.
        let load = |family: &str| match family {
            "first" => Some(stub(&sans, None)),
            "second" => Some(stub(&mono, Some(&sans))),
            _ => None,
        };
        let picked = pick_family(&["first", "second"], load).expect("a family");
        assert!(
            picked.presence()[FontStyle::Bold as usize],
            "the family that can actually do bold is the one chosen"
        );

        // Priority still decides between two families that both have bold.
        let both = |family: &str| match family {
            "first" => Some(stub(&sans, Some(&mono))),
            "second" => Some(stub(&mono, Some(&sans))),
            _ => None,
        };
        let picked = pick_family(&["first", "second"], both).expect("a family");
        assert_eq!(
            picked.measure_styled("iM", 24.0, FontStyle::Regular),
            stub(&sans, None).measure_styled("iM", 24.0, FontStyle::Regular),
            "with bold on both sides the first family still wins"
        );
    }

    #[test]
    fn a_chain_with_no_bold_anywhere_still_yields_its_first_hit() {
        let Some(sans) = fixture("DejaVuSans.ttf") else {
            return;
        };
        let load = |family: &str| match family {
            "only" => Some(stub(&sans, None)),
            _ => None,
        };
        assert!(
            pick_family(&["missing", "only"], load).is_some(),
            "no bold anywhere is still better than no text"
        );
        assert!(
            pick_family(&["missing"], load).is_none(),
            "and nothing at all is None"
        );
    }

    #[test]
    fn the_system_sans_bold_face_is_really_bolder() {
        let Some(font) = Font::load_sans() else {
            return;
        };
        if !font.presence()[FontStyle::Bold as usize] {
            // A host with no bold face for any candidate family: `resolve_style`
            // falls back to regular, which is the documented behavior.
            return;
        }
        let regular = font
            .measure_styled("Handgloves", 24.0, FontStyle::Regular)
            .0;
        let bold = font.measure_styled("Handgloves", 24.0, FontStyle::Bold).0;
        assert!(
            bold > regular,
            "a real bold face sets wider than its regular ({bold} vs {regular})"
        );
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

    #[test]
    fn font_style_from_flags() {
        assert_eq!(FontStyle::new(false, false), FontStyle::Regular);
        assert_eq!(FontStyle::new(true, false), FontStyle::Bold);
        assert_eq!(FontStyle::new(false, true), FontStyle::Italic);
        assert_eq!(FontStyle::new(true, true), FontStyle::BoldItalic);

        assert!(FontStyle::Bold.is_bold() && !FontStyle::Bold.is_italic());
        assert!(FontStyle::Italic.is_italic() && !FontStyle::Italic.is_bold());
        assert!(FontStyle::BoldItalic.is_bold() && FontStyle::BoldItalic.is_italic());
        assert!(!FontStyle::Regular.is_bold() && !FontStyle::Regular.is_italic());
    }

    #[test]
    fn resolve_uses_the_real_face_when_present() {
        let all = [true, true, true, true];
        assert_eq!(resolve_style(all, FontStyle::Bold), FontStyle::Bold);
        assert_eq!(resolve_style(all, FontStyle::Italic), FontStyle::Italic);
        assert_eq!(
            resolve_style(all, FontStyle::BoldItalic),
            FontStyle::BoldItalic
        );
    }

    #[test]
    fn resolve_falls_back_to_regular_when_a_face_is_missing() {
        // Only the regular face is loaded — every request resolves to it, never
        // to a synthesized style.
        let only_regular = [true, false, false, false];
        for style in [
            FontStyle::Regular,
            FontStyle::Bold,
            FontStyle::Italic,
            FontStyle::BoldItalic,
        ] {
            assert_eq!(resolve_style(only_regular, style), FontStyle::Regular);
        }
    }

    #[test]
    fn resolve_bold_italic_prefers_the_closest_real_face() {
        // No bold-italic face, but a bold one exists: keep the weight.
        let no_bi_has_bold = [true, true, false, false];
        assert_eq!(
            resolve_style(no_bi_has_bold, FontStyle::BoldItalic),
            FontStyle::Bold
        );
        // No bold-italic and no bold, but an italic exists: keep the slant.
        let no_bi_has_italic = [true, false, true, false];
        assert_eq!(
            resolve_style(no_bi_has_italic, FontStyle::BoldItalic),
            FontStyle::Italic
        );
    }
}
