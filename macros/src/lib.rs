//! `include_svg!` — read an SVG at **compile time** and bake it into a set of
//! flattened, filled polygons that saudade can replay at runtime without any
//! SVG machinery in the binary.
//!
//! The heavy lifting (XML parsing, attribute inheritance, shape→path
//! conversion, transform resolution, curve flattening, stroke-to-outline
//! expansion, and `clip-path` intersection) all happens here, at build time,
//! using [`usvg`] + [`kurbo`] + `i_overlay`. What the macro *emits* is plain
//! geometry: a [`saudade::SvgImage`] holding `&'static` slices of polygon rings
//! and their fill colors, framed to the SVG's declared viewport (so any padding
//! the artwork carries inside its viewBox is preserved). The runtime side only
//! has to fill polygons — see `saudade::svg` — so none of those crates ever
//! reach a shipped program.
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

use i_overlay::core::fill_rule::FillRule as ClipFill;
use i_overlay::core::overlay_rule::OverlayRule;
use i_overlay::float::simplify::SimplifyShape;
use i_overlay::float::single::SingleFloatOverlay;
use usvg::tiny_skia_path::PathSegment;

/// A normalized clip region in canvas coordinates: a set of i_overlay "shapes",
/// each an outer contour followed by its holes. Empty means "clip everything
/// away"; the absence of a clip is carried as `Option<&Region>`, not an empty
/// `Region`. Coordinates are `f64` because i_overlay's boolean engine works in
/// double precision before we round back to the baked `f32` rings.
type Region = Vec<Vec<Vec<[f64; 2]>>>;

/// Flattening tolerance, in viewBox units. The polygons are stored in viewBox
/// space and scaled up at draw time, so this has to stay a good deal finer than
/// one viewBox unit to survive enlargement. 0.05 unit keeps a 32-unit icon
/// crisp well past 8× without exploding the vertex count for these simple
/// marks.
const TOLERANCE: f64 = 0.05;

/// How many flat-colored bands approximate a gradient. The runtime fills only
/// solid polygons, so a gradient is baked as this many slices — strips across a
/// linear gradient, nested disks for a radial one — each clipped to the painted
/// shape. More bands mean a smoother ramp at the cost of more polygons and
/// build-time clipping work; 12 reads as smooth for the small marks this targets.
const GRADIENT_BANDS: usize = 12;

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
/// Element transforms (including the viewBox→viewport mapping), `clip-path`, and
/// gradient paint (approximated by flat-color bands) are honored. Any feature
/// still outside the bakeable subset (pattern paint, `mask` / `filter`, group
/// opacity, embedded `<image>`s, `<text>`) is dropped, and the macro emits a
/// `deprecated`-lint warning at the call site naming what was skipped — so an
/// unexpected SVG fails loudly instead of silently rendering blank.
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

    let mut baked = Baked::default();
    walk(tree.root(), &mut baked, None);

    // Frame the image by the SVG's declared viewport (`tree.size()`), the same
    // box resvg renders into — *not* the tight bounding box of the geometry.
    // `emit_path` already maps every contour through `abs_transform`, which folds
    // in the viewBox→viewport mapping (origin offset + scale), so the baked
    // geometry lives in viewport pixel coordinates: a shifted-origin viewBox
    // lands at 0,0 and a viewBox→viewport scale is applied, both without any
    // re-framing here. Honoring the declared viewport preserves whatever padding
    // the artwork carries inside it — the scrollbar/dropdown/dialog/checkbox
    // marks are deliberately drawn small inside a larger viewBox so the runtime
    // aspect-fit reproduces the classic glyph footprint. Cropping to the content
    // would scale every mark up to fill its rect, which it must not do.
    let size = tree.size();
    let (width, height) = (size.width(), size.height());

    Ok(OutImage {
        width,
        height,
        polygons: baked.polygons,
        dropped: baked.dropped.into_iter().map(str::to_owned).collect(),
    })
}

/// Walk the usvg tree in document order, flattening every visible path. Groups
/// recurse; images and text are outside the supported subset, so they are noted
/// as dropped rather than baked.
///
/// `clip` is the clip region inherited from ancestors (in canvas coordinates),
/// or `None` when nothing constrains this subtree. A group carrying its own
/// `clip-path` intersects its region with the inherited one and passes the
/// result down, so nested clips compose the way SVG specifies.
fn walk(group: &usvg::Group, baked: &mut Baked, clip: Option<&Region>) {
    note_group(group, baked);

    // Fold this group's own clip-path (if any) into the inherited clip.
    let owned_clip: Option<Region> = group.clip_path().map(|cp| {
        let region = clip_region(to_affine(group.abs_transform()), cp);
        match clip {
            Some(parent) => region_intersect(parent, &region),
            None => region,
        }
    });
    let clip = owned_clip.as_ref().or(clip);

    for node in group.children() {
        match node {
            usvg::Node::Group(g) => walk(g, baked, clip),
            usvg::Node::Path(p) => emit_path(p, baked, clip),
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

/// Flag group-level compositing the flattener can't reproduce. Element
/// transforms *are* honored — `emit_path` bakes each contour through its
/// `abs_transform` (the full ancestor chain, viewBox→viewport included) — and
/// `clip-path` is applied geometrically (see [`walk`]/[`clip_region`]). Group
/// opacity, masks, and filters are still silently ignored, which can change the
/// result.
fn note_group(group: &usvg::Group, baked: &mut Baked) {
    if group.opacity().get() < 1.0 {
        baked.dropped.insert("group opacity (baked fully opaque)");
    }
    if group.mask().is_some() {
        baked.dropped.insert("a mask (ignored)");
    }
    if !group.filters().is_empty() {
        baked.dropped.insert("a filter (ignored)");
    }
}

/// Flatten one path's fill and stroke into [`OutPoly`]s. usvg keeps path data in
/// the element's *local* coordinates and hands the element→canvas mapping out
/// separately as [`abs_transform`](usvg::Path::abs_transform) (the full ancestor
/// chain, viewBox→viewport scale included), so each contour is mapped into
/// canvas space before flattening: fill the path, then expand+fill the stroke on
/// top (SVG's default fill-then-stroke order, which every supported icon uses).
/// Solid paint bakes directly; gradient paint is approximated by a stack of
/// flat-colored bands (see [`emit_gradient`]); only patterns are dropped.
fn emit_path(path: &usvg::Path, baked: &mut Baked, clip: Option<&Region>) {
    if !path.is_visible() {
        return;
    }
    let affine = to_affine(path.abs_transform());
    let local = to_bez(path.data());

    if let Some(fill) = path.fill() {
        let even_odd = fill.rule() == usvg::FillRule::EvenOdd;
        match fill.paint() {
            usvg::Paint::Color(c) => {
                let mut bez = local.clone();
                bez.apply_affine(affine);
                let rings = flatten_rings(&bez);
                push_clipped(baked, argb(c, fill.opacity().get()), even_odd, rings, clip);
            }
            usvg::Paint::Pattern(_) => {
                baked.dropped.insert("a pattern fill");
            }
            paint => {
                let mut bez = local.clone();
                bez.apply_affine(affine);
                let rings = flatten_rings(&bez);
                emit_gradient(baked, &rings, even_odd, paint, affine, clip);
            }
        }
    }

    if let Some(stroke) = path.stroke() {
        // Stroke width is defined in the path's local units, so expand the
        // outline there and *then* map it to canvas space — SVG applies the
        // transform to the already-stroked shape, so a non-uniform transform
        // skews the stroke just as a browser would. A stroke outline is always
        // filled nonzero, regardless of the path's own fill-rule.
        match stroke.paint() {
            usvg::Paint::Color(c) => {
                let mut outline = expand_stroke(&local, stroke);
                outline.apply_affine(affine);
                let rings = flatten_rings(&outline);
                push_clipped(baked, argb(c, stroke.opacity().get()), false, rings, clip);
            }
            usvg::Paint::Pattern(_) => {
                baked.dropped.insert("a pattern stroke");
            }
            paint => {
                let mut outline = expand_stroke(&local, stroke);
                outline.apply_affine(affine);
                let rings = flatten_rings(&outline);
                emit_gradient(baked, &rings, false, paint, affine, clip);
            }
        }
    }
}

/// Push one baked paint operation, applying the active clip region first. With
/// no clip the rings go straight in; with a clip the contour set is intersected
/// against the clip and the result re-emitted. The intersection output is always
/// nonzero-fillable (outer contours wound opposite their holes), so the clipped
/// polygon is stored as `NonZero` regardless of the source `even_odd`.
fn push_clipped(
    baked: &mut Baked,
    argb: u32,
    even_odd: bool,
    rings: Vec<Vec<(f32, f32)>>,
    clip: Option<&Region>,
) {
    if rings.iter().all(|r| r.len() < 3) {
        return;
    }
    match clip {
        None => baked.polygons.push(OutPoly {
            argb,
            even_odd,
            rings,
        }),
        Some(region) => {
            // Resolve the subject under its own fill-rule, then intersect the
            // resulting region with the clip (both nonzero-normalized).
            let rule = if even_odd {
                ClipFill::EvenOdd
            } else {
                ClipFill::NonZero
            };
            let subject = shape_to_f64(&rings).simplify_shape(rule);
            if subject.is_empty() {
                return;
            }
            let clipped = subject.overlay(region, OverlayRule::Intersect, ClipFill::NonZero);
            let out = shapes_to_rings(clipped);
            if !out.is_empty() {
                baked.polygons.push(OutPoly {
                    argb,
                    even_odd: false,
                    rings: out,
                });
            }
        }
    }
}

/// Approximate a gradient-filled shape by a stack of flat-colored bands. The
/// runtime only fills solid polygons, so a smooth gradient becomes
/// [`GRADIENT_BANDS`] slices, each clipped to the painted geometry (`rings`, in
/// canvas coordinates) and to the active `clip`:
///
/// * a linear gradient becomes parallel strips across its axis;
/// * a radial gradient becomes nested disks painted outer-to-inner, so the inner
///   stops land on top.
///
/// `abs` is the path's canvas transform; composed with the gradient's own
/// transform it maps gradient coordinates into canvas space. Patterns (the only
/// remaining non-solid paint) are handled by the caller. `spreadMethod` other
/// than `pad` and a radial focal point offset are approximated as pad/centered.
fn emit_gradient(
    baked: &mut Baked,
    rings: &[Vec<(f32, f32)>],
    even_odd: bool,
    paint: &usvg::Paint,
    abs: kurbo::Affine,
    clip: Option<&Region>,
) {
    // (whole-shape base color drawn first — the radial pad region, None for
    // linear, where strips cover everything) and the ordered band list.
    let (base, bands) = match paint {
        usvg::Paint::LinearGradient(g) => {
            if g.stops().is_empty() {
                return;
            }
            (None, linear_bands(g, abs * to_affine(g.transform())))
        }
        usvg::Paint::RadialGradient(g) => {
            if g.stops().is_empty() {
                return;
            }
            let base = sample_stops(g.stops(), 1.0);
            (Some(base), radial_bands(g, abs * to_affine(g.transform())))
        }
        _ => return,
    };

    // Resolve the painted shape under its own fill-rule and clip it once; every
    // band is then just an intersection against this region.
    let rule = if even_odd {
        ClipFill::EvenOdd
    } else {
        ClipFill::NonZero
    };
    let mut subject = shape_to_f64(rings).simplify_shape(rule);
    if let Some(c) = clip {
        subject = region_intersect(&subject, c);
    }
    if subject.is_empty() {
        return;
    }

    if let Some(argb) = base {
        let out = shapes_to_rings(subject.clone());
        if !out.is_empty() {
            baked.polygons.push(OutPoly {
                argb,
                even_odd: false,
                rings: out,
            });
        }
    }
    for (argb, band) in bands {
        let piece = region_intersect(&subject, &band);
        if !piece.is_empty() {
            baked.polygons.push(OutPoly {
                argb,
                even_odd: false,
                rings: shapes_to_rings(piece),
            });
        }
    }
}

/// Parallel strips across a linear gradient's axis, in canvas coordinates. Each
/// strip spans one band's parameter range along `p1`→`p2`; the first and last
/// extend to ±infinity so the `pad` regions beyond the stops are covered.
fn linear_bands(g: &usvg::LinearGradient, m: kurbo::Affine) -> Vec<(u32, Region)> {
    let stops = g.stops();
    let p1 = kurbo::Point::new(g.x1() as f64, g.y1() as f64);
    let p2 = kurbo::Point::new(g.x2() as f64, g.y2() as f64);
    let axis = p2 - p1;
    // A span far larger than any mark; strips are clipped to the shape anyway.
    const BIG: f64 = 1.0e4;
    if axis.hypot2() == 0.0 {
        // Degenerate axis: one flat band of the mid color over everything.
        let quad = square_region(p1, BIG, m);
        return vec![(sample_stops(stops, 0.5), quad)];
    }
    let perp = kurbo::Vec2::new(-axis.y, axis.x).normalize() * BIG;
    let mut bands = Vec::with_capacity(GRADIENT_BANDS);
    for k in 0..GRADIENT_BANDS {
        let a = if k == 0 {
            -BIG
        } else {
            k as f64 / GRADIENT_BANDS as f64
        };
        let b = if k == GRADIENT_BANDS - 1 {
            BIG
        } else {
            (k + 1) as f64 / GRADIENT_BANDS as f64
        };
        let pa = p1 + axis * a;
        let pb = p1 + axis * b;
        let quad = [pa - perp, pb - perp, pb + perp, pa + perp];
        let contour: Vec<[f64; 2]> = quad.iter().map(|&pt| map_point(m, pt)).collect();
        let color = sample_stops(stops, (k as f32 + 0.5) / GRADIENT_BANDS as f32);
        bands.push((color, vec![vec![contour]]));
    }
    bands
}

/// Nested disks for a radial gradient, ordered largest (outer, last stop) to
/// smallest (inner, first stop) so later disks paint over earlier ones. The
/// caller fills the whole shape with the last-stop color first to cover the
/// `pad` region beyond the radius; the focal point is approximated by the center.
fn radial_bands(g: &usvg::RadialGradient, m: kurbo::Affine) -> Vec<(u32, Region)> {
    let stops = g.stops();
    let center = kurbo::Point::new(g.cx() as f64, g.cy() as f64);
    let r = g.r().get() as f64;
    let mut bands = Vec::with_capacity(GRADIENT_BANDS);
    for k in (1..=GRADIENT_BANDS).rev() {
        let radius = r * k as f64 / GRADIENT_BANDS as f64;
        let color = sample_stops(stops, (k as f32 - 0.5) / GRADIENT_BANDS as f32);
        bands.push((color, vec![vec![circle(center, radius, m)]]));
    }
    bands
}

/// A `radius`-sized circle around `center` (gradient space) as a polygon ring in
/// canvas coordinates. A non-uniform `m` turns it into the matching ellipse.
fn circle(center: kurbo::Point, radius: f64, m: kurbo::Affine) -> Vec<[f64; 2]> {
    const SEGMENTS: usize = 64;
    (0..SEGMENTS)
        .map(|i| {
            let a = i as f64 / SEGMENTS as f64 * std::f64::consts::TAU;
            map_point(
                m,
                kurbo::Point::new(center.x + radius * a.cos(), center.y + radius * a.sin()),
            )
        })
        .collect()
}

/// An axis-aligned square of half-extent `half` around `center`, as a one-shape
/// region in canvas coordinates. Used as a catch-all band for degenerate axes.
fn square_region(center: kurbo::Point, half: f64, m: kurbo::Affine) -> Region {
    let corners = [
        kurbo::Point::new(center.x - half, center.y - half),
        kurbo::Point::new(center.x + half, center.y - half),
        kurbo::Point::new(center.x + half, center.y + half),
        kurbo::Point::new(center.x - half, center.y + half),
    ];
    vec![vec![corners.iter().map(|&p| map_point(m, p)).collect()]]
}

/// Apply `m` to a point and return it as an `[f64; 2]` for i_overlay.
fn map_point(m: kurbo::Affine, p: kurbo::Point) -> [f64; 2] {
    let q = m * p;
    [q.x, q.y]
}

/// Sample a gradient's stops at parameter `t`, returning a straight-alpha
/// `0xAARRGGBB` word. Color and alpha are interpolated linearly in sRGB between
/// the bracketing stops (SVG's default), and `t` is clamped to the stop range so
/// the ends act as `pad`. `stops` must be non-empty and offset-sorted (usvg
/// guarantees both).
fn sample_stops(stops: &[usvg::Stop], t: f32) -> u32 {
    let lerp = |a: f32, b: f32, f: f32| a + (b - a) * f;
    let rgba = |s: &usvg::Stop| {
        let c = s.color();
        [
            c.red as f32,
            c.green as f32,
            c.blue as f32,
            s.opacity().get() * 255.0,
        ]
    };
    let pack = |c: [f32; 4]| {
        let q = |v: f32| v.round().clamp(0.0, 255.0) as u32;
        (q(c[3]) << 24) | (q(c[0]) << 16) | (q(c[1]) << 8) | q(c[2])
    };

    let first = &stops[0];
    let last = &stops[stops.len() - 1];
    if t <= first.offset().get() {
        return pack(rgba(first));
    }
    if t >= last.offset().get() {
        return pack(rgba(last));
    }
    for pair in stops.windows(2) {
        let (lo, hi) = (&pair[0], &pair[1]);
        let (a, b) = (lo.offset().get(), hi.offset().get());
        if t >= a && t <= b {
            let f = if b > a { (t - a) / (b - a) } else { 0.0 };
            let (ca, cb) = (rgba(lo), rgba(hi));
            return pack([
                lerp(ca[0], cb[0], f),
                lerp(ca[1], cb[1], f),
                lerp(ca[2], cb[2], f),
                lerp(ca[3], cb[3], f),
            ]);
        }
    }
    pack(rgba(last))
}

/// Build a clip region in canvas coordinates from a usvg [`ClipPath`] referenced
/// by an element whose canvas transform is `base`. Mirrors resvg's compositing:
/// the clip's shapes are drawn at `base · clip.transform()`, and a `clip-path`
/// on the clipPath itself further intersects the region.
fn clip_region(base: kurbo::Affine, clip: &usvg::ClipPath) -> Region {
    let t = base * to_affine(clip.transform());
    let mut region = group_region(clip.root(), t);
    if let Some(inner) = clip.clip_path() {
        let inner_region = clip_region(base, inner);
        region = region_intersect(&region, &inner_region);
    }
    region
}

/// Union of every shape drawn by `group`'s children, transformed by `t` into
/// canvas coordinates — the filled area that acts as a clip. Each path is
/// resolved under its own clip-rule; nested groups recurse, applying their own
/// `clip-path` before being unioned in. Paint is irrelevant here: a clip cares
/// only about geometry, so even gradient-filled clip shapes contribute.
fn group_region(group: &usvg::Group, t: kurbo::Affine) -> Region {
    let mut region: Region = Vec::new();
    for node in group.children() {
        match node {
            usvg::Node::Path(p) => {
                if !p.is_visible() {
                    continue;
                }
                let mut bez = to_bez(p.data());
                bez.apply_affine(t);
                let rings = flatten_rings(&bez);
                if rings.is_empty() {
                    continue;
                }
                let rule = p
                    .fill()
                    .map(|f| f.rule())
                    .unwrap_or(usvg::FillRule::NonZero);
                let shape = shape_to_f64(&rings).simplify_shape(to_clip_fill(rule));
                region = region_union(region, shape);
            }
            usvg::Node::Group(g) => {
                let sub_t = t * to_affine(g.transform());
                let mut sub = group_region(g, sub_t);
                if let Some(gc) = g.clip_path() {
                    let gc_region = clip_region(sub_t, gc);
                    sub = region_intersect(&sub, &gc_region);
                }
                region = region_union(region, sub);
            }
            _ => {}
        }
    }
    region
}

/// Intersect two normalized clip regions.
fn region_intersect(a: &Region, b: &Region) -> Region {
    a.overlay(b, OverlayRule::Intersect, ClipFill::NonZero)
}

/// Union two normalized clip regions, short-circuiting the empty cases (an empty
/// region carries no area, so the union is just the other operand).
fn region_union(a: Region, b: Region) -> Region {
    if a.is_empty() {
        b
    } else if b.is_empty() {
        a
    } else {
        a.overlay(&b, OverlayRule::Union, ClipFill::NonZero)
    }
}

/// Convert baked `(f32, f32)` rings into i_overlay's `[f64; 2]` contour shape.
fn shape_to_f64(rings: &[Vec<(f32, f32)>]) -> Vec<Vec<[f64; 2]>> {
    rings
        .iter()
        .map(|ring| ring.iter().map(|&(x, y)| [x as f64, y as f64]).collect())
        .collect()
}

/// Flatten i_overlay's `Shapes` result back into baked `(f32, f32)` rings,
/// dropping any contour too small to carry area.
fn shapes_to_rings(shapes: Region) -> Vec<Vec<(f32, f32)>> {
    let mut out = Vec::new();
    for shape in shapes {
        for contour in shape {
            if contour.len() >= 3 {
                out.push(
                    contour
                        .into_iter()
                        .map(|p| (p[0] as f32, p[1] as f32))
                        .collect(),
                );
            }
        }
    }
    out
}

/// Map usvg's fill/clip rule onto i_overlay's.
fn to_clip_fill(rule: usvg::FillRule) -> ClipFill {
    match rule {
        usvg::FillRule::EvenOdd => ClipFill::EvenOdd,
        usvg::FillRule::NonZero => ClipFill::NonZero,
    }
}

/// Convert a usvg (tiny-skia) affine transform into the kurbo equivalent.
/// tiny-skia maps `(x, y)` to `(sx·x + kx·y + tx, ky·x + sy·y + ty)`; kurbo's
/// `Affine::new([a, b, c, d, e, f])` maps it to `(a·x + c·y + e, b·x + d·y + f)`.
fn to_affine(ts: usvg::Transform) -> kurbo::Affine {
    kurbo::Affine::new([
        ts.sx as f64,
        ts.ky as f64,
        ts.kx as f64,
        ts.sy as f64,
        ts.tx as f64,
        ts.ty as f64,
    ])
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

    /// Tight bounding box of every baked point: `(min_x, min_y, max_x, max_y)`.
    fn bounds(img: &OutImage) -> (f32, f32, f32, f32) {
        let (mut nx, mut ny, mut xx, mut xy) = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
        for poly in &img.polygons {
            for ring in &poly.rings {
                for &(x, y) in ring {
                    nx = nx.min(x);
                    ny = ny.min(y);
                    xx = xx.max(x);
                    xy = xy.max(y);
                }
            }
        }
        (nx, ny, xx, xy)
    }

    /// `tessellate` is pure SVG→geometry (no `proc_macro` types), so the bake +
    /// drop-detection logic is unit-testable without expanding the macro.
    #[test]
    fn clean_svg_bakes_with_no_dropped_features() {
        let svg = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 10 10">
            <rect x="1" y="1" width="8" height="8" fill="#102030"/></svg>"##;
        let img = tessellate(svg).unwrap();
        // The image is framed to the declared viewport (the 10×10 canvas), not
        // the 8×8 rect — the 1-unit margin around the rect is part of the image,
        // so the runtime aspect-fit preserves it.
        assert_eq!((img.width, img.height), (10.0, 10.0));
        // The rect itself bakes at its viewBox position (1,1)–(9,9).
        let (nx, ny, xx, xy) = bounds(&img);
        assert!(
            (nx - 1.0).abs() < 0.1
                && (ny - 1.0).abs() < 0.1
                && (xx - 9.0).abs() < 0.1
                && (xy - 9.0).abs() < 0.1,
            "rect should keep its 1-unit margin, got ({nx},{ny})–({xx},{xy})",
        );
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

    /// A viewBox that doesn't start at (0,0) is mapped into the viewport by
    /// `abs_transform`, so the geometry lands in 0..viewport space — nothing in
    /// negative coordinates (the bug that pushed Ubuntu's mark off-frame), and no
    /// re-framing needed.
    #[test]
    fn shifted_viewbox_origin_maps_into_the_viewport() {
        let svg = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="-5 -5 10 10">
            <rect x="-4" y="-4" width="8" height="8" fill="#102030"/></svg>"##;
        let img = tessellate(svg).unwrap();
        // No width/height, so the viewport is the viewBox's 10×10.
        assert!((img.width - 10.0).abs() < 0.1 && (img.height - 10.0).abs() < 0.1);
        // The viewBox→viewport translation puts the -4..4 rect at 1..9 — inside
        // the viewport, never negative.
        let (nx, ny, xx, xy) = bounds(&img);
        assert!(
            nx >= -0.01 && ny >= -0.01,
            "content must map out of negative space, got min ({nx}, {ny})",
        );
        assert!(
            (nx - 1.0).abs() < 0.1
                && (ny - 1.0).abs() < 0.1
                && (xx - 9.0).abs() < 0.1
                && (xy - 9.0).abs() < 0.1,
            "rect should map to (1,1)–(9,9), got ({nx},{ny})–({xx},{xy})",
        );
    }

    /// A canvas far larger than the drawing (artwork tucked into one corner of a
    /// big `width`/`height`) keeps the declared viewport — the drawing stays in
    /// its corner, exactly as resvg renders it. Honoring the viewport, not the
    /// content box, is what preserves intentional padding.
    #[test]
    fn oversized_canvas_keeps_the_declared_viewport() {
        let svg = r##"<svg xmlns="http://www.w3.org/2000/svg" width="100" height="100">
            <rect x="10" y="10" width="20" height="30" fill="#102030"/></svg>"##;
        let img = tessellate(svg).unwrap();
        assert_eq!(
            (img.width, img.height),
            (100.0, 100.0),
            "image should be the declared 100×100 canvas, not the 20×30 drawing",
        );
        // The drawing keeps its position in the corner: (10,10)–(30,40).
        let (nx, ny, xx, xy) = bounds(&img);
        assert!(
            (nx - 10.0).abs() < 0.1
                && (ny - 10.0).abs() < 0.1
                && (xx - 30.0).abs() < 0.1
                && (xy - 40.0).abs() < 0.1,
            "drawing should stay in its corner, got ({nx},{ny})–({xx},{xy})",
        );
    }

    /// When the `<svg>` width/height differ from the viewBox, the geometry lives
    /// in viewBox units but is baked in viewport units — i.e. the viewBox→viewport
    /// scale is applied to the geometry (the bug that shrank Fedora's mark), while
    /// the image keeps the declared viewport size.
    #[test]
    fn viewbox_to_viewport_scale_is_applied() {
        // viewBox is 10 wide but the viewport is 100, a 10× scale. A 5-unit rect
        // in viewBox space bakes to 50 viewport units; without the scale it would
        // be 5. The image itself is the full 100×100 viewport.
        let svg = r##"<svg xmlns="http://www.w3.org/2000/svg" width="100" height="100" viewBox="0 0 10 10">
            <rect x="0" y="0" width="5" height="5" fill="#102030"/></svg>"##;
        let img = tessellate(svg).unwrap();
        assert!(
            (img.width - 100.0).abs() < 0.5 && (img.height - 100.0).abs() < 0.5,
            "image should be the 100×100 viewport, got {}×{}",
            img.width,
            img.height,
        );
        let (_, _, xx, xy) = bounds(&img);
        assert!(
            (xx - 50.0).abs() < 0.5 && (xy - 50.0).abs() < 0.5,
            "expected the 10× viewBox→viewport scale on the geometry, got max ({xx}, {xy})",
        );
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

    /// A `clip-path` is applied geometrically: a wide rectangle clipped to a
    /// tall one yields their overlap, and the clip is no longer reported as a
    /// dropped feature.
    #[test]
    fn clip_path_intersects_the_drawn_shape() {
        // A 10-wide, 2-tall bar clipped to a 2-wide, 10-tall bar: the visible
        // result is their 2×2 overlap in the middle.
        let svg = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 10 10">
            <defs><clipPath id="c"><rect x="4" y="0" width="2" height="10"/></clipPath></defs>
            <g clip-path="url(#c)">
                <rect x="0" y="4" width="10" height="2" fill="#102030"/>
            </g></svg>"##;
        let img = tessellate(svg).unwrap();
        assert!(
            img.dropped.is_empty(),
            "clipPath must not be reported as dropped: {:?}",
            img.dropped,
        );
        assert!(!img.polygons.is_empty(), "the clipped bar should bake");
        // The image keeps the 10×10 viewport; only the baked geometry is the 2×2
        // overlap, sitting at (4,4)–(6,6) where the bar and clip cross.
        assert!(
            (img.width - 10.0).abs() < 0.1 && (img.height - 10.0).abs() < 0.1,
            "image should keep the 10×10 viewport, got {}×{}",
            img.width,
            img.height,
        );
        let (nx, ny, xx, xy) = bounds(&img);
        assert!(
            (nx - 4.0).abs() < 0.1
                && (ny - 4.0).abs() < 0.1
                && (xx - 6.0).abs() < 0.1
                && (xy - 6.0).abs() < 0.1,
            "expected the 2×2 intersection at (4,4)–(6,6), got ({nx},{ny})–({xx},{xy})",
        );
    }

    /// A shape drawn entirely outside its clip region bakes to nothing.
    #[test]
    fn clip_path_can_remove_a_shape_entirely() {
        let svg = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 10 10">
            <defs><clipPath id="c"><rect x="0" y="0" width="2" height="2"/></clipPath></defs>
            <g clip-path="url(#c)">
                <rect x="6" y="6" width="3" height="3" fill="#102030"/>
            </g></svg>"##;
        let img = tessellate(svg).unwrap();
        assert!(
            img.polygons.is_empty(),
            "a shape outside its clip should bake to nothing",
        );
    }

    /// A linear gradient is approximated by a stack of flat bands rather than
    /// dropped: the shape bakes to several polygons whose colors span red→blue,
    /// and nothing is reported as unsupported.
    #[test]
    fn a_linear_gradient_bakes_as_color_bands() {
        let svg = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 10 10">
            <defs><linearGradient id="g">
                <stop offset="0" stop-color="#ff0000"/>
                <stop offset="1" stop-color="#0000ff"/>
            </linearGradient></defs>
            <rect x="1" y="1" width="8" height="8" fill="url(#g)"/></svg>"##;
        let img = tessellate(svg).unwrap();
        assert!(
            img.dropped.is_empty(),
            "a gradient should be approximated, not dropped: {:?}",
            img.dropped,
        );
        assert!(
            img.polygons.len() > 1,
            "expected several bands, got {}",
            img.polygons.len(),
        );
        // Bands range from (near) pure red to (near) pure blue.
        let reds = img.polygons.iter().filter(|p| p.argb & 0x00FF_0000 != 0);
        let blues = img.polygons.iter().filter(|p| p.argb & 0x0000_00FF != 0);
        assert!(
            reds.count() > 0 && blues.count() > 0,
            "expected a red→blue ramp"
        );
    }

    /// A radial gradient also bakes (as nested disks) instead of being dropped.
    #[test]
    fn a_radial_gradient_bakes_instead_of_dropping() {
        let svg = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 10 10">
            <defs><radialGradient id="g" gradientUnits="userSpaceOnUse" cx="5" cy="5" r="5">
                <stop offset="0" stop-color="#ffffff"/>
                <stop offset="1" stop-color="#102030"/>
            </radialGradient></defs>
            <rect x="0" y="0" width="10" height="10" fill="url(#g)"/></svg>"##;
        let img = tessellate(svg).unwrap();
        assert!(img.dropped.is_empty(), "got {:?}", img.dropped);
        assert!(img.polygons.len() > 1, "expected nested disks");
    }

    /// A pattern fill is still reported as dropped (only gradients are baked).
    #[test]
    fn a_pattern_fill_is_reported_as_dropped() {
        let svg = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 10 10">
            <defs><pattern id="p" width="2" height="2" patternUnits="userSpaceOnUse">
                <rect width="1" height="1" fill="#000000"/>
            </pattern></defs>
            <rect x="1" y="1" width="8" height="8" fill="url(#p)"/></svg>"##;
        let img = tessellate(svg).unwrap();
        assert!(
            img.dropped.iter().any(|d| d.contains("pattern")),
            "expected a pattern drop, got {:?}",
            img.dropped,
        );
    }

    #[test]
    fn unsupported_warning_is_silent_when_nothing_was_dropped() {
        assert!(unsupported_warning("x.svg", &[]).is_empty());
        assert!(!unsupported_warning("x.svg", &["a mask (ignored)".into()]).is_empty());
    }
}
