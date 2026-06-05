//! `include_svg!` — read an SVG at **compile time** and bake it into a set of
//! flattened, filled polygons that saudade can replay at runtime without any
//! SVG machinery in the binary.
//!
//! The heavy lifting (XML parsing, attribute inheritance, shape→path
//! conversion, curve flattening, stroke-to-outline expansion) all happens here,
//! at build time, using [`usvg`] + [`kurbo`]. What the macro *emits* is plain
//! geometry: a [`saudade::SvgImage`] holding `&'static` slices of polygon rings
//! and their fill colors. The runtime side only has to fill polygons — see
//! `saudade::svg` — so neither usvg nor kurbo ever reach a shipped program.
//!
//! ```ignore
//! use saudade::include_svg;
//! // Path is resolved relative to the *invoking crate's* CARGO_MANIFEST_DIR,
//! // not the source file (a stable-Rust proc-macro can't see the call site's
//! // file). So name it from the crate root:
//! const POWER: saudade::SvgImage = include_svg!("assets/icons/power.svg");
//! ```

use std::collections::BTreeSet;
use std::path::PathBuf;

use proc_macro::TokenStream;
use quote::quote;
use syn::{LitStr, parse_macro_input};

use usvg::tiny_skia_path::PathSegment;

/// Flattening tolerance, in viewBox units. The polygons are stored in viewBox
/// space and scaled up at draw time, so this has to stay a good deal finer than
/// one viewBox unit to survive enlargement. 0.05 unit keeps a 32-unit icon
/// crisp well past 8× without exploding the vertex count for these simple
/// marks.
const TOLERANCE: f64 = 0.05;

/// One paint operation baked from a `<path>` (a fill *or* a stroke): a solid
/// ARGB color, a winding rule, and the contours to fill under it.
struct OutPoly {
    argb: u32,
    even_odd: bool,
    rings: Vec<Vec<(f32, f32)>>,
}

/// The whole image: the viewBox box the polygons live in, the polygons in
/// document (painter's-algorithm) order, and a sorted list of human-readable
/// descriptions of any SVG features that fell outside the supported subset and
/// were dropped (used to warn at the call site).
struct OutImage {
    width: f32,
    height: f32,
    polygons: Vec<OutPoly>,
    dropped: Vec<String>,
}

/// Accumulator threaded through the tree walk: the baked polygons plus the set
/// of unsupported features encountered. The set keeps the warning stable and
/// duplicate-free no matter how many nodes trip the same feature.
#[derive(Default)]
struct Baked {
    polygons: Vec<OutPoly>,
    dropped: BTreeSet<&'static str>,
}

/// Embed an SVG file as a `saudade::SvgImage` constant.
///
/// `include_svg!("path/to/icon.svg")` — the path is resolved relative to the
/// invoking crate's `CARGO_MANIFEST_DIR`. The expansion is a `const`-friendly
/// expression, so it can initialize a `const` / `static` or be used inline.
///
/// Any SVG feature outside the bakeable subset (gradient / pattern paint,
/// `clipPath` / `mask` / `filter`, group opacity, embedded `<image>`s, `<text>`)
/// is dropped, and the macro emits a `deprecated`-lint warning at the call site
/// naming what was skipped — so an unexpected SVG fails loudly instead of
/// silently rendering blank.
#[proc_macro]
pub fn include_svg(input: TokenStream) -> TokenStream {
    let lit = parse_macro_input!(input as LitStr);
    let rel = lit.value();

    let manifest = std::env::var("CARGO_MANIFEST_DIR").unwrap_or_default();
    let path = PathBuf::from(&manifest).join(&rel);

    let svg = match std::fs::read_to_string(&path) {
        Ok(s) => s,
        Err(e) => {
            return err(&lit, format!("cannot read {}: {e}", path.display()));
        }
    };

    let image = match tessellate(&svg) {
        Ok(image) => image,
        Err(e) => return err(&lit, e),
    };

    // Absolute path of the resolved file. Emitting an `include_bytes!` of it
    // does nothing at runtime but registers the SVG as a build input, so cargo
    // re-runs this macro whenever the file changes (a stable proc-macro can't
    // call the unstable `tracked_path` API).
    let abs = path.to_string_lossy().into_owned();
    emit(&image, &rel, &abs).into()
}

/// Turn a `syn::Error` into a `compile_error!` token stream anchored at `lit`.
fn err(lit: &LitStr, msg: impl std::fmt::Display) -> TokenStream {
    syn::Error::new(lit.span(), format!("include_svg!: {msg}"))
        .to_compile_error()
        .into()
}

/// Parse + normalize the SVG and flatten every path into solid-color polygons,
/// recording any features that couldn't be baked.
fn tessellate(svg: &str) -> Result<OutImage, String> {
    let options = usvg::Options::default();
    let tree = usvg::Tree::from_str(svg, &options).map_err(|e| e.to_string())?;

    let size = tree.size();
    let mut baked = Baked::default();
    walk(tree.root(), &mut baked);

    Ok(OutImage {
        width: size.width(),
        height: size.height(),
        polygons: baked.polygons,
        dropped: baked.dropped.into_iter().map(str::to_owned).collect(),
    })
}

/// Walk the usvg tree in document order, flattening every visible path. Groups
/// recurse; images and text are outside the supported subset, so they are noted
/// as dropped rather than baked.
fn walk(group: &usvg::Group, baked: &mut Baked) {
    note_group(group, baked);
    for node in group.children() {
        match node {
            usvg::Node::Group(g) => walk(g, baked),
            usvg::Node::Path(p) => emit_path(p, baked),
            usvg::Node::Image(_) => {
                baked.dropped.insert("an embedded raster <image>");
            }
            usvg::Node::Text(_) => {
                baked
                    .dropped
                    .insert("<text> (convert text to paths before baking)");
            }
        }
    }
}

/// Flag group-level compositing the flattener can't reproduce. usvg folds
/// transforms into the absolute path coordinates we read, so those are *not*
/// dropped — but group opacity, clip paths, masks, and filters are silently
/// ignored, which can change the result.
fn note_group(group: &usvg::Group, baked: &mut Baked) {
    if group.opacity().get() < 1.0 {
        baked.dropped.insert("group opacity (baked fully opaque)");
    }
    if group.clip_path().is_some() {
        baked.dropped.insert("a clipPath (ignored)");
    }
    if group.mask().is_some() {
        baked.dropped.insert("a mask (ignored)");
    }
    if !group.filters().is_empty() {
        baked.dropped.insert("a filter (ignored)");
    }
}

/// Flatten one path's fill and stroke into [`OutPoly`]s. usvg gives path data in
/// absolute coordinates with all inheritance already resolved, so this is just:
/// build a kurbo path, fill it, then expand+fill the stroke on top (SVG's
/// default fill-then-stroke order, which every supported icon uses). Non-solid
/// paint (gradients, patterns) can't be baked, so it is noted as dropped.
fn emit_path(path: &usvg::Path, baked: &mut Baked) {
    if !path.is_visible() {
        return;
    }
    let bez = to_bez(path.data());

    if let Some(fill) = path.fill() {
        match fill.paint() {
            usvg::Paint::Color(c) => {
                let rings = flatten_rings(&bez);
                if !rings.is_empty() {
                    baked.polygons.push(OutPoly {
                        argb: argb(c, fill.opacity().get()),
                        even_odd: fill.rule() == usvg::FillRule::EvenOdd,
                        rings,
                    });
                }
            }
            _ => {
                baked.dropped.insert("a gradient or pattern fill");
            }
        }
    }

    if let Some(stroke) = path.stroke() {
        match stroke.paint() {
            usvg::Paint::Color(c) => {
                let outline = expand_stroke(&bez, stroke);
                let rings = flatten_rings(&outline);
                if !rings.is_empty() {
                    baked.polygons.push(OutPoly {
                        argb: argb(c, stroke.opacity().get()),
                        // A stroke outline is always filled nonzero, regardless
                        // of the path's own fill-rule.
                        even_odd: false,
                        rings,
                    });
                }
            }
            _ => {
                baked.dropped.insert("a gradient or pattern stroke");
            }
        }
    }
}

/// Convert tiny-skia (usvg) path segments into a kurbo `BezPath`.
fn to_bez(path: &usvg::tiny_skia_path::Path) -> kurbo::BezPath {
    let pt = |p: usvg::tiny_skia_path::Point| kurbo::Point::new(p.x as f64, p.y as f64);
    let mut bez = kurbo::BezPath::new();
    for seg in path.segments() {
        match seg {
            PathSegment::MoveTo(p) => bez.move_to(pt(p)),
            PathSegment::LineTo(p) => bez.line_to(pt(p)),
            PathSegment::QuadTo(c, p) => bez.quad_to(pt(c), pt(p)),
            PathSegment::CubicTo(c1, c2, p) => bez.curve_to(pt(c1), pt(c2), pt(p)),
            PathSegment::Close => bez.close_path(),
        }
    }
    bez
}

/// Expand a stroked path into the fillable outline of the stroke, mapping the
/// SVG join/cap style onto kurbo's.
fn expand_stroke(bez: &kurbo::BezPath, stroke: &usvg::Stroke) -> kurbo::BezPath {
    let join = match stroke.linejoin() {
        usvg::LineJoin::Miter | usvg::LineJoin::MiterClip => kurbo::Join::Miter,
        usvg::LineJoin::Round => kurbo::Join::Round,
        usvg::LineJoin::Bevel => kurbo::Join::Bevel,
    };
    let cap = match stroke.linecap() {
        usvg::LineCap::Butt => kurbo::Cap::Butt,
        usvg::LineCap::Round => kurbo::Cap::Round,
        usvg::LineCap::Square => kurbo::Cap::Square,
    };
    let style = kurbo::Stroke::new(stroke.width().get() as f64)
        .with_join(join)
        .with_caps(cap)
        .with_miter_limit(stroke.miterlimit().get() as f64);
    kurbo::stroke(
        bez.elements().iter().copied(),
        &style,
        &kurbo::StrokeOpts::default(),
        TOLERANCE,
    )
}

/// Flatten a (possibly curved) kurbo path into closed polygon rings — one ring
/// per subpath. Subpaths with fewer than 3 points carry no area and are dropped.
fn flatten_rings(bez: &kurbo::BezPath) -> Vec<Vec<(f32, f32)>> {
    let mut rings: Vec<Vec<(f32, f32)>> = Vec::new();
    let mut cur: Vec<(f32, f32)> = Vec::new();
    let flush = |cur: &mut Vec<(f32, f32)>, rings: &mut Vec<Vec<(f32, f32)>>| {
        if cur.len() >= 3 {
            rings.push(std::mem::take(cur));
        } else {
            cur.clear();
        }
    };
    kurbo::flatten(bez.elements().iter().copied(), TOLERANCE, |el| match el {
        kurbo::PathEl::MoveTo(p) => {
            flush(&mut cur, &mut rings);
            cur.push((p.x as f32, p.y as f32));
        }
        kurbo::PathEl::LineTo(p) => cur.push((p.x as f32, p.y as f32)),
        kurbo::PathEl::ClosePath => flush(&mut cur, &mut rings),
        // `flatten` only ever yields MoveTo / LineTo / ClosePath.
        kurbo::PathEl::QuadTo(..) | kurbo::PathEl::CurveTo(..) => {}
    });
    flush(&mut cur, &mut rings);
    rings
}

/// Pack a usvg color + opacity into a straight-alpha `0xAARRGGBB` word matching
/// saudade's `Color` layout.
fn argb(c: &usvg::Color, opacity: f32) -> u32 {
    let a = (opacity.clamp(0.0, 1.0) * 255.0).round() as u32;
    (a << 24) | ((c.red as u32) << 16) | ((c.green as u32) << 8) | c.blue as u32
}

/// Emit the `saudade::SvgImage` literal. Wrapped in a block whose leading
/// `const _` registers the source file as a build dependency, and — if anything
/// was dropped — a self-deprecating marker that surfaces a call-site warning.
fn emit(image: &OutImage, rel: &str, abs: &str) -> proc_macro2::TokenStream {
    let width = image.width;
    let height = image.height;
    let warning = unsupported_warning(rel, &image.dropped);

    let polygons = image.polygons.iter().map(|poly| {
        let argb = poly.argb;
        let rule = if poly.even_odd {
            quote!(::saudade::FillRule::EvenOdd)
        } else {
            quote!(::saudade::FillRule::NonZero)
        };
        let rings = poly.rings.iter().map(|ring| {
            let pts = ring.iter().map(|&(x, y)| quote!((#x, #y)));
            quote!(&[#(#pts),*])
        });
        quote! {
            ::saudade::SvgPolygon {
                color: ::saudade::Color(#argb),
                fill_rule: #rule,
                rings: &[#(#rings),*],
            }
        }
    });

    quote! {
        {
            const _: &[u8] = include_bytes!(#abs);
            #warning
            ::saudade::SvgImage {
                width: #width,
                height: #height,
                polygons: &[#(#polygons),*],
            }
        }
    }
}

/// Build the call-site warning for dropped features, or nothing if the SVG baked
/// cleanly.
///
/// Stable proc-macros can't emit diagnostics directly (the `Diagnostic` API is
/// nightly-only), so we lean on the `deprecated` lint instead: define a unit
/// struct carrying the message as its deprecation note and reference it once.
/// rustc then reports it as a `warning` at the macro call site. Everything is
/// `const`-compatible, so it doesn't disturb `const SvgImage = include_svg!(…)`.
fn unsupported_warning(rel: &str, dropped: &[String]) -> proc_macro2::TokenStream {
    if dropped.is_empty() {
        return quote!();
    }
    let note = format!(
        "include_svg!(\"{rel}\"): skipped unsupported SVG feature(s): {}. \
         They were not baked into the icon and will not render — \
         see the supported subset in saudade's `include_svg!` docs.",
        dropped.join(", "),
    );
    quote! {
        #[deprecated(note = #note)]
        #[allow(non_camel_case_types)]
        struct _IncludeSvgUnsupportedFeatures;
        let _ = _IncludeSvgUnsupportedFeatures;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `tessellate` is pure SVG→geometry (no `proc_macro` types), so the bake +
    /// drop-detection logic is unit-testable without expanding the macro.
    #[test]
    fn clean_svg_bakes_with_no_dropped_features() {
        let svg = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 10 10">
            <rect x="1" y="1" width="8" height="8" fill="#102030"/></svg>"##;
        let img = tessellate(svg).unwrap();
        assert_eq!((img.width, img.height), (10.0, 10.0));
        assert!(
            !img.polygons.is_empty(),
            "the rect should bake to a polygon"
        );
        assert!(
            img.dropped.is_empty(),
            "nothing should be dropped: {:?}",
            img.dropped
        );
        // 0xAARRGGBB, fully opaque.
        assert_eq!(img.polygons[0].argb, 0xFF10_2030);
    }

    #[test]
    fn a_stroke_only_path_expands_into_a_fillable_outline() {
        let svg = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 10 10">
            <line x1="1" y1="1" x2="9" y2="9" stroke="#000000" stroke-width="2"/></svg>"##;
        let img = tessellate(svg).unwrap();
        assert!(img.dropped.is_empty());
        assert!(
            !img.polygons.is_empty(),
            "the stroke must become a filled outline polygon",
        );
    }

    #[test]
    fn a_gradient_fill_is_reported_as_dropped() {
        let svg = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 10 10">
            <defs><linearGradient id="g">
                <stop offset="0" stop-color="#ff0000"/>
                <stop offset="1" stop-color="#0000ff"/>
            </linearGradient></defs>
            <rect x="1" y="1" width="8" height="8" fill="url(#g)"/></svg>"##;
        let img = tessellate(svg).unwrap();
        assert!(
            img.dropped.iter().any(|d| d.contains("gradient")),
            "expected a gradient drop, got {:?}",
            img.dropped,
        );
        // The gradient rect is the only shape, so nothing solid baked.
        assert!(img.polygons.is_empty());
    }

    #[test]
    fn unsupported_warning_is_silent_when_nothing_was_dropped() {
        assert!(unsupported_warning("x.svg", &[]).is_empty());
        assert!(!unsupported_warning("x.svg", &["a mask (ignored)".into()]).is_empty());
    }
}
