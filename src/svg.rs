//! Runtime side of [`include_svg!`](crate::include_svg): replaying baked SVGs.
//!
//! An [`SvgImage`] is *not* parsed at runtime. It is produced at compile time by
//! the `include_svg!` macro, which does all of the SVG work (parsing, curve
//! flattening, stroke expansion) and emits the result as `&'static` polygon
//! data. All this module has to do is fill those polygons — so a saudade binary
//! carries no SVG parser, no `usvg`, no `resvg`, nothing of that tree.
//!
//! The polygons are stored in the SVG's own viewBox coordinate space; [`SvgImage::draw`]
//! scales them to fit the target rectangle (aspect-preserving, centered) and
//! fills each one with an anti-aliased scanline rasterizer, blending straight
//! onto whatever is already in the buffer in document order — the painter's
//! algorithm SVG itself uses. Because the geometry is resolution-independent,
//! the same image fills crisply at any DPI or size with no per-size raster cache
//! (the thing resvg forces, since re-rasterizing it is expensive).

use std::cmp::Ordering;

use crate::geometry::{Color, Rect};
use crate::painter::Painter;

/// Vertical supersampling factor. Each output row is probed at `SAMPLES`
/// sub-scanlines; horizontal coverage within a row is computed analytically
/// from the exact span endpoints. 4 sub-scanlines is plenty for the small marks
/// this is built for and keeps the per-row cost trivial.
const SAMPLES: i32 = 4;

/// Which rule decides whether a point is inside an overlapping set of contours.
/// Mirrors SVG's `fill-rule`. Stroke outlines are always filled [`NonZero`].
///
/// [`NonZero`]: FillRule::NonZero
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum FillRule {
    /// Inside where the signed winding number is non-zero.
    NonZero,
    /// Inside where an odd number of contour crossings lie to the left.
    EvenOdd,
}

/// One solid-color paint operation: a set of contours (`rings`) filled together
/// with one color under one [`FillRule`]. A `<path>`'s fill and its stroke each
/// become one of these. Built by `include_svg!`; not meant to be hand-written.
#[derive(Clone, Copy)]
pub struct SvgPolygon {
    /// Straight-alpha fill color (`0xAARRGGBB`).
    pub color: Color,
    /// Winding rule for this contour set.
    pub fill_rule: FillRule,
    /// Closed contours, each a ring of `(x, y)` points in viewBox coordinates.
    pub rings: &'static [&'static [(f32, f32)]],
}

/// A compile-time-baked vector image: the viewBox box its geometry lives in,
/// plus the polygons to paint, in back-to-front document order.
///
/// Produced by [`include_svg!`](crate::include_svg). Draw it with
/// [`SvgImage::draw`] (or [`Painter::draw_svg`]).
#[derive(Clone, Copy)]
pub struct SvgImage {
    /// viewBox width the polygon coordinates are expressed in.
    pub width: f32,
    /// viewBox height the polygon coordinates are expressed in.
    pub height: f32,
    /// Polygons in document order; later ones paint over earlier ones.
    pub polygons: &'static [SvgPolygon],
}

impl SvgImage {
    /// Fill the image into the logical-pixel rectangle `rect`, scaled to fit
    /// while preserving aspect ratio and centered, anti-aliased and blended over
    /// the current buffer contents.
    ///
    /// Like saudade's other DPI-aware drawing, this drops to physical pixels via
    /// [`Painter::physical`], so the mark is rasterized crisply at the device
    /// footprint the live scale factor implies — not a logical raster stretched
    /// up.
    pub fn draw(&self, painter: &mut Painter, rect: Rect) {
        self.render(painter, rect, None);
    }

    /// Like [`draw`](Self::draw), but paint every polygon in `tint` instead of
    /// its baked color (anti-aliased coverage is preserved). Intended for
    /// single-color glyphs whose SVG color is just a placeholder — e.g. a
    /// scrollbar arrow that should follow the theme's text color. A multi-color
    /// image would be flattened to `tint`, which is rarely what you want.
    pub fn draw_tinted(&self, painter: &mut Painter, rect: Rect, tint: Color) {
        self.render(painter, rect, Some(tint));
    }

    /// Shared body of [`draw`](Self::draw) / [`draw_tinted`](Self::draw_tinted):
    /// `tint` overrides every polygon's color when `Some`.
    fn render(&self, painter: &mut Painter, rect: Rect, tint: Option<Color>) {
        if self.width <= 0.0 || self.height <= 0.0 || self.polygons.is_empty() {
            return;
        }
        painter.physical(rect, |p, phys| self.fill_phys(p, phys, tint));
    }

    /// Fill into the physical-pixel rectangle `phys` (origin-relative device
    /// coordinates, as handed in by [`Painter::physical`]).
    fn fill_phys(&self, painter: &mut Painter, phys: Rect, tint: Option<Color>) {
        if phys.w <= 0 || phys.h <= 0 {
            return;
        }
        // Aspect-fit the viewBox into the footprint and center it.
        let scale = (phys.w as f32 / self.width).min(phys.h as f32 / self.height);
        if scale <= 0.0 {
            return;
        }
        let tx = phys.x as f32 + (phys.w as f32 - self.width * scale) * 0.5;
        let ty = phys.y as f32 + (phys.h as f32 - self.height * scale) * 0.5;

        let mut raster = Rasterizer::new(phys);
        for poly in self.polygons {
            raster.fill(painter, poly, scale, tx, ty, tint);
        }
    }
}

/// A single non-horizontal polygon edge, oriented top-to-bottom, plus the
/// winding direction of the original (un-oriented) edge.
struct Edge {
    /// Top / bottom y of the edge (`y_top < y_bot`).
    y_top: f32,
    y_bot: f32,
    /// x at `y_top`, and dx/dy so x at any y is `x_top + (y - y_top) * dxdy`.
    x_top: f32,
    dxdy: f32,
    /// `+1` if the source edge ran downward, `-1` if upward.
    dir: i32,
}

/// Reused scratch for filling polygons within one footprint, so per-polygon
/// work allocates nothing beyond its own edge list.
struct Rasterizer {
    /// Footprint bounds in device pixels: `[x0, x1) × [y0, y1)`.
    x0: i32,
    y0: i32,
    x1: i32,
    y1: i32,
    /// Per-row coverage accumulator, sized to the footprint width and reused.
    coverage: Vec<f32>,
    /// Edge/x-crossing scratch, reused across rows.
    edges: Vec<Edge>,
    crossings: Vec<(f32, i32)>,
}

impl Rasterizer {
    fn new(phys: Rect) -> Self {
        let width = phys.w.max(0) as usize;
        Self {
            x0: phys.x,
            y0: phys.y,
            x1: phys.x + phys.w,
            y1: phys.y + phys.h,
            coverage: vec![0.0; width],
            edges: Vec::new(),
            crossings: Vec::new(),
        }
    }

    /// Fill one polygon, transforming its rings by `p * scale + (tx, ty)` and
    /// blending the result onto `painter`. `tint`, when `Some`, replaces the
    /// polygon's own color (its alpha still scales the anti-aliased coverage).
    fn fill(
        &mut self,
        painter: &mut Painter,
        poly: &SvgPolygon,
        scale: f32,
        tx: f32,
        ty: f32,
        tint: Option<Color>,
    ) {
        self.edges.clear();
        let map = |&(x, y): &(f32, f32)| (tx + x * scale, ty + y * scale);

        // Build edges and the polygon's tight device-pixel bounding box.
        let (mut min_x, mut min_y) = (f32::MAX, f32::MAX);
        let (mut max_x, mut max_y) = (f32::MIN, f32::MIN);
        for ring in poly.rings {
            let n = ring.len();
            if n < 3 {
                continue;
            }
            for i in 0..n {
                let (x, y) = map(&ring[i]);
                min_x = min_x.min(x);
                min_y = min_y.min(y);
                max_x = max_x.max(x);
                max_y = max_y.max(y);
                let (xa, ya) = (x, y);
                let (xb, yb) = map(&ring[(i + 1) % n]);
                if ya == yb {
                    continue; // horizontal edges never cross a scanline
                }
                let (y_top, x_top, y_bot, x_bot, dir) = if ya < yb {
                    (ya, xa, yb, xb, 1)
                } else {
                    (yb, xb, ya, xa, -1)
                };
                self.edges.push(Edge {
                    y_top,
                    y_bot,
                    x_top,
                    dxdy: (x_bot - x_top) / (y_bot - y_top),
                    dir,
                });
            }
        }
        if self.edges.is_empty() {
            return;
        }

        // Clamp the scan range to the footprint; blend_pixel_phys clips the rest.
        let row_lo = (min_y.floor() as i32).max(self.y0);
        let row_hi = (max_y.ceil() as i32).min(self.y1);
        let col_lo = (min_x.floor() as i32).max(self.x0);
        let col_hi = (max_x.ceil() as i32).min(self.x1);
        if row_hi <= row_lo || col_hi <= col_lo {
            return;
        }

        // Window the coverage row to the polygon's own column span, not the
        // whole footprint — a handful of small polygons over a large footprint
        // would otherwise pay to zero and scan hundreds of empty columns a row.
        let width = (col_hi - col_lo) as usize;
        if self.coverage.len() < width {
            self.coverage.resize(width, 0.0);
        }
        let cov = &mut self.coverage[..width];

        let color = tint.unwrap_or(poly.color);
        let max_alpha = color.alpha() as f32;
        let weight = 1.0 / SAMPLES as f32;
        for iy in row_lo..row_hi {
            for c in cov.iter_mut() {
                *c = 0.0;
            }
            for s in 0..SAMPLES {
                let sy = iy as f32 + (s as f32 + 0.5) * weight;
                self.crossings.clear();
                for e in &self.edges {
                    if sy >= e.y_top && sy < e.y_bot {
                        self.crossings
                            .push((e.x_top + (sy - e.y_top) * e.dxdy, e.dir));
                    }
                }
                if self.crossings.len() < 2 {
                    continue;
                }
                self.crossings
                    .sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(Ordering::Equal));

                let mut winding = 0i32;
                for i in 0..self.crossings.len() - 1 {
                    winding += self.crossings[i].1;
                    let inside = match poly.fill_rule {
                        FillRule::NonZero => winding != 0,
                        FillRule::EvenOdd => (i as i32 + 1) & 1 == 1,
                    };
                    if !inside {
                        continue;
                    }
                    let xa = self.crossings[i].0.max(col_lo as f32);
                    let xb = self.crossings[i + 1].0.min(col_hi as f32);
                    add_span(cov, col_lo, xa, xb, weight);
                }
            }

            for (k, &cv) in cov.iter().enumerate() {
                if cv <= 0.0 {
                    continue;
                }
                let alpha = (max_alpha * cv.min(1.0)).round() as i32;
                if alpha <= 0 {
                    continue;
                }
                painter.blend_pixel_phys(col_lo + k as i32, iy, color, alpha.min(255) as u8);
            }
        }
    }
}

/// Add `weight × horizontal_coverage` of the span `[xa, xb)` to the coverage
/// row. End pixels get the fraction they're actually covered by; interior
/// pixels get the full `weight`. `base_x` is the device x of `cov[0]`.
fn add_span(cov: &mut [f32], base_x: i32, xa: f32, xb: f32, weight: f32) {
    if xb <= xa {
        return;
    }
    let start = xa.floor() as i32;
    let end = xb.ceil() as i32;
    for ix in start..end {
        let left = (ix as f32).max(xa);
        let right = ((ix + 1) as f32).min(xb);
        let frac = (right - left).clamp(0.0, 1.0);
        if frac <= 0.0 {
            continue;
        }
        let idx = ix - base_x;
        if idx >= 0 && (idx as usize) < cov.len() {
            cov[idx as usize] += frac * weight;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Paint `image` into a fresh `size × size` device buffer over `WHITE` at
    /// scale 1.0 and return the pixels.
    fn render(image: &SvgImage, size: i32) -> Vec<u32> {
        let mut pixels = vec![Color::WHITE.0; (size * size) as usize];
        {
            let mut p = Painter::new(&mut pixels, size, size, 1.0, 0, 0, None, None);
            image.draw(&mut p, Rect::new(0, 0, size, size));
        }
        pixels
    }

    /// A solid black unit square filling its viewBox — opaque at every interior
    /// pixel, so we can probe the footprint without depending on AA details.
    const SQUARE: SvgImage = SvgImage {
        width: 1.0,
        height: 1.0,
        polygons: &[SvgPolygon {
            color: Color::BLACK,
            fill_rule: FillRule::NonZero,
            rings: &[&[(0.0, 0.0), (1.0, 0.0), (1.0, 1.0), (0.0, 1.0)]],
        }],
    };

    #[test]
    fn fills_the_whole_box_opaquely() {
        let px = render(&SQUARE, 16);
        let at = |x: i32, y: i32| Color(px[(y * 16 + x) as usize]);
        // Interior is solid black; the box fills the entire footprint.
        for (x, y) in [(0, 0), (15, 0), (0, 15), (15, 15), (8, 8)] {
            assert_eq!(at(x, y), Color::BLACK, "({x},{y}) should be filled");
        }
    }

    #[test]
    fn draw_tinted_recolors_every_polygon() {
        // The black SQUARE drawn tinted RED fills the footprint with RED instead
        // of its baked black — coverage is unchanged, only the color.
        let size = 8;
        let mut px = vec![Color::WHITE.0; (size * size) as usize];
        {
            let mut p = Painter::new(&mut px, size, size, 1.0, 0, 0, None, None);
            SQUARE.draw_tinted(&mut p, Rect::new(0, 0, size, size), Color::RED);
        }
        let at = |x: i32, y: i32| Color(px[(y * size + x) as usize]);
        for (x, y) in [(0, 0), (7, 7), (4, 4)] {
            assert_eq!(at(x, y), Color::RED, "({x},{y}) should be tinted red");
        }
    }

    #[test]
    fn empty_or_degenerate_images_draw_nothing() {
        // No polygons.
        let blank = SvgImage {
            width: 1.0,
            height: 1.0,
            polygons: &[],
        };
        assert!(render(&blank, 8).iter().all(|&p| Color(p) == Color::WHITE));

        // A ring with too few points carries no area.
        let sliver = SvgImage {
            width: 1.0,
            height: 1.0,
            polygons: &[SvgPolygon {
                color: Color::BLACK,
                fill_rule: FillRule::NonZero,
                rings: &[&[(0.0, 0.0), (1.0, 1.0)]],
            }],
        };
        assert!(render(&sliver, 8).iter().all(|&p| Color(p) == Color::WHITE));
    }

    #[test]
    fn even_odd_rule_punches_a_hole() {
        // An outer square with an inner square; even-odd leaves the inner region
        // unfilled (a donut). Probe the center (hole) vs. the band (filled).
        let donut = SvgImage {
            width: 9.0,
            height: 9.0,
            polygons: &[SvgPolygon {
                color: Color::BLACK,
                fill_rule: FillRule::EvenOdd,
                rings: &[
                    &[(0.0, 0.0), (9.0, 0.0), (9.0, 9.0), (0.0, 9.0)],
                    &[(3.0, 3.0), (6.0, 3.0), (6.0, 6.0), (3.0, 6.0)],
                ],
            }],
        };
        let px = render(&donut, 9);
        let at = |x: i32, y: i32| Color(px[(y * 9 + x) as usize]);
        assert_eq!(at(1, 1), Color::BLACK, "outer band is filled");
        assert_eq!(at(4, 4), Color::WHITE, "even-odd leaves the center hole");
    }

    #[test]
    fn blends_partial_coverage_at_edges() {
        // A box covering only the left 1.5 px of a 3px-wide buffer: column 0 is
        // fully covered (black), column 1 is half covered (a gray blend), column
        // 2 is untouched (white). Confirms analytic horizontal AA.
        let half = SvgImage {
            width: 3.0,
            height: 1.0,
            polygons: &[SvgPolygon {
                color: Color::BLACK,
                fill_rule: FillRule::NonZero,
                rings: &[&[(0.0, 0.0), (1.5, 0.0), (1.5, 1.0), (0.0, 1.0)]],
            }],
        };
        // Draw into a 3x1 footprint at scale 1 so 1 viewBox unit == 1 px.
        let mut pixels = vec![Color::WHITE.0; 3];
        {
            let mut p = Painter::new(&mut pixels, 3, 1, 1.0, 0, 0, None, None);
            half.draw(&mut p, Rect::new(0, 0, 3, 1));
        }
        assert_eq!(Color(pixels[0]), Color::BLACK, "fully covered column");
        assert_eq!(Color(pixels[2]), Color::WHITE, "untouched column");
        let mid = Color(pixels[1]);
        assert!(
            mid != Color::BLACK && mid != Color::WHITE,
            "half-covered column should be a gray blend, got {mid:?}",
        );
    }
}
