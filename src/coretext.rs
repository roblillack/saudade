//! The Mac's own text stack, used for the *system* fonts.
//!
//! saudade rasterizes text itself: one glyph at a time, at a physical pixel
//! size, into an 8-bit coverage bitmap that [`Painter`](crate::Painter) blends.
//! That is exactly the shape Core Text draws in too, so this module is a second
//! source for the same three questions [`Font`](crate::Font) asks — which styles
//! exist, how wide is this glyph, what does it look like — with Core Text
//! answering instead of fontdue.
//!
//! It exists because the portable path cannot answer the first question for a
//! *variable* system font. macOS ships San Francisco as one file whose weight is
//! an axis rather than a set of faces, so a font database reports a single
//! regular face and there is no bold to load — which is how a Mac app ended up
//! with headings indistinguishable from body text. Core Text instantiates that
//! axis, so asking it for the bold system font yields a real bold.
//!
//! Two things come along for free: the antialiasing every other app on the
//! desktop uses, and glyph *fallback* — a character the UI font has no glyph for
//! (an emoji, a CJK ideograph) is drawn by whichever installed face does have
//! it, where the portable path draws nothing at all.
//!
//! Fonts an app supplies as bytes stay on fontdue, on every platform: an app
//! that bundles a face wants that face rendered the same way everywhere, and the
//! snapshot tests depend on it.

use std::cell::RefCell;
use std::collections::HashMap;
use std::ptr::NonNull;

use objc2_core_foundation::{
    CFIndex, CFRange, CFRetained, CFString, CGAffineTransform, CGFloat, CGPoint, CGRect,
};
use objc2_core_graphics::{CGBitmapContextCreate, CGContext, CGGlyph, CGImageAlphaInfo};
use objc2_core_text::{CTFont, CTFontOrientation, CTFontSymbolicTraits, CTFontUIFontType};

use crate::font::{FontStyle, GlyphMetrics, STYLE_COUNT};

/// Size the emphasis probe runs at. Any size would do — whether a family has a
/// bold face does not depend on how big it is drawn — so this is just a typical
/// UI body size.
const PROBE_SIZE: CGFloat = 13.0;

/// Slack around a glyph's reported bounding box, in pixels. Antialiasing puts
/// coverage slightly outside the outline's own bounds, and a clipped stem is far
/// more visible than a transparent border: the blend loop skips zero pixels
/// anyway.
const BLEED: i32 = 1;

/// What to ask Core Text for. Kept as a recipe rather than a font object because
/// the *size* is part of the request: macOS picks a different optical variant of
/// the system font for a caption than for a headline, and we want that.
enum Recipe {
    /// One of the interface fonts, by role — what the OS itself would use.
    Ui(CTFontUIFontType),
    /// A family by name, for the roles the interface has no font for.
    Named(CFRetained<CFString>),
}

/// A font family as Core Text sees it, in every style the host can really draw.
pub(crate) struct Family {
    recipe: Recipe,
    /// Which styles Core Text will actually give us — see [`Family::probe`].
    presence: [bool; STYLE_COUNT],
    /// `CTFont`s already built, keyed by the style, point size and DPI scale
    /// they were built for. A window redraws the same handful of sizes over and
    /// over, and creating a font is not free.
    sized: RefCell<HashMap<(FontStyle, u32, u32), CFRetained<CTFont>>>,
}

impl Family {
    /// The interface font — what the Finder, the menu bar and every stock
    /// control are set in.
    pub(crate) fn system() -> Option<Self> {
        Self::from_recipe(Recipe::Ui(CTFontUIFontType::System))
    }

    /// The interface *fixed-pitch* font, i.e. whatever the user's monospace
    /// preference resolves to.
    pub(crate) fn fixed_pitch() -> Option<Self> {
        Self::from_recipe(Recipe::Ui(CTFontUIFontType::UserFixedPitch))
    }

    /// A family by name, for a role the interface fonts don't cover (there is no
    /// UI serif). Returns `None` when the host has no such family: Core Text
    /// substitutes a default rather than failing, so the name is read back and
    /// compared instead of trusted.
    pub(crate) fn named(family: &str) -> Option<Self> {
        let name = CFString::from_str(family);
        let font = unsafe { CTFont::with_name(&name, PROBE_SIZE, std::ptr::null()) };
        let resolved = unsafe { font.family_name() };
        if !resolved.to_string().eq_ignore_ascii_case(family) {
            return None;
        }
        Self::from_recipe(Recipe::Named(name))
    }

    fn from_recipe(recipe: Recipe) -> Option<Self> {
        let base = base_font(&recipe, PROBE_SIZE)?;
        Some(Self {
            presence: probe(&base),
            recipe,
            sized: RefCell::new(HashMap::new()),
        })
    }

    /// Which styles have a real face behind them, in the order
    /// [`FontStyle`] numbers them.
    pub(crate) fn presence(&self) -> [bool; STYLE_COUNT] {
        self.presence
    }

    /// Advance width of one glyph of `size`-point text, in `style`, in logical
    /// pixels. Independent of the DPI: the glyphs are drawn `scale` times bigger
    /// on a Retina display, but the layout is the same one.
    pub(crate) fn advance(&self, ch: char, size: f32, style: FontStyle) -> f32 {
        let Some(font) = self.font_for(style, size, 1.0) else {
            return 0.0;
        };
        with_glyph(&font, ch, |font, glyph| {
            let mut advance = CGPoint::ZERO;
            // `CGSize` and `CGPoint` are both two `CGFloat`s; the call fills in
            // the advance's width and height, and only the width matters for
            // horizontal text.
            unsafe {
                font.advances_for_glyphs(
                    CTFontOrientation::Default,
                    NonNull::from(&glyph),
                    (&mut advance as *mut CGPoint).cast(),
                    1,
                );
            }
            advance.x as f32
        })
        .unwrap_or(0.0)
    }

    /// Rasterize one glyph of `size`-point text drawn at DPI `scale`, into an
    /// 8-bit coverage bitmap, top row first — the same layout the portable path
    /// produces. The metrics come back in physical pixels.
    pub(crate) fn rasterize(
        &self,
        ch: char,
        size: f32,
        scale: f32,
        style: FontStyle,
    ) -> (GlyphMetrics, Vec<u8>) {
        let empty = (GlyphMetrics::default(), Vec::new());
        let Some(font) = self.font_for(style, size, scale) else {
            return empty;
        };
        with_glyph(&font, ch, raster).unwrap_or(empty)
    }

    /// The `CTFont` for `size`-point text in `style`, drawn at DPI `scale`.
    /// Built once and kept.
    ///
    /// The point size and the DPI are deliberately kept apart. Core Text picks a
    /// different *optical* variant of the system font depending on the size
    /// asked for — San Francisco sets tighter for a headline than for a caption,
    /// and re-resolves that on every size change, so simply asking for a
    /// 26-pixel font on a Retina display would silently switch a 13-point label
    /// to display metrics: about 10% narrower, and no longer the same layout the
    /// same app has on a 1x screen. So the size stays the *point* size, which is
    /// what selects the variant (as it does in a native app, where the backing
    /// scale never reaches the font), and the DPI arrives as a scale matrix,
    /// which magnifies the glyphs without touching that choice. Advances then
    /// come out exactly `scale` times the logical ones.
    fn font_for(&self, style: FontStyle, size: f32, scale: f32) -> Option<CFRetained<CTFont>> {
        let key = (style, size.to_bits(), scale.to_bits());
        if let Some(font) = self.sized.borrow().get(&key) {
            return Some(font.clone());
        }
        let size = size as CGFloat;
        let base = base_font(&self.recipe, size)?;
        let styled = emphasized(&base, size, style)?;
        let font = magnified(&styled, size, scale);
        self.sized.borrow_mut().insert(key, font.clone());
        Some(font)
    }
}

/// Build the plain, upright font of `recipe` at `size`.
fn base_font(recipe: &Recipe, size: CGFloat) -> Option<CFRetained<CTFont>> {
    match recipe {
        Recipe::Ui(kind) => unsafe { CTFont::new_ui_font_for_language(*kind, size, None) },
        Recipe::Named(name) => Some(unsafe { CTFont::with_name(name, size, std::ptr::null()) }),
    }
}

/// `font` with its glyphs scaled by `scale`, its point size left alone.
fn magnified(font: &CTFont, size: CGFloat, scale: f32) -> CFRetained<CTFont> {
    if scale == 1.0 {
        // Retaining a borrowed CF object is sound: the caller's own reference
        // keeps it alive until this one takes effect.
        return unsafe { CFRetained::retain(NonNull::from(font)) };
    }
    let scale = scale as CGFloat;
    let matrix = CGAffineTransform {
        a: scale,
        b: 0.0,
        c: 0.0,
        d: scale,
        tx: 0.0,
        ty: 0.0,
    };
    unsafe { font.copy_with_attributes(size, &matrix, None) }
}

/// The bold / italic bits `style` asks for.
fn traits_of(style: FontStyle) -> CTFontSymbolicTraits {
    let mut traits = CTFontSymbolicTraits::empty();
    if style.is_bold() {
        traits |= CTFontSymbolicTraits::TraitBold;
    }
    if style.is_italic() {
        traits |= CTFontSymbolicTraits::TraitItalic;
    }
    traits
}

/// `base` restyled, or `None` when the family has no such face.
///
/// Core Text returns null when it cannot satisfy the traits — but it is also
/// willing to hand back a face that only *partly* matches, so the result's own
/// traits are read back and checked. That is the same suspicion the font-database
/// path applies to a query result, and for the same reason: a regular face
/// masquerading as bold is worse than an honest fallback, because the fallback
/// is visible to the code that decides what to draw.
fn emphasized(base: &CTFont, size: CGFloat, style: FontStyle) -> Option<CFRetained<CTFont>> {
    let wanted = traits_of(style);
    if wanted.is_empty() {
        // Retaining a borrowed CF object is sound: the caller's own reference
        // keeps it alive until this one takes effect.
        return Some(unsafe { CFRetained::retain(NonNull::from(base)) });
    }
    let font = unsafe { base.copy_with_symbolic_traits(size, std::ptr::null(), wanted, wanted) }?;
    let got = unsafe { font.symbolic_traits() };
    got.contains(wanted).then_some(font)
}

/// Which styles this font can really be drawn in.
fn probe(base: &CTFont) -> [bool; STYLE_COUNT] {
    std::array::from_fn(|i| {
        let style = STYLES[i];
        style == FontStyle::Regular || emphasized(base, PROBE_SIZE, style).is_some()
    })
}

/// The styles in the order `FontStyle` numbers them, so a `[bool; STYLE_COUNT]`
/// can be built by index.
const STYLES: [FontStyle; STYLE_COUNT] = [
    FontStyle::Regular,
    FontStyle::Bold,
    FontStyle::Italic,
    FontStyle::BoldItalic,
];

/// Look up `ch` in `font` and hand the glyph to `f`.
///
/// A character the font has no glyph for is not given up on: Core Text is asked
/// which installed face *does* have it, and `f` receives that face instead. This
/// is how an emoji or a CJK character in an otherwise Latin UI gets drawn at all.
/// `None` means nothing on the system can draw it.
fn with_glyph<R>(font: &CTFont, ch: char, f: impl FnOnce(&CTFont, CGGlyph) -> R) -> Option<R> {
    let mut utf16 = [0u16; 2];
    let units = ch.encode_utf16(&mut utf16).len();

    if let Some(glyph) = glyph_id(font, &mut utf16, units) {
        return Some(f(font, glyph));
    }

    // Nothing in this face; ask the system for one that has it. A surrogate
    // pair is one character to Core Text but two UTF-16 units to the range.
    let text = CFString::from_str(&ch.to_string());
    let fallback = unsafe {
        font.for_string(
            &text,
            CFRange {
                location: 0,
                length: units as CFIndex,
            },
        )
    };
    let glyph = glyph_id(&fallback, &mut utf16, units)?;
    Some(f(&fallback, glyph))
}

/// The glyph `chars` maps to in `font`, or `None` when the font has none.
fn glyph_id(font: &CTFont, chars: &mut [u16; 2], units: usize) -> Option<CGGlyph> {
    let mut glyphs = [0u16; 2];
    let mapped = unsafe {
        font.glyphs_for_characters(
            NonNull::from(&mut chars[0]),
            NonNull::from(&mut glyphs[0]),
            units as CFIndex,
        )
    };
    // Glyph 0 is `.notdef`, which is a miss however it was reported.
    (mapped && glyphs[0] != 0).then_some(glyphs[0])
}

/// Draw one glyph into a fresh alpha-only bitmap and return it with the metrics
/// the blend loop needs to place it.
fn raster(font: &CTFont, glyph: CGGlyph) -> (GlyphMetrics, Vec<u8>) {
    let mut bounds = CGRect::ZERO;
    unsafe {
        font.bounding_rects_for_glyphs(
            CTFontOrientation::Default,
            NonNull::from(&glyph),
            &mut bounds,
            1,
        );
    }
    let mut advance = CGPoint::ZERO;
    unsafe {
        font.advances_for_glyphs(
            CTFontOrientation::Default,
            NonNull::from(&glyph),
            (&mut advance as *mut CGPoint).cast(),
            1,
        );
    }

    // The bitmap covers whole pixels around the outline's bounds, plus a pixel
    // of slack for the antialiasing.
    let xmin = (bounds.origin.x.floor() as i32) - BLEED;
    let ymin = (bounds.origin.y.floor() as i32) - BLEED;
    let xmax = ((bounds.origin.x + bounds.size.width).ceil() as i32) + BLEED;
    let ymax = ((bounds.origin.y + bounds.size.height).ceil() as i32) + BLEED;
    let width = (xmax - xmin).max(0) as usize;
    let height = (ymax - ymin).max(0) as usize;

    let metrics = GlyphMetrics {
        width,
        height,
        xmin,
        ymin,
        advance: advance.x as f32,
    };
    if width == 0 || height == 0 {
        // A space, or a glyph with no ink: the advance still matters.
        return (metrics, Vec::new());
    }

    let mut bitmap = vec![0u8; width * height];
    // An alpha-only context has no color space and one byte per pixel, which is
    // the coverage bitmap we want with no conversion afterwards. Core Graphics
    // stores it top row first while drawing into it bottom-up, which is exactly
    // the convention the blend loop expects.
    let context = unsafe {
        CGBitmapContextCreate(
            bitmap.as_mut_ptr().cast(),
            width,
            height,
            8,
            width,
            None,
            CGImageAlphaInfo::Only.0,
        )
    };
    let Some(context) = context else {
        return (metrics, Vec::new());
    };
    unsafe {
        CGContext::set_should_antialias(Some(&context), true);
        // Font *smoothing* is subpixel antialiasing, which would mean colored
        // fringes — meaningless in a single-channel mask, and wrong for text
        // this crate composites itself.
        CGContext::set_should_smooth_fonts(Some(&context), false);
        // Place the glyph's origin so its bounding box lands on the bitmap.
        let origin = CGPoint {
            x: -xmin as CGFloat,
            y: -ymin as CGFloat,
        };
        font.draw_glyphs(NonNull::from(&glyph), NonNull::from(&origin), 1, &context);
    }
    (metrics, bitmap)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every Mac has an interface font, and Core Text can always emphasize it —
    /// that is the whole reason this module exists, since the font database path
    /// sees only one weight of a variable system font.
    #[test]
    fn the_interface_font_has_a_real_bold() {
        let family = Family::system().expect("a Mac has an interface font");
        let presence = family.presence();
        assert!(presence[FontStyle::Regular as usize]);
        assert!(
            presence[FontStyle::Bold as usize],
            "the system font can be emphasized"
        );
        let regular = family.advance('M', 13.0, FontStyle::Regular);
        let bold = family.advance('M', 13.0, FontStyle::Bold);
        assert!(
            bold > regular,
            "and its bold really is wider ({bold} vs {regular})"
        );
    }

    /// A glyph drawn on a Retina display has to be the *same* glyph, `scale`
    /// times bigger — not the one Core Text would pick for a font of that many
    /// points. Otherwise a 13pt label lays out one way on one display and
    /// another way on the next, since macOS re-picks the optical variant on
    /// every size change.
    #[test]
    fn the_dpi_scales_a_glyph_without_reselecting_the_face() {
        let family = Family::system().expect("interface font");
        for scale in [1.0f32, 1.5, 2.0, 3.0] {
            let logical = family.advance('M', 13.0, FontStyle::Regular);
            let (metrics, _) = family.rasterize('M', 13.0, scale, FontStyle::Regular);
            let expected = logical * scale;
            assert!(
                (metrics.advance - expected).abs() < 0.01,
                "at {scale}x the advance should be {expected}, got {}",
                metrics.advance
            );
        }
    }

    /// Sizes still differ from each other — the point size is what selects the
    /// optical variant, so this is the behavior the scale matrix preserves.
    #[test]
    fn a_bigger_point_size_may_be_a_different_face() {
        let family = Family::system().expect("interface font");
        let small = family.advance('M', 13.0, FontStyle::Regular);
        let large = family.advance('M', 26.0, FontStyle::Regular);
        assert!(large > small, "26pt is wider than 13pt in absolute terms");
    }

    /// A character the interface font has no glyph for is drawn by whichever
    /// installed face does have it, rather than silently dropped.
    #[test]
    fn a_character_the_face_lacks_falls_back_to_one_that_has_it() {
        let family = Family::system().expect("interface font");
        for ch in ['漢', '✓'] {
            let (metrics, bitmap) = family.rasterize(ch, 16.0, 1.0, FontStyle::Regular);
            assert!(
                metrics.advance > 0.0 && bitmap.iter().any(|&a| a > 0),
                "{ch:?} should have been drawn by a fallback face"
            );
        }
    }

    #[test]
    fn a_family_the_host_does_not_have_is_not_substituted() {
        assert!(
            Family::named("Definitely Not An Installed Font").is_none(),
            "Core Text substitutes a default; that must not pass for a match"
        );
        assert!(
            Family::named("Helvetica").is_some(),
            "and a family the host does have resolves"
        );
    }
}
