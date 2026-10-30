use crate::background::BackgroundPattern;
use crate::font::{Font, FontFamily, FontSet, FontStyle};
use crate::geometry::{Color, Rect, Size};
use crate::theme::Theme;

/// Below this scale a logical pixel of chrome stays a single device pixel:
/// outlines keep the design's drawn widths and thin lines (dividers, carets,
/// grid rules) stay one device pixel thick. Bevels do too, up to
/// [`BEVEL_PIN`].
///
/// The pin covers 125% and 150%, where a scaled 2-pixel border looks blurry
/// next to the design's crisp 1-pixel one and the room the extra scale buys
/// is better spent on the face. Above it the same holds a whole multiple at a
/// time — see [`chrome_px`].
const CHROME_PIN: f32 = 1.75;

/// From this scale up to [`CHROME_PIN`] the outline stays a single device pixel
/// but the bevel inside it — the highlight and shadow rings that give a button
/// its relief — is drawn at 1.5x: a 2-pixel bevel 3 deep. Below it a 1-pixel
/// bevel still holds its own against the face; from here the face is half as
/// big again as the design's or more, and a bevel no deeper than the design's
/// all but disappears into it. Doubling the outline too would bring back the
/// heavy border [`CHROME_PIN`] keeps out. See [`bevel_depth`].
const BEVEL_PIN: f32 = 1.5;

/// Device pixels one logical pixel of chrome is worth: always a whole number
/// `k`, so the chrome is the design's own drawn `k` times over and every line
/// in it is `k` device pixels — never the mix of 2s and 3s a fractional
/// multiple rounds to. Shared by [`Frame`] and [`Painter::chrome_unit`], by
/// the pattern recipes that stamp a line one chrome unit thick, and by the
/// window chrome in [`crate::chrome`].
///
/// `k` is the scale's whole part, so a scale that is `k` times one under
/// [`CHROME_PIN`] wears that one's chrome `k` times over: 2.5x is the 1.25x
/// chrome doubled, 3x the 1.0x one tripled, and what the fractional rest of the
/// scale buys goes to the face. The exception is 1.75x up to 2x, where a
/// whole part of 1 would leave the face at or over the pin; it takes 2.
pub(crate) fn chrome_px(scale: f32) -> i32 {
    let whole = (scale.floor() as i32).max(1);
    if scale / whole as f32 >= CHROME_PIN {
        whole + 1
    } else {
        whole
    }
}

/// Device pixels `n` logical pixels of *bevel* run to: `n` [`chrome_px`]s,
/// except from [`BEVEL_PIN`] up to [`CHROME_PIN`], where the bevel is drawn at
/// 1.5x under an outline of 1. Shared by [`Frame::bevel_ring`] and the window
/// chrome's own buttons in [`crate::chrome`].
///
/// At 1.5x a 2-pixel bevel is exactly 3 deep, so its two rings cannot both be
/// whole: the outer one takes the half pixel and comes out 2, the inner one 1.
/// That way round, a recipe that paints one ring on its own — a scrollbar
/// arrow's highlight, a pressed button's — gets 2 device pixels, where the
/// other way it would get the single pixel this band exists to thicken. It
/// costs the light chrome some of its lopsidedness — a highlight of 2 over a
/// shadow of 3 where the design says 1 and 2 — and the corner's diagonal steps
/// 2 then 1, which takes a magnifier to see. A whole 2 a ring kept both, but
/// put a 4-pixel bevel under the 1-pixel outline, which swamped it.
///
/// The band is drawn at 1.5x all the way up, not at the scale itself, so every
/// scale in it wears the same bevel, as every scale shares an outline.
///
/// It needs no echo further up. A scale of 2 or more draws its chrome at its
/// whole part `k` (or `k + 1`), and the rest it leaves the face is under
/// `(k + 1) / k` — never as much as 1.5 — so no multiple of the design is drawn
/// there with a face as outsized as this band's.
pub(crate) fn bevel_depth(n: i32, scale: f32) -> i32 {
    let n = n.max(0);
    if (BEVEL_PIN..CHROME_PIN).contains(&scale) {
        // 1.5 n, the half rounded up.
        (3 * n + 1) / 2
    } else {
        n * chrome_px(scale)
    }
}

/// Opaque clip-stack token returned by [`Painter::push_clip`]. Pass it back
/// to [`Painter::restore_clip`] to pop the clip the caller installed.
#[derive(Clone, Copy)]
pub struct SavedClip(Option<(i32, i32, i32, i32)>);

/// Which edges of a widget's frame lie on a *neighbouring* widget's own
/// frame line.
///
/// Widgets that abut share their border by overlapping one logical pixel — a
/// list field's right edge sits on the column its scrollbar's left border
/// occupies, a scrollbar thumb parked against an arrow button reaches one
/// pixel into it — so the two 1-pixel borders collapse into a single line
/// instead of stacking into a 2-pixel band. At integer scales that is the
/// whole story. At fractional scales it stops lining up: each widget's
/// [`Frame`] draws its line [`Frame::depth(1)`](Frame::depth) device pixels
/// thick *from its own snapped edge*, and the two edges snap independently, so
/// `snap(x + 1) - snap(x)` is one device pixel more or less than the line's
/// thickness at about every other position — and the two borders land a device
/// pixel apart instead of on top of each other.
///
/// Marking the shared edge as merged re-anchors it to the very device pixels
/// the *neighbour's* line occupies, which is knowable without asking the
/// neighbour: its line is `depth(1)` thick, measured inward from its own
/// snapped edge, and its edge is one logical pixel inside ours. Everything
/// else about the frame — every other edge, every depth — is untouched, so the
/// border keeps its uniform thickness and the face reaches exactly to the
/// shared line.
///
/// Pass it to [`Painter::crisp_merged`] or to the merged variants of the frame
/// recipes ([`Painter::stroke_rect_merged`],
/// [`Painter::sunken_bevel_merged`], …), and to
/// [`Painter::push_clip_frame`] so content is clipped to the same interior the
/// merged frame painted. [`Merged::NONE`] everywhere is exactly the unmerged
/// behaviour, so the plain entry points just delegate with it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Merged {
    pub left: bool,
    pub top: bool,
    pub right: bool,
    pub bottom: bool,
}

impl Merged {
    /// No shared edges: the frame is freestanding.
    pub const NONE: Self = Self::new(false, false, false, false);
    /// The left edge sits on the frame line of a neighbour to the left.
    pub const LEFT: Self = Self::new(true, false, false, false);
    /// The top edge sits on the frame line of a neighbour above.
    pub const TOP: Self = Self::new(false, true, false, false);
    /// The right edge sits on the frame line of a neighbour to the right —
    /// the field-with-a-scrollbar composition.
    pub const RIGHT: Self = Self::new(false, false, true, false);
    /// The bottom edge sits on the frame line of a neighbour below.
    pub const BOTTOM: Self = Self::new(false, false, false, true);

    const fn new(left: bool, top: bool, right: bool, bottom: bool) -> Self {
        Self {
            left,
            top,
            right,
            bottom,
        }
    }
}

/// A widget's frame, in device pixels, ready to be measured in the *depths* its
/// design is written in.
///
/// Handed to a [`Painter::crisp`] recipe. Win 3.1 chrome is described as a set
/// of depths in logical pixels — a button's black outline occupies the first,
/// the bevel under it the next two — and this turns any such depth into device
/// pixels measured from the widget's own snapped edge. Nothing accumulates,
/// because neighbouring rings meet on the boundary they share, and a ring is
/// the same thickness on all four sides and on every widget at that scale.
/// Snapping each line from its own position in the window, which is what the
/// plain drawing path does, could promise neither.
///
/// A depth is worth a whole number of device pixels — [`Painter::chrome_unit`]
/// of them — rather than the scale itself, so the frame is the design's own
/// drawn at an integer zoom and every ring in it is exactly as thick as every
/// other. Scaling the depths by a fractional factor only distorts the frame's
/// proportions, since half a device pixel of rounding error is most of a line
/// one or two pixels thick. At 1.25x it would put a 3-pixel bevel under a
/// 1-pixel border where the design says 2 and 1, and at 2.5x a 3-pixel border
/// over a bevel of a 2-pixel ring and a 3-pixel one where the design says 1
/// and 1 and 1. A whole multiple keeps the drawn proportions exactly — at 2.5x
/// the frame is the 1.25x one doubled — and spends the rest of the scale on
/// the face instead: the button comes out a little roomier and a lot sharper.
///
/// Spending it all on the face goes too far from 1.5x up to 1.75x, where the
/// face is half as big again as the design's and the 1-pixel bevel reads as no
/// relief at all. There the bevel alone is drawn at 1.5x under the 1-pixel
/// outline — 3 device pixels for the design's 2, in rings of 2 and 1 — and
/// [`Frame::bevel_ring`] is how a recipe asks for it.
///
/// A frame whose widget shares an edge with a neighbouring frame — the
/// 1-logical-pixel-overlap composition — is built over a rect whose shared
/// edges are re-anchored to the neighbour's own line first: see [`Merged`] and
/// [`Painter::crisp_merged`].
#[derive(Clone, Copy)]
pub struct Frame {
    /// The widget's rect, snapped to device pixels.
    rect: Rect,
    /// Device pixels one logical pixel of *depth* is worth: [`chrome_px`] at
    /// the window's scale. Not the 1.0 the painter itself runs at inside the
    /// pass.
    per_pixel: i32,
    /// The window's scale, which [`bevel_depth`] measures a bevel's rings by.
    scale: f32,
}

impl Frame {
    /// A frame over `rect` (already in device pixels) for a window drawn at
    /// `scale`. Its depths are the design's own times a whole number of device
    /// pixels, [`chrome_px`], and not times the scale: the type docs say why.
    /// See [`bevel_depth`] for what the uneven bevel rings cost.
    fn new(rect: Rect, scale: f32) -> Self {
        Self {
            rect,
            per_pixel: chrome_px(scale),
            scale,
        }
    }

    /// The frame's outer bounds, in device pixels.
    pub fn rect(&self) -> Rect {
        self.rect
    }

    /// Device pixels from the frame's edge in to logical depth `d`. `depth(0)`
    /// is the edge itself; `depth(1)` the far side of a one-pixel border.
    ///
    /// Exactly `d` [`Painter::chrome_unit`]s — see [`Frame`], where the
    /// reasoning lives.
    pub fn depth(&self, d: i32) -> i32 {
        (d * self.per_pixel).max(0)
    }

    /// What is left of the frame inside `d` logical pixels of chrome — the face
    /// of a button, the field of an input. The scale-aware `inset(d)`.
    pub fn inside(&self, d: i32) -> Rect {
        // `Rect::inset` floors the result at zero size, so a depth deeper than
        // the widget leaves nothing to paint rather than an inverted rect.
        self.rect.inset(self.depth(d))
    }

    /// The four sides of the one-logical-pixel ring at depth `d`: from
    /// `depth(d)` in to `depth(d + 1)`.
    ///
    /// A band several logical pixels deep is drawn as several rings rather than
    /// one taller rect, because each ring is also *inset* by its own depth —
    /// that is the staircase a Win 3.1 bevel steps down at its corners. The
    /// depth stays exact all the same: neighbouring rings meet on a shared
    /// `depth(d + 1)`, so nothing accumulates and the pair spans exactly
    /// `depth(d + 2) - depth(d)`.
    pub fn ring(&self, d: i32) -> Ring {
        self.ring_between(self.depth(d), self.depth(d + 1))
    }

    /// Ring `i` of a bevel inside `outline` logical pixels of outline — the
    /// light-and-shadow ring a recipe paints just inside its border is
    /// `bevel_ring(1, 0)`, and the one behind that `bevel_ring(1, 1)`.
    ///
    /// The same ring as [`ring(outline + i)`](Self::ring) at every scale but
    /// 1.5x up to 1.75x, where the bevel is drawn at 1.5x under an outline of
    /// one: the first ring 2 device pixels, the second 1. The first takes the
    /// odd pixel so that a line one ring deep on its own — a scrollbar arrow's
    /// highlight — comes out 2 rather than the single pixel the band exists to
    /// thicken. Inside the bevel the rings meet on the boundary they share,
    /// exactly as [`Self::ring`]'s do.
    pub fn bevel_ring(&self, outline: i32, i: i32) -> Ring {
        let at = |i: i32| self.depth(outline) + bevel_depth(i, self.scale);
        self.ring_between(at(i), at(i + 1))
    }

    /// The band from the frame's edge in to logical depth `d`, with its four
    /// corners rounded off — the outline a Win 3.1 button wears. See
    /// [`RoundedBand`] for the curve.
    pub fn rounded_band(&self, d: i32) -> RoundedBand {
        let r = self.rect;
        let radius = self.depth(2).min(r.w / 2).min(r.h / 2).max(0);
        RoundedBand {
            rect: r,
            thickness: self.axis_depth(self.depth(d)),
            radius,
            inner_radius: (radius - self.depth(d)).max(0),
        }
    }

    /// The four sides of the ring from `a` device pixels in to `b`, the shared
    /// tail of [`Self::ring`] and [`Self::bevel_ring`].
    fn ring_between(&self, a: i32, b: i32) -> Ring {
        let (ax, ay) = self.axis_depth(a);
        let (bx, by) = self.axis_depth(b);
        let r = self.rect;
        // Horizontal sides span what is left after the *vertical* inset, and
        // vice versa, so the corners belong to whichever side paints last.
        Ring {
            top: Rect::new(r.x + ax, r.y + ay, r.w - 2 * ax, by - ay),
            bottom: Rect::new(r.x + ax, r.bottom() - by, r.w - 2 * ax, by - ay),
            left: Rect::new(r.x + ax, r.y + ay, bx - ax, r.h - 2 * ay),
            right: Rect::new(r.right() - bx, r.y + ay, bx - ax, r.h - 2 * ay),
        }
    }

    /// A device depth clamped to each axis of the frame, so a widget shallower
    /// than its own chrome overlaps itself rather than spilling over its
    /// neighbour. Every ring goes through this.
    fn axis_depth(&self, depth: i32) -> (i32, i32) {
        (depth.min(self.rect.w.max(0)), depth.min(self.rect.h.max(0)))
    }
}

/// One ring of a [`Frame`]: its four sides at a single logical depth, each
/// already a device-pixel rect.
///
/// `top` / `left` are the pair a bevel lights and `bottom` / `right` the pair it
/// shadows, which is why [`Painter::fill_ring`] takes its two colours in that
/// order. A recipe that wants only one half of a ring — a pressed scrollbar
/// arrow's shadow, with no highlight opposite it — paints it with
/// [`Painter::fill_ring_near`] or [`Painter::fill_ring_far`]. A side is a
/// device-pixel rect like any other, so one on its own is just a
/// [`Painter::fill_rect`].
#[derive(Clone, Copy)]
pub struct Ring {
    pub top: Rect,
    pub left: Rect,
    pub bottom: Rect,
    pub right: Rect,
}

/// The outer band of a [`Frame`] with its corners rounded off, from
/// [`Frame::rounded_band`], ready for [`Painter::fill_rounded_band`].
///
/// At 1.0x a Win 3.1 button's outline is the 1-pixel ring with its corner
/// pixels left out. That is exactly the curve a rounded rectangle 2 pixels in
/// radius, stroked 1 pixel thick, rasterizes to: the corner pixel's centre
/// lies outside the outer arc, its two neighbours along the edges inside it,
/// and the pixel diagonally in from it inside the inner arc, so the outline's
/// inner edge stays square. The band is drawn as that curve — its outer arc
/// [`Frame::depth(2)`](Frame::depth) in radius, its inner one concentric with
/// it the band's thickness further in — rasterized at the device resolution
/// by the same rule, a pixel being in when its centre is.
///
/// Below 1.75x, where a depth is a single device pixel, that is the one
/// missing pixel the design draws. From 1.75x up the corner keeps the design's
/// shape rather than its pixels: blowing the missing pixel up with the rest of
/// the chrome cut a square notch out of each corner as wide as the outline,
/// and the curve gets a real arc out of the same few device pixels instead —
/// three left out of each corner at 2x where the notch took four, and the
/// inner edge rounded off the bevel's corner pixel to match. A band two
/// logical pixels deep, the default button's, shares the curve outside and
/// keeps a square inner edge, as the design's does.
#[derive(Clone, Copy)]
pub struct RoundedBand {
    /// The frame's rect, in device pixels.
    rect: Rect,
    /// How deep the band runs, across and down: [`Frame::depth`] clamped to
    /// each axis, as a [`Ring`]'s sides are.
    thickness: (i32, i32),
    /// The outer arc's radius, in device pixels, and the side of the square
    /// each corner is drawn in.
    radius: i32,
    /// The inner arc's radius: where the band stops and whatever it frames
    /// shows through again. Zero once the band is as deep as the outer radius,
    /// which leaves its inner edge square.
    inner_radius: i32,
}

impl RoundedBand {
    /// The band's run across row `i` of a corner square, counted in from the
    /// frame's edge — as `(from, to)` device pixels in from its side. Left of
    /// `from` lies outside the outer arc and is left unpainted; from `to` on
    /// lies inside the inner one. The same run serves all four corners,
    /// mirrored.
    fn corner_run(&self, i: i32) -> (i32, i32) {
        // Doubled, so every pixel centre is a whole number: the arcs' common
        // centre sits at `2 * radius` along both axes, a pixel's centre at
        // `2 * x + 1`. A centre's squared distance is then a sum of two odd
        // squares, which is never a multiple of 4, so no pixel ever lies
        // exactly on either arc and `<=` and `<` agree.
        let dy = 2 * (self.radius - i) - 1;
        let within = |x: i32, radius: i32| {
            let dx = 2 * (self.radius - x) - 1;
            dx * dx + dy * dy <= 4 * radius * radius
        };
        let from = (0..self.radius)
            .find(|&x| within(x, self.radius))
            .unwrap_or(self.radius);
        let to = (from..self.radius)
            .find(|&x| within(x, self.inner_radius))
            .unwrap_or(self.radius);
        (from, to)
    }
}

/// Pixel-perfect 2D painter over an ARGB32 framebuffer.
///
/// Widgets paint in **logical pixels**: density-independent design units. The
/// painter applies the scale factor the window is drawn at (which may be
/// fractional — 1.0, 1.25, 1.5, 2.25, …) and writes straight into the physical
/// surface buffer. Rectangle edges are snapped independently so adjacent
/// rectangles always share an exact physical-pixel boundary. Text is
/// rasterized once at its final physical size via fontdue — no resampling, no
/// smudge.
///
/// Frames are the exception to independent snapping: a button's outline, bevel
/// and inner highlight are lines a logical pixel wide sitting right on top of
/// one another, and snapped one at a time — each from its own position in the
/// window — they round to different widths, so a frame that should read as one
/// object comes out heavier along one edge than the opposite one and different
/// again on the widget beside it. Every frame primitive here instead draws in
/// device pixels off a [`Frame`], which places each ring at the depth the
/// design puts it at, times a whole number of device pixels. See
/// [`Painter::crisp`].
pub struct Painter<'a> {
    pixels: &'a mut [u32],
    /// Physical buffer width in pixels.
    width: i32,
    /// Physical buffer height in pixels.
    height: i32,
    /// Logical→physical scale. Equals winit's `scale_factor` for the current
    /// monitor (always ≥ 1 in practice).
    scale: f32,
    /// The OS/display scale the window is *presented* at — e.g. 1.5 on a 150%
    /// display. Always below `scale`, which carries the UI scale on top of it,
    /// and further below on a Wayland compositor with no `wp_viewporter`, where
    /// `scale` then also carries the integer buffer scale the content is
    /// rasterized at before being resampled down to the fractional size. Equal
    /// to `scale` until a backend calls
    /// [`Self::set_system_scale`]. Unlike `scale`, it is *not* swapped out by
    /// [`Self::with_scale`] / [`Self::physical`] — it describes the window, not
    /// the current draw transform.
    system_scale: f32,
    /// Physical-pixel offset of the logical origin within the buffer. The
    /// runtime sets this to center the content when the window has been
    /// resized larger than the design — surroundings become clean letterbox.
    origin_x: i32,
    origin_y: i32,
    /// The sans / serif / mono families this painter draws with. `text` /
    /// `text_centered` and `measure_text` use the sans family; the text editors
    /// use the mono family; `text_styled` / `measure_text_styled` pick a family
    /// explicitly. Any family may be `None` (its draws no-op).
    fonts: FontSet<'a>,
    /// `Some(anchor)` when this painter is drawing into a popup top-level
    /// window, where `anchor` is the popup's [`PopupRequest::rect`] (the
    /// same value the runtime opened the popup window with). `None` in the
    /// main pass. Widgets that maintain floating overlays (menu popups,
    /// tooltips) inspect this in `paint_overlay` so they only draw on the
    /// surface that actually hosts them — and, when several popups are
    /// stacked (e.g. a dropdown opened inside a dialog), only into the one
    /// whose anchor matches their own [`Widget::popup_request`].
    popup_anchor: Option<Rect>,
    /// Physical-pixel clip rectangle. When set, all draws are restricted to
    /// pixels inside this rect. The runtime uses this to keep the popup
    /// pass from leaking widget content past the popup's footprint.
    clip: Option<(i32, i32, i32, i32)>,
    /// Where the screen is, in this painter's logical coordinates — see
    /// [`Self::screen_area`].
    screen: Option<Rect>,
    /// How many device pixels the display fits to the inch — see
    /// [`Self::display_dpi`].
    display_dpi: Option<f32>,
}

impl<'a> Painter<'a> {
    pub fn new(
        pixels: &'a mut [u32],
        width: i32,
        height: i32,
        scale: f32,
        origin_x: i32,
        origin_y: i32,
        fonts: FontSet<'a>,
    ) -> Self {
        Self::with_popup_anchor(
            pixels, width, height, scale, origin_x, origin_y, fonts, None,
        )
    }

    /// Like [`Painter::new`] but tags the painter as running inside a
    /// popup top-level window whose
    /// [`PopupRequest::rect`](crate::PopupRequest::rect) is `anchor`.
    /// `None` means the main window pass (equivalent to [`Painter::new`]).
    /// Widgets compare `anchor` against their own popup request in
    /// `paint_overlay` so that, when several popups are stacked, the
    /// dropdown / menu / dialog body is drawn only into the surface that
    /// actually hosts it.
    #[allow(clippy::too_many_arguments)]
    pub fn with_popup_anchor(
        pixels: &'a mut [u32],
        width: i32,
        height: i32,
        scale: f32,
        origin_x: i32,
        origin_y: i32,
        fonts: FontSet<'a>,
        popup_anchor: Option<Rect>,
    ) -> Self {
        Self {
            pixels,
            width,
            height,
            scale: scale.max(0.01),
            system_scale: scale.max(0.01),
            origin_x,
            origin_y,
            fonts,
            popup_anchor,
            clip: None,
            screen: None,
            display_dpi: None,
        }
    }

    /// Record where the screen is, for widgets that place something on it —
    /// see [`Self::screen_area`]. Only the runtime has the answer, so only the
    /// runtime calls this; an offscreen render leaves it unset.
    pub fn with_screen(mut self, area: Option<Rect>) -> Self {
        self.screen = area;
        self
    }

    /// Record how dense the display is, for widgets that draw something at its
    /// real size — see [`Self::display_dpi`]. Like [`Self::with_screen`], only
    /// the runtime has the answer.
    pub fn with_display_dpi(mut self, dpi: Option<f32>) -> Self {
        self.display_dpi = dpi;
        self
    }

    pub fn is_popup_pass(&self) -> bool {
        self.popup_anchor.is_some()
    }

    /// [`PopupRequest::rect`](crate::PopupRequest::rect) of the popup this
    /// painter is drawing into, or `None` in the main pass. Widgets that report a
    /// [`Widget::popup_request`](crate::Widget::popup_request) use this in
    /// `paint_overlay` to decide whether the current popup pass is *theirs*
    /// — only then should they draw their popup body.
    pub fn popup_anchor(&self) -> Option<Rect> {
        self.popup_anchor
    }

    /// The part of the display a window may occupy, in the *root widget's*
    /// logical coordinates — the same space [`Widget::bounds`] and
    /// [`PopupRequest::rect`] live in, so it is usually largely negative in x/y
    /// and much bigger than the window.
    ///
    /// Widgets that place something in a top-level window of their own — a menu
    /// panel, a tooltip — use this to keep it on screen: a popup is not bounded
    /// by the app's window, so the display is the only thing that does bound
    /// it. It excludes the space the desktop reserves for its own furniture
    /// (the macOS menu bar and Dock) where the platform reports it.
    ///
    /// `None` when the runtime has no answer: an offscreen
    /// [`MockBackend`](crate::mock::MockBackend) render, or a window the
    /// platform has not placed on a display yet. Treat that as "unbounded"
    /// rather than "nothing fits".
    ///
    /// [`Widget::bounds`]: crate::Widget::bounds
    /// [`PopupRequest::rect`]: crate::PopupRequest::rect
    pub fn screen_area(&self) -> Option<Rect> {
        self.screen
    }

    /// How many device pixels — the ones [`Self::physical`] draws in — fit in
    /// an inch of the display the window is on, going by the physical size the
    /// display reports for itself.
    ///
    /// This is what a widget drawing something at its real size wants, a ruler
    /// or a life-size card. [`Self::scale`] says how many device pixels a
    /// logical one is worth, but not how big a device pixel is: that is down to
    /// the display, and to the scaling the user picked for it — 150% in
    /// Windows' Display Settings, "Larger Text" on a Mac — which the scale only
    /// carries mixed in with the display's density.
    ///
    /// A device pixel is not always one of the panel's. On macOS it is a pixel
    /// of the framebuffer: a 27" 4K panel in the 2560x1440 mode macOS picks for
    /// it renders 5120 wide and is resampled down, so it comes out at 217 here
    /// rather than the panel's own 163. On Wayland it is the panel's wherever
    /// `wp_viewporter` lets the buffer be the fractional size the compositor
    /// presents; on a compositor without it, a fractional display is
    /// oversampled at the integer scale above and resampled down, so a 157-dpi
    /// panel presented at 125% comes out at 252 — the density of what we are
    /// drawing into, either way.
    ///
    /// `None` when there is no answer: an offscreen
    /// [`MockBackend`](crate::mock::MockBackend) render, unless it fakes one;
    /// and a display that reports no size of its own, as a projector may not,
    /// nor a virtual output such as the one WSLg gives an external display.
    pub fn display_dpi(&self) -> Option<f32> {
        self.display_dpi
    }

    /// Restrict all subsequent drawing to a physical-pixel rectangle. Used
    /// by the popup runtime to confine paint operations to the popup's
    /// footprint inside its (often oversized) host window.
    pub fn set_clip_phys(&mut self, x: i32, y: i32, w: i32, h: i32) {
        self.clip = Some((x, y, x + w, y + h));
    }

    pub fn clear_clip(&mut self) {
        self.clip = None;
    }

    /// Restrict subsequent draws to the intersection of `rect` (in logical
    /// pixels) and any clip already in effect. Returns the previous clip
    /// state — pass it to [`Painter::restore_clip`] when done. Widget code
    /// uses this to keep overflow text from leaking past its field edges
    /// without having to know its own physical-pixel placement.
    pub fn push_clip(&mut self, rect: Rect) -> SavedClip {
        let x0 = self.origin_x + self.snap(rect.x);
        let y0 = self.origin_y + self.snap(rect.y);
        let x1 = self.origin_x + self.snap(rect.x + rect.w);
        let y1 = self.origin_y + self.snap(rect.y + rect.h);
        self.intersect_clip(x0, y0, x1, y1)
    }

    /// Restrict subsequent draws to what is left of a widget's rect inside `d`
    /// logical pixels of crisp chrome: the same interior
    /// [`Frame::inside`]`(d)` hands a frame recipe, as a clip. The
    /// device-accurate companion to `push_clip(rect.inset(d))`, whose
    /// logically-snapped boundary can sit a device pixel inside or outside the
    /// frame line at fractional scales — enough for a fill clipped by it to
    /// either overwrite the line's inner pixel or leave a seam of stale
    /// background along it.
    ///
    /// `merged` names the edges shared with a neighbouring frame, and must
    /// match what the chrome was painted with, so the clip reaches exactly to
    /// the shared line. Returns the previous clip state — pass it to
    /// [`Painter::restore_clip`] when done.
    pub fn push_clip_frame(&mut self, rect: Rect, d: i32, merged: Merged) -> SavedClip {
        let phys = self.rect_to_physical_merged(rect, merged);
        let inner = Frame::new(phys, self.scale).inside(d);
        self.intersect_clip(
            self.origin_x + inner.x,
            self.origin_y + inner.y,
            self.origin_x + inner.x + inner.w,
            self.origin_y + inner.y + inner.h,
        )
    }

    /// Install the intersection of the physical-pixel bounds `x0..x1` /
    /// `y0..y1` and any clip already in effect, handing back the previous
    /// state for [`Painter::restore_clip`]. The shared tail of both
    /// `push_clip` entry points.
    fn intersect_clip(&mut self, x0: i32, y0: i32, x1: i32, y1: i32) -> SavedClip {
        let prev = SavedClip(self.clip);
        let combined = match self.clip {
            Some((px0, py0, px1, py1)) => (x0.max(px0), y0.max(py0), x1.min(px1), y1.min(py1)),
            None => (x0, y0, x1, y1),
        };
        self.clip = Some(combined);
        prev
    }

    pub fn restore_clip(&mut self, saved: SavedClip) {
        self.clip = saved.0;
    }

    fn clip_bounds(&self) -> (i32, i32, i32, i32) {
        match self.clip {
            Some((x0, y0, x1, y1)) => (
                x0.max(0),
                y0.max(0),
                x1.min(self.width),
                y1.min(self.height),
            ),
            None => (0, 0, self.width, self.height),
        }
    }

    /// Horizontal extent of the active clip in glyph-pen space — physical
    /// pixels relative to the logical origin, the coordinate space
    /// [`Font::draw_phys`](crate::font::Font::draw_phys) lays glyphs out in.
    /// The font renderer uses this to skip glyphs that fall entirely outside
    /// the visible span instead of blending every character of a long,
    /// mostly-off-screen line.
    pub(crate) fn glyph_clip_x(&self) -> (i32, i32) {
        let (x0, _, x1, _) = self.clip_bounds();
        (x0 - self.origin_x, x1 - self.origin_x)
    }

    pub fn size(&self) -> Size {
        Size::new(self.width, self.height)
    }

    pub fn scale(&self) -> f32 {
        self.scale
    }

    /// The OS/display scale the window is presented at (e.g. 1.5 on a 150%
    /// display) — the figure a native app on the same display would see.
    ///
    /// It is not [`Self::scale`], which is what a logical pixel is really worth
    /// in the pixels being drawn: that carries the UI scale aiming a logical
    /// pixel at 1/72 in, and on a Wayland compositor without `wp_viewporter`
    /// the integer buffer scale as well, the content being rasterized at that
    /// and resampled down to the fractional size the compositor presents. A
    /// widget asking how the *display* is set wants this one; one placing
    /// pixels wants `scale`. Defaults to [`Self::scale`] until the backend
    /// records the display's own scale.
    pub fn system_scale(&self) -> f32 {
        self.system_scale
    }

    /// Backend hook: record the true display scale when it differs from the
    /// integer buffer [`Self::scale`] — the Wayland fractional-scaling case,
    /// where the compositor resamples our oversampled buffer down to the
    /// fractional size. Other backends leave it equal to `scale`.
    ///
    /// Crate-internal on purpose: like the scale factor itself, the system
    /// scale is owned by the OS. Applications can *read* it via
    /// [`Self::system_scale`] but must not be able to override it.
    ///
    /// Two backends have something to say here: Wayland, whose integer buffer
    /// scale is not the fractional scale the compositor presents at, and the
    /// winit runtime, whose scale carries saudade's UI scale on top of the
    /// factor the display reports.
    pub(crate) fn set_system_scale(&mut self, scale: f32) {
        self.system_scale = scale.max(0.01);
    }

    /// Translate a logical-pixel `rect` to the physical-pixel rectangle it
    /// would occupy on the buffer, snapping each edge independently the same
    /// way the draw primitives do. Used by [`Self::physical`] to hand the
    /// closure its region in device pixels; callers reach it through that.
    fn rect_to_physical(&self, rect: Rect) -> Rect {
        let x0 = self.snap(rect.x);
        let y0 = self.snap(rect.y);
        let x1 = self.snap(rect.x + rect.w);
        let y1 = self.snap(rect.y + rect.h);
        Rect::new(x0, y0, x1 - x0, y1 - y0)
    }

    /// [`Self::rect_to_physical`] with each [`Merged`] edge re-anchored to the
    /// neighbouring frame line it shares — the device pixels the *neighbour's*
    /// own crisp frame puts that line on, worked out from this side of the
    /// overlap: the neighbour's edge is one logical pixel inside ours, and its
    /// line runs one [`Self::chrome_unit`] inward from where that edge snaps.
    /// A frame built over the result lands its `ring(0)` on the shared edge
    /// exactly on top of the neighbour's line, and measures every depth from
    /// it, so the face reaches the line with neither a seam nor an overwrite.
    fn rect_to_physical_merged(&self, rect: Rect, merged: Merged) -> Rect {
        // The thickness the neighbour's line is drawn at: `Frame::depth(1)` at
        // this scale, which `chrome_unit` equals at every scale (both are
        // `chrome_px`).
        let line = self.chrome_unit();
        let x0 = if merged.left {
            self.snap(rect.x + 1) - line
        } else {
            self.snap(rect.x)
        };
        let y0 = if merged.top {
            self.snap(rect.y + 1) - line
        } else {
            self.snap(rect.y)
        };
        let x1 = if merged.right {
            self.snap(rect.right() - 1) + line
        } else {
            self.snap(rect.right())
        };
        let y1 = if merged.bottom {
            self.snap(rect.bottom() - 1) + line
        } else {
            self.snap(rect.bottom())
        };
        Rect::new(x0, y0, x1 - x0, y1 - y0)
    }

    /// Device pixels one logical pixel of chrome is worth: a whole number, 1
    /// below 1.75x, 2 from 1.75x up to 3x, and the scale's whole part from
    /// there on. The thickness every ring of a [`Frame`] is drawn at (bar a
    /// bevel's from 1.5x up to 1.75x — see [`Frame::bevel_ring`]), the same
    /// `unit` the window chrome in [`crate::chrome`] draws its frame lines at,
    /// and what a widget wants for a *lone* thin line — a divider, a caret, a
    /// grid rule.
    ///
    /// A frame is still built off a [`Frame`] rather than out of this: the
    /// frame measures every ring from the widget's own snapped edge, which a
    /// line placed from its logical position cannot promise.
    pub fn chrome_unit(&self) -> i32 {
        chrome_px(self.scale)
    }

    /// Run `f` with the painter dropped to physical pixels — one unit maps to
    /// one device pixel — and with `rect` snapped to its physical bounds. The
    /// snap uses the real scale, so the region lands exactly where the scaled
    /// draw would have put it; inside `f`, a 1-unit line is exactly one device
    /// pixel and `inset(1)` trims one device pixel.
    ///
    /// Drawing helpers use this to implement a crisp special-case at awkward
    /// fractional scales and then re-invoke themselves, so the recipe lives in
    /// a single place (see [`Self::button`]). Calling it when the painter is
    /// already at physical resolution (`scale == 1.0`) is a transparent
    /// pass-through — which both serves a real 1.0× display and breaks the
    /// helper's self-recursion.
    pub fn physical(&mut self, rect: Rect, f: impl FnOnce(&mut Painter, Rect)) {
        if self.scale == 1.0 {
            return f(self, rect);
        }
        let phys = self.rect_to_physical(rect);
        let saved = self.scale;
        self.scale = 1.0;
        f(self, phys);
        self.scale = saved.max(0.01);
    }

    /// Draw a frame recipe crisply: `f` runs in device pixels, against a
    /// [`Frame`] that places its rings at the design's depths, each a whole
    /// [`Self::chrome_unit`] deep — bar a bevel's, see [`Frame::bevel_ring`].
    ///
    /// The painter is dropped to device pixels exactly as [`Self::physical`]
    /// hands it over, and the `Frame` carries `rect` snapped to its device
    /// bounds together with the *outer* scale. A recipe asks the frame for the
    /// ring it wants to paint — `f.ring(0)` for the outline, `f.bevel_ring(1, 0)`
    /// and `f.bevel_ring(1, 1)` for the bevel behind it, `f.inside(1)` for the
    /// face the bevel is painted over — and every one of those comes back with
    /// its boundaries already rounded to device pixels. Because the outer rect
    /// is the snapped one, the frame still lands exactly where the scaled draw
    /// would have put it, and still shares its edge pixels with the widget next
    /// door.
    ///
    /// At 1.0x, and at any integer scale where every depth is an exact multiple,
    /// this paints the same pixels the plain logical path would. It diverges
    /// where that path was snapping each line from its own position in the
    /// window and so came out uneven — heavier along one edge of a frame than
    /// the opposite one, and different again on the widget beside it — and
    /// wherever the scale is fractional, since the frame is drawn at a whole
    /// multiple of the design and the face takes the rest.
    ///
    /// Every frame primitive here is built on it ([`Self::stroke_rect`],
    /// [`Self::raised_bevel`], [`Self::button`], [`Self::light_button`],
    /// [`Self::focus_rect`], [`Self::etched_h_line`]); a widget drawing chrome
    /// of its own reaches for it directly:
    ///
    /// ```ignore
    /// painter.crisp(box_rect, |p, f| {
    ///     p.fill_rect(f.inside(1), fill);                      // the field
    ///     p.fill_ring(f.ring(0), theme.border, theme.border);  // 1px outline
    /// });
    /// ```
    ///
    /// Note that the painter inside `f` sits at `scale == 1.0`, so the
    /// self-managing primitives above are no use there — each would draw a
    /// single device pixel. Take the geometry from the `Frame`.
    pub fn crisp(&mut self, rect: Rect, f: impl FnOnce(&mut Painter, Frame)) {
        self.crisp_merged(rect, Merged::NONE, f);
    }

    /// [`Self::crisp`] for a frame that shares its `merged` edges with a
    /// neighbouring widget's own frame line — the 1-logical-pixel-overlap
    /// composition [`Merged`] describes. Each merged edge of the [`Frame`] is
    /// re-anchored to the device pixels the neighbour's line occupies, so the
    /// recipe's `ring(0)` lands exactly on that line and its deeper rings and
    /// face measure from it. With [`Merged::NONE`] this *is* `crisp`.
    pub fn crisp_merged(
        &mut self,
        rect: Rect,
        merged: Merged,
        f: impl FnOnce(&mut Painter, Frame),
    ) {
        let scale = self.scale;
        if scale == 1.0 {
            // Pass-through, like `physical`: at 1.0x the snap is the identity
            // and a merged edge re-derives the same coordinate.
            return f(self, Frame::new(rect, scale));
        }
        let phys = self.rect_to_physical_merged(rect, merged);
        self.scale = 1.0;
        f(self, Frame::new(phys, scale));
        self.scale = scale.max(0.01);
    }

    /// Render `f` into the logical-pixel region `area` as though the painter
    /// were running at the scale factor `scale` instead of the window's real
    /// one. Inside `f`, coordinates are local to `area`'s top-left corner and
    /// one logical unit maps to `scale` physical pixels, so widgets paint
    /// exactly as they would in a window the OS reported at that DPI — snapped
    /// chrome and re-rasterized text included. Drawing is clipped to `area`
    /// (intersected with any clip already in effect), so content that would
    /// overflow at large scales is trimmed rather than spilling past it.
    ///
    /// This is the in-place building block behind [`Self::draw_scaled`], which
    /// is the public entry point for rendering content at a chosen scale (and
    /// the path apps reach for to *preview* a scale without touching the
    /// window's actual scale factor, which only the OS controls). It allocates
    /// no nested buffer or font cache — it just relocates the logical origin
    /// and swaps the scale for the duration of the call — so it is cheap enough
    /// to run every frame.
    pub(crate) fn with_scale(&mut self, area: Rect, scale: f32, f: impl FnOnce(&mut Painter)) {
        if area.w <= 0 || area.h <= 0 {
            return;
        }
        // `area` is in the *current* logical coords, so snap it (and clip to
        // it) before swapping the scale out from under `snap`. `push_clip`
        // intersects with whatever clip is already installed, so a caller can
        // confine the preview to a smaller pane first and still center an
        // oversized `area` within it.
        let saved_clip = self.push_clip(area);
        let origin_x = self.origin_x + self.snap(area.x);
        let origin_y = self.origin_y + self.snap(area.y);
        let saved_scale = self.scale;
        let saved_origin = (self.origin_x, self.origin_y);
        // The nested coordinate space is `area`-local at another scale, so the
        // window's screen rect no longer describes it — drop it rather than
        // hand `f` a rect in the wrong space.
        let saved_screen = self.screen.take();
        self.scale = scale.max(0.01);
        self.origin_x = origin_x;
        self.origin_y = origin_y;
        f(self);
        self.scale = saved_scale;
        (self.origin_x, self.origin_y) = saved_origin;
        self.screen = saved_screen;
        self.restore_clip(saved_clip);
    }

    /// Render `f` at logical→physical scale `scale` into the on-screen region
    /// `area`, over a `bg` backdrop, then magnify the rendered pixels by the
    /// integer factor `zoom` using nearest-neighbor replication. `zoom == 1`
    /// draws straight into `area`, with no allocation;
    /// `zoom > 1` renders once into an offscreen buffer at `scale` and blits it
    /// `zoom`× larger, so each device pixel becomes a `zoom × zoom` block.
    ///
    /// The magnification is a pure pixel copy applied *after* drawing — it
    /// never feeds back into `scale`, so chrome stays snapped and glyphs stay
    /// rasterized exactly as `scale` alone would produce them, just enlarged.
    /// That's the point of it: on a HiDPI display, where device pixels are
    /// tiny, a `zoom` of 2 lets the eye actually see the per-pixel snapping and
    /// rasterization a given scale yields. `bg` is the backdrop the content is
    /// composited onto; it matters for the anti-aliased edges of text and
    /// glyphs, which blend toward it.
    ///
    /// `area` is the *final* on-screen footprint (already accounting for
    /// `zoom`). Drawing is clipped to it, intersected with any active clip.
    pub fn draw_scaled(
        &mut self,
        area: Rect,
        scale: f32,
        zoom: i32,
        bg: Color,
        f: impl FnOnce(&mut Painter),
    ) {
        if area.w <= 0 || area.h <= 0 {
            return;
        }
        let zoom = zoom.max(1);
        // Fill the footprint first: the in-place path draws over it, and the
        // zoomed path uses it to back any sub-pixel gap the integer
        // magnification can leave at the right / bottom edge.
        self.fill_rect(area, bg);
        if zoom == 1 {
            self.with_scale(area, scale, f);
            return;
        }
        // Render once at `scale` into an offscreen buffer 1/zoom the footprint,
        // then magnify. The buffer starts as `bg` so anti-aliased edges blend
        // against the same backdrop the in-place path draws over.
        let phys_w = self.snap(area.x + area.w) - self.snap(area.x);
        let phys_h = self.snap(area.y + area.h) - self.snap(area.y);
        let off_w = (phys_w / zoom).max(1);
        let off_h = (phys_h / zoom).max(1);
        let mut buf = vec![bg.0; (off_w * off_h) as usize];
        {
            let mut p = Painter::new(&mut buf, off_w, off_h, scale, 0, 0, self.fonts);
            f(&mut p);
        }
        let dst_x = self.origin_x + self.snap(area.x);
        let dst_y = self.origin_y + self.snap(area.y);
        self.blit_zoomed(&buf, off_w, off_h, dst_x, dst_y, zoom);
    }

    /// Opaque-copy a `src_w × src_h` ARGB buffer onto the surface with its
    /// top-left at physical pixel `(dst_x, dst_y)`, expanding every source
    /// pixel to a `zoom × zoom` block. Honors the active clip. The magnifying
    /// half of [`Self::draw_scaled`].
    fn blit_zoomed(
        &mut self,
        src: &[u32],
        src_w: i32,
        src_h: i32,
        dst_x: i32,
        dst_y: i32,
        zoom: i32,
    ) {
        let (cx0, cy0, cx1, cy1) = self.clip_bounds();
        for sy in 0..src_h {
            let src_row = (sy * src_w) as usize;
            for ry in 0..zoom {
                let dy = dst_y + sy * zoom + ry;
                if dy < cy0 || dy >= cy1 {
                    continue;
                }
                let dst_row = (dy * self.width) as usize;
                for sx in 0..src_w {
                    let px = src[src_row + sx as usize];
                    let bx = dst_x + sx * zoom;
                    for rx in 0..zoom {
                        let dx = bx + rx;
                        if dx >= cx0 && dx < cx1 {
                            self.pixels[dst_row + dx as usize] = px;
                        }
                    }
                }
            }
        }
    }

    /// Render `f` at logical→physical scale `scale` into an offscreen buffer
    /// `content` logical pixels large, then resample that buffer into `area` on
    /// screen, area-averaging the source pixels each destination pixel covers.
    ///
    /// Where [`Self::draw_scaled`] ties the footprint to the scale — content
    /// drawn at `scale` lands on screen at `scale`, so raising it grows the
    /// result — this pins the footprint and lets the scale change only *how
    /// many device pixels the content was rendered from*. Holding `area` still
    /// while `scale` rises is what a denser display of the same physical size
    /// shows: the panel keeps its size and gains resolution. That makes this
    /// the tool for a preview pane that must not resize itself as the previewed
    /// DPI is dragged around.
    ///
    /// `content` is the extent, in the *preview's* logical pixels, the buffer
    /// covers: `f` draws within `(0, 0)`–`(content.w, content.h)` and anything
    /// past that falls off the buffer. `area` need not share its aspect ratio —
    /// each axis is resampled independently — so a caller who wants the content
    /// undistorted gives `area` the proportions of `content`.
    ///
    /// The resample is an area average: a destination pixel is the mean of the
    /// source pixels it overlaps, each weighted by how much of it the
    /// destination pixel covers (to a 256th of a source pixel). Downscaling
    /// therefore blends rather than drops pixels, so a scale's extra
    /// resolution reads as detail instead of dropouts; and
    /// where `area` is a whole multiple of the buffer every destination pixel
    /// falls inside a single source pixel, which reproduces
    /// [`Self::draw_scaled`]'s nearest-neighbor magnification exactly.
    ///
    /// `bg` is the backdrop the offscreen buffer starts as, so anti-aliased
    /// edges blend toward the color they will be seen against — and it needs no
    /// separate fill of `area`, since every pixel of it is written. Drawing is
    /// confined to `area`, intersected with any active clip.
    pub fn draw_resampled(
        &mut self,
        area: Rect,
        content: Size,
        scale: f32,
        bg: Color,
        f: impl FnOnce(&mut Painter),
    ) {
        if area.w <= 0 || area.h <= 0 || content.w <= 0 || content.h <= 0 {
            return;
        }
        let scale = scale.max(0.01);
        // The buffer holds the content at its *own* scale — the device pixels a
        // window opened at `scale` would give it — whatever room `area` has for
        // the result.
        let src_w = ((content.w as f32 * scale).round() as i32).max(1);
        let src_h = ((content.h as f32 * scale).round() as i32).max(1);
        let mut buf = vec![bg.0; (src_w * src_h) as usize];
        {
            let mut p = Painter::new(&mut buf, src_w, src_h, scale, 0, 0, self.fonts);
            f(&mut p);
        }
        let dst = Rect::new(
            self.origin_x + self.snap(area.x),
            self.origin_y + self.snap(area.y),
            self.snap(area.x + area.w) - self.snap(area.x),
            self.snap(area.y + area.h) - self.snap(area.y),
        );
        self.blit_resampled(&buf, src_w, src_h, dst);
    }

    /// Opaque-copy a `src_w × src_h` ARGB buffer onto the physical rectangle
    /// `dst`, resampling it to fit: every destination pixel is the
    /// area-weighted mean of the source pixels it covers. Honors the active
    /// clip. The resampling half of [`Self::draw_resampled`].
    ///
    /// Three shapes of destination pixel are pulled out of the general case,
    /// because this runs on every frame of a preview being dragged and most
    /// pixels are one of the cheap ones. A pixel that lies inside a single
    /// source pixel is a copy. A *row* that lies inside a single source row
    /// blends horizontally only. And a row whose source row is the one the row
    /// above already used is a copy of the row just written — which, when the
    /// buffer is being magnified, is most of the rows on screen.
    fn blit_resampled(&mut self, src: &[u32], src_w: i32, src_h: i32, dst: Rect) {
        let (cx0, cy0, cx1, cy1) = self.clip_bounds();
        let (x0, x1) = (dst.x.max(cx0), (dst.x + dst.w).min(cx1));
        if x1 <= x0 {
            return;
        }
        let cols = AxisMap::new(src_w, dst.w);
        let rows = AxisMap::new(src_h, dst.h);
        // Where the last row went and which source row it was a plain copy of,
        // for the wholesale row copy. `None` while the last row was a blend of
        // several, which no other row repeats.
        let mut repeatable: Option<(usize, i32)> = None;
        for dy in 0..dst.h {
            let py = dst.y + dy;
            if py < cy0 || py >= cy1 {
                repeatable = None;
                continue;
            }
            let base = (py * self.width) as usize;
            let row_taps = rows.taps(dy);
            let [(sy, _)] = row_taps else {
                // Rows to blend: the general case, a full 16-bit average.
                repeatable = None;
                for px in x0..x1 {
                    let col_taps = cols.taps(px - dst.x);
                    let (mut a, mut r, mut g, mut b) = (0u32, 0u32, 0u32, 0u32);
                    for &(sy, wy) in row_taps {
                        let row = (sy * src_w) as usize;
                        for &(sx, wx) in col_taps {
                            let weight = (wx * wy) as u32;
                            let pixel = src[row + sx as usize];
                            a += ((pixel >> 24) & 0xFF) * weight;
                            r += ((pixel >> 16) & 0xFF) * weight;
                            g += ((pixel >> 8) & 0xFF) * weight;
                            b += (pixel & 0xFF) * weight;
                        }
                    }
                    self.pixels[base + px as usize] = resampled(a, r, g, b, 2 * WEIGHT_BITS);
                }
                continue;
            };
            // One source row, so nothing to average vertically.
            if let Some((from, previous)) = repeatable
                && previous == *sy
            {
                let span = from + x0 as usize..from + x1 as usize;
                self.pixels.copy_within(span, base + x0 as usize);
                repeatable = Some((base, *sy));
                continue;
            }
            let row = (sy * src_w) as usize;
            for px in x0..x1 {
                let col_taps = cols.taps(px - dst.x);
                self.pixels[base + px as usize] = match col_taps {
                    // One source pixel: the copy every pixel of a whole-number
                    // magnification takes.
                    [(sx, _)] => src[row + *sx as usize],
                    _ => {
                        let (mut a, mut r, mut g, mut b) = (0u32, 0u32, 0u32, 0u32);
                        for &(sx, wx) in col_taps {
                            let weight = wx as u32;
                            let pixel = src[row + sx as usize];
                            a += ((pixel >> 24) & 0xFF) * weight;
                            r += ((pixel >> 16) & 0xFF) * weight;
                            g += ((pixel >> 8) & 0xFF) * weight;
                            b += (pixel & 0xFF) * weight;
                        }
                        resampled(a, r, g, b, WEIGHT_BITS)
                    }
                };
            }
            repeatable = Some((base, *sy));
        }
    }

    /// Fill a compile-time-baked [`SvgImage`](crate::SvgImage) into the logical
    /// rectangle `rect`, aspect-fit and centered. Convenience wrapper for
    /// [`SvgImage::draw`](crate::SvgImage::draw).
    pub fn draw_svg(&mut self, image: &crate::svg::SvgImage, rect: Rect) {
        image.draw(self, rect);
    }

    /// Like [`draw_svg`](Self::draw_svg) but recolor the image with `tint` —
    /// the wrapper for [`SvgImage::draw_tinted`](crate::SvgImage::draw_tinted),
    /// meant for single-color glyphs that should follow a theme color.
    pub fn draw_svg_tinted(&mut self, image: &crate::svg::SvgImage, rect: Rect, tint: Color) {
        image.draw_tinted(self, rect, tint);
    }

    /// The font families this painter draws with.
    pub fn fonts(&self) -> FontSet<'a> {
        self.fonts
    }

    /// The sans-serif family — what `text` / `measure_text` use.
    pub fn font(&self) -> Option<&Font> {
        self.fonts.sans
    }

    /// The serif family.
    pub fn serif_font(&self) -> Option<&Font> {
        self.fonts.serif
    }

    /// The monospace family — what the text editors and `text_styled` with
    /// [`FontFamily::Mono`] use.
    pub fn mono_font(&self) -> Option<&Font> {
        self.fonts.mono
    }

    /// The loaded font for `family`, if any. Returns the `'a`-lifetime reference
    /// from the [`FontSet`] (not one borrowed from `&self`) so a caller can hold
    /// it across a `&mut self` draw call, the way `text` / `text_styled` do.
    fn family_font(&self, family: FontFamily) -> Option<&'a Font> {
        match family {
            FontFamily::Sans => self.fonts.sans,
            FontFamily::Serif => self.fonts.serif,
            FontFamily::Mono => self.fonts.mono,
        }
    }

    /// Snap a logical-pixel coordinate (edge or position) to a physical pixel.
    /// Edges of adjacent rectangles are snapped *independently*, so they
    /// always meet on the same physical pixel without gaps or overlap.
    fn snap(&self, logical: i32) -> i32 {
        (logical as f32 * self.scale).round() as i32
    }

    /// Fill the whole physical buffer with a solid color.
    pub fn fill(&mut self, color: Color) {
        self.pixels.fill(color.0);
    }

    /// Paint a regular top-level window's background: flood the whole buffer
    /// with `base`, then stamp `pattern` on top in `fg`. The pattern grid is
    /// anchored to the logical origin (so it doesn't crawl when the window is
    /// resized or letterboxed) and its spacing scales with the DPI so the
    /// texture keeps its proportions. [`BackgroundPattern::None`] is a plain
    /// `base` fill; [`BackgroundPattern::Solid`] is a plain `fg` fill.
    pub fn fill_pattern(&mut self, base: Color, pattern: BackgroundPattern, fg: Color) {
        self.stamp_pattern(base, pattern, fg, None);
    }

    /// [`fill_pattern`](Self::fill_pattern) for a window whose root widget
    /// paints every pixel of `covered` itself: the pattern is laid down only in
    /// the letterbox around that rect, since anything inside it would be
    /// overdrawn before the frame ever reaches the screen. A root that fills its
    /// whole bounds — which most do — leaves nothing to paint at all.
    ///
    /// `covered` is a logical rect, and its edges are snapped to device pixels
    /// the way [`fill_rect`](Self::fill_rect) snaps its own, so the pattern and
    /// the widget meet exactly with no seam and no double-painted column.
    pub fn fill_pattern_around(
        &mut self,
        base: Color,
        pattern: BackgroundPattern,
        fg: Color,
        covered: Rect,
    ) {
        self.stamp_pattern(base, pattern, fg, Some(covered));
    }

    /// Body of both `fill_pattern` entry points. `covered`, when given, is left
    /// untouched — see [`fill_pattern_around`](Self::fill_pattern_around).
    ///
    /// Every one of these patterns repeats after a handful of rows, so only the
    /// first period is computed pixel by pixel and the rest of the buffer is
    /// filled by copying whole rows. That turns a `rem_euclid` per pixel of the
    /// window into a memcpy per row — which is worth having on a HiDPI surface,
    /// where the same window is four times as many pixels.
    ///
    /// Like the other whole-surface fills, this ignores the clip rect: it runs
    /// before the widget tree, as the ground everything else is painted onto.
    fn stamp_pattern(
        &mut self,
        base: Color,
        pattern: BackgroundPattern,
        fg: Color,
        covered: Option<Rect>,
    ) {
        // The gap the caller fills itself, in physical pixels. An empty or
        // absent rect degenerates to "no gap", i.e. paint everything.
        let gap = covered.and_then(|r| {
            let x0 = (self.origin_x + self.snap(r.x)).clamp(0, self.width);
            let y0 = (self.origin_y + self.snap(r.y)).clamp(0, self.height);
            let x1 = (self.origin_x + self.snap(r.x + r.w)).clamp(0, self.width);
            let y1 = (self.origin_y + self.snap(r.y + r.h)).clamp(0, self.height);
            (x1 > x0 && y1 > y0).then_some((x0, y0, x1, y1))
        });
        let (gx0, gy0, gx1, gy1) = gap.unwrap_or((0, 0, 0, 0));
        // The whole surface belongs to the caller: nothing to do.
        if gx0 == 0 && gy0 == 0 && gx1 >= self.width && gy1 >= self.height {
            return;
        }

        // Grid spacing + feature thickness in physical pixels. `near` is the
        // tight 4px grid (lines, diagonal hatching); `wide` is the looser 6px
        // grid used by the staggered dots and the cross-stitch weave.
        let near = (4.0 * self.scale).round().max(1.0) as i32;
        let wide = (6.0 * self.scale).round().max(1.0) as i32;
        let thick = chrome_px(self.scale);
        // How many rows until the pattern comes back into phase: the dot field
        // staggers alternate rows, so it takes two grid steps to repeat; the
        // diagonals and lines are back in phase after one.
        let period = match pattern {
            BackgroundPattern::Dots => wide * 2,
            BackgroundPattern::CrossStitch => wide,
            _ => near,
        };
        // The flat ground under the pattern, and whether there is a pattern to
        // stamp on it at all.
        let ground = match pattern {
            BackgroundPattern::Solid => fg.0,
            _ => base.0,
        };
        let stamped = !matches!(pattern, BackgroundPattern::None | BackgroundPattern::Solid);
        // A staggered dot field: dots sit on a `step` grid, but every other
        // row is shifted half a step so each dot falls in the gap of the row
        // above — the quincunx of the classic Mac desktop, not a square grid.
        let dotted = |ax: i32, ay: i32, step: i32| -> bool {
            if ay.rem_euclid(step) >= thick {
                return false;
            }
            let shift = if ay.div_euclid(step).rem_euclid(2) == 1 {
                step / 2
            } else {
                0
            };
            (ax - shift).rem_euclid(step) < thick
        };
        let fg = fg.0;
        for y in 0..self.height {
            // The x-spans this row paints: the whole row, or the strips either
            // side of the caller's gap.
            let in_gap = y >= gy0 && y < gy1;
            let spans = if in_gap {
                [(0, gx0), (gx1, self.width)]
            } else {
                [(0, self.width), (0, 0)]
            };
            let row = (y * self.width) as usize;
            // A row `period` back carries the same pattern phase; when it also
            // paints the same spans, copy it rather than recompute it.
            let src = y - period;
            if src >= 0 && (src >= gy0 && src < gy1) == in_gap {
                let src_row = (src * self.width) as usize;
                for (x0, x1) in spans {
                    if x1 > x0 {
                        self.pixels.copy_within(
                            src_row + x0 as usize..src_row + x1 as usize,
                            row + x0 as usize,
                        );
                    }
                }
                continue;
            }
            let ay = y - self.origin_y;
            for (x0, x1) in spans {
                if x1 <= x0 {
                    continue;
                }
                self.pixels[row + x0 as usize..row + x1 as usize].fill(ground);
                if !stamped {
                    continue;
                }
                // Offset by the logical origin so the grid stays put regardless
                // of letterboxing; `rem_euclid` keeps it stable for negative
                // offsets.
                for x in x0..x1 {
                    let ax = x - self.origin_x;
                    let on = match pattern {
                        BackgroundPattern::Dots => dotted(ax, ay, wide),
                        BackgroundPattern::Lines => ay.rem_euclid(near) < thick,
                        BackgroundPattern::DiagonalForward => (ax + ay).rem_euclid(near) < thick,
                        BackgroundPattern::CrossStitch => {
                            (ax + ay).rem_euclid(wide) < thick || (ax - ay).rem_euclid(wide) < thick
                        }
                        // Handled by `stamped` above.
                        BackgroundPattern::None | BackgroundPattern::Solid => false,
                    };
                    if on {
                        self.pixels[row + x as usize] = fg;
                    }
                }
            }
        }
    }

    /// Fill the logical rectangle `rect` with a two-tone checkerboard: a solid
    /// `base` fill stippled with `fg` on alternating cells. The cell size tracks
    /// the DPI (one logical pixel, rounded to whole device pixels) and the grid
    /// is anchored to the logical origin, so the texture keeps its proportions
    /// and doesn't crawl when the content is letterboxed. This is the Win 3.1
    /// scrollbar track's "newsprint" pattern; at the default 1.0x it's a classic
    /// 1px black-on-gray checker.
    pub fn fill_checker(&mut self, rect: Rect, base: Color, fg: Color) {
        self.fill_rect(rect, base);
        let x0 = self.origin_x + self.snap(rect.x);
        let y0 = self.origin_y + self.snap(rect.y);
        let x1 = self.origin_x + self.snap(rect.x + rect.w);
        let y1 = self.origin_y + self.snap(rect.y + rect.h);
        let (cx0, cy0, cx1, cy1) = self.clip_bounds();
        let xs = x0.max(cx0);
        let ys = y0.max(cy0);
        let xe = x1.min(cx1);
        let ye = y1.min(cy1);
        let step = chrome_px(self.scale);
        let fg = fg.0;
        for y in ys..ye {
            let ay = (y - self.origin_y).div_euclid(step);
            let row = (y * self.width) as usize;
            for x in xs..xe {
                let ax = (x - self.origin_x).div_euclid(step);
                if (ax + ay).rem_euclid(2) == 0 {
                    self.pixels[row + x as usize] = fg;
                }
            }
        }
    }

    /// Solid-fill a physical-pixel rectangle. Used internally after logical
    /// coordinates have been snapped + offset.
    fn fill_phys(&mut self, x: i32, y: i32, w: i32, h: i32, color: Color) {
        if w <= 0 || h <= 0 {
            return;
        }
        let (cx0, cy0, cx1, cy1) = self.clip_bounds();
        let x0 = x.max(cx0);
        let y0 = y.max(cy0);
        let x1 = (x + w).min(cx1);
        let y1 = (y + h).min(cy1);
        for yy in y0..y1 {
            let row = (yy * self.width) as usize;
            for xx in x0..x1 {
                self.pixels[row + xx as usize] = color.0;
            }
        }
    }

    /// Alpha-blend a single physical-pixel pixel. Coordinates are relative to
    /// the logical origin — the origin offset and clipping happen here. Used
    /// by glyph rasterization in [`Font::draw_phys`].
    pub(crate) fn blend_pixel_phys(&mut self, x: i32, y: i32, color: Color, alpha: u8) {
        let x = x + self.origin_x;
        let y = y + self.origin_y;
        let (cx0, cy0, cx1, cy1) = self.clip_bounds();
        if x < cx0 || y < cy0 || x >= cx1 || y >= cy1 {
            return;
        }
        if alpha == 0 {
            return;
        }
        if alpha == 255 {
            self.pixels[(y * self.width + x) as usize] = color.0;
            return;
        }
        let idx = (y * self.width + x) as usize;
        let dst = self.pixels[idx];
        let a = alpha as u32;
        // Source-over, with the destination's own alpha in play: the weight it
        // keeps is what the source lets through, of however much of it there
        // was. An opaque destination — every pixel of a window that paints its
        // own background — leaves `dw == 255 - a` and `out == 255`, which is a
        // plain lerp between the two colours. A transparent one (the field a
        // popup window with a real alpha channel is cleared to) leaves
        // `dw == 0`, so the source keeps its own colour at its own coverage
        // instead of blending toward the black behind the nothing.
        let dw = ((dst >> 24) & 0xFF) * (255 - a) / 255;
        let out = a + dw;
        if out == 0 {
            return;
        }
        let sr = color.red() as u32;
        let sg = color.green() as u32;
        let sb = color.blue() as u32;
        let dr = (dst >> 16) & 0xFF;
        let dg = (dst >> 8) & 0xFF;
        let db = dst & 0xFF;
        let r = (sr * a + dr * dw) / out;
        let g = (sg * a + dg * dw) / out;
        let b = (sb * a + db * dw) / out;
        self.pixels[idx] = (out << 24) | (r << 16) | (g << 8) | b;
    }

    /// Logical-coordinate single-pixel write — a 1×1 logical pixel becomes the
    /// physical area between (x, y) and (x+1, y+1) after edge snapping.
    pub fn pixel(&mut self, x: i32, y: i32, color: Color) {
        self.fill_rect(Rect::new(x, y, 1, 1), color);
    }

    pub fn fill_rect(&mut self, rect: Rect, color: Color) {
        self.fill_rect_with_phys_offset(rect, 0, 0, color);
    }

    /// Fill a rectangle with an additional physical-pixel offset applied
    /// *after* the logical→physical snap. Pair with `text_with_phys_offset`
    /// when you want chrome (e.g., a mnemonic underline) to track text that
    /// has been nudged a fraction of a logical pixel.
    pub fn fill_rect_with_phys_offset(
        &mut self,
        rect: Rect,
        dx_phys: i32,
        dy_phys: i32,
        color: Color,
    ) {
        let x0 = self.origin_x + self.snap(rect.x) + dx_phys;
        let y0 = self.origin_y + self.snap(rect.y) + dy_phys;
        let x1 = self.origin_x + self.snap(rect.x + rect.w) + dx_phys;
        let y1 = self.origin_y + self.snap(rect.y + rect.h) + dy_phys;
        self.fill_phys(x0, y0, x1 - x0, y1 - y0, color);
    }

    pub fn h_line(&mut self, x: i32, y: i32, w: i32, color: Color) {
        self.fill_rect(Rect::new(x, y, w, 1), color);
    }

    pub fn v_line(&mut self, x: i32, y: i32, h: i32, color: Color) {
        self.fill_rect(Rect::new(x, y, 1, h), color);
    }

    /// Blit a block of pre-composited, **opaque** ARGB pixels with its top-left
    /// at logical `(x, y)`. `src` is row-major and exactly `w`×`h` logical
    /// pixels (`src.len() == w * h`); each logical pixel is snapped to physical
    /// coordinates and written as a solid block — the same result a grid of
    /// per-pixel [`pixel`](Self::pixel) calls would produce, but far cheaper:
    /// the logical→physical snap runs once per column and row instead of twice
    /// per pixel, the clip is resolved once, and no per-pixel `Rect` is built.
    ///
    /// Alpha is ignored (the source is assumed already flattened to opaque), so
    /// this performs no blending — it is the bulk path for drawing a decoded /
    /// composed image, where per-pixel `pixel()` calls are the bottleneck.
    pub fn blit_argb(&mut self, x: i32, y: i32, w: u32, h: u32, src: &[u32]) {
        if w == 0 || h == 0 {
            return;
        }
        debug_assert_eq!(src.len(), (w * h) as usize, "src must hold w*h pixels");
        let (cx0, cy0, cx1, cy1) = self.clip_bounds();
        // Snapped physical x of every logical column edge (w + 1 of them), so
        // the inner loop indexes them instead of snapping per pixel.
        let xs: Vec<i32> = (0..=w as i32)
            .map(|i| self.origin_x + self.snap(x + i))
            .collect();
        for j in 0..h as i32 {
            let py0 = (self.origin_y + self.snap(y + j)).max(cy0);
            let py1 = (self.origin_y + self.snap(y + j + 1)).min(cy1);
            if py1 <= py0 {
                continue;
            }
            let src_row = (j as u32 * w) as usize;
            for i in 0..w as usize {
                let px0 = xs[i].max(cx0);
                let px1 = xs[i + 1].min(cx1);
                if px1 <= px0 {
                    continue;
                }
                let color = src[src_row + i];
                for yy in py0..py1 {
                    let base = (yy * self.width) as usize;
                    for xx in px0..px1 {
                        self.pixels[base + xx as usize] = color;
                    }
                }
            }
        }
    }

    /// Paint a [`Ring`]: `near` along its top and left, `far` along its bottom
    /// and right. The same colour twice is an outline; two colours is a bevel,
    /// lit on the near pair and shadowed on the far one.
    ///
    /// The two meet at the top-right and bottom-left on the diagonal a Win 3.1
    /// bevel runs along — see [`Self::fill_ring_far`].
    pub fn fill_ring(&mut self, ring: Ring, near: Color, far: Color) {
        self.fill_ring_near(ring, near);
        self.fill_ring_far(ring, far);
    }

    /// Paint the near half of a [`Ring`], its top and left, corners and all.
    pub fn fill_ring_near(&mut self, ring: Ring, color: Color) {
        self.fill_rect(ring.top, color);
        self.fill_rect(ring.left, color);
    }

    /// Paint the far half of a [`Ring`], its bottom and right.
    ///
    /// Where they meet the near sides, at the top-right and bottom-left, the
    /// two halves share the corner square between them along its diagonal, a
    /// device row at a time: the far side takes the pixels on the diagonal and
    /// beyond it, and leaves the near one whatever lies inside — the near
    /// side's colour, or whatever was painted there before when only this
    /// half is drawn. At 1.0x the corner is a single pixel, which the far side
    /// takes whole, as the design has it. A ring several device pixels thick
    /// would otherwise give its far side the whole square, and a bevel's
    /// diagonal would come out a staircase with steps as deep as its rings;
    /// split, every ring's square lies on the one 45° line through the
    /// frame's corner, and the light and dark meet along it a device pixel at
    /// a time.
    pub fn fill_ring_far(&mut self, ring: Ring, color: Color) {
        let (b, r) = (ring.bottom, ring.right);
        // How far the corner squares reach along each far side: as wide as the
        // left side, and as tall as the top one.
        let (across, down) = (ring.left.w.min(b.w), ring.top.h.min(r.h));
        self.fill_rect(Rect::new(b.x + across, b.y, b.w - across, b.h), color);
        self.fill_rect(Rect::new(r.x, r.y + down, r.w, r.h - down), color);
        // Row `v` of the top-right square, counted down from the top: the far
        // side takes the last `v + 1` pixels of it.
        for v in 0..down {
            let n = (v + 1).min(r.w);
            self.fill_rect(Rect::new(r.right() - n, r.y + v, n, 1), color);
        }
        // Row `v` of the bottom-left square, counted up from the bottom: the
        // same, mirrored, so the far side takes all but the first `v` pixels.
        for v in 0..b.h {
            let skip = v.min(across);
            self.fill_rect(
                Rect::new(b.x + skip, b.bottom() - 1 - v, across - skip, 1),
                color,
            );
        }
    }

    /// Paint a [`RoundedBand`] in `color`: its four sides, each stopping a
    /// corner radius short of the corner, and the corners a device row at a
    /// time along the curve. Outside the curve nothing is painted, so what lies
    /// under the widget shows through; inside the band's inner arc, whatever
    /// was painted there already — a button's bevel — is left alone. Paint the
    /// band last, so that inner arc can round off the corner of what it frames.
    pub fn fill_rounded_band(&mut self, band: RoundedBand, color: Color) {
        let (r, rad) = (band.rect, band.radius);
        let (across, down) = band.thickness;
        self.fill_rect(Rect::new(r.x + rad, r.y, r.w - 2 * rad, down), color);
        self.fill_rect(
            Rect::new(r.x + rad, r.bottom() - down, r.w - 2 * rad, down),
            color,
        );
        self.fill_rect(Rect::new(r.x, r.y + rad, across, r.h - 2 * rad), color);
        self.fill_rect(
            Rect::new(r.right() - across, r.y + rad, across, r.h - 2 * rad),
            color,
        );
        for i in 0..rad {
            let (from, to) = band.corner_run(i);
            let (left, right) = (r.x + from, r.right() - to);
            for y in [r.y + i, r.bottom() - 1 - i] {
                self.fill_rect(Rect::new(left, y, to - from, 1), color);
                self.fill_rect(Rect::new(right, y, to - from, 1), color);
            }
        }
    }

    pub fn stroke_rect(&mut self, rect: Rect, color: Color) {
        self.stroke_rect_merged(rect, Merged::NONE, color);
    }

    /// [`Self::stroke_rect`] for an outline whose `merged` edges sit on a
    /// neighbouring widget's own frame line — see [`Merged`].
    pub fn stroke_rect_merged(&mut self, rect: Rect, merged: Merged, color: Color) {
        if rect.w <= 0 || rect.h <= 0 {
            return;
        }
        self.crisp_merged(rect, merged, |p, f| p.fill_ring(f.ring(0), color, color));
    }

    /// Raised 3D bevel: light highlight on top/left, dark shadow on bottom/right.
    pub fn raised_bevel(&mut self, rect: Rect, highlight: Color, shadow: Color) {
        self.raised_bevel_merged(rect, Merged::NONE, highlight, shadow);
    }

    /// [`Self::raised_bevel`] for a bevel whose `merged` edges sit on a
    /// neighbouring widget's own frame line — see [`Merged`].
    pub fn raised_bevel_merged(
        &mut self,
        rect: Rect,
        merged: Merged,
        highlight: Color,
        shadow: Color,
    ) {
        if rect.w <= 0 || rect.h <= 0 {
            return;
        }
        self.crisp_merged(rect, merged, |p, f| {
            p.fill_ring(f.ring(0), highlight, shadow)
        });
    }

    pub fn sunken_bevel(&mut self, rect: Rect, highlight: Color, shadow: Color) {
        self.raised_bevel(rect, shadow, highlight);
    }

    /// [`Self::sunken_bevel`] for a bevel whose `merged` edges sit on a
    /// neighbouring widget's own frame line — see [`Merged`].
    pub fn sunken_bevel_merged(
        &mut self,
        rect: Rect,
        merged: Merged,
        highlight: Color,
        shadow: Color,
    ) {
        self.raised_bevel_merged(rect, merged, shadow, highlight);
    }

    /// Two-tone horizontal etched line (dark + light) — the divider above the
    /// system stats block in the Win 3.1 about box.
    ///
    /// Two logical pixels tall, each tone running to its own depth — so they
    /// come out even, where snapping them separately gave whichever of the two
    /// happened to round up first: 2 device pixels of shadow over 1 of highlight
    /// at 1.25x with the divider on one row, 1 over 2 with it on the next.
    pub fn etched_h_line(&mut self, x: i32, y: i32, w: i32, theme: &Theme) {
        self.crisp(Rect::new(x, y, w, 2), |p, f| {
            let r = f.rect();
            // Clamped to the snapped slot, which can be a device pixel shy of
            // `depth(2)` depending on the row the divider landed on.
            let (first, second) = (f.depth(1).min(r.h), f.depth(2).min(r.h));
            p.fill_rect(Rect::new(r.x, r.y, r.w, first), theme.shadow);
            p.fill_rect(
                Rect::new(r.x, r.y + first, r.w, second - first),
                theme.highlight,
            );
        });
    }

    /// Full Win 3.1 button chrome: every button has a 1px black outer border
    /// with rounded corners, left unpainted outside the curve; the default
    /// button's border is two pixels deep, the same curve outside and square
    /// inside. Light-gray face, raised bevel, sunken when pressed.
    ///
    /// Drawn in one [`Self::crisp`] pass, with every ring a whole
    /// [`Self::chrome_unit`] deep and measured from the button's snapped edge.
    /// That is what keeps the bevel even — 2 device pixels over 2 at 2.5x,
    /// where scaling the depths by 2.5 put a ring of 2 over a ring of 3 under
    /// a 3-pixel outline. From 1.5x up to 1.75x the bevel is the exception,
    /// 3 device pixels in rings of 2 and 1 under the 1-pixel outline: see
    /// [`Frame::bevel_ring`]. The corners are drawn as the curve the 1.0x
    /// corner is a rasterization of, at the device resolution, rather than as
    /// its missing pixel blown up: see [`RoundedBand`].
    pub fn button(&mut self, rect: Rect, theme: &Theme, pressed: bool, default: bool) {
        if rect.w <= 0 || rect.h <= 0 {
            return;
        }
        self.crisp(rect, |p, f| {
            // The default button's border is two rings deep, the same black, so
            // it reads as one heavier border whose depth is exactly `depth(2)`.
            let face = if default { 2 } else { 1 };
            p.fill_rect(f.inside(face), theme.face);
            // The bevel is two logical pixels deep: two rings, meeting on a
            // shared depth, so together they span exactly `bevel_depth(2)` and
            // still step at the corners.
            let (outer, inner) = (f.bevel_ring(face, 0), f.bevel_ring(face, 1));
            if pressed {
                // Sunken: the shadow takes both pixels along the top/left, and
                // a single highlight line sits opposite it.
                p.fill_ring(outer, theme.shadow, theme.highlight);
                p.fill_ring_near(inner, theme.shadow);
            } else {
                p.fill_ring(outer, theme.highlight, theme.shadow);
                p.fill_ring(inner, theme.highlight, theme.shadow);
            }
            // The outline goes on last: from 1.75x its rounded inner edge
            // reaches across the corner of the bevel.
            p.fill_rounded_band(f.rounded_band(face), theme.border);
        });
    }

    /// Lighter Win 3.1 chrome used by the scrollbar's arrow buttons and thumb
    /// and the dropdown's arrow button: a square (un-rounded) 1px black
    /// outline, a single highlight line on the top/left, and a 2px shadow on
    /// the bottom/right. Reads as lighter than [`Self::button`], whose rounded
    /// outline and doubled highlight give the heavier "dialog" chrome.
    ///
    /// When `pressed`, the face is drawn pushed in: the top/left carries a
    /// single shadow line (no highlight) and the bottom/right loses its shadow
    /// — the inverse of the raised look, the way a held scrollbar arrow sinks
    /// in Win 3.1.
    pub fn light_button(&mut self, rect: Rect, theme: &Theme, pressed: bool) {
        self.light_button_merged(rect, Merged::NONE, theme, pressed);
    }

    /// [`Self::light_button`] for a button whose `merged` edges sit on a
    /// neighbouring widget's own frame line — what the scrollbar thumb uses
    /// when it is parked flush against an arrow button, so its outline
    /// collapses onto the button's rather than landing a device pixel off it
    /// at fractional scales. See [`Merged`].
    pub fn light_button_merged(
        &mut self,
        rect: Rect,
        merged: Merged,
        theme: &Theme,
        pressed: bool,
    ) {
        if rect.w <= 0 || rect.h <= 0 {
            return;
        }
        self.crisp_merged(rect, merged, |p, f| {
            p.fill_ring(f.ring(0), theme.border, theme.border);
            p.fill_rect(f.inside(1), theme.face);
            let inner = f.bevel_ring(1, 0);
            if pressed {
                // Depressed: a single shadow line on the top/left, no highlight
                // and no bottom/right shadow, so the button reads as pushed in.
                p.fill_ring_near(inner, theme.shadow);
                return;
            }
            // One highlight line on the top/left against a 2px shadow on the
            // bottom/right: two rings sharing a depth, so the pair is exactly
            // `bevel_depth(2)` deep. The deeper ring's shadow meets the face
            // on the same diagonal the first one meets the highlight on.
            p.fill_ring(inner, theme.highlight, theme.shadow);
            p.fill_ring_far(f.bevel_ring(1, 1), theme.shadow);
        });
    }

    /// Dotted Win 3.1 focus rectangle: a dashed outline — one dot, one gap —
    /// tracing the edges of `rect`.
    ///
    /// What the eye reads in a dash is its rhythm. The dot is
    /// [`Frame::depth(1)`](Frame::depth) square and the pitch twice that, so the
    /// ring is evenly dotted at every scale; placing each dot at its exact
    /// scaled position instead would put 2- and 3-pixel gaps between 2-pixel
    /// dots at 2.25x, which reads as a smear and not as dots.
    pub fn focus_rect(&mut self, rect: Rect, color: Color) {
        if rect.w <= 0 || rect.h <= 0 {
            return;
        }
        self.crisp(rect, |p, f| {
            let r = f.rect();
            let dot = f.depth(1).min(r.w).min(r.h).max(1);
            let pitch = 2 * dot;
            // Both runs start on the rect's own corner, so the dash is anchored
            // there rather than wherever the pitch happens to land — the way the
            // Win 3.1 ring reads. A dot that would hang over the far edge is
            // trimmed to it instead of spilling out.
            let mut x = r.x;
            while x < r.right() {
                let w = dot.min(r.right() - x);
                p.fill_rect(Rect::new(x, r.y, w, dot), color);
                p.fill_rect(Rect::new(x, r.bottom() - dot, w, dot), color);
                x += pitch;
            }
            let mut y = r.y;
            while y < r.bottom() {
                let h = dot.min(r.bottom() - y);
                p.fill_rect(Rect::new(r.x, y, dot, h), color);
                p.fill_rect(Rect::new(r.right() - dot, y, dot, h), color);
                y += pitch;
            }
        });
    }

    /// Draw a line of regular-weight text. `x` / `y` and `size` are in logical
    /// units; the painter rasterizes glyphs once at `size × scale` physical
    /// pixels for crisp output regardless of fractional DPI.
    pub fn text(&mut self, x: i32, y: i32, text: &str, size: f32, color: Color) {
        self.text_with_phys_offset(x, y, 0, 0, text, size, color);
    }

    /// Draw a line of text in the given [`FontFamily`] and [`FontStyle`]. Bold /
    /// italic / bold-italic render with the host's *real* styled faces (see
    /// [`Font`]); a style the system ships no face for falls back to the nearest
    /// real face it does have, never a synthesized one. Drawing in a family the
    /// painter has no font for is a no-op. Like [`text`](Self::text), `x` / `y`
    /// and `size` are logical and glyphs rasterize at `size × scale`.
    #[allow(clippy::too_many_arguments)]
    pub fn text_styled(
        &mut self,
        x: i32,
        y: i32,
        text: &str,
        size: f32,
        color: Color,
        family: FontFamily,
        style: FontStyle,
    ) {
        let Some(font) = self.family_font(family) else {
            return;
        };
        let x_phys = self.snap(x) as f32;
        let y_phys = self.snap(y) as f32;
        let scale = self.scale;
        font.draw_phys(self, text, x_phys, y_phys, size, scale, color, style);
    }

    /// Draw text with an additional physical-pixel offset applied *after*
    /// the logical→physical snap. Useful for fine alignment tweaks (e.g.
    /// nudging menu-bar labels down a single physical pixel) that don't
    /// correspond cleanly to any whole logical-pixel value.
    #[allow(clippy::too_many_arguments)]
    pub fn text_with_phys_offset(
        &mut self,
        x: i32,
        y: i32,
        dx_phys: i32,
        dy_phys: i32,
        text: &str,
        size: f32,
        color: Color,
    ) {
        let Some(font) = self.fonts.sans else {
            return;
        };
        let x_phys = (self.snap(x) + dx_phys) as f32;
        let y_phys = (self.snap(y) + dy_phys) as f32;
        let scale = self.scale;
        font.draw_phys(
            self,
            text,
            x_phys,
            y_phys,
            size,
            scale,
            color,
            FontStyle::Regular,
        );
    }

    pub fn text_centered(&mut self, rect: Rect, text: &str, size: f32, color: Color) {
        self.text_centered_styled(
            rect,
            text,
            size,
            color,
            FontFamily::Sans,
            FontStyle::Regular,
        );
    }

    /// [`text_centered`](Self::text_centered) in a given family and style. The
    /// centering measures the very face it draws with, so a bold label sits
    /// centered on its own — wider — width rather than on the regular face's,
    /// which would park it a pixel or two left of center.
    pub fn text_centered_styled(
        &mut self,
        rect: Rect,
        text: &str,
        size: f32,
        color: Color,
        family: FontFamily,
        style: FontStyle,
    ) {
        let Some(font) = self.family_font(family) else {
            return;
        };
        let (w, h) = font.measure_styled(text, size, style);
        let tx = rect.x + ((rect.w as f32 - w) / 2.0).round() as i32;
        let ty = rect.y + ((rect.h as f32 - h) / 2.0).round() as i32;
        self.text_styled(tx, ty, text, size, color, family, style);
    }

    pub fn measure_text(&self, text: &str, size: f32) -> Size {
        self.measure_text_styled(text, size, FontFamily::Sans, FontStyle::Regular)
    }

    /// Measure text as it would be drawn by [`text_styled`](Self::text_styled)
    /// in the same family and style. Serif vs sans and bold/italic faces carry
    /// their own advances, so measuring in the style the text is drawn keeps
    /// word-wrap pixel-accurate. Zero when that family has no font.
    pub fn measure_text_styled(
        &self,
        text: &str,
        size: f32,
        family: FontFamily,
        style: FontStyle,
    ) -> Size {
        let Some(font) = self.family_font(family) else {
            return Size::new(0, 0);
        };
        let (w, h) = font.measure_styled(text, size, style);
        Size::new(w.ceil() as i32, h.ceil() as i32)
    }

    /// Word-wrap `text` into the lines it takes to draw it with
    /// [`text`](Self::text) at `size` in a column `max_width` logical pixels
    /// wide: no line [measures](Self::measure_text) wider than that, unless a
    /// single character does. See
    /// [`wrap_text_styled`](Self::wrap_text_styled) for where lines break.
    pub fn wrap_text<'t>(&self, text: &'t str, size: f32, max_width: i32) -> Vec<&'t str> {
        self.wrap_text_styled(text, size, max_width, FontFamily::Sans, FontStyle::Regular)
    }

    /// Word-wrap `text` as it would be drawn by
    /// [`text_styled`](Self::text_styled) in the same family and style, into
    /// lines no wider than `max_width` logical pixels.
    ///
    /// Every `\n` starts a new line. Otherwise a line breaks after the spaces
    /// between two words, which are dropped there, or after a hyphen or dash
    /// joining two words; the spaces inside a line, and a paragraph's
    /// indentation, are kept. A word too wide for a line of its own is broken
    /// where it has to be, after a `/` or `\` if it has one — between the
    /// components of a path. A non-positive `max_width`, or a family without
    /// a font, only splits the text at its newlines.
    pub fn wrap_text_styled<'t>(
        &self,
        text: &'t str,
        size: f32,
        max_width: i32,
        family: FontFamily,
        style: FontStyle,
    ) -> Vec<&'t str> {
        match self.family_font(family) {
            Some(font) => crate::wrap::wrap(text, max_width, |ch| font.advance(ch, size, style)),
            None => crate::wrap::wrap(text, 0, |_| 0.0),
        }
    }

    /// Cumulative caret x-offsets for `text` drawn in `family` and `style` at
    /// `size` logical pixels — `out[i]` is the x where the caret sits before the
    /// i-th character, `out[len]` the end of the string. One O(n) pass over the
    /// font's per-glyph advance cache, so a text editor can rebuild its per-row
    /// caret table every frame without the O(n²) cost of remeasuring each
    /// prefix. Returns `[0]` when that family has no font.
    pub fn cumulative_widths(
        &self,
        text: &str,
        size: f32,
        family: FontFamily,
        style: FontStyle,
    ) -> Vec<i32> {
        match self.family_font(family) {
            Some(font) => font.cumulative_widths(text, size, style),
            None => vec![0],
        }
    }
}

/// Fractional bits one axis' resampling weights carry, and the whole a weight
/// of that many bits stands for. Two axes multiply to 16 bits of fraction —
/// more than an 8-bit channel can show — and being a power of two, the average
/// they feed comes out with a shift where a divide by the covered area would
/// otherwise be, which is worth having in a loop that runs per pixel per frame.
const WEIGHT_BITS: u32 = 8;
const WEIGHT_ONE: i32 = 1 << WEIGHT_BITS;

/// Fold four weight-scaled channel sums back into an ARGB word, rounding each
/// to nearest. `bits` is how much fraction the weights carried: [`WEIGHT_BITS`]
/// per axis that was averaged.
///
/// Inlined even in an unoptimized build — with a call and four closure calls
/// per pixel this is most of the work in a debug preview, and none of it once
/// it is folded into the loop.
#[inline(always)]
fn resampled(a: u32, r: u32, g: u32, b: u32, bits: u32) -> u32 {
    let half = 1 << (bits - 1);
    (((a + half) >> bits).min(0xFF) << 24)
        | (((r + half) >> bits).min(0xFF) << 16)
        | (((g + half) >> bits).min(0xFF) << 8)
        | ((b + half) >> bits).min(0xFF)
}

/// Premultiply, in place, the straight-alpha `0xAARRGGBB` pixels a painter
/// writes, for a platform that composites a see-through window from
/// premultiplied ones: a Wayland `Argb8888` buffer, a Windows layered window.
/// An opaque pixel reads the same either way, so only a buffer with see-through
/// pixels in it needs this.
#[cfg(any(windows, all(unix, not(target_os = "macos"))))]
pub(crate) fn premultiply(pixels: &mut [u32]) {
    for px in pixels {
        let a = *px >> 24;
        match a {
            255 => {}
            // Whatever colour a fully transparent pixel carries, premultiplied
            // it is nothing — and a compositor adds what is left.
            0 => *px = 0,
            _ => {
                let v = *px;
                let scaled = |shift: u32| (((v >> shift) & 0xFF) * a + 127) / 255;
                *px = (a << 24) | (scaled(16) << 16) | (scaled(8) << 8) | scaled(0);
            }
        }
    }
}

/// One axis of a resample, worked out once for every destination pixel: the
/// source pixels it draws from, and how much of each it covers, in
/// [`WEIGHT_ONE`]ths of a source pixel.
///
/// The overlaps are exact — counted in units of 1/`dst_n` of a source pixel, so
/// both grids land on whole numbers — and quantized cumulatively, each tap
/// taking the difference between two rounded running totals. Rounding therefore
/// cannot drift, and one destination pixel's weights add up to exactly
/// `WEIGHT_ONE` however the grids fall against each other: the average divides
/// by a constant the two axes fix in advance rather than by a per-pixel sum.
/// Where the ratio divides evenly a destination pixel lands inside a single
/// source pixel and gets that one alone at full weight — which is what makes a
/// whole-number magnification an exact pixel copy.
struct AxisMap {
    /// Destination pixel `d` draws from `taps[starts[d]..starts[d + 1]]`, each
    /// a source index and its weight.
    starts: Vec<u32>,
    taps: Vec<(i32, i32)>,
}

impl AxisMap {
    fn new(src_n: i32, dst_n: i32) -> Self {
        let mut starts = Vec::with_capacity(dst_n as usize + 1);
        let mut taps = Vec::with_capacity(dst_n as usize * 2);
        let (src, dst) = (src_n as i64, dst_n as i64);
        for d in 0..dst_n {
            starts.push(taps.len() as u32);
            let (start, end) = (d as i64 * src, (d as i64 + 1) * src);
            let first = (start / dst) as i32;
            let last = (((end - 1) / dst) as i32).min(src_n - 1);
            let (mut covered, mut quantized) = (0i64, 0i64);
            for i in first..=last {
                let lo = (i as i64 * dst).max(start);
                let hi = ((i as i64 + 1) * dst).min(end);
                if hi <= lo {
                    continue;
                }
                covered += hi - lo;
                // The rounded share of the destination pixel the taps up to and
                // including this one account for; this tap gets what that adds.
                let upto = (2 * covered * WEIGHT_ONE as i64 + src) / (2 * src);
                let weight = (upto - quantized) as i32;
                quantized = upto;
                if weight > 0 {
                    taps.push((i, weight));
                }
            }
        }
        starts.push(taps.len() as u32);
        Self { starts, taps }
    }

    /// Inlined even unoptimized: this is called once per destination pixel.
    #[inline(always)]
    fn taps(&self, d: i32) -> &[(i32, i32)] {
        let (from, to) = (self.starts[d as usize], self.starts[d as usize + 1]);
        &self.taps[from as usize..to as usize]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::background::BackgroundPattern;

    /// Paint `pattern` into a fresh `w × h` buffer at scale 1 and hand the
    /// pixels back for inspection.
    fn render(w: i32, h: i32, pattern: BackgroundPattern, fg: Color) -> Vec<u32> {
        let mut pixels = vec![0u32; (w * h) as usize];
        {
            let mut p = Painter::new(&mut pixels, w, h, 1.0, 0, 0, FontSet::default());
            p.fill_pattern(Color::WHITE, pattern, fg);
        }
        pixels
    }

    /// The straightforward per-pixel version of `stamp_pattern`, kept here as
    /// the thing the row-copying implementation has to agree with exactly.
    fn reference(
        w: i32,
        h: i32,
        scale: f32,
        origin: (i32, i32),
        pattern: BackgroundPattern,
        base: Color,
        fg: Color,
    ) -> Vec<u32> {
        let (origin_x, origin_y) = origin;
        let near = (4.0 * scale).round().max(1.0) as i32;
        let wide = (6.0 * scale).round().max(1.0) as i32;
        let thick = chrome_px(scale);
        let ground = match pattern {
            BackgroundPattern::Solid => fg.0,
            _ => base.0,
        };
        let mut px = vec![ground; (w * h) as usize];
        if matches!(pattern, BackgroundPattern::None | BackgroundPattern::Solid) {
            return px;
        }
        let dotted = |ax: i32, ay: i32, step: i32| -> bool {
            if ay.rem_euclid(step) >= thick {
                return false;
            }
            let shift = if ay.div_euclid(step).rem_euclid(2) == 1 {
                step / 2
            } else {
                0
            };
            (ax - shift).rem_euclid(step) < thick
        };
        for y in 0..h {
            let ay = y - origin_y;
            for x in 0..w {
                let ax = x - origin_x;
                let on = match pattern {
                    BackgroundPattern::Dots => dotted(ax, ay, wide),
                    BackgroundPattern::Lines => ay.rem_euclid(near) < thick,
                    BackgroundPattern::DiagonalForward => (ax + ay).rem_euclid(near) < thick,
                    BackgroundPattern::CrossStitch => {
                        (ax + ay).rem_euclid(wide) < thick || (ax - ay).rem_euclid(wide) < thick
                    }
                    BackgroundPattern::None | BackgroundPattern::Solid => false,
                };
                if on {
                    px[(y * w + x) as usize] = fg.0;
                }
            }
        }
        px
    }

    /// The sample content both resampling tests render: a few opaque blocks and
    /// a lone pixel, drawn at scale 1 so one content pixel is one buffer pixel.
    fn blocks(p: &mut Painter) {
        p.fill_rect(Rect::new(1, 1, 3, 2), Color::BLACK);
        p.fill_rect(Rect::new(0, 4, 7, 1), Color::rgb(0x40, 0x40, 0xC0));
        p.pixel(5, 3, Color::rgb(0x00, 0x80, 0x00));
    }

    /// A whole-number magnification has no pixels to average — each destination
    /// pixel sits inside one source pixel — so `draw_resampled` has to land on
    /// exactly the blocks `draw_scaled`'s nearest-neighbor zoom produces.
    #[test]
    fn resampling_a_whole_multiple_matches_the_zoom_blit() {
        const CONTENT: Size = Size { w: 7, h: 5 };
        for zoom in [1, 2, 3, 4] {
            let (w, h) = (CONTENT.w * zoom, CONTENT.h * zoom);
            let area = Rect::new(0, 0, w, h);
            let mut zoomed = vec![0u32; (w * h) as usize];
            {
                let mut p = Painter::new(&mut zoomed, w, h, 1.0, 0, 0, FontSet::default());
                p.draw_scaled(area, 1.0, zoom, Color::WHITE, blocks);
            }
            let mut resampled = vec![0u32; (w * h) as usize];
            {
                let mut p = Painter::new(&mut resampled, w, h, 1.0, 0, 0, FontSet::default());
                p.draw_resampled(area, CONTENT, 1.0, Color::WHITE, blocks);
            }
            assert_eq!(zoomed, resampled, "zoom {zoom}");
        }
    }

    /// Downscaling blends: a destination pixel covering four source pixels is
    /// their mean, not whichever one a nearest-neighbor pick would land on.
    #[test]
    fn resampling_down_averages_the_pixels_it_merges() {
        let mut pixels = vec![0u32; 4];
        {
            let mut p = Painter::new(&mut pixels, 2, 2, 1.0, 0, 0, FontSet::default());
            // Four content pixels per destination pixel: a black/white checker
            // in the left half, solid red in the right.
            p.draw_resampled(
                Rect::new(0, 0, 2, 1),
                Size::new(4, 2),
                1.0,
                Color::WHITE,
                |p| {
                    p.pixel(0, 0, Color::BLACK);
                    p.pixel(1, 1, Color::BLACK);
                    p.fill_rect(Rect::new(2, 0, 2, 2), Color::rgb(0xFF, 0, 0));
                },
            );
        }
        // Two black and two white averages to mid-grey (128 after rounding),
        // and a uniform block comes through unchanged.
        assert_eq!(pixels[0], Color::rgb(0x80, 0x80, 0x80).0);
        assert_eq!(pixels[1], Color::rgb(0xFF, 0, 0).0);
        // The row below the one-pixel-tall area is untouched.
        assert_eq!(&pixels[2..], &[0, 0]);
    }

    /// The weights of one destination pixel always account for exactly one
    /// source pixel's worth of coverage — the whole the averaging divides by —
    /// whether the axis is being stretched, squeezed, or left alone.
    #[test]
    fn axis_weights_always_add_up_to_a_whole_pixel() {
        for src_n in 1..=17 {
            for dst_n in 1..=17 {
                let map = AxisMap::new(src_n, dst_n);
                for d in 0..dst_n {
                    let taps = map.taps(d);
                    assert!(
                        taps.iter().all(|&(i, w)| i >= 0 && i < src_n && w > 0),
                        "{src_n}->{dst_n} pixel {d}: {taps:?}"
                    );
                    assert_eq!(
                        taps.iter().map(|&(_, w)| w).sum::<i32>(),
                        WEIGHT_ONE,
                        "{src_n}->{dst_n} pixel {d}"
                    );
                }
            }
        }
    }

    const PATTERNS: [BackgroundPattern; 6] = [
        BackgroundPattern::None,
        BackgroundPattern::Solid,
        BackgroundPattern::Dots,
        BackgroundPattern::Lines,
        BackgroundPattern::DiagonalForward,
        BackgroundPattern::CrossStitch,
    ];

    #[test]
    fn copying_rows_paints_exactly_what_the_per_pixel_loop_would() {
        // Sizes that are deliberately not multiples of either grid step, so a
        // partial final period would show up as a mismatch.
        for (w, h) in [(37, 41), (12, 12), (1, 1), (64, 3)] {
            for scale in [1.0f32, 1.25, 1.5, 2.0, 3.0] {
                for origin in [(0, 0), (7, 3), (-5, -11)] {
                    for pattern in PATTERNS {
                        let mut pixels = vec![0u32; (w * h) as usize];
                        {
                            let mut p = Painter::new(
                                &mut pixels,
                                w,
                                h,
                                scale,
                                origin.0,
                                origin.1,
                                FontSet::default(),
                            );
                            p.fill_pattern(Color::WHITE, pattern, Color::BLACK);
                        }
                        assert_eq!(
                            pixels,
                            reference(w, h, scale, origin, pattern, Color::WHITE, Color::BLACK),
                            "{pattern:?} at {w}x{h}, scale {scale}, origin {origin:?}"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn a_covered_rect_is_left_untouched_and_its_surroundings_are_not() {
        let (w, h) = (24, 24);
        let mut pixels = vec![0xdeadbeef_u32; (w * h) as usize];
        {
            let mut p = Painter::new(&mut pixels, w, h, 1.0, 0, 0, FontSet::default());
            p.fill_pattern_around(
                Color::WHITE,
                BackgroundPattern::Lines,
                Color::BLACK,
                Rect::new(8, 8, 8, 8),
            );
        }
        let at = |x: i32, y: i32| pixels[(y * w + x) as usize];
        // The gap still holds what the buffer had: the caller paints it.
        assert_eq!(at(8, 8), 0xdeadbeef, "the covered rect is not painted");
        assert_eq!(at(15, 15), 0xdeadbeef, "right up to its last pixel");
        // Everything around it is patterned, on both sides of the gap and on
        // the rows above and below it.
        assert_eq!(at(7, 8), Color::BLACK.0, "the strip left of the gap");
        assert_eq!(at(16, 8), Color::BLACK.0, "the strip right of it");
        assert_eq!(at(8, 4), Color::BLACK.0, "the band above it");
        assert_eq!(at(8, 20), Color::BLACK.0, "the band below it");
        assert_eq!(at(8, 5), Color::WHITE.0, "and the ground between lines");
    }

    #[test]
    fn a_root_covering_the_whole_surface_leaves_the_backdrop_unpainted() {
        let (w, h) = (16, 16);
        let mut pixels = vec![0xdeadbeef_u32; (w * h) as usize];
        {
            let mut p = Painter::new(&mut pixels, w, h, 1.0, 0, 0, FontSet::default());
            p.fill_pattern_around(
                Color::WHITE,
                BackgroundPattern::Dots,
                Color::BLACK,
                Rect::new(0, 0, 16, 16),
            );
        }
        assert!(
            pixels.iter().all(|&c| c == 0xdeadbeef),
            "not a single pixel is touched when the root covers the window"
        );
    }

    #[test]
    fn none_pattern_is_a_plain_base_fill() {
        let px = render(8, 8, BackgroundPattern::None, Color::BLACK);
        assert!(px.iter().all(|&c| c == Color::WHITE.0));
    }

    #[test]
    fn solid_pattern_floods_with_foreground() {
        let px = render(8, 8, BackgroundPattern::Solid, Color::BLACK);
        assert!(px.iter().all(|&c| c == Color::BLACK.0));
    }

    #[test]
    fn dots_are_staggered_between_rows() {
        let px = render(12, 12, BackgroundPattern::Dots, Color::BLACK);
        let at = |x: i32, y: i32| px[(y * 12 + x) as usize];
        // Even dot-row (y == 0): dots on the 6px grid.
        assert_eq!(at(0, 0), Color::BLACK.0);
        assert_eq!(at(6, 0), Color::BLACK.0);
        assert_eq!(at(3, 0), Color::WHITE.0);
        // Odd dot-row (y == 6): shifted half a step, so dots land in the gaps
        // of the row above rather than directly beneath it.
        assert_eq!(at(3, 6), Color::BLACK.0);
        assert_eq!(at(9, 6), Color::BLACK.0);
        assert_eq!(at(0, 6), Color::WHITE.0);
        assert_eq!(at(6, 6), Color::WHITE.0);
        // Rows between dot-rows stay blank.
        assert_eq!(at(0, 1), Color::WHITE.0);
        assert_eq!(at(3, 3), Color::WHITE.0);
    }

    #[test]
    fn lines_fill_whole_rows() {
        let px = render(8, 8, BackgroundPattern::Lines, Color::BLACK);
        let at = |x: i32, y: i32| px[(y * 8 + x) as usize];
        for x in 0..8 {
            assert_eq!(at(x, 0), Color::BLACK.0, "row 0 should be a line");
            assert_eq!(at(x, 4), Color::BLACK.0, "row 4 should be a line");
            assert_eq!(at(x, 1), Color::WHITE.0, "row 1 should be blank");
        }
    }

    #[test]
    fn cross_stitch_weave_is_wider_than_the_diagonals() {
        let px = render(12, 12, BackgroundPattern::CrossStitch, Color::BLACK);
        let at = |x: i32, y: i32| px[(y * 12 + x) as usize];
        // Lit where (x+y) or (x-y) is a multiple of the 6px cross spacing.
        assert_eq!(at(0, 0), Color::BLACK.0);
        assert_eq!(at(6, 0), Color::BLACK.0);
        assert_eq!(at(3, 3), Color::BLACK.0); // forward diagonal: x+y == 6
        assert_eq!(at(1, 1), Color::BLACK.0); // back diagonal: x-y == 0
        // The "slightly wider" check: a 4px step is now blank — the plain
        // diagonals (still on the 4px grid) would have drawn here.
        assert_eq!(at(4, 0), Color::WHITE.0);
        assert_eq!(at(2, 0), Color::WHITE.0);
    }

    #[test]
    fn with_scale_places_drawing_at_the_nested_scale() {
        let (w, h) = (20, 20);
        let mut pixels = vec![0u32; (w * h) as usize];
        {
            let mut p = Painter::new(&mut pixels, w, h, 1.0, 0, 0, FontSet::default());
            p.with_scale(Rect::new(2, 2, 8, 8), 2.0, |p| {
                assert_eq!(p.scale(), 2.0, "the closure sees the nested scale");
                // A 1×1 logical pixel at local (1,1) maps to a 2×2 device block
                // anchored at the region's top-left + 1 unit: (2 + 1·2, …).
                p.fill_rect(Rect::new(1, 1, 1, 1), Color::BLACK);
            });
            assert_eq!(p.scale(), 1.0, "the outer scale is restored afterwards");
        }
        let at = |x: i32, y: i32| pixels[(y * w + x) as usize];
        assert_eq!(at(4, 4), Color::BLACK.0);
        assert_eq!(at(5, 5), Color::BLACK.0);
        assert_eq!(at(3, 3), 0, "nothing leaks above-left of the block");
        assert_eq!(at(6, 6), 0, "nothing leaks below-right of the block");
    }

    #[test]
    fn with_scale_clips_overflow_to_the_region() {
        let (w, h) = (20, 20);
        let mut pixels = vec![0u32; (w * h) as usize];
        {
            let mut p = Painter::new(&mut pixels, w, h, 1.0, 0, 0, FontSet::default());
            // A fill far larger than the 8×8 region is trimmed to it.
            p.with_scale(Rect::new(2, 2, 8, 8), 2.0, |p| {
                p.fill_rect(Rect::new(0, 0, 100, 100), Color::WHITE);
            });
        }
        let at = |x: i32, y: i32| pixels[(y * w + x) as usize];
        assert_eq!(at(2, 2), Color::WHITE.0, "region top-left is painted");
        assert_eq!(at(9, 9), Color::WHITE.0, "region bottom-right is painted");
        assert_eq!(at(1, 1), 0, "just outside the region stays untouched");
        assert_eq!(at(10, 10), 0, "past the region stays untouched");
    }

    #[test]
    fn draw_scaled_magnifies_the_result_with_nearest_neighbor() {
        let (w, h) = (20, 20);
        let mut pixels = vec![0u32; (w * h) as usize];
        {
            let mut p = Painter::new(&mut pixels, w, h, 1.0, 0, 0, FontSet::default());
            // Footprint (2,2)+8×8 at scale 1.0, zoomed 2×: the offscreen is
            // rendered at 4×4 then each pixel becomes a 2×2 block.
            p.draw_scaled(Rect::new(2, 2, 8, 8), 1.0, 2, Color::WHITE, |p| {
                assert_eq!(p.size().w, 4, "offscreen is 1/zoom the footprint");
                p.fill_rect(Rect::new(0, 0, 1, 1), Color::BLACK);
            });
        }
        let at = |x: i32, y: i32| pixels[(y * w + x) as usize];
        // One black source pixel → a 2×2 black block at the footprint origin.
        assert_eq!(at(2, 2), Color::BLACK.0);
        assert_eq!(at(3, 3), Color::BLACK.0);
        // The pixel next to it came from a white source pixel.
        assert_eq!(at(4, 2), Color::WHITE.0);
        assert_eq!(at(2, 4), Color::WHITE.0);
        // The backdrop fills the rest of the footprint, nothing outside it.
        assert_eq!(at(9, 9), Color::WHITE.0);
        assert_eq!(at(1, 1), 0);
        assert_eq!(at(10, 10), 0);
    }

    #[test]
    fn diagonal_uses_the_tight_grid() {
        // DiagonalForward is unchanged: lit where x+y is a multiple of 4px.
        let fwd = render(8, 8, BackgroundPattern::DiagonalForward, Color::BLACK);
        let at = |x: i32, y: i32| fwd[(y * 8 + x) as usize];
        assert_eq!(at(4, 0), Color::BLACK.0); // x+y == 4
        assert_eq!(at(2, 2), Color::BLACK.0); // x+y == 4
        assert_eq!(at(1, 0), Color::WHITE.0); // x+y == 1
    }

    #[test]
    fn blit_argb_copies_pixels_at_an_offset_scale_1() {
        let (w, h) = (8, 8);
        let mut pixels = vec![0u32; (w * h) as usize];
        // A 2×2 source: red, green / blue, white (already opaque ARGB).
        let src = [0xFFFF0000, 0xFF00FF00, 0xFF0000FF, 0xFFFFFFFF];
        {
            let mut p = Painter::new(&mut pixels, w, h, 1.0, 0, 0, FontSet::default());
            p.blit_argb(3, 2, 2, 2, &src);
        }
        let at = |x: i32, y: i32| pixels[(y * w + x) as usize];
        // At scale 1 the block lands one device pixel per source pixel at (3,2).
        assert_eq!(at(3, 2), 0xFFFF0000);
        assert_eq!(at(4, 2), 0xFF00FF00);
        assert_eq!(at(3, 3), 0xFF0000FF);
        assert_eq!(at(4, 3), 0xFFFFFFFF);
        // Nothing leaks around it.
        assert_eq!(at(2, 2), 0);
        assert_eq!(at(5, 3), 0);
        assert_eq!(at(3, 4), 0);
    }

    #[test]
    fn blit_argb_snaps_each_source_pixel_to_a_block_at_scale_2() {
        let (w, h) = (12, 12);
        let mut pixels = vec![0u32; (w * h) as usize];
        let src = [0xFFFF0000, 0xFF00FF00, 0xFF0000FF, 0xFFFFFFFF];
        {
            let mut p = Painter::new(&mut pixels, w, h, 2.0, 0, 0, FontSet::default());
            // Logical (1,1): the top-left source pixel maps to the 2×2 device
            // block anchored at (2,2).
            p.blit_argb(1, 1, 2, 2, &src);
        }
        let at = |x: i32, y: i32| pixels[(y * w + x) as usize];
        // Top-left source pixel → 2×2 red block at (2,2)..(4,4).
        assert_eq!(at(2, 2), 0xFFFF0000);
        assert_eq!(at(3, 3), 0xFFFF0000);
        // Top-right source pixel → green block at x in 4..6.
        assert_eq!(at(4, 2), 0xFF00FF00);
        assert_eq!(at(5, 3), 0xFF00FF00);
        // Bottom-left → blue block at y in 4..6.
        assert_eq!(at(2, 4), 0xFF0000FF);
        assert_eq!(at(3, 5), 0xFF0000FF);
        // Nothing above-left of the block.
        assert_eq!(at(1, 1), 0);
    }

    #[test]
    fn blit_argb_is_clipped() {
        let (w, h) = (8, 8);
        let mut pixels = vec![0u32; (w * h) as usize];
        let src = vec![0xFFFFFFFFu32; 16]; // 4×4 opaque white
        {
            let mut p = Painter::new(&mut pixels, w, h, 1.0, 0, 0, FontSet::default());
            // Clip to a 2×2 window; only that corner of the 4×4 blit survives.
            p.set_clip_phys(1, 1, 2, 2);
            p.blit_argb(0, 0, 4, 4, &src);
        }
        let at = |x: i32, y: i32| pixels[(y * w + x) as usize];
        assert_eq!(at(1, 1), 0xFFFFFFFF, "inside the clip is painted");
        assert_eq!(at(2, 2), 0xFFFFFFFF, "inside the clip is painted");
        assert_eq!(at(0, 0), 0, "outside the clip stays untouched");
        assert_eq!(at(3, 3), 0, "outside the clip stays untouched");
    }

    /// Every scale a saudade window is plausibly drawn at: the quarters Windows,
    /// X11 and macOS hand over, the thirds that come of aiming a logical pixel
    /// at 72 dpi rather than 96, and the twelfths `SAUDADE_UI_DPI` can land on
    /// between them — see [`crate::ui_scale`]. The frame primitives have
    /// to hold their geometry on all of them, in order, which is what a
    /// [`Frame`]'s depths are for.
    const CHROME_SCALES: &[f32] = &[
        1.0,
        7.0 / 6.0,
        1.25,
        4.0 / 3.0,
        1.5,
        5.0 / 3.0,
        1.75,
        2.0,
        2.25,
        7.0 / 3.0,
        2.5,
        8.0 / 3.0,
        2.75,
        3.0,
    ];

    /// Paint `f` into a fresh `w × h` *device-pixel* buffer at `scale`.
    fn chrome_buffer(w: i32, h: i32, scale: f32, f: impl FnOnce(&mut Painter)) -> Vec<u32> {
        let mut pixels = vec![0u32; (w * h) as usize];
        {
            let mut p = Painter::new(&mut pixels, w, h, scale, 0, 0, FontSet::default());
            f(&mut p);
        }
        pixels
    }

    /// Run-length encode a line of pixels into `(color, length)` pairs. What a
    /// frame test wants to know is how thick each band came out, in order.
    fn runs(line: impl Iterator<Item = u32>) -> Vec<(u32, i32)> {
        let mut out: Vec<(u32, i32)> = Vec::new();
        for px in line {
            match out.last_mut() {
                Some((color, len)) if *color == px => *len += 1,
                _ => out.push((px, 1)),
            }
        }
        out
    }

    /// The bands crossed going down column `x` / across row `y`.
    fn column(px: &[u32], w: i32, h: i32, x: i32) -> Vec<(u32, i32)> {
        runs((0..h).map(|y| px[(y * w + x) as usize]))
    }

    fn row(px: &[u32], w: i32, y: i32) -> Vec<(u32, i32)> {
        runs((0..w).map(|x| px[(y * w + x) as usize]))
    }

    /// The device size a logical rect of `(w, h)` at the origin occupies.
    fn phys(w: i32, h: i32, scale: f32) -> (i32, i32) {
        (
            (w as f32 * scale).round() as i32,
            (h as f32 * scale).round() as i32,
        )
    }

    /// What [`Frame::depth`] will answer: the only arithmetic a frame does,
    /// a whole number of device pixels per logical one.
    fn depth(d: i32, scale: f32) -> i32 {
        d * chrome_px(scale)
    }

    /// How deep `n` rings of [`Frame::bevel_ring`] run, from wherever the
    /// outline inside which they lie stops.
    fn bevel(n: i32, scale: f32) -> i32 {
        bevel_depth(n, scale)
    }

    /// One below [`CHROME_PIN`], then the scale's whole part — except from
    /// 1.75x up to 2x, where that would leave the face a scale of 1.75 or more.
    #[test]
    fn a_chrome_unit_is_a_whole_multiple_of_the_design() {
        let expected = [
            (1.0, 1),
            (7.0 / 6.0, 1),
            (1.25, 1),
            (4.0 / 3.0, 1),
            (1.5, 1),
            (5.0 / 3.0, 1),
            (1.75, 2),
            (2.0, 2),
            (2.25, 2),
            (7.0 / 3.0, 2),
            (2.5, 2),
            (8.0 / 3.0, 2),
            (2.75, 2),
            (3.0, 3),
            (3.5, 3),
            (3.75, 3),
            (4.0, 4),
        ];
        for (scale, unit) in expected {
            let mut pixels = [0u32; 4];
            let p = Painter::new(&mut pixels, 2, 2, scale, 0, 0, FontSet::default());
            assert_eq!(p.chrome_unit(), unit, "the unit at {scale}x");
        }
    }

    /// A bevel is a chrome unit a ring everywhere but from 1.5x up to 1.75x,
    /// where it is drawn at 1.5x under an outline of 1 — rings of 2 and 1, the
    /// same all the way up the band — and nowhere further up the ladder does
    /// the face outgrow its chrome by enough to need the same.
    #[test]
    fn a_bevel_parts_from_its_outline_only_from_one_and_a_half_to_one_and_three_quarters() {
        for step in 1..=96 {
            let scale = step as f32 / 12.0;
            let line = chrome_px(scale);
            let rings = (bevel(1, scale), bevel(2, scale) - bevel(1, scale));
            if (1.5..1.75).contains(&scale) {
                assert_eq!((line, rings), (1, (2, 1)), "outline and bevel at {scale}x");
            } else {
                assert_eq!(rings, (line, line), "the bevel at {scale}x");
                if scale >= 1.0 {
                    assert!(scale / (line as f32) < 1.5, "the face's share at {scale}x");
                }
            }
        }
    }

    /// A section through a plain button, at every scale: the black outline runs
    /// to `depth(1)` and the bevel two bevel rings on from there, on all four
    /// sides. Both bands are measured from the button's edge, so neither
    /// carries the other's rounding error.
    #[test]
    fn a_button_frames_bands_land_on_their_scaled_depths() {
        let theme = Theme::windows_31();
        for &scale in CHROME_SCALES {
            let (border, bevel) = (depth(1, scale), bevel(2, scale));
            let (w, h) = phys(60, 30, scale);
            let px = chrome_buffer(w, h, scale, |p| {
                p.button(Rect::new(0, 0, 60, 30), &theme, false, false)
            });
            let expected = |across: i32| {
                vec![
                    (theme.border.0, border),
                    (theme.highlight.0, bevel),
                    (theme.face.0, across - 2 * (border + bevel)),
                    (theme.shadow.0, bevel),
                    (theme.border.0, border),
                ]
            };
            assert_eq!(
                column(&px, w, h, w / 2),
                expected(h),
                "vertical section of a button at {scale}x"
            );
            assert_eq!(
                row(&px, w, h / 2),
                expected(w),
                "horizontal section of a button at {scale}x"
            );
        }
    }

    /// Below 1.5x the frame keeps the widths it is drawn at, and the room the
    /// scale buys goes to the face instead. A button at 1.25x is a quarter
    /// bigger around the same 1-pixel border and 2-pixel bevel: roomier, and
    /// far sharper than the 3-pixel bevel under a 1-pixel border that scaling
    /// the depths there would give.
    #[test]
    fn below_one_and_a_half_a_frame_keeps_its_drawn_widths() {
        let theme = Theme::windows_31();
        for &scale in &[1.0f32, 1.1, 1.25, 1.4, 1.49] {
            let (w, h) = phys(60, 30, scale);
            let px = chrome_buffer(w, h, scale, |p| {
                p.button(Rect::new(0, 0, 60, 30), &theme, false, false)
            });
            assert_eq!(
                column(&px, w, h, w / 2),
                vec![
                    (theme.border.0, 1),
                    (theme.highlight.0, 2),
                    (theme.face.0, h - 6),
                    (theme.shadow.0, 2),
                    (theme.border.0, 1),
                ],
                "a button's frame at {scale}x, which should be the 1.0x one"
            );
        }
    }

    /// From 1.5x up to 1.75x the outline stays at one device pixel but the
    /// bevel inside it is drawn at 1.5x — the 1.0x bevel's relief would all but
    /// vanish against a face half as big again — and from 1.75x the border
    /// doubles too.
    #[test]
    fn from_one_and_a_half_the_bevel_deepens_inside_a_one_pixel_outline() {
        let theme = Theme::windows_31();
        for &scale in &[1.5f32, 1.6, 5.0 / 3.0, 1.74] {
            let (w, h) = phys(60, 30, scale);
            let px = chrome_buffer(w, h, scale, |p| {
                p.button(Rect::new(0, 0, 60, 30), &theme, false, false)
            });
            assert_eq!(
                column(&px, w, h, w / 2),
                vec![
                    (theme.border.0, 1),
                    (theme.highlight.0, 3),
                    (theme.face.0, h - 8),
                    (theme.shadow.0, 3),
                    (theme.border.0, 1),
                ],
                "a button's frame at {scale}x"
            );
        }
        // From 1.75x the depths double, and the border thickens with them.
        let (w, h) = phys(60, 30, 1.75);
        let px = chrome_buffer(w, h, 1.75, |p| {
            p.button(Rect::new(0, 0, 60, 30), &theme, false, false)
        });
        assert_eq!(
            column(&px, w, h, w / 2)[0],
            (theme.border.0, 2),
            "the border at 1.75x, where depths start doubling"
        );
    }

    /// The default button's extra ring is the same black directly inside the
    /// outline, so the border reads as one band two logical pixels deep — and
    /// lands on `depth(2)` exactly, not on twice whatever the first ring
    /// rounded to. It is outline, not bevel, so it stays a chrome unit deep
    /// where the bevel inside it doubles.
    #[test]
    fn a_default_buttons_doubled_border_lands_on_depth_two() {
        let theme = Theme::windows_31();
        for &scale in CHROME_SCALES {
            let (border, bevel) = (depth(2, scale), bevel(2, scale));
            let (w, h) = phys(60, 30, scale);
            let px = chrome_buffer(w, h, scale, |p| {
                p.button(Rect::new(0, 0, 60, 30), &theme, false, true)
            });
            assert_eq!(
                column(&px, w, h, w / 2),
                vec![
                    (theme.border.0, border),
                    (theme.highlight.0, bevel),
                    (theme.face.0, h - 2 * (border + bevel)),
                    (theme.shadow.0, bevel),
                    (theme.border.0, border),
                ],
                "vertical section of a default button at {scale}x"
            );
        }
    }

    /// The `n × n` device pixels of a buffer `w` wide from `(x0, y0)`, a
    /// string a row: `.` where nothing was painted, `#` for an outline, `h`
    /// and `s` for highlight and shadow and `f` for a face.
    fn picture(px: &[u32], w: i32, (x0, y0): (i32, i32), n: i32) -> Vec<String> {
        let theme = Theme::windows_31();
        (y0..y0 + n)
            .map(|y| {
                (x0..x0 + n)
                    .map(|x| match px[(y * w + x) as usize] {
                        0 => '.',
                        c if c == theme.border.0 => '#',
                        c if c == theme.highlight.0 => 'h',
                        c if c == theme.shadow.0 => 's',
                        c if c == theme.face.0 => 'f',
                        _ => '?',
                    })
                    .collect()
            })
            .collect()
    }

    /// The [`picture`] of the `n × n` device pixels in a button's top-left
    /// corner.
    fn corner_art(scale: f32, default: bool, n: i32) -> Vec<String> {
        let theme = Theme::windows_31();
        let (w, h) = phys(60, 30, scale);
        let px = chrome_buffer(w, h, scale, |p| {
            p.button(Rect::new(0, 0, 60, 30), &theme, false, default)
        });
        picture(&px, w, (0, 0), n)
    }

    /// The rows of a [`picture`], written one to a line.
    fn art(rows: &str) -> Vec<String> {
        rows.split_whitespace().map(String::from).collect()
    }

    /// Below 1.75x a button's outline is a device pixel thick and leaves out
    /// just the pixel in each corner, the design's own — and the bevel inside
    /// runs square into the corner, 1-pixel rings or 1.5x ones alike.
    #[test]
    fn below_one_and_three_quarters_a_button_leaves_out_its_corner_pixel() {
        for &scale in CHROME_SCALES.iter().filter(|&&s| s < CHROME_PIN) {
            assert_eq!(
                corner_art(scale, false, 3),
                art("
                    .##
                    #hh
                    #hh
                "),
                "a button's corner at {scale}x"
            );
            assert_eq!(
                corner_art(scale, true, 3),
                art("
                    .##
                    ###
                    ##h
                "),
                "a default button's corner at {scale}x"
            );
        }
    }

    /// From 1.75x the corner is the curve the 1.0x one is a rasterization of,
    /// drawn at the device resolution, and not its missing pixel blown up into
    /// a square notch: the outer arc two depths in radius, the inner one a
    /// depth, so the outline's inner edge rounds off the bevel's corner pixel
    /// too. The default button's border shares the outer arc and keeps its
    /// inner edge square.
    #[test]
    fn from_one_and_three_quarters_a_button_corner_is_drawn_as_a_curve() {
        for scale in [1.75, 2.0, 2.5] {
            assert_eq!(
                corner_art(scale, false, 5),
                art("
                    ..###
                    .####
                    ###hh
                    ##hhh
                    ##hhh
                "),
                "a button's corner at {scale}x"
            );
            assert_eq!(
                corner_art(scale, true, 5),
                art("
                    ..###
                    .####
                    #####
                    #####
                    ####h
                "),
                "a default button's corner at {scale}x"
            );
        }
        assert_eq!(
            corner_art(3.0, false, 7),
            art("
                ....###
                ..#####
                .######
                .###hhh
                ###hhhh
                ###hhhh
                ###hhhh
            "),
            "a button's corner at 3x"
        );
        assert_eq!(
            corner_art(4.0, false, 9),
            art("
                .....####
                ...######
                ..#######
                .########
                .#####hhh
                #####hhhh
                ####hhhhh
                ####hhhhh
                ####hhhhh
            "),
            "a button's corner at 4x"
        );
    }

    /// The four corners of a button are one curve, mirrored: what is left
    /// unpainted and what is outline match corner for corner at every scale,
    /// whichever side of the bevel each one lies on.
    #[test]
    fn a_button_rounds_all_four_corners_alike() {
        let theme = Theme::windows_31();
        for &scale in CHROME_SCALES {
            for default in [false, true] {
                let (w, h) = phys(60, 30, scale);
                let px = chrome_buffer(w, h, scale, |p| {
                    p.button(Rect::new(0, 0, 60, 30), &theme, false, default)
                });
                let n = 3 * chrome_px(scale);
                let outline = |x: i32, y: i32| match px[(y * w + x) as usize] {
                    0 => '.',
                    c if c == theme.border.0 => '#',
                    _ => 'o',
                };
                for y in 0..n {
                    for x in 0..n {
                        let corners = [
                            outline(x, y),
                            outline(w - 1 - x, y),
                            outline(x, h - 1 - y),
                            outline(w - 1 - x, h - 1 - y),
                        ];
                        assert!(
                            corners.iter().all(|&c| c == corners[0]),
                            "pixel ({x}, {y}) of each corner at {scale}x \
                             (default: {default}): {corners:?}"
                        );
                    }
                }
            }
        }
    }

    /// Where a bevel's light side meets its dark one, at the top-right and the
    /// bottom-left, the two share the corner along its diagonal a device pixel
    /// at a time, at every scale. The dark side takes the diagonal itself, so
    /// at 1.0x it takes the single corner pixel, as the design has it; given
    /// the whole corner square, it made the diagonal a staircase with steps as
    /// deep as the ring.
    #[test]
    fn a_bevels_light_and_dark_sides_meet_on_a_diagonal() {
        let theme = Theme::windows_31();
        for &scale in CHROME_SCALES {
            let (w, h) = phys(40, 20, scale);
            let px = chrome_buffer(w, h, scale, |p| {
                p.raised_bevel(Rect::new(0, 0, 40, 20), theme.highlight, theme.shadow)
            });
            for d in 0..depth(1, scale) {
                assert_eq!(
                    row(&px, w, d),
                    vec![(theme.highlight.0, w - 1 - d), (theme.shadow.0, d + 1)],
                    "row {d} of a bevel at {scale}x"
                );
                assert_eq!(
                    column(&px, w, h, d),
                    vec![(theme.highlight.0, h - 1 - d), (theme.shadow.0, d + 1)],
                    "column {d} of a bevel at {scale}x"
                );
            }
        }
    }

    /// A button's top-right corner, where its highlight meets its shadow: one
    /// diagonal, a device pixel a step, through both bevel rings and on under
    /// the rounded outline. The light chrome's deeper shadow ring meets the
    /// face along the same line its first ring meets the highlight on.
    #[test]
    fn a_buttons_bevel_turns_its_corner_on_one_diagonal() {
        let theme = Theme::windows_31();
        let top_right = |scale: f32, n: i32, paint: &dyn Fn(&mut Painter, Rect)| {
            let (w, h) = phys(60, 30, scale);
            let px = chrome_buffer(w, h, scale, |p| paint(p, Rect::new(0, 0, 60, 30)));
            picture(&px, w, (w - n, 0), n)
        };
        let button = |p: &mut Painter, r: Rect| p.button(r, &theme, false, false);
        let light = |p: &mut Painter, r: Rect| p.light_button(r, &theme, false);
        assert_eq!(
            top_right(2.0, 5, &button),
            art("
                ###..
                ####.
                hh###
                hss##
                sss##
            "),
            "a button's top-right corner at 2x"
        );
        assert_eq!(
            top_right(3.0, 7, &button),
            art("
                ###....
                #####..
                ######.
                hhh###.
                hhss###
                hsss###
                ssss###
            "),
            "a button's top-right corner at 3x"
        );
        assert_eq!(
            top_right(2.0, 6, &light),
            art("
                ######
                ######
                hhhs##
                hhss##
                fsss##
                ssss##
            "),
            "a light button's top-right corner at 2x"
        );
    }

    /// Pressed, the bevel inverts: the shadow takes both logical pixels on the
    /// top/left and a single highlight line sits opposite it.
    #[test]
    fn a_pressed_button_inverts_its_bevel() {
        let theme = Theme::windows_31();
        for &scale in CHROME_SCALES {
            let (border, one, two) = (depth(1, scale), bevel(1, scale), bevel(2, scale));
            let (w, h) = phys(60, 30, scale);
            let px = chrome_buffer(w, h, scale, |p| {
                p.button(Rect::new(0, 0, 60, 30), &theme, true, false)
            });
            assert_eq!(
                column(&px, w, h, w / 2),
                vec![
                    (theme.border.0, border),
                    (theme.shadow.0, two),
                    (theme.face.0, h - 2 * border - two - one),
                    (theme.highlight.0, one),
                    (theme.border.0, border),
                ],
                "vertical section of a pressed button at {scale}x"
            );
        }
    }

    /// The scrollbar's lighter chrome: outline, one highlight line, and the
    /// doubled shadow opposite — the band that used to double in width from one
    /// quarter step to the next, and now runs exactly two bevel rings in from
    /// the outline.
    #[test]
    fn a_light_buttons_doubled_shadow_is_two_bevel_rings_deep() {
        let theme = Theme::windows_31();
        for &scale in CHROME_SCALES {
            let (border, one, two) = (depth(1, scale), bevel(1, scale), bevel(2, scale));
            let (w, h) = phys(30, 30, scale);
            let px = chrome_buffer(w, h, scale, |p| {
                p.light_button(Rect::new(0, 0, 30, 30), &theme, false)
            });
            assert_eq!(
                column(&px, w, h, w / 2),
                vec![
                    (theme.border.0, border),
                    (theme.highlight.0, one),
                    (theme.face.0, h - 2 * border - one - two),
                    (theme.shadow.0, two),
                    (theme.border.0, border),
                ],
                "vertical section of a light button at {scale}x"
            );
        }
    }

    /// A scale that is a whole multiple of one below [`CHROME_PIN`] wears that
    /// one's frame the same number of times over — 2.5x is the 1.25x button
    /// doubled, 3x the 1.0x one tripled — so its lines are exactly as fine for
    /// their size. Scaling the depths by the fractional factor instead gave a
    /// 2.5x button a 3-pixel outline over a bevel of a 2-pixel ring and a
    /// 3-pixel one, which looked heavier and blurrier than 1.25x blown up.
    #[test]
    fn a_multiple_of_a_pinned_scale_wears_its_frame_that_many_times_over() {
        let theme = Theme::windows_31();
        let section = |scale: f32| {
            let (w, h) = phys(60, 30, scale);
            let px = chrome_buffer(w, h, scale, |p| {
                p.button(Rect::new(0, 0, 60, 30), &theme, false, false)
            });
            column(&px, w, h, w / 2)
        };
        for (scale, pinned, times) in [
            (2.0, 1.0, 2),
            (7.0 / 3.0, 7.0 / 6.0, 2),
            (2.5, 1.25, 2),
            (8.0 / 3.0, 4.0 / 3.0, 2),
            (2.75, 1.375, 2),
            (3.0, 1.0, 3),
            (3.5, 7.0 / 6.0, 3),
            (3.75, 1.25, 3),
            (4.5, 1.125, 4),
        ] {
            let (big, small) = (section(scale), section(pinned));
            assert_eq!(big.len(), small.len(), "the bands at {scale}x");
            // Outline and bevel on either side; the face between them takes
            // whatever the scale has left over, so it is not a multiple.
            for i in [0, 1, 3, 4] {
                let ((color, deep), (pinned_color, pinned_deep)) = (big[i], small[i]);
                assert_eq!(color, pinned_color, "band {i} at {scale}x");
                assert_eq!(
                    deep,
                    times * pinned_deep,
                    "band {i} at {scale}x, which should be {times} times the {pinned}x one"
                );
            }
        }
    }

    /// A dot, a gap, a dot — every one of them the same size, starting from the
    /// lit corner. What the eye reads here is the rhythm, and dots spaced at
    /// their exact scaled positions would sit 2 and 3 pixels apart at 2.25x.
    #[test]
    fn a_focus_ring_dashes_at_a_uniform_pitch() {
        for &scale in CHROME_SCALES {
            let dot = depth(1, scale);
            let (w, h) = phys(40, 20, scale);
            let px = chrome_buffer(w, h, scale, |p| {
                p.focus_rect(Rect::new(0, 0, 40, 20), Color::BLACK)
            });
            for (edge, bands) in [
                ("top", row(&px, w, 0)),
                ("bottom", row(&px, w, h - 1)),
                ("left", column(&px, w, h, 0)),
                ("right", column(&px, w, h, w - 1)),
            ] {
                // The tail of the run is where a trimmed final dot may sit, so
                // check the first three dots and the gaps between them.
                for (i, &band) in bands.iter().take(5).enumerate() {
                    let ink = if i % 2 == 0 { Color::BLACK.0 } else { 0 };
                    assert_eq!(band, (ink, dot), "dash {i} of the {edge} edge at {scale}x");
                }
            }
        }
    }

    /// The etched divider's two tones each run to their own depth, so neither
    /// takes the other's rounding: one device pixel apiece below 1.75x, and 2
    /// over 2 at 2.5x. Nothing hangs out the bottom onto whatever it divides.
    #[test]
    fn an_etched_divider_splits_its_two_tones_at_their_depths() {
        let theme = Theme::windows_31();
        for &scale in CHROME_SCALES {
            let (w, slot) = phys(40, 2, scale);
            let (first, second) = (depth(1, scale).min(slot), depth(2, scale).min(slot));
            let h = slot + 4;
            let px = chrome_buffer(w, h, scale, |p| p.etched_h_line(0, 0, 40, &theme));
            assert_eq!(
                column(&px, w, h, w / 2),
                vec![
                    (theme.shadow.0, first),
                    (theme.highlight.0, second - first),
                    (0, h - second),
                ],
                "the two tones of an etched line at {scale}x"
            );
        }
    }

    /// A field overlapping a scrollbar-like neighbour by one logical pixel,
    /// its right edge merged: the field's right border line must occupy
    /// exactly the device pixels the neighbour's own left line does — however
    /// the snap's parity falls at the shared position, which is why every
    /// position in a range is tried.
    #[test]
    fn a_merged_right_edge_lands_on_the_neighbours_frame_line() {
        for &scale in CHROME_SCALES {
            let unit = depth(1, scale);
            for nx in 5..25 {
                // Where the neighbour's left border line sits: `depth(1)`
                // device pixels inward from its own snapped left edge.
                let line = (nx as f32 * scale).round() as i32;
                let w = line + unit + 4;
                let h = (8.0 * scale).round() as i32;
                let px = chrome_buffer(w, h, scale, |p| {
                    p.stroke_rect_merged(Rect::new(0, 0, nx + 1, 8), Merged::RIGHT, Color::BLACK)
                });
                // A row through the field's middle, clear of both horizontal
                // border bands.
                let mid = h / 2;
                for x in 0..w {
                    let expected = x < unit || (line..line + unit).contains(&x);
                    let painted = px[(mid * w + x) as usize] == Color::BLACK.0;
                    assert_eq!(
                        painted,
                        expected,
                        "column {x} of a right-merged field ending at {} at {scale}x \
                         (the neighbour's line runs {line}..{})",
                        nx + 1,
                        line + unit
                    );
                }
            }
        }
    }

    /// The other direction of the overlap — the scrollbar thumb parked flush
    /// under an arrow button, reaching one logical pixel up into it: a merged
    /// top edge must land on the button's own bottom line, which runs
    /// `depth(1)` device pixels *up* from the button's snapped bottom edge.
    #[test]
    fn a_merged_top_edge_lands_on_the_neighbours_frame_line() {
        for &scale in CHROME_SCALES {
            let unit = depth(1, scale);
            for ny in 5..25 {
                // The neighbour above ends at `ny`; its bottom line's top row.
                let edge = (ny as f32 * scale).round() as i32;
                let line = edge - unit;
                let h = edge + (10.0 * scale).round() as i32 + 4;
                let w = (8.0 * scale).round() as i32;
                let px = chrome_buffer(w, h, scale, |p| {
                    p.stroke_rect_merged(Rect::new(0, ny - 1, 8, 10), Merged::TOP, Color::BLACK)
                });
                let bottom = ((ny - 1 + 10) as f32 * scale).round() as i32;
                // A column through the frame's middle, clear of both vertical
                // border bands.
                let mid = w / 2;
                for y in 0..h {
                    let expected =
                        (line..line + unit).contains(&y) || (bottom - unit..bottom).contains(&y);
                    let painted = px[(y * w + mid) as usize] == Color::BLACK.0;
                    assert_eq!(
                        painted,
                        expected,
                        "row {y} of a top-merged frame starting at {} at {scale}x \
                         (the neighbour's line runs {line}..{edge})",
                        ny - 1
                    );
                }
            }
        }
    }

    /// At 1.0x — and at any integer scale, where the snap and the depths agree
    /// everywhere — a merged edge re-derives the very coordinate the plain
    /// snap gives, so merging changes nothing.
    #[test]
    fn merged_edges_change_nothing_at_integer_scales() {
        let all = Merged {
            left: true,
            top: true,
            right: true,
            bottom: true,
        };
        for scale in [1.0, 2.0, 3.0] {
            let (w, h) = phys(20, 12, scale);
            let rect = Rect::new(3, 2, 14, 8);
            let plain = chrome_buffer(w, h, scale, |p| p.stroke_rect(rect, Color::BLACK));
            let merged = chrome_buffer(w, h, scale, |p| {
                p.stroke_rect_merged(rect, all, Color::BLACK)
            });
            assert_eq!(plain, merged, "merged vs plain outline at {scale}x");
        }
    }

    /// The frame clip confines a fill to exactly the frame's interior: not a
    /// device pixel of the border line is overwritten and not a device pixel
    /// of the face goes unpainted — where `push_clip(rect.inset(1))` snapped
    /// its boundary a pixel to either side of the line at fractional scales.
    #[test]
    fn a_frame_clip_stops_fills_exactly_at_the_border() {
        let red = Color::rgb(0xFF, 0, 0);
        for &scale in CHROME_SCALES {
            for x in 0..3 {
                let rect = Rect::new(x, 0, 20, 12);
                let unit = depth(1, scale);
                let w = (rect.right() as f32 * scale).round() as i32 + 2;
                let h = (rect.bottom() as f32 * scale).round() as i32 + 2;
                let px = chrome_buffer(w, h, scale, |p| {
                    p.stroke_rect(rect, Color::BLACK);
                    let saved = p.push_clip_frame(rect, 1, Merged::NONE);
                    // A fill far larger than the field: the clip must trim it
                    // to the frame's interior.
                    p.fill_rect(Rect::new(-50, -50, 200, 200), red);
                    p.restore_clip(saved);
                });
                let (x0, y0) = ((rect.x as f32 * scale).round() as i32, 0);
                let (x1, y1) = (
                    (rect.right() as f32 * scale).round() as i32,
                    (rect.bottom() as f32 * scale).round() as i32,
                );
                for py in 0..h {
                    for pxx in 0..w {
                        let inside_frame = pxx >= x0 && pxx < x1 && py >= y0 && py < y1;
                        let on_border = inside_frame
                            && (pxx < x0 + unit
                                || pxx >= x1 - unit
                                || py < y0 + unit
                                || py >= y1 - unit);
                        let expected = if on_border {
                            Color::BLACK.0
                        } else if inside_frame {
                            red.0
                        } else {
                            0
                        };
                        assert_eq!(
                            px[(py * w + pxx) as usize],
                            expected,
                            "pixel ({pxx}, {py}) of a clipped fill at {scale}x, field at x={x}"
                        );
                    }
                }
            }
        }
    }

    /// A widget shallower than the chrome it wears overlaps itself rather than
    /// spilling over its neighbour: a 1-logical-pixel rule at 2.75x is three
    /// device pixels of buffer under an outline two deep from either side, so
    /// the rule fills solid and stops.
    #[test]
    fn a_frame_deeper_than_its_widget_stays_inside_its_bounds() {
        let (w, h) = phys(8, 1, 2.75);
        let px = chrome_buffer(w, h + 4, 2.75, |p| {
            p.stroke_rect(Rect::new(0, 0, 8, 1), Color::BLACK)
        });
        assert_eq!(
            column(&px, w, h + 4, w / 2),
            vec![(Color::BLACK.0, h), (0, 4)],
            "the outline fills the rule and stops at its bottom edge"
        );
    }

    /// Anti-aliasing onto a see-through pixel — what a transparent popup window
    /// is cleared to — keeps the colour drawn at the coverage it was drawn
    /// with, rather than darkening it toward the black of `Color::TRANSPARENT`.
    /// Onto an opaque pixel it is the plain blend it always was.
    #[test]
    fn blending_onto_a_transparent_pixel_keeps_the_colour() {
        let mut px = [Color::TRANSPARENT.0, Color::WHITE.0];
        {
            let mut p = Painter::new(&mut px, 2, 1, 1.0, 0, 0, FontSet::default());
            p.blend_pixel_phys(0, 0, Color::rgb(0xFF, 0x80, 0x00), 0x80);
            p.blend_pixel_phys(1, 0, Color::BLACK, 0x80);
        }
        assert_eq!(px, [0x80FF_8000, 0xFF7F_7F7F]);
    }

    /// Premultiplying leaves an opaque pixel alone, turns a fully transparent
    /// one into nothing whatever colour it carried, and scales a partial one's
    /// colour by its coverage.
    #[cfg(any(windows, all(unix, not(target_os = "macos"))))]
    #[test]
    fn premultiplying_scales_colour_by_coverage() {
        let mut px = [0xFF12_3456, 0x00FF_FFFF, 0x80FF_8000, 0x0000_0000];
        premultiply(&mut px);
        assert_eq!(px, [0xFF12_3456, 0x0000_0000, 0x8080_4000, 0x0000_0000]);
    }
}
